//! Where the Wine prefixes of Windows games live, and whether games share
//! them. A prefix holds a game's Windows environment and its saves, and takes
//! 250 MB to 1 GB: the player may put them on another disk, such as an
//! external one, and may let games without a prefix of their own share one
//! per engine, rather than make one for each.
use crate::{games_folder, supervisor::plain_absolute};
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
};

/// What the player chose, in Obelisk's library; absent for the defaults.
const FILE: &str = "prefixes.json";
/// Obelisk's own folder of prefixes, inside its library.
const DEFAULT_DIRECTORY: &str = "Prefixes";
/// The folder Obelisk makes inside the one the player chooses.
pub const CHOSEN_DIRECTORY: &str = "Obelisk Windows";
/// The name of the prefix games without one of their own share.
pub const SHARED: &str = "Shared";

/// Prefixes are being moved: no Windows game starts meanwhile.
static MOVING: AtomicBool = AtomicBool::new(false);

/// What the player chose for the prefixes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Setting {
    /// The folder that holds them; Obelisk's own when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    /// A game with no prefix of its own uses the one every such game shares,
    /// made for its engine when the first needs it.
    #[serde(default)]
    pub shared: bool,
}

/// Why the prefixes could not move.
#[derive(Debug, thiserror::Error)]
pub enum MoveFailure {
    /// The folder is not an existing one.
    #[error("missing")]
    Missing,
    /// It is inside Obelisk's library, or inside the folder the prefixes leave.
    #[error("inside")]
    Inside,
    /// A prefix of the same name is already there.
    #[error("occupied")]
    Occupied,
    /// A game runs, or prefixes are already moving.
    #[error("busy")]
    Busy,
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// Obelisk's own folder of prefixes.
pub fn default_folder(library: &Path) -> PathBuf {
    library.join(DEFAULT_DIRECTORY)
}

/// What the player chose, or the defaults when nothing valid is kept.
pub fn load(library: &Path) -> Setting {
    let setting: Setting = fs::read(library.join(FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    Setting {
        path: setting
            .path
            .filter(|path| plain_absolute(path) && *path != default_folder(library)),
        ..setting
    }
}

fn save(library: &Path, setting: &Setting) -> io::Result<()> {
    let file = library.join(FILE);
    if *setting == Setting::default() {
        return match fs::remove_file(&file) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        };
    }
    obelisk_core::write_atomic(&file, &serde_json::to_vec(setting)?)
}

/// The folder that holds the prefixes now.
pub fn folder(library: &Path) -> PathBuf {
    load(library)
        .path
        .unwrap_or_else(|| default_folder(library))
}

/// Whether games without a prefix of their own share one.
pub fn shared(library: &Path) -> bool {
    load(library).shared
}

/// Lets games without a prefix of their own share one, or not. The prefixes
/// there are never merged or split: each game keeps what it has.
pub fn set_shared(library: &Path, shared: bool) -> io::Result<()> {
    let setting = load(library);
    save(library, &Setting { shared, ..setting })
}

/// Whether a Windows game cannot start for want of its prefixes: they are
/// moving, or their disk is not connected.
pub fn unavailable(library: &Path) -> bool {
    MOVING.load(Ordering::Acquire) || games_folder::disconnected(&folder(library))
}

/// The prefix, for Rosetta's Wine, of the installation `name` names in
/// `library`'s prefixes: the one it has, else the shared one when the player
/// asked for it, else one of its own. Its Linux engine's prefix sits beside it.
pub fn of(library: &Path, name: &std::ffi::OsStr) -> PathBuf {
    let folder = folder(library);
    let own = folder.join(name);
    let mut linux = name.to_owned();
    linux.push("-linux");
    if shared(library) && !own.exists() && !folder.join(linux).exists() {
        return folder.join(SHARED);
    }
    own
}

/// Whether `prefix`, one of Rosetta's, is the one games share.
pub fn is_shared(prefix: &Path) -> bool {
    prefix.file_name().is_some_and(|name| name == SHARED)
}

/// The size of the prefixes now, in bytes. It walks the folder.
pub fn size(library: &Path) -> u64 {
    fn walk(path: &Path) -> u64 {
        let Ok(entries) = fs::read_dir(path) else {
            return 0;
        };
        entries
            .flatten()
            .map(|entry| match entry.file_type() {
                Ok(kind) if kind.is_dir() => walk(&entry.path()),
                Ok(kind) if kind.is_file() => entry.metadata().map(|m| m.len()).unwrap_or(0),
                _ => 0,
            })
            .sum()
    }
    walk(&folder(library))
}

/// The folder `chosen` gives: itself when it is Obelisk's own, else the one
/// Obelisk makes inside it.
fn destination(library: &Path, chosen: &Path) -> Result<PathBuf, MoveFailure> {
    if !plain_absolute(chosen) {
        return Err(MoveFailure::Missing);
    }
    let chosen = chosen.canonicalize().map_err(|_| MoveFailure::Missing)?;
    if !chosen.is_dir() {
        return Err(MoveFailure::Missing);
    }
    let library = library
        .canonicalize()
        .unwrap_or_else(|_| library.to_owned());
    if chosen == default_folder(&library) {
        return Ok(chosen);
    }
    if chosen.starts_with(&library) {
        return Err(MoveFailure::Inside);
    }
    if chosen
        .file_name()
        .is_some_and(|name| name == CHOSEN_DIRECTORY)
    {
        return Ok(chosen);
    }
    Ok(chosen.join(CHOSEN_DIRECTORY))
}

/// Ends the move: Windows games may start again.
struct Moving;

impl Moving {
    fn begin() -> Result<Self, MoveFailure> {
        MOVING
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| Self)
            .map_err(|_| MoveFailure::Busy)
    }
}

impl Drop for Moving {
    fn drop(&mut self) {
        MOVING.store(false, Ordering::Release);
    }
}

/// Moves every prefix to the folder `chosen` gives, one at a time, and keeps
/// it as the folder of the prefixes from then on. A prefix is copied beside
/// its place, then renamed there, then the original goes, so a failure leaves
/// every game its prefix where it was. Nothing moves while a game runs, which
/// `running` tells.
pub fn relocate(library: &Path, chosen: &Path, running: bool) -> Result<(), MoveFailure> {
    if running {
        return Err(MoveFailure::Busy);
    }
    let to = destination(library, chosen)?;
    let from = folder(library);
    let canonical = |path: &Path| path.canonicalize().unwrap_or_else(|_| path.to_owned());
    if canonical(&to) == canonical(&from) {
        return Ok(());
    }
    if canonical(&to).starts_with(canonical(&from)) {
        return Err(MoveFailure::Inside);
    }
    let _moving = Moving::begin()?;
    fs::create_dir_all(&to)?;
    let names: Vec<_> = match fs::read_dir(&from) {
        Ok(entries) => entries
            .flatten()
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .map(|entry| entry.file_name())
            .collect(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error.into()),
    };
    if names.iter().any(|name| to.join(name).exists()) {
        return Err(MoveFailure::Occupied);
    }
    for name in &names {
        move_one(&from.join(name), &to.join(name))?;
    }
    let setting = load(library);
    let path = (to != default_folder(library)).then_some(to);
    save(library, &Setting { path, ..setting })?;
    // Obelisk's own folder stays; one the player made goes once empty.
    if from != default_folder(library) {
        let _ = fs::remove_dir(&from);
    }
    Ok(())
}

/// Moves the prefix `from` to `to`: a rename on one disk, else a copy that
/// keeps links, permissions and attributes, then the original removed.
fn move_one(from: &Path, to: &Path) -> Result<(), MoveFailure> {
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    let mut staging = to.as_os_str().to_owned();
    staging.push(".obelisk-moving");
    let staging = PathBuf::from(staging);
    let _ = fs::remove_dir_all(&staging);
    let copied = Command::new("/usr/bin/ditto")
        .arg(from)
        .arg(&staging)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if !copied.success() {
        let _ = fs::remove_dir_all(&staging);
        return Err(io::Error::other("ditto failed").into());
    }
    fs::rename(&staging, to)?;
    fs::remove_dir_all(from)?;
    Ok(())
}

#[cfg(test)]
mod tests;
