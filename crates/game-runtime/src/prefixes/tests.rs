use super::*;
use std::ffi::OsStr;

/// A library and another place, both canonical.
struct Places {
    _root: tempfile::TempDir,
    library: PathBuf,
    elsewhere: PathBuf,
}

fn places() -> Places {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().canonicalize().unwrap();
    let (library, elsewhere) = (path.join("library"), path.join("elsewhere"));
    fs::create_dir_all(&library).unwrap();
    fs::create_dir_all(&elsewhere).unwrap();
    Places {
        _root: root,
        library,
        elsewhere,
    }
}

/// A prefix named `name` in `folder`, holding one file, and a link as Wine's
/// own do.
fn prefix(folder: &Path, name: &str) -> PathBuf {
    let prefix = folder.join(name);
    fs::create_dir_all(prefix.join("drive_c/users")).unwrap();
    fs::write(prefix.join("system.reg"), name).unwrap();
    std::os::unix::fs::symlink("drive_c", prefix.join("dosdevices")).unwrap();
    prefix
}

#[test]
fn prefixes_are_in_obelisk_s_library_and_unshared_until_the_player_chooses() {
    let places = places();
    assert_eq!(folder(&places.library), places.library.join("Prefixes"));
    assert!(!shared(&places.library));
    assert_eq!(
        of(&places.library, OsStr::new("steam-620-windows")),
        places.library.join("Prefixes/steam-620-windows")
    );
}

#[test]
fn a_game_without_a_prefix_shares_one_when_asked_and_a_game_with_one_keeps_it() {
    let places = places();
    let folder = folder(&places.library);
    prefix(&folder, "steam-620-windows");
    fs::create_dir_all(folder.join("epic-game-linux")).unwrap();
    set_shared(&places.library, true).unwrap();
    assert!(shared(&places.library));

    let own = of(&places.library, OsStr::new("steam-620-windows"));
    assert_eq!(own, folder.join("steam-620-windows"));
    assert!(!is_shared(&own));
    // Its Linux engine's prefix alone is a prefix of its own.
    assert_eq!(
        of(&places.library, OsStr::new("epic-game")),
        folder.join("epic-game")
    );
    let other = of(&places.library, OsStr::new("steam-730-windows"));
    assert_eq!(other, folder.join(SHARED));
    assert!(is_shared(&other));

    set_shared(&places.library, false).unwrap();
    assert!(!places.library.join(FILE).exists(), "nothing left to keep");
    assert_eq!(
        of(&places.library, OsStr::new("steam-730-windows")),
        folder.join("steam-730-windows")
    );
}

#[test]
fn the_prefixes_move_to_a_chosen_folder_with_their_links_and_stay_there() {
    let places = places();
    let from = folder(&places.library);
    prefix(&from, "steam-620-windows");
    prefix(&from, "steam-620-windows-linux");
    assert_eq!(
        size(&places.library),
        ("steam-620-windows".len() + "steam-620-windows-linux".len()) as u64
    );

    relocate(&places.library, &places.elsewhere, false).unwrap();
    let to = places.elsewhere.join(CHOSEN_DIRECTORY);
    assert_eq!(folder(&places.library), to);
    assert_eq!(
        fs::read_to_string(to.join("steam-620-windows/system.reg")).unwrap(),
        "steam-620-windows"
    );
    assert_eq!(
        fs::read_link(to.join("steam-620-windows/dosdevices")).unwrap(),
        Path::new("drive_c")
    );
    assert!(to.join("steam-620-windows-linux").is_dir());
    assert!(!from.join("steam-620-windows").exists());
    assert_eq!(
        of(&places.library, OsStr::new("steam-620-windows")),
        to.join("steam-620-windows")
    );

    // Back to Obelisk's own folder forgets the choice.
    relocate(&places.library, &from, false).unwrap();
    assert_eq!(folder(&places.library), from);
    assert!(from.join("steam-620-windows/system.reg").is_file());
    assert!(!places.library.join(FILE).exists());
}

#[test]
fn nothing_moves_while_a_game_runs_into_a_folder_that_cannot_hold_them_or_over_a_prefix() {
    let places = places();
    let from = folder(&places.library);
    prefix(&from, "steam-620-windows");
    assert!(matches!(
        relocate(&places.library, &places.elsewhere, true),
        Err(MoveFailure::Busy)
    ));
    assert!(matches!(
        relocate(&places.library, &places.elsewhere.join("missing"), false),
        Err(MoveFailure::Missing)
    ));
    let inside = places.library.join("Somewhere");
    fs::create_dir_all(&inside).unwrap();
    assert!(matches!(
        relocate(&places.library, &inside, false),
        Err(MoveFailure::Inside)
    ));
    // A prefix of the same name already there is never overwritten.
    prefix(
        &places.elsewhere.join(CHOSEN_DIRECTORY),
        "steam-620-windows",
    );
    assert!(matches!(
        relocate(&places.library, &places.elsewhere, false),
        Err(MoveFailure::Occupied)
    ));
    assert!(from.join("steam-620-windows/system.reg").is_file());
    assert_eq!(folder(&places.library), from);
    assert!(!unavailable(&places.library), "the failed move ended");
}

#[test]
fn a_prefixes_disk_that_is_not_connected_makes_them_unavailable() {
    let places = places();
    assert!(!unavailable(&places.library));
    fs::write(
        places.library.join(FILE),
        serde_json::to_vec(&serde_json::json!({
            "path": "/Volumes/Obelisk Test Disk That Is Not There/Obelisk Windows"
        }))
        .unwrap(),
    )
    .unwrap();
    assert!(unavailable(&places.library));
}

#[test]
fn a_kept_choice_that_cannot_be_read_or_names_a_relative_folder_means_the_defaults() {
    let places = places();
    fs::write(places.library.join(FILE), b"not json").unwrap();
    assert_eq!(load(&places.library), Setting::default());
    fs::write(
        places.library.join(FILE),
        serde_json::to_vec(&serde_json::json!({ "path": "relative/folder", "shared": true }))
            .unwrap(),
    )
    .unwrap();
    let setting = load(&places.library);
    assert_eq!(setting.path, None);
    assert!(setting.shared, "the rest of the choice stays");
}
