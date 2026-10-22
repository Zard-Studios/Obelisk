use super::*;
use crate::transfer::DownloadPhase;
use std::sync::{Arc, Mutex};

#[derive(Debug, thiserror::Error)]
enum Failure {
    #[error("busy")]
    Busy,
    #[error("cancelled")]
    Cancelled,
    #[error("engine failed")]
    Engine,
    #[error("{0}")]
    Install(InstallFailure),
}

impl From<InstallFailure> for Failure {
    fn from(error: InstallFailure) -> Self {
        Self::Install(error)
    }
}
impl From<Cancelled> for Failure {
    fn from(_: Cancelled) -> Self {
        Self::Cancelled
    }
}
impl From<TransferBusy> for Failure {
    fn from(_: TransferBusy) -> Self {
        Self::Busy
    }
}
impl TransferError for Failure {
    fn is_cancellation(&self) -> bool {
        matches!(self, Self::Cancelled)
    }
}

#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
struct Receipt {
    build: u32,
}

const NAME: &str = "store-7-macos";

fn files(build: u32) -> InstalledFiles<Receipt> {
    InstalledFiles {
        receipt: Receipt { build },
        total_bytes: 30,
        network_bytes: 12,
        content_bytes: 10,
    }
}

/// Installs through the real sequence with an engine that writes one file.
async fn install_build(
    transfers: &Transfers<TransferProgress>,
    parent: &Path,
    build: u32,
    engine: impl FnOnce(&Path) -> Result<(), Failure>,
) -> Result<Receipt, Failure> {
    install(
        transfers,
        "7",
        Destination {
            parent,
            directory: NAME,
        },
        |_, _| async { Ok::<_, Failure>(()) },
        |(), root, _| async move {
            engine(&root)?;
            Ok(files(build))
        },
    )
    .await
}

