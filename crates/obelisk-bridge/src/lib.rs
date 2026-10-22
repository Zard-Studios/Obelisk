//! The complete Swift/Rust seam. Swift owns UI state and OS presentation.
//! Domain code and storefront kits have no dependency on UniFFI.

use std::sync::Arc;

mod steam;
pub use steam::*;
mod steam_chat;
pub use steam_chat::*;
mod steam_friends;
pub use steam_friends::*;
mod steam_native;
pub use steam_native::*;
mod epic;
pub use epic::*;
mod credentials;
mod epic_cloud;
pub use credentials::CredentialSave;
mod epic_friends;
pub use epic_friends::*;
mod gog;
pub use gog::*;
mod cloud;
pub use cloud::*;
mod game_session;
pub use game_session::*;
mod locations;
pub use locations::*;
mod diagnostics;
pub use diagnostics::*;
mod installations;
pub use installations::*;
mod games_folder;
pub use games_folder::{GamesFolder, GamesFolderInspection, GamesFolderState};
mod prefixes;
pub use prefixes::{PrefixesFolder, PrefixesState};
mod launch_settings;
pub use launch_settings::*;
mod downloads;
pub use downloads::*;
mod library_cache;
pub use library_cache::{CachedEpicGame, CachedGogGame, CachedSteamGame, GameLaunch, LibraryCache};

uniffi::setup_scaffolding!();

#[derive(Clone, Debug, thiserror::Error, uniffi::Error)]
pub enum CoreError {
    #[error("Invalid game: {reason}")]
    InvalidGame { reason: String },
    #[error("Cannot access the library: {reason}")]
    Storage { reason: String },
    #[error("The library cannot be read: {reason}")]
    CorruptLibrary { reason: String },
    #[error("Game not found: {id}")]
    NotFound { id: String },
}

impl From<obelisk_core::CoreError> for CoreError {
    fn from(error: obelisk_core::CoreError) -> Self {
        match error {
            obelisk_core::CoreError::InvalidGame { reason } => Self::InvalidGame { reason },
            obelisk_core::CoreError::Storage { reason } => Self::Storage { reason },
            obelisk_core::CoreError::CorruptLibrary { reason } => Self::CorruptLibrary { reason },
            obelisk_core::CoreError::NotFound { id } => Self::NotFound { id },
        }
    }
}

#[derive(Clone, Copy, Debug, uniffi::Enum)]
pub enum GameKind {
    NativeMac,
    WindowsExecutable,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct Game {
    pub id: String,
    pub title: String,
    pub path: String,
    pub added_at: u64,
    pub last_played_at: Option<u64>,
    pub kind: GameKind,
}

impl From<obelisk_core::Game> for Game {
    fn from(game: obelisk_core::Game) -> Self {
        Self {
            id: game.id,
            title: game.title,
            path: game.path,
            added_at: game.added_at,
            last_played_at: game.last_played_at,
            kind: match game.kind {
                obelisk_core::GameKind::NativeMac => GameKind::NativeMac,
                obelisk_core::GameKind::WindowsExecutable => GameKind::WindowsExecutable,
            },
        }
    }
}

/// Lifecycle of one storefront transfer, shared by every store's progress record.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum TransferState {
    Preparing,
    Verifying,
    Downloading,
    /// The installation moves into the games folder.
    Moving,
    Complete,
    Cancelled,
    Failed,
}

impl From<obelisk_core::transfer::TransferState> for TransferState {
    fn from(state: obelisk_core::transfer::TransferState) -> Self {
        use obelisk_core::transfer::TransferState as State;
        match state {
            State::Preparing => Self::Preparing,
            State::Verifying => Self::Verifying,
            State::Downloading => Self::Downloading,
            State::Moving => Self::Moving,
            State::Complete => Self::Complete,
            State::Cancelled => Self::Cancelled,
            State::Failed => Self::Failed,
        }
    }
}

/// One installation Transfer's progress, the same record for every storefront.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct GameTransfer {
    /// The storefront's own identity for the game: Steam's app ID, Epic's app name.
    pub game: String,
    pub state: TransferState,
    pub completed_bytes: u64,
    pub total_bytes: u64,
    pub network_bytes: u64,
    pub content_bytes: u64,
    pub current_file: String,
    pub message: String,
}

impl From<TransferState> for obelisk_core::transfer::TransferState {
    fn from(state: TransferState) -> Self {
        match state {
            TransferState::Preparing => Self::Preparing,
            TransferState::Verifying => Self::Verifying,
            TransferState::Downloading => Self::Downloading,
            TransferState::Moving => Self::Moving,
            TransferState::Complete => Self::Complete,
            TransferState::Cancelled => Self::Cancelled,
            TransferState::Failed => Self::Failed,
        }
    }
}

