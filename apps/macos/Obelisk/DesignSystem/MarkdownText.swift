import SwiftUI

/// Markdown laid out block by block, as GitHub shows release notes: headings,
/// paragraphs, lists, quotes, code and rules. Text shows each line's bold,
/// italics, code and links itself; Foundation's parser makes bare URLs links.
/// Shown `.equatable()`, it parses again only when its Markdown changes.
struct MarkdownText: View, Equatable {
    let markdown: String

    init(_ markdown: String) {
        self.markdown = markdown
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(MarkdownBlock.blocks(of: markdown)) { block in
                view(of: block)
                    .padding(.top, block.kind.isHeading && block.id > 0 ? 6 : 0)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .textSelection(.enabled)
    }

    @ViewBuilder private func view(of block: MarkdownBlock) -> some View {
        switch block.kind {
        case .heading(let level):
            Text(block.text)
                .font(.system(size: level <= 1 ? 16 : level == 2 ? 14 : 13, weight: .bold))
                .accessibilityAddTraits(.isHeader)
        case .paragraph:
            lines(block.text)
        case .item(let ordinal, let depth):
            HStack(alignment: .firstTextBaseline, spacing: 7) {
                Text(ordinal.map { "\($0)." } ?? "•")
                    .font(.system(size: 13).monospacedDigit())
                    .foregroundStyle(.secondary)
                    .accessibilityHidden(true)
                lines(block.text)
            }
            .padding(.leading, CGFloat(depth - 1) * 16)
        case .quote:
            lines(block.text)
                .foregroundStyle(.secondary)
                .padding(.leading, 10)
                .overlay(alignment: .leading) {
                    Capsule().fill(.quaternary).frame(width: 3)
                }
        case .code:
            Text(block.text)
                .font(.system(size: 12, design: .monospaced))
                .padding(10)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(.quinary, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        case .rule:
            Divider().padding(.vertical, 4)
        }
    }

    private func lines(_ text: AttributedString) -> some View {
        Text(text)
            .font(.system(size: 13))
            .fixedSize(horizontal: false, vertical: true)
    }
}

/// One block of Markdown and its text, inline styles and links kept.
struct MarkdownBlock: Identifiable, Equatable {
    enum Kind: Equatable {
        case heading(level: Int)
        case paragraph
        /// An item of a list nested `depth` lists deep; `ordinal` numbers an
        /// ordered list's.
        case item(ordinal: Int?, depth: Int)
        case quote
        case code
        case rule

        var isHeading: Bool {
            if case .heading = self { true } else { false }
        }
    }

    let id: Int
    let kind: Kind
    let text: AttributedString

    /// The blocks of `markdown`, in order. Text Foundation cannot parse shows
    /// as one paragraph, as it is.
    static func blocks(of markdown: String) -> [MarkdownBlock] {
        let options = AttributedString.MarkdownParsingOptions(
            interpretedSyntax: .full, failurePolicy: .returnPartiallyParsedIfPossible)
        guard let parsed = try? AttributedString(markdown: markdown, options: options) else {
            return [MarkdownBlock(id: 0, kind: .paragraph, text: AttributedString(markdown))]
        }
        // Every block has an intent of its own, so each run is one block.
        return parsed.runs[\.presentationIntent].enumerated().map { index, run in
            let (intent, range) = run
            var text = AttributedString(parsed[range])
            text.presentationIntent = nil
            let kind = kind(of: intent)
            // A code block keeps its last line's break.
            if kind == .code, text.characters.last == "\n" {
                text.characters.removeLast()
            }
            return MarkdownBlock(id: index, kind: kind, text: text)
        }
    }

    /// What a block is, from its intent's components, innermost first.
    private static func kind(of intent: PresentationIntent?) -> Kind {
        guard let intent else { return .paragraph }
        var item: Int?
        var ordered = false
        var depth = 0
        var quote = false
        for component in intent.components {
            switch component.kind {
            case .header(let level): return .heading(level: level)
            case .codeBlock: return .code
            case .thematicBreak: return .rule
            case .listItem(let ordinal) where item == nil: item = ordinal
            case .orderedList:
                if depth == 0 { ordered = true }
                depth += 1
            case .unorderedList: depth += 1
            case .blockQuote: quote = true
            default: break
            }
        }
        if let item { return .item(ordinal: ordered ? item : nil, depth: max(depth, 1)) }
        return quote ? .quote : .paragraph
    }
}