fn receipt_on_disk(root: &Path) -> Option<Receipt> {
    fs::read(root.join(RECEIPT_FILE))
        .ok()
        .map(|bytes| serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn the_receipt_is_published_after_every_file_with_the_final_progress() {
    let parent = tempfile::tempdir().unwrap();
    let transfers = Transfers::default();
    let root = parent.path().canonicalize().unwrap().join(NAME);
    let receipt = install_build(&transfers, parent.path(), 1, |root| {
        assert!(
            !root.join(RECEIPT_FILE).exists(),
            "no receipt while files change"
        );
        fs::write(root.join("game"), b"files").map_err(|_| Failure::Engine)
    })
    .await
    .unwrap();
    assert_eq!(receipt, Receipt { build: 1 });
    assert_eq!(receipt_on_disk(&root), Some(Receipt { build: 1 }));
    let progress = transfers.snapshot().unwrap();
    assert_eq!(progress.game, "7");
    assert_eq!(progress.state, TransferState::Complete);
    assert_eq!((progress.completed_bytes, progress.total_bytes), (30, 30));
    assert_eq!((progress.network_bytes, progress.content_bytes), (12, 10));
    assert_eq!(progress.message, "backend.transfer.complete");
}

#[tokio::test]
async fn a_failed_update_withdraws_the_old_receipt() {
    let parent = tempfile::tempdir().unwrap();
    let transfers = Transfers::default();
    let root = parent.path().canonicalize().unwrap().join(NAME);
    install_build(&transfers, parent.path(), 1, |_| Ok(()))
        .await
        .unwrap();
    assert!(receipt_on_disk(&root).is_some());

    let failed = install_build(&transfers, parent.path(), 2, |_| Err(Failure::Engine)).await;

    assert!(matches!(failed, Err(Failure::Engine)));
    assert_eq!(
        receipt_on_disk(&root),
        None,
        "a half-updated installation is not launchable"
    );
    let progress = transfers.snapshot().unwrap();
    assert_eq!(progress.state, TransferState::Failed);
    assert_eq!(progress.message, "engine failed");
}

#[tokio::test]
async fn an_engine_cancellation_is_the_transfers_outcome() {
    let parent = tempfile::tempdir().unwrap();
    let transfers = Transfers::default();
    let result = install_build(&transfers, parent.path(), 1, |_| Err(Failure::Cancelled)).await;
    assert!(matches!(result, Err(Failure::Cancelled)));
    assert_eq!(
        transfers.snapshot().unwrap().state,
        TransferState::Cancelled
    );
    assert_eq!(transfers.snapshot().unwrap().message, "cancelled");
}

#[tokio::test]
async fn preparation_runs_before_the_folder_or_its_lock_exists() {
    let parent = tempfile::tempdir().unwrap();
    let transfers = Transfers::default();
    let root = parent.path().canonicalize().unwrap().join(NAME);
    let seen = Arc::new(Mutex::new(None));
    let during = Arc::clone(&seen);
    install(
        &transfers,
        "7",
        Destination {
            parent: parent.path(),
            directory: NAME,
        },
        |prepared_root, _| async move {
            *during.lock().unwrap() = Some((prepared_root.clone(), prepared_root.exists()));
            Ok::<_, Failure>(())
        },
        |(), root, _| async move {
            assert!(
                matches!(lock(&root), Err(InstallFailure::Locked)),
                "files are written under the installation lock"
            );
            assert!(matches!(
                open::<Receipt>(&root),
                Err(InstallFailure::Locked)
            ));
            Ok(files(1))
        },
    )
    .await
    .unwrap();
    assert_eq!(*seen.lock().unwrap(), Some((root.clone(), false)));
    assert!(
        lock(&root).is_ok(),
        "the lock is released with the Transfer"
    );
}

#[tokio::test]
async fn a_cancelled_preparation_touches_no_file() {
    let parent = tempfile::tempdir().unwrap();
    let transfers = Transfers::default();
    let result = install(
        &transfers,
        "7",
        Destination {
            parent: parent.path(),
            directory: NAME,
        },
        |_, cancel| async move {
            cancel.cancel();
            Ok::<_, Failure>(())
        },
        |(), _, _| async { panic!("no file after a cancelled preparation") },
    )
    .await;
    assert!(matches!(result, Err::<Receipt, _>(Failure::Cancelled)));
    assert!(!parent.path().join(NAME).exists());
}

#[tokio::test]
async fn the_destination_is_confined_before_preparation() {
    let parent = tempfile::tempdir().unwrap();
    let transfers = Transfers::default();
    let attempt = |parent: PathBuf, directory: &'static str| {
        let transfers = &transfers;
        async move {
            install(
                transfers,
                "7",
                Destination {
                    parent: &parent,
                    directory,
                },
                |_, _| async { panic!("an unsafe destination is refused before preparation") },
                |(), _, _| async { Ok::<_, Failure>(files(1)) },
            )
            .await
            .map(|_: Receipt| ())
        }
    };
    let missing = parent.path().join("missing");
    assert!(matches!(
        attempt(missing, NAME).await,
        Err(Failure::Install(InstallFailure::ParentMissing))
    ));
    assert!(matches!(
        attempt(PathBuf::from("relative"), NAME).await,
        Err(Failure::Install(InstallFailure::ParentMissing))
    ));
    for directory in ["", ".", "..", "a/b", "a\\b", ".obelisk-content"] {
        assert!(
            matches!(
                attempt(parent.path().to_owned(), directory).await,
                Err(Failure::Install(InstallFailure::UnsafeDirectory(_)))
            ),
            "{directory:?}"
        );
    }
    fs::write(parent.path().join("file"), b"").unwrap();
    assert!(matches!(
        attempt(parent.path().to_owned(), "file").await,
        Err(Failure::Install(InstallFailure::DestinationIsFile(_)))
    ));
    #[cfg(unix)]
    {
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), parent.path().join("link")).unwrap();
        assert!(matches!(
            attempt(parent.path().to_owned(), "link").await,
            Err(Failure::Install(InstallFailure::DestinationIsLink(_)))
        ));
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
    }
}

