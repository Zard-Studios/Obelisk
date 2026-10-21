import AppKit
import XCTest
@testable import Obelisk

/// A games folder the test publishes, which records the player's changes.
@MainActor
final class FakeGamesFolder: GamesFolderBackend {
    enum Call: Equatable {
        case choose(String), chooseDefault, add(String), prefer(String), remove(String), gather, volumesChanged
    }

    private(set) var calls: [Call] = []
    var inspection: GamesFolderInspection = .missing
    var failure: Error?
    private var current: GamesFolderState
    private var waiters: [CheckedContinuation<GamesFolderState, Never>] = []

    init(path: String = "/library/Games") {
        current = GamesFolderState(revision: 1, path: path, isDefault: true, isConnected: true,
                                   locations: [GamesLocation(path: path, isDefault: true, isTarget: true,
                                                             isConnected: true)])
    }

    func publish(path: String, isDefault: Bool, isConnected: Bool = true, locations: [GamesLocation]? = nil) {
        current = GamesFolderState(
            revision: current.revision + 1, path: path, isDefault: isDefault, isConnected: isConnected,
            locations: locations ?? [GamesLocation(path: path, isDefault: isDefault, isTarget: true,
                                                   isConnected: isConnected)])
        let waiting = waiters
        waiters = []
        for waiter in waiting { waiter.resume(returning: current) }
    }

    func state() -> GamesFolderState { current }

    func changed(after: UInt64) async -> GamesFolderState {
        if current.revision > after { return current }
        return await withCheckedContinuation { waiters.append($0) }
    }

    func inspect(_ folder: String) -> GamesFolderInspection { inspection }
    func inspectDefault() -> GamesFolderInspection { inspection }

    func choose(_ folder: String) throws {
        calls.append(.choose(folder))
        if let failure { throw failure }
    }

    func chooseDefault() throws { calls.append(.chooseDefault) }
    func add(_ folder: String) throws -> String {
        calls.append(.add(folder))
        if let failure { throw failure }
        return folder
    }
    func prefer(_ location: String) throws { calls.append(.prefer(location)) }
    func remove(_ location: String) throws {
        calls.append(.remove(location))
        if let failure { throw failure }
    }
    func gather() { calls.append(.gather) }
    func volumesChanged() { calls.append(.volumesChanged) }
}

@MainActor
final class GamesFolderModelTests: XCTestCase {
    private func model(_ backend: FakeGamesFolder, center: NotificationCenter = NotificationCenter()) -> GamesFolderModel {
        GamesFolderModel(library: FileManager.default.temporaryDirectory, backend: backend, workspace: center)
    }

    func testAPickedFolderShowsWhatMovesThereThenTheGamesMoveOnceThePlayerAgrees() async throws {
        let backend = FakeGamesFolder()
        let model = model(backend)
        XCTAssertTrue(model.isDefault)
        let picked = FileManager.default.temporaryDirectory
        backend.inspection = .ready(path: picked.appending(path: "Obelisk Games").path, games: 3,
                                    copiedBytes: 1_000)

        model.propose(picked)
        let proposal = try XCTUnwrap(model.proposal)
        XCTAssertEqual(proposal.games, 3)
        XCTAssertEqual(proposal.copiedBytes, 1_000)
        XCTAssertFalse(proposal.volume.isEmpty)
        XCTAssertTrue(proposal.canChoose, "the Mac's own disk holds a kilobyte")
        XCTAssertTrue(backend.calls.isEmpty, "nothing moves before the player agrees")

        model.confirm()
        XCTAssertNil(model.proposal)
        XCTAssertEqual(backend.calls, [.choose(picked.path)])
        backend.publish(path: proposal.path, isDefault: false)
        try await eventually { model.path == proposal.path }
        XCTAssertFalse(model.isDefault)
        XCTAssertTrue(model.holds(proposal.path + "/Steam/steam-620-macos"))
        XCTAssertFalse(model.holds(proposal.path + " 2/Steam/steam-620-macos"))
    }

    func testAFolderThatCannotHoldTheGamesSaysWhyAndAsksNothing() {
        let backend = FakeGamesFolder()
        let model = model(backend)
        backend.inspection = .inside
        model.propose(FileManager.default.temporaryDirectory)
        XCTAssertNil(model.proposal)
        XCTAssertNotNil(model.failure)
    }

    func testAFailedChoiceIsShown() {
        let backend = FakeGamesFolder()
        let model = model(backend)
        backend.inspection = .ready(path: "/Volumes/Disk/Obelisk Games", games: 0, copiedBytes: 0)
        backend.failure = GameSessionError.Failure(reason: "model.gamesFolder.refused")
        model.propose(FileManager.default.temporaryDirectory)
        model.confirm()
        XCTAssertEqual(model.failure, L10n.string("model.gamesFolder.failed", L10n.string("model.gamesFolder.refused")))
    }

