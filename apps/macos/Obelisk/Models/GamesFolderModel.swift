import AppKit
import Foundation
import Observation

/// Obelisk's games folder, as Rust keeps it. Each change returns at once;
/// the moves it queues show in the download queue.
@MainActor
protocol GamesFolderBackend: AnyObject {
    func state() -> GamesFolderState
    /// Waits for a revision newer than `after`.
    func changed(after: UInt64) async -> GamesFolderState
    /// What choosing `folder` would do. It reads the disk.
    func inspect(_ folder: String) -> GamesFolderInspection
    /// What choosing Obelisk's own folder again would do. It makes that folder.
    func inspectDefault() -> GamesFolderInspection
    func choose(_ folder: String) throws
    func chooseDefault() throws
    /// Adds the games folder `folder` gives to those that hold games, moving
    /// nothing, and returns its path.
    func add(_ folder: String) throws -> String
    /// Makes `location`, one of the folders that hold games, the one new
    /// games go to, moving nothing.
    func prefer(_ location: String) throws
    /// Takes `location`, a folder the player added, out of those that hold
    /// games, once no game is left there.
    func remove(_ location: String) throws
    /// Queues the move of the games outside every folder that holds games.
    func gather()
    /// A disk came or left.
    func volumesChanged()
    /// Whether a game of the disk mounted at `volume` runs, or a Transfer,
    /// move or uninstall works there.
    func isBusy(on volume: String) -> Bool
    /// The disk mounted at `volume` is about to leave: the Transfers and
    /// moves on it stop.
    func volumeLeaving(_ volume: String)
}

/// The games folder every account's installations of the library share in Rust.
final class RustGamesFolder: GamesFolderBackend {
    private let core: GamesFolder

    init(library: URL) {
        core = GamesFolder(library: library.path)
    }

    func state() -> GamesFolderState { core.state() }
    func changed(after: UInt64) async -> GamesFolderState { await core.changed(after: after) }
    func inspect(_ folder: String) -> GamesFolderInspection { core.inspect(chosen: folder) }
    func inspectDefault() -> GamesFolderInspection { core.inspectDefault() }
    func choose(_ folder: String) throws { try core.choose(chosen: folder) }
    func chooseDefault() throws { try core.chooseDefault() }
    func add(_ folder: String) throws -> String { try core.add(chosen: folder) }
    func prefer(_ location: String) throws { try core.prefer(location: location) }
    func remove(_ location: String) throws { try core.remove(location: location) }
    func gather() { core.gather() }
    func volumesChanged() { core.volumesChanged() }
    func isBusy(on volume: String) -> Bool { core.busyOn(volume: volume) }
    func volumeLeaving(_ volume: String) { core.volumeLeaving(volume: volume) }
}

/// A folder the player picked for the games, and what choosing it does,
/// before they agree.
struct GamesFolderProposal: Identifiable, Equatable {
    let id = UUID()
    /// The folder the player picked; nil for Obelisk's own on this Mac.
    let chosen: URL?
    /// Where the games go: Obelisk's folder inside the picked one.
    let path: String
    /// The installed games that move there.
    let games: Int
    /// What those of another disk copy there.
    let copiedBytes: UInt64
    /// The disk's name, as Finder shows it.
    let volume: String
    let availableBytes: UInt64?
    /// The disk's format when games may not all run from it, such as ExFAT.
    let format: String?
    let isReadOnly: Bool

    /// Whether the copies outgrow the disk's free space.
    var lacksSpace: Bool { availableBytes.map { $0 < copiedBytes } ?? false }
    var canChoose: Bool { !isReadOnly && !lacksSpace }