#[cfg(unix)]
#[tokio::test]
async fn a_linked_lock_or_receipt_is_refused() {
    let parent = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().join("target");
    fs::write(&target, b"unchanged").unwrap();
    let root = parent.path().canonicalize().unwrap().join(NAME);
    fs::create_dir(&root).unwrap();
    std::os::unix::fs::symlink(&target, root.join(LOCK_FILE)).unwrap();
    let transfers = Transfers::default();
    assert!(matches!(
        install_build(&transfers, parent.path(), 1, |_| Ok(())).await,
        Err(Failure::Install(InstallFailure::UnsafeLock(_)))
    ));
    fs::remove_file(root.join(LOCK_FILE)).unwrap();
    fs::hard_link(&target, root.join(RECEIPT_FILE)).unwrap();
    assert!(matches!(
        install_build(&transfers, parent.path(), 1, |_| Ok(())).await,
        Err(Failure::Install(InstallFailure::UnsafeReceipt(_)))
    ));
    fs::remove_file(root.join(RECEIPT_FILE)).unwrap();
    // A link substituted while files are written is refused at publication.
    let substituted = target.clone();
    assert!(matches!(
        install_build(&transfers, parent.path(), 1, move |root| {
            std::os::unix::fs::symlink(&substituted, root.join(RECEIPT_FILE))
                .map_err(|_| Failure::Engine)
        })
        .await,
        Err(Failure::Install(InstallFailure::UnsafeReceipt(_)))
    ));
    assert_eq!(fs::read(&target).unwrap(), b"unchanged");
}

#[tokio::test]
async fn an_installation_opens_with_its_receipt_under_the_lock() {
    let parent = tempfile::tempdir().unwrap();
    let transfers = Transfers::default();
    let root = parent.path().canonicalize().unwrap().join(NAME);
    install_build(&transfers, parent.path(), 3, |_| Ok(()))
        .await
        .unwrap();

    let opened = open::<Receipt>(&parent.path().join(NAME)).unwrap();
    assert_eq!(opened.root, root);
    assert_eq!(opened.receipt, Some(Receipt { build: 3 }));
    assert!(matches!(lock(&root), Err(InstallFailure::Locked)));
    assert_eq!(
        read_receipt::<Receipt>(&root).unwrap(),
        Some(Receipt { build: 3 }),
        "a held installation's receipt still reads"
    );
    let busy = install_build(&transfers, parent.path(), 4, |_| Ok(())).await;
    assert!(matches!(
        busy,
        Err(Failure::Install(InstallFailure::Locked))
    ));
    assert_eq!(
        receipt_on_disk(&root),
        Some(Receipt { build: 3 }),
        "a refused Transfer withdraws nothing"
    );
    drop(opened);

    fs::write(root.join(RECEIPT_FILE), b"not json").unwrap();
    assert_eq!(open::<Receipt>(&root).unwrap().receipt, None);
    assert_eq!(read_receipt::<Receipt>(&root).unwrap(), None);
    assert!(matches!(
        open::<Receipt>(&parent.path().join("missing")),
        Err(InstallFailure::Io(_))
    ));
    assert!(matches!(
        read_receipt::<Receipt>(&parent.path().join("missing")),
        Err(InstallFailure::Io(_))
    ));
}

#[tokio::test]
async fn an_up_to_date_installation_reports_no_downloaded_content() {
    let parent = tempfile::tempdir().unwrap();
    let transfers = Transfers::default();
    install(
        &transfers,
        "7",
        Destination {
            parent: parent.path(),
            directory: NAME,
        },
        |_, _| async { Ok::<_, Failure>(()) },
        |(), _, _| async {
            Ok(InstalledFiles {
                content_bytes: 0,
                ..files(1)
            })
        },
    )
    .await
    .unwrap();
    assert_eq!(
        transfers.snapshot().unwrap().message,
        "backend.transfer.upToDate"
    );
}