impl From<GameTransfer> for obelisk_core::installation::TransferProgress {
    fn from(transfer: GameTransfer) -> Self {
        Self {
            game: transfer.game,
            state: transfer.state.into(),
            completed_bytes: transfer.completed_bytes,
            total_bytes: transfer.total_bytes,
            network_bytes: transfer.network_bytes,
            content_bytes: transfer.content_bytes,
            current_file: transfer.current_file,
            message: transfer.message,
        }
    }
}

impl From<obelisk_core::installation::TransferProgress> for GameTransfer {
    fn from(progress: obelisk_core::installation::TransferProgress) -> Self {
        Self {
            game: progress.game,
            state: progress.state.into(),
            completed_bytes: progress.completed_bytes,
            total_bytes: progress.total_bytes,
            network_bytes: progress.network_bytes,
            content_bytes: progress.content_bytes,
            current_file: progress.current_file,
            message: progress.message,
        }
    }
}

#[derive(Clone, Copy, Debug, uniffi::Enum)]
pub enum StorefrontId {
    Steam,
    Epic,
    Gog,
    Other,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct StorefrontInfo {
    pub id: StorefrontId,
    pub display_name: String,
    pub available: bool,
}

impl From<obelisk_core::storefront::StorefrontDescriptor> for StorefrontInfo {
    fn from(storefront: obelisk_core::storefront::StorefrontDescriptor) -> Self {
        use obelisk_core::storefront::StorefrontId as Id;
        Self {
            id: match storefront.id {
                Id::Steam => StorefrontId::Steam,
                Id::Epic => StorefrontId::Epic,
                Id::Gog => StorefrontId::Gog,
                Id::Other => StorefrontId::Other,
            },
            display_name: storefront.display_name.to_owned(),
            available: storefront.capabilities.any_available(),
        }
    }
}

#[uniffi::export]
pub fn storefronts() -> Vec<StorefrontInfo> {
    use obelisk_core::storefront::{StorefrontCapabilities, StorefrontDescriptor};
    vec![
        steam_kit::descriptor().into(),
        epic_kit::descriptor().into(),
        gog_kit::descriptor().into(),
        StorefrontDescriptor {
            id: obelisk_core::storefront::StorefrontId::Other,
            display_name: "Autres",
            capabilities: StorefrontCapabilities {
                local_import: true,
                ..Default::default()
            },
        }
        .into(),
    ]
}

#[derive(Debug, uniffi::Object)]
pub struct Library {
    inner: obelisk_core::Library,
}

#[uniffi::export]
impl Library {
    #[uniffi::constructor]
    pub fn new(storage_dir: String) -> Result<Arc<Self>, CoreError> {
        Ok(Arc::new(Self {
            inner: obelisk_core::Library::open(storage_dir)?,
        }))
    }

    pub fn games(&self) -> Result<Vec<Game>, CoreError> {
        Ok(self.inner.games()?.into_iter().map(Into::into).collect())
    }

    pub fn add_local_game(&self, path: String) -> Result<Game, CoreError> {
        Ok(self.inner.add_local_game(path)?.into())
    }

    pub fn remove_game(&self, id: String) -> Result<(), CoreError> {
        self.inner.remove_game(&id).map_err(Into::into)
    }

    pub fn record_launch(&self, id: String) -> Result<(), CoreError> {
        self.inner.record_launch(&id).map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exported_catalog_reports_real_availability() {
        let sources = storefronts();
        assert_eq!(sources.len(), 4);
        assert!(sources[0].available);
        assert!(sources[1].available);
        assert!(sources[2].available);
        assert!(sources[3].available);
    }

    #[test]
    fn bridge_maps_games_and_errors() {
        let root = tempfile::TempDir::new().unwrap();
        let library =
            Library::new(root.path().join("catalog").to_str().unwrap().to_owned()).unwrap();
        let path = root.path().join("Example.exe");
        std::fs::write(&path, "fixture").unwrap();
        let game = library
            .add_local_game(path.to_str().unwrap().to_owned())
            .unwrap();
        assert!(matches!(game.kind, GameKind::WindowsExecutable));
        assert_eq!(library.games().unwrap()[0].id, game.id);
        assert!(matches!(
            library.remove_game("missing".to_owned()),
            Err(CoreError::NotFound { .. })
        ));
    }
}
