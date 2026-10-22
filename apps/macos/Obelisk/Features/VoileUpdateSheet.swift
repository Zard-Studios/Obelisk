import SwiftUI

/// A newer Voile release a launch offers before its game starts: what each
/// release brings over the one in place, then the player's answer. Its
/// download shows on the game's button once taken. The cross stops the
/// launch; a release the game needs cannot be skipped.
struct VoileUpdateSheet: View {
    let offer: VoileUpdateOffer
    let answer: (VoileUpdateAnswer) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            SheetHeader(title: L10n.string("ui.voile_update.title"),
                        subtitle: offer.required ? L10n.string("ui.voile_update.required", offer.version)
                            : L10n.string("ui.voile_update.message", offer.version),
                        symbol: "sailboat.fill", colors: TileColors.blue,
                        close: { answer(.cancel) }, closeIdentifier: "voile-update-cancel")
            SheetSection(title: L10n.string("ui.voile_update.changes")) {
                ScrollView {
                    notes.padding(16)
                }
                .scrollBounceBehavior(.basedOnSize)
                // As tall as the notes, up to a scrolling limit.
                .frame(maxHeight: 340)
                .fixedSize(horizontal: false, vertical: true)
                .modifier(GamePageCard(padding: 0))
            }
            HStack(spacing: 10) {
                Spacer(minLength: 0)
                if !offer.required {
                    GlassCapsuleButton(title: L10n.string("ui.voile_update.skip"), size: .large,
                                       identifier: "voile-update-skip") { answer(.skip) }
                }
                AccentCapsuleButton(title: L10n.string("ui.voile_update.update"), size: .large,
                                    identifier: "voile-update-update") { answer(.update) }
                    .keyboardShortcut(.defaultAction)
            }
        }
        .padding(24)
        .frame(width: 520)
        .tint(ObeliskStyle.accent)
        .interactiveDismissDisabled()
        .accessibilityIdentifier("voile-update-prompt")
    }

    /// Each release's notes, newest first, under its version when there are
    /// several.
    @ViewBuilder private var notes: some View {
        let releases = offer.releases.filter { !$0.notes.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
        if releases.isEmpty {
            Text(L10n.string("ui.voile_update.no_notes"))
                .font(.system(size: 13))
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, alignment: .leading)
        } else {
            VStack(alignment: .leading, spacing: 20) {
                ForEach(releases, id: \.version) { release in
                    VStack(alignment: .leading, spacing: 8) {
                        if releases.count > 1 {
                            Text(L10n.string("ui.voile_update.release", release.version))
                                .font(.system(size: 12, weight: .semibold))
                                .foregroundStyle(.secondary)
                        }
                        MarkdownText(release.notes).equatable()
                    }
                }
            }
        }
    }
}