    /// The disk facts of `folder`, which it reads.
    static func volume(of folder: URL) -> (name: String, available: UInt64?, format: String?, readOnly: Bool) {
        let keys: Set<URLResourceKey> = [
            .volumeLocalizedNameKey, .volumeAvailableCapacityForImportantUsageKey, .volumeAvailableCapacityKey,
            .volumeIsReadOnlyKey, .volumeTypeNameKey, .volumeLocalizedFormatDescriptionKey,
        ]
        let values = try? folder.resourceValues(forKeys: keys)
        // An external disk answers zero for important usage: only the Mac's
        // own counts what it can purge.
        let important = values?.volumeAvailableCapacityForImportantUsage.map { UInt64(max(0, $0)) }
        let plain = values?.volumeAvailableCapacity.map { UInt64(max(0, $0)) }
        let available = [important, plain].compactMap { $0 }.max()
        // Only Mac formats keep everything a game holds, such as its links.
        let type = values?.volumeTypeName?.lowercased()
        let recommended = type.map { ["apfs", "hfs"].contains($0) } ?? true
        return (
            values?.volumeLocalizedName ?? folder.lastPathComponent,
            available,
            recommended ? nil : (values?.volumeLocalizedFormatDescription ?? values?.volumeTypeName),
            values?.volumeIsReadOnly ?? false
        )
    }
}

extension GamesLocation: Identifiable {
    public var id: String { path }

    /// How the player names it: this Mac for Obelisk's own folder, else its
    /// disk, as Finder shows it.
    var title: String {
        if isDefault { return L10n.string("ui.games_folder.this_mac") }
        return GamesFolderProposal.volume(of: existing).name
    }

    /// The space left on its disk, when it is connected.
    var availableBytes: UInt64? {
        isConnected ? GamesFolderProposal.volume(of: existing).available : nil
    }

    /// The folder, or its nearest parent there: Obelisk's own is only made
    /// once a game needs it.
    private var existing: URL {
        var url = URL(filePath: path, directoryHint: .isDirectory)
        while !FileManager.default.fileExists(atPath: url.path), url.pathComponents.count > 1 {
            url.deleteLastPathComponent()
        }
        return url
    }
}

/// A game waiting for the player to pick the folder it installs in.
struct InstallPlacement: Identifiable {
    let id = UUID()
    let gameName: String
    /// The connected folders that hold games, the games folder first.
    let locations: [GamesLocation]
    let install: @MainActor (String) -> Void
}

/// Where Obelisk installs games, and the player's change of it: the folder
/// they pick, what moving the games there does, then the moves, which run in
/// the download queue. Other folders the player adds hold games too.
@MainActor @Observable
final class GamesFolderModel {
    private(set) var path: String
    /// Obelisk's own folder, inside its library on this Mac.
    private(set) var isDefault: Bool
    /// Whether the folder's disk is there: Obelisk's own folder always is.
    private(set) var isConnected: Bool
    /// Every folder that holds games: Obelisk's own, then the one new games
    /// go to, then those the player added.
    private(set) var locations: [GamesLocation]
    /// A game waiting for the player to pick the folder it installs in.
    var placement: InstallPlacement?
    /// The folder the player picked, waiting for them to agree.
    var proposal: GamesFolderProposal?
    /// Why the last folder the player picked was not taken.
    var failure: String?
    /// What the player should know of a folder just added, such as its
    /// disk's format.
    var notice: String?

    @ObservationIgnored private let backend: any GamesFolderBackend
    @ObservationIgnored private var revision: UInt64 = 0
    @ObservationIgnored private var observation: Task<Void, Never>?
    @ObservationIgnored private var volumes: [NSObjectProtocol] = []
    @ObservationIgnored private let center: NotificationCenter

