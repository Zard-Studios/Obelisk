import AppKit
import SwiftUI
import XCTest
@testable import Obelisk

final class SteamAchievementsTests: XCTestCase {
    @MainActor
    func testResetAndReplacementDiscardLateAccountOrGameResponses() async throws {
        let old = AchievementResponse()
        let model = GameAchievementsModel(storeID: "steam", describe: SteamModel.message)
        model.load { await old.get() }
        await old.waitUntilRequested()
        model.reset()
        model.load { [Self.achievement("new", unlocked: true)] }
        try await eventually { model.items != nil }
        await old.resolve([Self.achievement("old", unlocked: false)])
        await Task.yield()
        XCTAssertEqual(model.items?.map(\.id), ["new"])
        XCTAssertEqual(model.countText, "1/1")
        XCTAssertEqual(model.fraction, 1)
        model.isPresented = true
        model.reset()
        XCTAssertNil(model.items)
        XCTAssertNil(model.fraction)
        XCTAssertFalse(model.isPresented)
    }

    @MainActor
    func testUnavailableIsDistinctFromZeroAchievementsAndRetryLoadsProgress() async throws {
        let model = GameAchievementsModel(storeID: "steam", describe: SteamModel.message)
        model.load { throw CocoaError(.fileReadNoPermission) }
        try await eventually { !model.isLoading }
        XCTAssertNotNil(model.error)
        XCTAssertNil(model.items)
        XCTAssertNil(model.fraction)
        XCTAssertEqual(model.countText, "—")
        model.load { [] }
        try await eventually { !model.isLoading }
        XCTAssertNil(model.error)
        XCTAssertEqual(model.countText, "0/0")
        XCTAssertNil(model.fraction)
        model.load { [Self.achievement("yes", unlocked: true), Self.achievement("no", unlocked: false)] }
        try await eventually { !model.isLoading }
        XCTAssertEqual(model.countText, "1/2")
        XCTAssertEqual(model.fraction, 0.5)
    }

