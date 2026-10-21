//! Where Obelisk installs games, as Swift presents and changes it. Every
//! account's installations of one library share its games folder.
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

use game_runtime::games_folder::{self, Candidate, ChoiceFailure, Location, Refusal};

use crate::GameSessionError;

/// The games folder of Obelisk's `library`, the same for every caller.
pub(crate) fn shared(library: &Path) -> Arc<games_folder::GamesFolder> {
    static FOLDERS: OnceLock<Mutex<HashMap<PathBuf, Arc<games_folder::GamesFolder>>>> =
        OnceLock::new();
    FOLDERS
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .entry(library.to_owned())
        .or_insert_with(|| games_folder::GamesFolder::new(library))
        .clone()
}

/// Where new installations go. `revision` grows with every change.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct GamesFolderState {
    pub revision: u64,
    pub path: String,
    /// Obelisk's own folder, inside its library.
    pub is_default: bool,
    /// Its disk is there: Obelisk's own folder always is.
    pub is_connected: bool,
    /// Every folder that holds games: Obelisk's own, then the one new
    /// installations go to, then those the player added.
    pub locations: Vec<GamesLocation>,
}

/// One folder that holds games.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct GamesLocation {
    pub path: String,
    /// Obelisk's own folder, inside its library.
    pub is_default: bool,
    /// New installations go there.
    pub is_target: bool,
    /// Its disk is there: Obelisk's own folder always is.
    pub is_connected: bool,
}

impl From<games_folder::View> for GamesFolderState {
    fn from(view: games_folder::View) -> Self {
        Self {
            revision: view.revision,
            path: view.path.to_string_lossy().into_owned(),
            is_default: view.default,
            is_connected: view.connected,
            locations: view.locations.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<Location> for GamesLocation {
    fn from(location: Location) -> Self {
        Self {
            path: location.path.to_string_lossy().into_owned(),
            is_default: location.default,
            is_target: location.target,
            is_connected: location.connected,
        }
    }
}

/// What choosing a folder would do.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum GamesFolderInspection {
    /// The games go in `path`: `games` installations move there, and
    /// `copied_bytes` of theirs are copied from another disk.
    Ready {
        path: String,
        games: u32,
        copied_bytes: u64,
    },
    /// The folder is gone.
    Missing,
    /// The folder is inside Obelisk's library or a game's installation.
    Inside,
}

impl From<Result<Candidate, Refusal>> for GamesFolderInspection {
    fn from(inspection: Result<Candidate, Refusal>) -> Self {
        match inspection {
            Ok(candidate) => Self::Ready {
                path: candidate.path.to_string_lossy().into_owned(),
                games: candidate.games,
                copied_bytes: candidate.copied_bytes,
            },
            Err(Refusal::Missing | Refusal::Occupied) => Self::Missing,
            Err(Refusal::Inside) => Self::Inside,
        }
    }
}

/// Obelisk's games folder.
#[derive(uniffi::Object)]
pub struct GamesFolder {
    folder: Arc<games_folder::GamesFolder>,
}

#[uniffi::export(async_runtime = "tokio")]
impl GamesFolder {
    /// The games folder of the Obelisk whose library is `library`.
    #[uniffi::constructor]
    pub fn new(library: String) -> Arc<Self> {
        Arc::new(Self {
            folder: shared(Path::new(&library)),
        })
    }

    pub fn state(&self) -> GamesFolderState {
        self.folder.view().into()
    }

    /// Waits for a revision newer than `after`.
    pub async fn changed(&self, after: u64) -> GamesFolderState {
        self.folder.changed(after).await.into()
    }

    /// What choosing `chosen` would do. It blocks on file reads.
    pub fn inspect(&self, chosen: String) -> GamesFolderInspection {
        self.folder.inspect(Path::new(&chosen)).into()
    }

    /// What making Obelisk's own folder the games folder again would do. It
    /// makes that folder, and blocks on file reads.
    pub fn inspect_default(&self) -> GamesFolderInspection {
        self.folder.inspect_default().into()
    }

    /// Makes the games folder `chosen` gives the one new installations go
    /// to, and queues the move of every installation into it.
    pub fn choose(&self, chosen: String) -> Result<(), GameSessionError> {
        self.folder
            .choose(Path::new(&chosen))
            .map_err(choice_failure)
    }

    /// Makes Obelisk's own folder the games folder again, and queues the
    /// move of every installation into it.
    pub fn choose_default(&self) -> Result<(), GameSessionError> {
        self.folder.choose_default().map_err(choice_failure)
    }

    /// Adds the games folder `chosen` gives to those that hold games, moving
    /// nothing, and returns its path.
    pub fn add(&self, chosen: String) -> Result<String, GameSessionError> {
        self.folder
            .add(Path::new(&chosen))
            .map(|path| path.to_string_lossy().into_owned())
            .map_err(choice_failure)
    }

    /// Makes `location`, one of the folders that hold games, the one new
    /// installations go to, moving nothing.
    pub fn prefer(&self, location: String) -> Result<(), GameSessionError> {
        self.folder
            .prefer(Path::new(&location))
            .map_err(choice_failure)
    }

    /// Takes `location`, a folder the player added, out of those that hold
    /// games, once no installation is left there.
    pub fn remove(&self, location: String) -> Result<(), GameSessionError> {
        self.folder
            .remove(Path::new(&location))
            .map_err(choice_failure)
    }

    /// Queues the move of the installations outside every folder that holds
    /// games into the games folder.
    pub fn gather(&self) {
        self.folder.gather()
    }

    /// Whether a game of the disk mounted at `volume` runs, or a Transfer,
    /// move or uninstall works there: ejecting it waits.
    pub fn busy_on(&self, volume: String) -> bool {
        self.folder.busy_on(Path::new(&volume))
    }

    /// The disk mounted at `volume` is about to leave: the Transfers and
    /// moves on it stop.
    pub fn volume_leaving(&self, volume: String) {
        self.folder.leaving(Path::new(&volume))
    }

    /// A disk came or left.
    pub fn volumes_changed(&self) {
        self.folder.volumes_changed()
    }
}

fn choice_failure(failure: ChoiceFailure) -> GameSessionError {
    match failure {
        ChoiceFailure::Refused(Refusal::Occupied) => {
            GameSessionError::new("model.gamesFolder.occupied")
        }
        ChoiceFailure::Refused(_) => GameSessionError::new("model.gamesFolder.refused"),
        ChoiceFailure::Io(error) => GameSessionError::new(error),
    }
}