    init(library: URL, backend: (any GamesFolderBackend)? = nil, workspace: NotificationCenter? = nil) {
        let backend = backend ?? RustGamesFolder(library: library)
        self.backend = backend
        let center = workspace ?? NSWorkspace.shared.notificationCenter
        self.center = center
        let state = backend.state()
        path = state.path
        isDefault = state.isDefault
        isConnected = state.isConnected
        locations = state.locations
        revision = state.revision
        observation = Task { [weak self, backend] in
            var after = self?.revision ?? 0
            while !Task.isCancelled {
                let state = await backend.changed(after: after)
                guard let self else { return }
                after = state.revision
                apply(state)
            }
        }
        // A game on a disk that left shows as not installed until it comes back.
        volumes = [NSWorkspace.didMountNotification, NSWorkspace.didUnmountNotification,
                   NSWorkspace.didRenameVolumeNotification].map { name in
            center.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.volumesChanged() }
            }
        }
        // A disk about to leave lets go of the files Obelisk writes there.
        volumes.append(center.addObserver(forName: NSWorkspace.willUnmountNotification, object: nil,
                                          queue: .main) { [weak self] notification in
            let volume = notification.userInfo?[NSWorkspace.volumeURLUserInfoKey] as? URL
            MainActor.assumeIsolated { if let volume { self?.backend.volumeLeaving(volume.path) } }
        })
    }

    isolated deinit {
        observation?.cancel()
        for volume in volumes { center.removeObserver(volume) }
    }

    private func apply(_ state: GamesFolderState) {
        guard state.revision > revision else { return }
        revision = state.revision
        if path != state.path { path = state.path }
        if isDefault != state.isDefault { isDefault = state.isDefault }
        if isConnected != state.isConnected { isConnected = state.isConnected }
        if locations != state.locations { locations = state.locations }
    }

    /// The games folder's new state follows through `changed(after:)`.
    private func volumesChanged() { backend.volumesChanged() }

    /// Whether `directory`, an installation's folder, is inside one of the
    /// folders that hold games.
    func holds(_ directory: String) -> Bool {
        locations.contains { Self.location($0.path, holds: directory) }
    }

    private static func location(_ location: String, holds directory: String) -> Bool {
        directory.hasPrefix(location.hasSuffix("/") ? location : location + "/")
    }

    /// The folders a game in `directory` may move to: those that hold games
    /// whose disk is there, but its own.
    func destinations(from directory: String) -> [GamesLocation] {
        locations.filter { $0.isConnected && !Self.location($0.path, holds: directory) }
    }

    /// Installs `gameName` through `install`, in the folder the player picks
    /// when several connected ones hold games; nil goes to the games folder.
    func place(_ gameName: String, install: @escaping @MainActor (String?) -> Void) {
        let connected = locations.filter(\.isConnected).sorted { $0.isTarget && !$1.isTarget }
        guard connected.count > 1 else { return install(nil) }
        placement = InstallPlacement(gameName: gameName, locations: connected, install: { install($0) })
    }

    /// Adds the folder the player picked to those that hold games: no game
    /// moves. A disk macOS only reads is refused, and one whose format
    /// limits what it holds is told.
    func add(_ folder: URL) {
        failure = nil
        notice = nil
        let volume = GamesFolderProposal.volume(of: folder)
        if volume.readOnly {
            failure = L10n.string("ui.games_folder.read_only", volume.name)
            return
        }
        do {
            _ = try backend.add(folder.path)
            apply(backend.state())
            notice = volume.format.map { L10n.string("ui.games_folder.format", volume.name, $0) }
        } catch {
            failure = L10n.string("model.gamesFolder.failed", L10n.message(Self.reason(error)))
        }
    }

    /// The disk `location` lies on, when the player may eject it: an
    /// external or removable one, connected.
    func ejectableVolume(of location: GamesLocation) -> URL? {
        guard !location.isDefault, location.isConnected else { return nil }
        let keys: Set<URLResourceKey> = [.volumeURLKey, .volumeIsEjectableKey, .volumeIsRemovableKey,
                                         .volumeIsInternalKey]
        guard let values = try? URL(filePath: location.path, directoryHint: .isDirectory).resourceValues(forKeys: keys),
              let volume = values.volume,
              values.volumeIsEjectable == true || values.volumeIsRemovable == true || values.volumeIsInternal == false
        else { return nil }
        return volume
    }

    /// Ejects the disk `location` lies on, once no game runs from it and
    /// nothing Obelisk does writes there.
    func eject(_ location: GamesLocation) {
        failure = nil
        guard let volume = ejectableVolume(of: location) else { return }
        if backend.isBusy(on: volume.path) {
            failure = L10n.string("model.gamesFolder.ejectBusy", location.title)
            return
        }
        Task.detached {
            do {
                try NSWorkspace.shared.unmountAndEjectDevice(at: volume)
            } catch {
                await MainActor.run { [weak self] in
                    self?.failure = L10n.string("model.gamesFolder.ejectFailed", location.title)
                }
            }
        }
    }

    /// New games go to `location`: no game moves.
    func prefer(_ location: GamesLocation) {
        failure = nil
        do {
            try backend.prefer(location.path)
            apply(backend.state())
        } catch {
            failure = L10n.string("model.gamesFolder.failed", L10n.message(Self.reason(error)))
        }
    }

    /// Takes a folder the player added out of those that hold games, once empty.
    func remove(_ location: GamesLocation) {
        failure = nil
        do {
            try backend.remove(location.path)
            apply(backend.state())
        } catch {
            failure = L10n.string("model.gamesFolder.failed", L10n.message(Self.reason(error)))
        }
    }

    /// Asks the player to move every game to `location`.
    func proposeMovingAll(to location: GamesLocation) {
        if location.isDefault {
            proposeDefault()
        } else {
            propose(URL(filePath: location.path, directoryHint: .isDirectory))
        }
    }

    /// Asks the player to move the games to the folder they picked.
    func propose(_ folder: URL) {
        propose(chosen: folder, backend.inspect(folder.path))
    }

    /// Asks the player to move the games back to Obelisk's own folder.
    func proposeDefault() {
        propose(chosen: nil, backend.inspectDefault())
    }

    private func propose(chosen: URL?, _ inspection: GamesFolderInspection) {
        failure = nil
        switch inspection {
        case let .ready(path, games, copiedBytes):
            // Obelisk's own folder exists once inspected; the one inside a
            // picked folder only once chosen.
            let volume = GamesFolderProposal.volume(of: chosen ?? URL(filePath: path, directoryHint: .isDirectory))
            proposal = GamesFolderProposal(
                chosen: chosen, path: path, games: Int(games), copiedBytes: copiedBytes, volume: volume.name,
                availableBytes: volume.available, format: volume.format, isReadOnly: volume.readOnly)
        case .missing:
            failure = L10n.string("model.gamesFolder.missing")
        case .inside:
            failure = L10n.string("model.gamesFolder.inside", AppIdentity.name)
        }
    }

    /// Takes the folder the player agreed to: the games move there.
    func confirm() {
        guard let proposal, proposal.canChoose else { return }
        self.proposal = nil
        do {
            if let chosen = proposal.chosen {
                try backend.choose(chosen.path)
            } else {
                try backend.chooseDefault()
            }
            apply(backend.state())
        } catch {
            failure = L10n.string("model.gamesFolder.failed", L10n.message(Self.reason(error)))
        }
    }

    /// Moves the games still outside the games folder.
    func gather() { backend.gather() }

    private static func reason(_ error: Error) -> String {
        if case let GameSessionError.Failure(reason) = error { return reason }
        return error.localizedDescription
    }
}

