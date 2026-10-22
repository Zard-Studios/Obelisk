import Foundation
import XCTest
@testable import Obelisk

final class MarkdownTextTests: XCTestCase {
    func testReleaseNotesSplitIntoTheBlocksGitHubShows() {
        let blocks = MarkdownBlock.blocks(of: """
            Voile gets **faster**.

            ### Fixes

            - Fewer crashes
            - Sound

            1. First
            2. Second
               - Nested

            > Quoted

            ```
            let x = 1
            ```

            ---

            **Full Changelog**: https://github.com/Zagrios/Voile/compare/v1...v2
            """)

        XCTAssertEqual(blocks.map(\.kind), [
            .paragraph,
            .heading(level: 3),
            .item(ordinal: nil, depth: 1),
            .item(ordinal: nil, depth: 1),
            .item(ordinal: 1, depth: 1),
            .item(ordinal: 2, depth: 1),
            .item(ordinal: nil, depth: 2),
            .quote,
            .code,
            .rule,
            .paragraph,
        ])
        XCTAssertEqual(blocks.map { String($0.text.characters) }, [
            "Voile gets faster.", "Fixes", "Fewer crashes", "Sound", "First", "Second", "Nested", "Quoted",
            "let x = 1", "⸻", "Full Changelog: https://github.com/Zagrios/Voile/compare/v1...v2",
        ])
        XCTAssertEqual(blocks[0].text.runs.first { $0.inlinePresentationIntent == .stronglyEmphasized }
                        .map { String(blocks[0].text[$0.range].characters) }, "faster")
        XCTAssertEqual(blocks.last?.text.runs.compactMap(\.link),
                       [URL(string: "https://github.com/Zagrios/Voile/compare/v1...v2")!],
                       "A bare URL is a link")
    }

    func testEmptyNotesHaveNoBlock() {
        XCTAssertEqual(MarkdownBlock.blocks(of: ""), [])
    }
}
