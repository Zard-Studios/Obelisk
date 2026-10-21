//! Where Obelisk installs games: one folder, Obelisk's own inside its library
//! until the player chooses another, such as one on an external disk. Each
//! storefront keeps its games in its own folder inside it. Choosing another
//! queues the move of every installation outside it, one game at a time in
//! the download queue, and points the new installations still waiting there.
//! The player may also add other folders that hold games without moving any:
//! a game then installs in, or moves to, the one they pick. An installation
//! whose disk is not connected shows as not installed until the disk comes
//! back.
use crate::supervisor::plain_absolute;
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    os::unix::fs::MetadataExt,
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex, Weak},
};
use tokio::sync::watch;

/// Obelisk's own games folder, inside its library.
const DEFAULT_DIRECTORY: &str = "Games";
/// The folder Obelisk makes inside the one the player chooses.
pub const CHOSEN_DIRECTORY: &str = "Obelisk Games";
/// The player's games folder, inside Obelisk's library; absent for its own.
const FILE: &str = "games-folder.json";

#[derive(Serialize, Deserialize)]
struct Saved {
    path: PathBuf,
    /// The other folders the player added.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    added: Vec<PathBuf>,
}

/// Where new installations go, as the player sees it. `revision` grows with
/// every change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    pub revision: u64,
    pub path: PathBuf,
    /// Obelisk's own folder, inside its library.
    pub default: bool,
    /// Its disk is there: Obelisk's own folder always is.
    pub connected: bool,
    /// Every folder that holds games: Obelisk's own, then the one new
    /// installations go to, then those the player added.
    pub locations: Vec<Location>,
}

/// One folder that holds games, as the player sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    pub path: PathBuf,
    /// Obelisk's own folder, inside its library.
    pub default: bool,
    /// New installations go there.
    pub target: bool,
    /// Its disk is there: Obelisk's own folder always is.
    pub connected: bool,
}

/// `path` as the player sees it, with the folders the player `added`, at
/// `revision`: Obelisk's own folder when it is `default`.
fn view(path: PathBuf, added: &[PathBuf], default: &Path, revision: u64) -> View {
    let locations = listed(&path, added, default)
        .into_iter()
        .map(|location| Location {
            default: location == default,
            target: location == path,
            connected: location == default || location.is_dir(),
            path: location,
        })
        .collect();
    let default = path == default;
    View {
        revision,
        connected: default || path.is_dir(),
        default,
        path,
        locations,
    }
}

/// Obelisk's own folder `default`, then `path`, then those `added`, each once.
fn listed(path: &Path, added: &[PathBuf], default: &Path) -> Vec<PathBuf> {
    let mut locations = vec![default.to_owned()];
    for location in std::iter::once(path).chain(added.iter().map(PathBuf::as_path)) {
        if !locations.iter().any(|kept| kept == location) {
            locations.push(location.to_owned());
        }
    }
    locations
}

/// What choosing a folder would do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    /// The games folder it gives: the chosen folder's own, else one made
    /// inside it.
    pub path: PathBuf,
    /// The connected installations outside it, which move into it.
    pub games: u32,
    /// What those of another disk copy there; the others only change their
    /// place.
    pub copied_bytes: u64,
}

/// Why a folder cannot hold the games.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// It is not an existing folder.
    Missing,
    /// It is inside Obelisk's library, or inside a game's installation.
    Inside,
    /// It still holds installations.
    Occupied,
}

/// An account's installations, as the games folder moves and finds them.
pub(crate) trait Resident: Send + Sync {
    /// The connected installations' folders, with their bytes.
    fn installations(&self) -> Vec<(PathBuf, u64)>;
    /// Queues the move into `folder` of each installation outside it and
    /// outside every folder `kept`, and points the new installations still
    /// waiting there.
    fn gather(self: Arc<Self>, folder: &Path, kept: &[PathBuf]);
    /// Hides the installations whose disk left, and shows those whose disk
    /// came back.
    fn refresh(&self);
}

/// Obelisk's games folder, which every account's installations share.
pub struct GamesFolder {
    library: PathBuf,
    /// The other folders the player added, in their order.
    added: Mutex<Vec<PathBuf>>,
    published: watch::Sender<View>,
    residents: Mutex<Vec<Weak<dyn Resident>>>,
}