#[test]
fn show_uses_inspection_bytes_while_files_are_inspected_and_clears_the_message() {
    let mut progress = TransferProgress::new("7");
    let snapshot = DownloadProgress {
        phase: DownloadPhase::Verifying,
        completed_bytes: 10,
        verification_bytes: Some(40),
        total_bytes: 100,
        network_bytes: 3,
        content_bytes: 2,
        current_file: "Game/data.pak".into(),
        ..DownloadProgress::default()
    };
    progress.show(snapshot);
    assert_eq!(progress.state, TransferState::Verifying);
    assert_eq!((progress.completed_bytes, progress.total_bytes), (40, 100));
    assert_eq!((progress.network_bytes, progress.content_bytes), (3, 2));
    assert_eq!(progress.current_file, "Game/data.pak");
    assert_eq!(
        progress.message, "",
        "a storefront labels the Transfer itself"
    );

    progress.show(DownloadProgress {
        phase: DownloadPhase::Downloading,
        completed_bytes: 25,
        total_bytes: 100,
        ..DownloadProgress::default()
    });
    assert_eq!(progress.state, TransferState::Downloading);
    assert_eq!(
        progress.completed_bytes, 25,
        "repair reports completed bytes"
    );
    assert_eq!(progress.current_file, "");
    assert_eq!(progress.game, "7");
}

fn installed(parent: &Path, name: &str) -> PathBuf {
    let directory = parent.join(name);
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("game.bin"), b"installed content").unwrap();
    directory
}

#[test]
fn removal_deletes_only_the_dedicated_installation() {
    let parent = tempfile::tempdir().unwrap();
    let game = installed(parent.path(), "steam-620-macos");
    let sibling = installed(parent.path(), "steam-570-macos");
    remove(&game, "steam-620-macos", &[parent.path()]).unwrap();
    assert!(!game.exists());
    assert!(sibling.join("game.bin").exists());
}

#[test]
fn removal_refuses_other_names_protected_folders_and_short_paths() {
    let parent = tempfile::tempdir().unwrap();
    let game = installed(parent.path(), "Imported folder");
    for (directory, expected) in [
        (game.as_path(), "steam-620-macos"),
        (game.as_path(), ""),
        (game.as_path(), "a/b"),
        (Path::new("relative/steam-620-macos"), "steam-620-macos"),
    ] {
        assert!(
            matches!(
                remove(directory, expected, &[]),
                Err(RemovalFailure::Unsafe(_))
            ),
            "{directory:?} {expected:?}"
        );
    }
    let library = parent.path().join("library");
    fs::create_dir_all(library.join("inside")).unwrap();
    assert!(matches!(
        remove(&library, "library", &[&library.join("inside")]),
        Err(RemovalFailure::Unsafe(_))
    ));
    assert!(matches!(
        remove(Path::new("/"), "", &[]),
        Err(RemovalFailure::Unsafe(_))
    ));
    assert!(matches!(
        remove(Path::new("/tmp"), "tmp", &[]),
        Err(RemovalFailure::Unsafe(_))
    ));
    assert!(game.join("game.bin").exists());
    assert!(library.join("inside").exists());
}

#[cfg(unix)]
#[test]
fn removal_refuses_links_to_the_folder_but_never_follows_links_inside() {
    let parent = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let target = installed(outside.path(), "steam-620-macos");
    let link = parent.path().join("steam-620-macos");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert!(matches!(
        remove(&link, "steam-620-macos", &[]),
        Err(RemovalFailure::Unsafe(_))
    ));
    let redirect = parent.path().join("redirect");
    std::os::unix::fs::symlink(outside.path(), &redirect).unwrap();
    assert!(matches!(
        remove(&redirect.join("steam-620-macos"), "steam-620-macos", &[]),
        Err(RemovalFailure::Unsafe(_))
    ));
    fs::remove_file(&link).unwrap();
    let game = installed(parent.path(), "steam-620-macos");
    std::os::unix::fs::symlink(outside.path(), game.join("external")).unwrap();
    remove(&game, "steam-620-macos", &[]).unwrap();
    assert!(!game.exists());
    assert!(target.join("game.bin").exists());
}

#[test]
fn removal_waits_for_the_installation_lock() {
    let parent = tempfile::tempdir().unwrap();
    let game = installed(parent.path(), "steam-620-macos");
    let held = lock(&game).unwrap();
    assert!(matches!(
        remove(&game, "steam-620-macos", &[]),
        Err(RemovalFailure::Busy)
    ));
    assert!(game.join("game.bin").exists());
    drop(held);
    remove(&game, "steam-620-macos", &[]).unwrap();
    assert!(!game.exists());
}