/// Where Windows games' Wine prefixes are, as Rust keeps them. A change
/// returns once the prefixes have moved.
@MainActor
protocol PrefixesBackend: AnyObject {
    func state() -> PrefixesState
    /// The prefixes' size in bytes. It walks their folder.
    func size() async -> UInt64
    func setShared(_ shared: Bool) throws
    /// Moves every prefix to the folder `chosen` gives. Nothing moves while
    /// a game `running`.
    func choose(_ chosen: String, running: Bool) async throws
    func chooseDefault(running: Bool) async throws
}

final class RustPrefixes: PrefixesBackend {
    private let core: PrefixesFolder

    init(library: URL) {
        core = PrefixesFolder(library: library.path)
    }

    func state() -> PrefixesState { core.state() }
    func size() async -> UInt64 { await core.size() }
    func setShared(_ shared: Bool) throws { try core.setShared(shared: shared) }
    func choose(_ chosen: String, running: Bool) async throws { try await core.choose(chosen: chosen, running: running) }
    func chooseDefault(running: Bool) async throws { try await core.chooseDefault(running: running) }
}

/// A folder the player picked for the Windows environments, before they agree.
struct PrefixesProposal: Identifiable, Equatable {
    let id = UUID()
    /// The folder the player picked; nil for Obelisk's own on this Mac.
    let chosen: URL?
    /// The disk's name, as Finder shows it.
    let volume: String
    let bytes: UInt64
}