impl GamesFolder {
    /// The games folder of Obelisk's `library`, as the player last chose it.
    pub fn new(library: &Path) -> Arc<Self> {
        // Installations name their canonical folder.
        let library = library
            .canonicalize()
            .unwrap_or_else(|_| library.to_owned());
        let default = library.join(DEFAULT_DIRECTORY);
        let saved = fs::read(library.join(FILE))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Saved>(&bytes).ok());
        let path = saved
            .as_ref()
            .map(|saved| saved.path.clone())
            .filter(|path| plain_absolute(path) && *path != default)
            .unwrap_or_else(|| default.clone());
        let added: Vec<PathBuf> = saved
            .map(|saved| saved.added)
            .unwrap_or_default()
            .into_iter()
            .filter(|added| plain_absolute(added))
            .collect();
        let added: Vec<PathBuf> = listed(&path, &added, &default)
            .into_iter()
            .filter(|location| *location != path && *location != default)
            .collect();
        // A disk that sleeps wakes slowly: Obelisk's start does not wait.
        let games = listed(&path, &added, &default);
        std::thread::spawn(move || games.iter().for_each(|games| clear_staging(games)));
        Arc::new(Self {
            library,
            published: watch::channel(view(path, &added, &default, 1)).0,
            added: Mutex::new(added),
            residents: Mutex::new(Vec::new()),
        })
    }

    pub fn view(&self) -> View {
        self.published.borrow().clone()
    }

    /// Obelisk's library, which holds its own games folder.
    pub fn library(&self) -> &Path {
        &self.library
    }

    /// Waits for a revision newer than `after`, or returns at once.
    pub async fn changed(&self, after: u64) -> View {
        let mut receiver = self.published.subscribe();
        let _ = receiver.wait_for(|view| view.revision > after).await;
        receiver.borrow().clone()
    }

    /// Where new installations go.
    pub fn path(&self) -> PathBuf {
        self.published.borrow().path.clone()
    }

    /// Whether new installations can go there now: Obelisk's own folder is
    /// made when needed, the player's must be there.
    pub fn connected(&self) -> bool {
        let view = self.view();
        view.default || view.path.is_dir()
    }

    fn default_path(&self) -> PathBuf {
        self.library.join(DEFAULT_DIRECTORY)
    }

    /// Every folder that holds games: Obelisk's own, then the one new
    /// installations go to, then those the player added.
    pub fn locations(&self) -> Vec<PathBuf> {
        let added = self.added.lock().unwrap();
        listed(&self.path(), &added, &self.default_path())
    }

    /// Whether new installations can go to `location`, one of the folders
    /// that hold games, now: Obelisk's own folder is made when needed, any
    /// other must be there.
    pub fn connected_at(&self, location: &Path) -> bool {
        location == self.default_path() || location.is_dir()
    }

    /// Publishes `path` as the games folder, the folders the player added,
    /// and whether their disks are connected.
    fn publish(&self, path: PathBuf) {
        let added = self.added.lock().unwrap().clone();
        let mut next = view(path, &added, &self.default_path(), 0);
        self.published.send_if_modified(|current| {
            next.revision = current.revision;
            if next == *current {
                return false;
            }
            next.revision += 1;
            *current = next;
            true
        });
    }

    /// Whether `games` is a games folder Obelisk made: its own, or the one
    /// inside a folder the player chose.
    fn ours(&self, games: &Path) -> bool {
        games == self.default_path()
            || games
                .file_name()
                .is_some_and(|name| name == CHOSEN_DIRECTORY)
    }

    /// The games folder `chosen` gives, and what choosing it moves.
    pub fn inspect(&self, chosen: &Path) -> Result<Candidate, Refusal> {
        let (path, installations) = self.destination(chosen)?;
        let device = fs::metadata(chosen).map_err(|_| Refusal::Missing)?.dev();
        let mut candidate = Candidate {
            path: path.clone(),
            games: 0,
            copied_bytes: 0,
        };
        let mut seen = Vec::new();
        for (installation, bytes) in installations {
            // Two accounts may share one installation: it moves once.
            if installation.starts_with(&path) || seen.contains(&installation) {
                continue;
            }
            candidate.games += 1;
            if fs::metadata(&installation).is_ok_and(|metadata| metadata.dev() != device) {
                candidate.copied_bytes += bytes;
            }
            seen.push(installation);
        }
        Ok(candidate)
    }

    /// What making Obelisk's own folder the games folder again would do. It
    /// makes that folder.
    pub fn inspect_default(&self) -> Result<Candidate, Refusal> {
        let path = self.default_path();
        // A folder it cannot make reads as missing.
        let _ = fs::create_dir_all(&path);
        self.inspect(&path)
    }

    /// The games folder `chosen` gives, and the connected installations,
    /// none of which holds it.
    fn destination(&self, chosen: &Path) -> Result<(PathBuf, Vec<(PathBuf, u64)>), Refusal> {
        let path = self.resolve(chosen)?;
        let installations: Vec<(PathBuf, u64)> = self
            .residents()
            .iter()
            .flat_map(|resident| resident.installations())
            .collect();
        if installations
            .iter()
            .any(|(installation, _)| path.starts_with(installation))
        {
            return Err(Refusal::Inside);
        }
        Ok((path, installations))
    }

    /// The games folder `chosen` gives: Obelisk's own folder, or the one
    /// Obelisk makes inside the chosen folder, unless that is one already.
    fn resolve(&self, chosen: &Path) -> Result<PathBuf, Refusal> {
        if !plain_absolute(chosen) {
            return Err(Refusal::Missing);
        }
        let chosen = chosen.canonicalize().map_err(|_| Refusal::Missing)?;
        if !chosen.is_dir() {
            return Err(Refusal::Missing);
        }
        if self.ours(&chosen) {
            return Ok(chosen);
        }
        if chosen.starts_with(&self.library) {
            return Err(Refusal::Inside);
        }
        Ok(chosen.join(CHOSEN_DIRECTORY))
    }

    /// Makes the games folder `chosen` gives the one new installations go
    /// to, and queues the move into it of every installation outside it.
    /// Every installation goes there, so no folder the player added stays.
    pub fn choose(&self, chosen: &Path) -> Result<(), ChoiceFailure> {
        let (path, _) = self.destination(chosen)?;
        fs::create_dir_all(&path)?;
        let added = std::mem::take(&mut *self.added.lock().unwrap());
        if let Err(error) = self.save(&path) {
            *self.added.lock().unwrap() = added;
            return Err(error.into());
        }
        self.publish(path.clone());
        for resident in self.residents() {
            resident.gather(&path, &[]);
        }
        Ok(())
    }

    /// Adds the games folder `chosen` gives to those that hold games,
    /// moving nothing, and returns it. One already there stays as it is.
    pub fn add(&self, chosen: &Path) -> Result<PathBuf, ChoiceFailure> {
        let (path, _) = self.destination(chosen)?;
        if self.locations().contains(&path) {
            return Ok(path);
        }
        fs::create_dir_all(&path)?;
        self.added.lock().unwrap().push(path.clone());
        if let Err(error) = self.save(&self.path()) {
            self.added.lock().unwrap().retain(|added| *added != path);
            return Err(error.into());
        }
        self.publish(self.path());
        Ok(path)
    }

    /// Makes `location`, one of the folders that hold games, the one new
    /// installations go to, moving nothing.
    pub fn prefer(&self, location: &Path) -> Result<(), ChoiceFailure> {
        let previous = self.path();
        if location == previous {
            return Ok(());
        }
        if !self.locations().iter().any(|listed| listed == location) {
            return Err(Refusal::Missing.into());
        }
        let before = self.added.lock().unwrap().clone();
        {
            let mut added = self.added.lock().unwrap();
            added.retain(|added| added != location);
            if previous != self.default_path() {
                added.insert(0, previous);
            }
        }
        if let Err(error) = self.save(location) {
            *self.added.lock().unwrap() = before;
            return Err(error.into());
        }
        self.publish(location.to_owned());
        Ok(())
    }

    /// Takes `location`, a folder the player added, out of those that hold
    /// games, once no installation is left there. Its emptied folders go.
    pub fn remove(&self, location: &Path) -> Result<(), ChoiceFailure> {
        if !self
            .added
            .lock()
            .unwrap()
            .iter()
            .any(|added| added == location)
        {
            return Err(Refusal::Missing.into());
        }
        let occupied = self
            .residents()
            .iter()
            .flat_map(|resident| resident.installations())
            .any(|(installation, _)| installation.starts_with(location));
        if occupied {
            return Err(Refusal::Occupied.into());
        }
        let before = self.added.lock().unwrap().clone();
        self.added.lock().unwrap().retain(|added| added != location);
        if let Err(error) = self.save(&self.path()) {
            *self.added.lock().unwrap() = before;
            return Err(error.into());
        }
        if self.ours(location)
            && let Ok(storefronts) = fs::read_dir(location)
        {
            for storefront in storefronts.flatten() {
                let _ = fs::remove_dir(storefront.path());
            }
            let _ = fs::remove_dir(location);
        }
        self.publish(self.path());
        Ok(())
    }

    /// Makes Obelisk's own folder the games folder again, and queues the
    /// move into it of every installation outside it.
    pub fn choose_default(&self) -> Result<(), ChoiceFailure> {
        let path = self.default_path();
        fs::create_dir_all(&path)?;
        self.choose(&path)
    }

    /// Queues the move into the games folder of the installations outside
    /// every folder that holds games.
    pub fn gather(&self) {
        let (path, kept) = (self.path(), self.locations());
        for resident in self.residents() {
            resident.gather(&path, &kept);
        }
    }

    /// A disk came or left: the games folder tells whether its disk is
    /// connected, and each account shows the installations whose disk is.
    pub fn volumes_changed(&self) {
        self.publish(self.path());
        for resident in self.residents() {
            resident.refresh();
        }
    }

    fn save(&self, path: &Path) -> io::Result<()> {
        let file = self.library.join(FILE);
        let added = self.added.lock().unwrap().clone();
        if path == self.default_path() && added.is_empty() {
            return match fs::remove_file(&file) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
                _ => Ok(()),
            };
        }
        let saved = Saved {
            path: path.to_owned(),
            added,
        };
        obelisk_core::write_atomic(&file, &serde_json::to_vec(&saved)?)
    }

    pub(crate) fn register(&self, resident: Weak<dyn Resident>) {
        let mut residents = self.residents.lock().unwrap();
        residents.retain(|kept| kept.strong_count() > 0);
        residents.push(resident);
    }

    fn residents(&self) -> Vec<Arc<dyn Resident>> {
        self.residents
            .lock()
            .unwrap()
            .iter()
            .filter_map(Weak::upgrade)
            .collect()
    }

    /// The installation `name` of a storefront whose games go in `storefront`,
    /// once moved into one of the folders that hold games, the games folder
    /// first.
    pub(crate) fn installation(&self, storefront: &str, name: &Path) -> Option<PathBuf> {
        let name = name.file_name()?;
        let mut locations = self.locations();
        locations.sort_by_key(|location| *location != self.path());
        locations
            .into_iter()
            .map(|location| location.join(storefront).join(name))
            .find(|found| found.is_dir())
    }

    /// Removes the storefront's folder an installation left, `storefront`,
    /// and the games folder holding it, once empty and neither the games
    /// folder nor one the player added: Obelisk's own is made again when
    /// needed. Only Obelisk's own folders go.
    pub(crate) fn tidy(&self, storefront: &Path) {
        let Some(games) = storefront.parent() else {
            return;
        };
        if games == self.path()
            || self
                .added
                .lock()
                .unwrap()
                .iter()
                .any(|added| added == games)
        {
            return;
        }
        if self.ours(games) && fs::remove_dir(storefront).is_ok() {
            let _ = fs::remove_dir(games);
        }
    }
}