    @MainActor
    func testRenderAchievementHeaderAndPanel() async throws {
        guard let path = ProcessInfo.processInfo.environment["OBELISK_RENDER_PREVIEWS"] else {
            throw XCTSkip("Set OBELISK_RENDER_PREVIEWS to render achievement fixtures")
        }
        let output = URL(filePath: path)
        try FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)
        let model = GameAchievementsModel(storeID: "steam", describe: SteamModel.message)
        model.load { (0..<52).map { Self.achievement("\($0)", unlocked: $0 < 13) } }
        try await eventually { !model.isLoading }
        let cloud = CloudIndicatorButton(
            presentation: CloudIndicatorPresentation(symbol: "icloud", value: "Synchronisé · il y a 2 h", tone: .normal),
            canSync: true, sync: {})
        let installed = GameMetrics(isInstalled: true, size: "1,2 Go", lastPlayed: "Aujourd’hui",
                                    playtime: "27,2 heures", cloud: cloud, achievements: model)
        let uninstalled = GameMetrics(isInstalled: false, size: "34,22 Go", lastPlayed: "7 déc. 2024",
                                      playtime: "6 minutes",
                                      achievements: GameAchievementsModel(storeID: "steam", describe: SteamModel.message))
        for scheme in [ColorScheme.dark, .light] {
            for width in [CGFloat(1100), CGFloat(860), CGFloat(740), CGFloat(620)] {
                for (name, metrics) in [("installed", installed), ("uninstalled", uninstalled)] {
                    let launches = metrics.isInstalled
                    // The game page's launch row and metrics, at the widths its header gets.
                    let view = GameLaunchRow(metrics: metrics) {
                        GameActionButton(action: GameAction(
                            kind: launches ? .launch : .download(.install),
                            title: L10n.string(launches ? "model.game.launch" : "model.game.download"),
                            isAvailable: true, launches: launches
                        ), gameName: "Thronefall")
                        if launches {
                            GameManagementButton(gameName: "Thronefall", management: GameManagement(
                                directory: "/Applications", offlineVolume: nil, canVerify: true, canRemove: true,
                                verify: {}, remove: {}))
                        }
                    }
                        .padding(24).frame(width: width).environment(\.colorScheme, scheme)
                        .tint(ObeliskStyle.accent).background(ObeliskPalette(colorScheme: scheme).background)
                    try await render(view, to: output.appending(path: "header-\(name)-\(scheme)-\(Int(width)).png"))
                }
            }
        }
        let downloading = GameTransfer(game: "620", state: .downloading, completedBytes: 200_000_000,
                                       totalBytes: 522_734_781, networkBytes: 0, contentBytes: 0, currentFile: "", message: "")
        let buttons = VStack(alignment: .leading, spacing: 12) {
            GameActionButton(action: GameAction(kind: .running(.quit), title: GameQuitStage.quit.title,
                                                isAvailable: true, launches: true), gameName: "Thronefall")
            GameActionButton(action: GameAction(kind: .running(.closing), title: GameQuitStage.closing.title,
                                                isAvailable: false, launches: true), gameName: "Thronefall")
            GameActionButton(action: GameAction(kind: .running(.forceQuit), title: GameQuitStage.forceQuit.title,
                                                isAvailable: true, launches: true), gameName: "Thronefall")
            GameActionButton(action: GameAction(kind: .transferring(.files),
                                                title: L10n.string("ui.transfer.preparing_short"),
                                                isAvailable: true, launches: false, isCancellable: true,
                                                transfer: { downloading }), gameName: "Thronefall")
            GameActionButton(action: GameAction(kind: .step(.cloudSync), title: L10n.string("ui.cloud.sync_short"),
                                                isAvailable: true, launches: true, isCancellable: true),
                             gameName: "Thronefall")
            GameActionButton(action: GameAction(kind: .step(.runtime(.voile)),
                                                title: L10n.string("model.game.installingVoile"),
                                                isAvailable: true, launches: true, isCancellable: true,
                                                stepFraction: { 0.4 }), gameName: "Thronefall")
        }
        try await render(buttons.padding(24).foregroundStyle(.white).environment(\.colorScheme, .dark).background(.black),
                         to: output.appending(path: "header-buttons.png"))
        try await render(GameAchievementsView(model: model, gameName: "Thronefall", retry: {})
            .environment(\.colorScheme, .dark).tint(ObeliskStyle.accent)
            .background(ObeliskPalette(colorScheme: .dark).background), to: output.appending(path: "achievements-panel.png"))
        model.load { [] }
        try await eventually { !model.isLoading }
        try await render(GameAchievementsView(model: model, gameName: "Thronefall", retry: {})
            .environment(\.colorScheme, .dark).tint(ObeliskStyle.accent)
            .background(ObeliskPalette(colorScheme: .dark).background), to: output.appending(path: "achievements-panel-empty.png"))
    }

    private static func achievement(_ id: String, unlocked: Bool) -> GameAchievement {
        GameAchievement(id: id, name: unlocked ? "Une première victoire" : "Un nouveau défi", description: "Terminer une partie et défendre le royaume.", iconUrl: "", hidden: false, unlocked: unlocked, unlockDate: unlocked ? Date(timeIntervalSince1970: 1_790_000_000) : nil)
    }
    @MainActor
    private func eventually(_ condition: () -> Bool) async throws {
        for _ in 0..<200 {
            if condition() { return }
            try await Task.sleep(for: .milliseconds(5))
        }
        XCTFail("Achievement request did not complete")
    }
    @MainActor
    private func render(_ view: some View, to url: URL) async throws {
        let host = NSHostingView(rootView: view)
        let size = host.fittingSize
        let rect = NSRect(x: -20_000, y: -20_000, width: size.width, height: size.height)
        let window = NSWindow(contentRect: rect, styleMask: .borderless, backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.sharingType = .none
        window.contentView = host
        window.setFrameOrigin(rect.origin)
        window.orderBack(nil)
        defer { window.close() }
        host.layoutSubtreeIfNeeded()
        try await Task.sleep(for: .milliseconds(200))
        host.layoutSubtreeIfNeeded()
        window.displayIfNeeded()
        let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds))
        host.cacheDisplay(in: host.bounds, to: bitmap)
        try XCTUnwrap(bitmap.representation(using: .png, properties: [:])).write(to: url)
    }

}

private actor AchievementResponse {
    private var continuation: CheckedContinuation<[GameAchievement], Never>?
    func get() async -> [GameAchievement] { await withCheckedContinuation { continuation = $0 } }
    func waitUntilRequested() async {
        while continuation == nil { await Task.yield() }
    }
    func resolve(_ value: [GameAchievement]) { continuation?.resume(returning: value); continuation = nil }
}
