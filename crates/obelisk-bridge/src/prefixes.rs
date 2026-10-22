//! Where Windows games' Wine prefixes are, as Swift presents and changes it.
use std::{path::Path, sync::Arc};

use game_runtime::prefixes::{self, MoveFailure};

use crate::GameSessionError;

/// Where the prefixes are now.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PrefixesState {
    pub path: String,
    /// Obelisk's own folder, inside its library.
    pub is_default: bool,
    /// Games without a prefix of their own share one.
    pub shared: bool,
    /// Their disk is there: Obelisk's own folder always is.
    pub is_connected: bool,
}

/// The Wine prefixes of Obelisk's `library`.
#[derive(uniffi::Object)]
pub struct PrefixesFolder {
    library: String,
}

#[uniffi::export(async_runtime = "tokio")]
impl PrefixesFolder {
    #[uniffi::constructor]
    pub fn new(library: String) -> Arc<Self> {
        Arc::new(Self { library })
    }

    pub fn state(&self) -> PrefixesState {
        let library = Path::new(&self.library);
        let folder = prefixes::folder(library);
        PrefixesState {
            is_default: folder == prefixes::default_folder(library),
            shared: prefixes::shared(library),
            is_connected: !prefixes::unavailable(library),
            path: folder.to_string_lossy().into_owned(),
        }
    }

    /// The prefixes' size in bytes. It walks their folder.
    pub async fn size(&self) -> u64 {
        let library = self.library.clone();
        tokio::task::spawn_blocking(move || prefixes::size(Path::new(&library)))
            .await
            .unwrap_or(0)
    }

    /// Lets games without a prefix of their own share one, or not; each game
    /// keeps the one it has.
    pub fn set_shared(&self, shared: bool) -> Result<(), GameSessionError> {
        prefixes::set_shared(Path::new(&self.library), shared).map_err(GameSessionError::new)
    }

    /// Moves every prefix to the folder `chosen` gives, which holds them from
    /// then on. `running` tells a game runs: nothing moves then.
    pub async fn choose(&self, chosen: String, running: bool) -> Result<(), GameSessionError> {
        let library = self.library.clone();
        tokio::task::spawn_blocking(move || {
            prefixes::relocate(Path::new(&library), Path::new(&chosen), running)
        })
        .await
        .map_err(GameSessionError::new)?
        .map_err(failure)
    }

    /// Moves every prefix back to Obelisk's own folder.
    pub async fn choose_default(&self, running: bool) -> Result<(), GameSessionError> {
        let library = self.library.clone();
        tokio::task::spawn_blocking(move || {
            let folder = prefixes::default_folder(Path::new(&library));
            std::fs::create_dir_all(&folder)?;
            prefixes::relocate(Path::new(&library), &folder, running)
        })
        .await
        .map_err(GameSessionError::new)?
        .map_err(failure)
    }
}

fn failure(failure: MoveFailure) -> GameSessionError {
    match failure {
        MoveFailure::Missing => GameSessionError::new("model.prefixes.missing"),
        MoveFailure::Inside => GameSessionError::new("model.prefixes.inside"),
        MoveFailure::Occupied => GameSessionError::new("model.prefixes.occupied"),
        MoveFailure::Busy => GameSessionError::new("model.prefixes.busy"),
        MoveFailure::Io(error) => GameSessionError::new(error),
    }
}