/// The Windows environments, the Wine prefixes of Windows games: where they
/// are, whether games share one, and the player's move of them.
@MainActor @Observable
final class PrefixesModel {
    private(set) var path: String
    /// Obelisk's own folder, inside its library on this Mac.
    private(set) var isDefault: Bool
    /// Whether their disk is there: Obelisk's own folder always is.
    private(set) var isConnected: Bool
    /// Games without an environment of their own share one.
    private(set) var isShared: Bool
    /// What they take, once counted.
    private(set) var bytes: UInt64?
    private(set) var isMoving = false
    /// The move the player is asked to agree to.
    var proposal: PrefixesProposal?
    /// Why the last change was not made.
    var failure: String?
    /// Whether a game runs: nothing moves then.
    @ObservationIgnored var isRunning: @MainActor () -> Bool = { false }

    @ObservationIgnored private let backend: any PrefixesBackend

    init(library: URL, backend: (any PrefixesBackend)? = nil) {
        let backend = backend ?? RustPrefixes(library: library)
        self.backend = backend
        let state = backend.state()
        path = state.path
        isDefault = state.isDefault
        isConnected = state.isConnected
        isShared = state.shared
    }

    /// Reads where they are and what they take, again.
    func refresh() {
        apply(backend.state())
        Task { bytes = await backend.size() }
    }

    private func apply(_ state: PrefixesState) {
        if path != state.path { path = state.path }
        if isDefault != state.isDefault { isDefault = state.isDefault }
        if isConnected != state.isConnected { isConnected = state.isConnected }
        if isShared != state.shared { isShared = state.shared }
    }

    /// Lets games without an environment share one, or not.
    func setShared(_ shared: Bool) {
        failure = nil
        do {
            try backend.setShared(shared)
        } catch {
            failure = L10n.string("model.prefixes.failed", L10n.message(Self.reason(error)))
        }
        apply(backend.state())
    }

    /// Asks the player to move the environments to the folder they picked.
    func propose(_ folder: URL) {
        let volume = GamesFolderProposal.volume(of: folder)
        // A prefix holds links, which only a Mac format keeps.
        if volume.readOnly {
            failure = L10n.string("ui.games_folder.read_only", volume.name)
        } else if let format = volume.format {
            failure = L10n.string("ui.prefixes.format", volume.name, format)
        } else {
            ask(chosen: folder, volume: volume)
        }
    }

    /// Asks the player to move the environments back to this Mac.
    func proposeDefault() {
        let library = URL(filePath: path, directoryHint: .isDirectory).deletingLastPathComponent()
        ask(chosen: nil, volume: GamesFolderProposal.volume(of: library))
    }

    private func ask(chosen: URL?, volume: (name: String, available: UInt64?, format: String?, readOnly: Bool)) {
        failure = nil
        Task {
            let bytes = await backend.size()
            if let available = volume.available, available < bytes {
                failure = L10n.string("ui.games_folder.no_space", volume.name,
                                      ByteCountFormatter.string(fromByteCount: Int64(clamping: bytes), countStyle: .file),
                                      ByteCountFormatter.string(fromByteCount: Int64(clamping: available), countStyle: .file))
                return
            }
            proposal = PrefixesProposal(chosen: chosen, volume: volume.name, bytes: bytes)
        }
    }

    /// Moves the environments where the player agreed.
    func confirm() {
        guard let proposal, !isMoving else { return }
        self.proposal = nil
        let running = isRunning()
        isMoving = true
        Task {
            defer {
                isMoving = false
                refresh()
            }
            do {
                if let chosen = proposal.chosen {
                    try await backend.choose(chosen.path, running: running)
                } else {
                    try await backend.chooseDefault(running: running)
                }
            } catch {
                failure = L10n.string("model.prefixes.failed", L10n.message(Self.reason(error)))
            }
        }
    }

    private static func reason(_ error: Error) -> String {
        if case let GameSessionError.Failure(reason) = error { return reason }
        return error.localizedDescription
    }
}