#[test]
fn an_unfinished_removal_keeps_an_installation_and_finds_no_folder_to_remove() {
    let parent = tempfile::tempdir().unwrap();
    let game = installed(parent.path(), "steam-620-macos");
    write_receipt(&game.canonicalize().unwrap(), &Receipt { build: 1 }).unwrap();
    assert!(!remove_unfinished(&game, "steam-620-macos", &[]).unwrap());
    assert!(game.join("game.bin").exists());

    fs::remove_file(game.join(RECEIPT_FILE)).unwrap();
    assert!(remove_unfinished(&game, "steam-620-macos", &[]).unwrap());
    assert!(!game.exists());
    assert!(!remove_unfinished(&game, "steam-620-macos", &[]).unwrap());
}

#[test]
fn removal_accepts_both_spellings_of_the_system_temporary_folder() {
    let component = format!("ObeliskRemovalAliases-{}", std::process::id());
    let temporary = std::env::temp_dir();
    let temporary = temporary.to_string_lossy().trim_end_matches('/').to_owned();
    let private = if temporary.starts_with("/var/") {
        format!("/private{temporary}")
    } else {
        temporary.clone()
    };
    for parent in [
        "/tmp".to_owned(),
        "/private/tmp".to_owned(),
        temporary,
        private,
    ] {
        let root = Path::new(&parent).join(&component);
        let game = installed(&root, "steam-620-macos");
        remove(&game, "steam-620-macos", &[&root]).unwrap();
        assert!(!game.exists(), "{parent}");
        let _ = fs::remove_dir_all(&root);
    }
}

/// An installation with its receipt, a nested file and a link inside it.
fn complete(parent: &Path, name: &str) -> PathBuf {
    let game = installed(parent, name);
    fs::create_dir_all(game.join("Contents/MacOS")).unwrap();
    fs::write(game.join("Contents/MacOS/game"), b"binary").unwrap();
    std::os::unix::fs::symlink("Contents/MacOS/game", game.join("launcher")).unwrap();
    let receipt = serde_json::json!({ "appId": 620, "directory": game.to_string_lossy() });
    fs::write(game.join(RECEIPT_FILE), receipt.to_string()).unwrap();
    game
}

fn recorded_directory(game: &Path) -> String {
    let receipt: serde_json::Value =
        serde_json::from_slice(&fs::read(game.join(RECEIPT_FILE)).unwrap()).unwrap();
    receipt["directory"].as_str().unwrap().to_owned()
}

#[test]
fn an_installation_moves_whole_and_its_receipt_names_its_new_place() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    let game = complete(&root.join("internal"), "steam-620-macos");
    let external = root.join("external");
    fs::create_dir_all(&external).unwrap();
    let mut heard = Vec::new();
    let moved = relocate(
        &game,
        &external,
        "steam-620-macos",
        &[&root],
        &CancelToken::new(),
        |copied, total| heard.push((copied, total)),
    )
    .unwrap();
    let destination = external.join("steam-620-macos");
    assert_eq!(moved.directory, destination);
    assert!(moved.original_removed && !game.exists());
    assert!(heard.is_empty(), "the same disk only renames the folder");
    assert_eq!(fs::read(destination.join("launcher")).unwrap(), b"binary");
    assert_eq!(
        recorded_directory(&destination),
        destination.to_string_lossy()
    );
    assert!(lock(&destination).is_ok(), "no lock stays held");
}

#[test]
fn a_copy_to_another_disk_keeps_links_modes_and_times_and_reports_its_bytes() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    let game = complete(&root.join("internal"), "steam-620-macos");
    let binary = game.join("Contents/MacOS/game");
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
    let modified =
        std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
    File::options()
        .write(true)
        .open(&binary)
        .unwrap()
        .set_modified(modified)
        .unwrap();
    let held = lock(&game).unwrap();
    let destination = root.join("external/steam-620-macos");
    fs::create_dir_all(destination.parent().unwrap()).unwrap();
    let mut heard = Vec::new();
    let removed = copy_across(
        &game,
        &destination,
        &CancelToken::new(),
        &mut |copied, total| heard.push((copied, total)),
    )
    .unwrap();
    drop(held);
    assert!(removed && !game.exists());
    let link = destination.join("launcher");
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::read_link(&link).unwrap(),
        Path::new("Contents/MacOS/game")
    );
    let copied = fs::metadata(destination.join("Contents/MacOS/game")).unwrap();
    assert_eq!(copied.permissions().mode() & 0o777, 0o755);
    assert_eq!(copied.modified().unwrap(), modified);
    assert!(
        !destination.join(LOCK_FILE).exists(),
        "the lock stays behind"
    );
    assert_eq!(heard.first(), Some(&(0, heard[0].1)));
    let &(copied, total) = heard.last().unwrap();
    assert_eq!(copied, total);
    assert!(total > 0);
    assert!(heard.windows(2).all(|pair| pair[0].0 <= pair[1].0));
    assert!(!staging(destination.parent().unwrap(), "steam-620-macos").exists());
}

