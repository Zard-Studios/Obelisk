import Foundation

/// An installation action that failed, as the player reads it. Reasons are
/// localization keys.
enum GameInstallationFailure: LocalizedError {
    case downloadDirectory(String)
    case uninstall(String)
    case move(String)
    /// The games folder's disk is not connected: this folder.
    case gamesFolderUnavailable(String)
    /// A disk's format could not hold the game's files: the folder that
    /// holds games there, and the format's name.
    case diskFormat(String, String)

    var errorDescription: String? {
        switch self {
        case let .downloadDirectory(message): L10n.string("model.game.downloadDirectoryFailed", message)
        case let .uninstall(reason): L10n.string("model.game.uninstallFailed", L10n.message(reason))
        case let .move(reason): L10n.string("model.game.moveFailed", L10n.message(reason))
        case let .gamesFolderUnavailable(path): L10n.string("model.game.gamesFolderUnavailable", path)
        case let .diskFormat(path, format):
            L10n.string("model.game.diskFormat",
                        GamesFolderProposal.volume(of: URL(filePath: path, directoryHint: .isDirectory)).name, format)
        }
    }
}