    func testADiskWithoutRoomOrWritingKeepsTheGamesWhereTheyAre() {
        let backend = FakeGamesFolder()
        let model = model(backend)
        let full = GamesFolderProposal(chosen: URL(filePath: "/Volumes/Disk"), path: "/Volumes/Disk/Obelisk Games",
                                       games: 2, copiedBytes: 10, volume: "Disk", availableBytes: 9, format: nil,
                                       isReadOnly: false)
        XCTAssertTrue(full.lacksSpace)
        model.proposal = full
        model.confirm()
        XCTAssertNotNil(model.proposal)
        let readOnly = GamesFolderProposal(chosen: URL(filePath: "/Volumes/Disk"), path: "/Volumes/Disk/Obelisk Games",
                                           games: 2, copiedBytes: 1, volume: "Disk", availableBytes: 9,
                                           format: "NTFS", isReadOnly: true)
        XCTAssertFalse(readOnly.canChoose)
        XCTAssertTrue(backend.calls.isEmpty)
    }

    func testMovingBackToThisMacChoosesObelisksOwnFolder() {
        let backend = FakeGamesFolder()
        let model = model(backend)
        backend.inspection = .ready(path: "/library/Games", games: 1, copiedBytes: 5)
        model.proposeDefault()
        XCTAssertNil(model.proposal?.chosen)
        model.confirm()
        XCTAssertEqual(backend.calls, [.chooseDefault])
    }

    func testADiskThatComesOrLeavesShowsItsGamesAgainOrHidesThem() async throws {
        let backend = FakeGamesFolder()
        let center = NotificationCenter()
        let model = model(backend, center: center)
        center.post(name: NSWorkspace.didMountNotification, object: nil)
        center.post(name: NSWorkspace.didUnmountNotification, object: nil)
        try await eventually { backend.calls.count == 2 }
        XCTAssertEqual(backend.calls, [.volumesChanged, .volumesChanged])
        backend.publish(path: "/Volumes/Gone/Obelisk Games", isDefault: false, isConnected: false)
        try await eventually { !model.isConnected }
        model.gather()
        XCTAssertEqual(backend.calls.last, .gather)
    }

    func testAGameInstallsAtOnceWhenOneConnectedFolderHoldsGamesElseThePlayerPicks() async throws {
        let backend = FakeGamesFolder()
        let model = model(backend)
        var installed: [String?] = []
        model.place("Portal 2") { installed.append($0) }
        XCTAssertEqual(installed, [nil])
        XCTAssertNil(model.placement)

        let own = GamesLocation(path: "/library/Games", isDefault: true, isTarget: false, isConnected: true)
        let disk = GamesLocation(path: "/Volumes/SSD/Obelisk Games", isDefault: false, isTarget: true, isConnected: true)
        let gone = GamesLocation(path: "/Volumes/Gone/Obelisk Games", isDefault: false, isTarget: false,
                                 isConnected: false)
        backend.publish(path: disk.path, isDefault: false, locations: [own, disk, gone])
        try await eventually { model.locations.count == 3 }

        model.place("Portal 2") { installed.append($0) }
        let placement = try XCTUnwrap(model.placement)
        XCTAssertEqual(placement.gameName, "Portal 2")
        XCTAssertEqual(placement.locations.map(\.path), [disk.path, own.path], "the games folder first, no lost disk")
        placement.install(own.path)
        XCTAssertEqual(installed, [nil, own.path])

        XCTAssertTrue(model.holds("/library/Games/Steam/steam-620-macos"))
        XCTAssertTrue(model.holds("/Volumes/Gone/Obelisk Games/Steam/steam-620-macos"))
        XCTAssertFalse(model.holds("/elsewhere/Steam/steam-620-macos"))
        XCTAssertEqual(model.destinations(from: "/library/Games/Steam/steam-620-macos").map(\.path), [disk.path])
    }

    func testAddingPreferringAndRemovingAFolderReachRustAndAFailureShows() {
        let backend = FakeGamesFolder()
        let model = model(backend)
        let picked = URL(filePath: "/Volumes/SSD/Jeux", directoryHint: .isDirectory)
        model.add(picked)
        let added = GamesLocation(path: "/Volumes/SSD/Jeux/Obelisk Games", isDefault: false, isTarget: false,
                                  isConnected: true)
        model.prefer(added)
        backend.failure = GameSessionError.Failure(reason: "model.gamesFolder.occupied")
        model.remove(added)
        XCTAssertEqual(backend.calls, [.add(picked.path), .prefer(added.path), .remove(added.path)])
        XCTAssertNotNil(model.failure)
    }

}