/// Removes the copies moves to the games folder `games` left when Obelisk
/// quit before they ended.
fn clear_staging(games: &Path) {
    let Ok(storefronts) = fs::read_dir(games) else {
        return;
    };
    for storefront in storefronts.flatten() {
        let Ok(entries) = fs::read_dir(storefront.path()) else {
            continue;
        };
        for entry in entries.flatten() {
            if obelisk_core::installation::is_staging(&entry.file_name().to_string_lossy())
                && entry.file_type().is_ok_and(|kind| kind.is_dir())
            {
                let _ = fs::remove_dir_all(entry.path());
            }
        }
    }
}

/// Why choosing a folder failed.
#[derive(Debug, thiserror::Error)]
pub enum ChoiceFailure {
    #[error("the folder cannot hold the games: {0:?}")]
    Refused(Refusal),
    #[error(transparent)]
    Io(#[from] io::Error),
}

impl From<Refusal> for ChoiceFailure {
    fn from(refusal: Refusal) -> Self {
        Self::Refused(refusal)
    }
}

/// Whether `path` lies on a disk that is not connected: an external disk's
/// folder under /Volumes whose disk is gone.
pub fn disconnected(path: &Path) -> bool {
    let mut components = path.components();
    let (Some(Component::RootDir), Some(Component::Normal(volumes)), Some(Component::Normal(disk))) =
        (components.next(), components.next(), components.next())
    else {
        return false;
    };
    volumes == "Volumes" && !Path::new("/Volumes").join(disk).exists()
}

#[cfg(test)]
mod tests;