#[test]
fn a_cancelled_copy_leaves_nothing_and_keeps_the_original() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    let game = complete(&root.join("internal"), "steam-620-macos");
    let destination = root.join("external/steam-620-macos");
    fs::create_dir_all(destination.parent().unwrap()).unwrap();
    let cancel = CancelToken::new();
    cancel.cancel();
    let outcome = copy_across(&game, &destination, &cancel, &mut |_, _| {});
    assert!(matches!(outcome, Err(RelocationFailure::Cancelled)));
    assert!(game.join("Contents/MacOS/game").exists());
    assert_eq!(
        fs::read_dir(destination.parent().unwrap()).unwrap().count(),
        0
    );
}

#[test]
fn a_copy_replaces_the_staging_folder_an_interrupted_move_left() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    let game = complete(&root.join("internal"), "steam-620-macos");
    let destination = root.join("external/steam-620-macos");
    let left = staging(destination.parent().unwrap(), "steam-620-macos");
    fs::create_dir_all(left.join("half")).unwrap();
    copy_across(&game, &destination, &CancelToken::new(), &mut |_, _| {}).unwrap();
    assert!(!left.exists());
    assert!(!destination.join("half").exists());
    assert!(destination.join("Contents/MacOS/game").exists());
}

#[test]
fn a_move_refuses_an_occupied_destination_and_any_other_folder() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    let game = complete(&root.join("internal"), "steam-620-macos");
    let external = root.join("external");
    fs::create_dir_all(external.join("steam-620-macos")).unwrap();
    let cancel = CancelToken::new();
    assert!(matches!(
        relocate(&game, &external, "steam-620-macos", &[], &cancel, |_, _| {}),
        Err(RelocationFailure::Occupied(_))
    ));
    assert!(matches!(
        relocate(&game, &external, "steam-570-macos", &[], &cancel, |_, _| {}),
        Err(RelocationFailure::Unsafe(_))
    ));
    assert!(matches!(
        relocate(
            &game,
            &external,
            "steam-620-macos",
            &[&game],
            &cancel,
            |_, _| {}
        ),
        Err(RelocationFailure::Unsafe(_))
    ));
    assert!(matches!(
        relocate(
            &game,
            game.parent().unwrap(),
            "steam-620-macos",
            &[],
            &cancel,
            |_, _| {}
        ),
        Err(RelocationFailure::Unsafe(_))
    ));
    let held = lock(&game).unwrap();
    fs::remove_dir_all(external.join("steam-620-macos")).unwrap();
    assert!(matches!(
        relocate(&game, &external, "steam-620-macos", &[], &cancel, |_, _| {}),
        Err(RelocationFailure::Busy)
    ));
    drop(held);
    assert!(game.join("game.bin").exists());
}

#[test]
fn a_rebased_receipt_keeps_its_other_fields() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    let game = complete(&root.join("internal"), "steam-620-macos");
    let moved = root.join("elsewhere/steam-620-macos");
    fs::create_dir_all(moved.parent().unwrap()).unwrap();
    fs::rename(&game, &moved).unwrap();
    rebase_receipt(&moved).unwrap();
    let receipt: serde_json::Value =
        serde_json::from_slice(&fs::read(moved.join(RECEIPT_FILE)).unwrap()).unwrap();
    assert_eq!(receipt["directory"], moved.to_string_lossy().as_ref());
    assert_eq!(receipt["appId"], 620);
}
