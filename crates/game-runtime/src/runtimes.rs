//! The runtimes a game needs beside its own files: Voile for a build that
//! runs in Wine, Valve's Steam components for every Steam game. A Transfer
//! gets the missing ones before the game's files. A launch that still misses
//! one gets it after its Admission, before its game is prepared. Each shows
//! the download its own way. A launch that finds a newer Voile release than
//! the one in place asks the player first, with what it brings: to take it
//! before the game starts, or to start in the release in place, unless the
//! game's profile needs the newer one. A Voile release a newer one replaced
//! goes only when a launch finds no Game session, of any storefront, that may
//! run a game from it.
use crate::{
    installations::blocking,
    sessions::{self, BoxFuture, Failure, failed},
};
use obelisk_core::{Platform, transfer::CancelToken};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::oneshot;

/// How often a download under way shows its progress.
const PACE: Duration = Duration::from_millis(250);

/// A runtime a build needs to launch, beside its own files.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Runtime {
    /// Voile, the Wine runtime Windows games run in.
    Voile,
    /// Valve's Steam components, which Obelisk's Steam host loads.
    SteamComponents,
}

/// What a launch makes of a runtime in place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Standing {
    /// In place, and nothing newer is offered.
    Current,
    /// In place, while a newer release is offered: the player takes it
    /// before the game starts, or skips it unless the game needs it.
    Outdated(Update),
    /// Not in place: the launch waits for it.
    Missing,
}

/// A newer Voile release a launch offers the player before its game starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Update {
    /// The release offered.
    pub version: String,
    /// The game needs it: the player takes it, or the launch stops.
    pub required: bool,
    /// What each release newer than the one in place brings, up to the one
    /// offered, newest first.
    pub releases: Vec<crate::voile::Notes>,
}

/// The player's answer to a newer Voile release a launch offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    /// Download it, then start the game in it.
    Update,
    /// Start the game in the release in place: the next launch offers the
    /// newer one again.
    Skip,
    /// Stop the launch.
    Cancel,
}

/// How much of a runtime has downloaded: the bytes received, and the total.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Progress {
    pub runtime: Runtime,
    pub received: u64,
    pub total: u64,
}

/// One runtime's download, which the one waiting for it runs, cancels and
/// watches.
pub(crate) trait RuntimeDownload: Send + Sync {
    /// The bytes received so far, and the total.
    fn progress(&self) -> (u64, u64);
    fn cancel(&self);
    /// A failure is the runtime's own localization key.
    fn run(&self) -> BoxFuture<'_, Result<(), Failure>>;
}

/// Where the runtimes are: where one stands, its download, what decides
/// which release a launch wants, and the Voile releases a newer one
/// replaced. Tests stand in for it.
pub(crate) trait RuntimeSource: Send + Sync {
    /// Where `runtime` stands for a game whose profile needs `voile` or a
    /// newer Voile release. It reads a few files, never a whole runtime.
    fn standing(&self, runtime: Runtime, voile: Option<&str>) -> Standing;
    /// The download of the release a launch wants: for Voile, the newest its
    /// feed offers once read again.
    fn download(&self, runtime: Runtime) -> Arc<dyn RuntimeDownload>;
    /// Reads anew what launches decide on: Voile's feed and the game
    /// profiles. Without the network, it soon gives up and keeps the last.
    fn refresh(&self) -> BoxFuture<'_, ()>;
    /// Removes the Voile releases a newer one replaced. What cannot go now
    /// goes with a later launch.
    fn remove_replaced(&self);
}

/// The process's runtimes, which every storefront's Transfers and launches
/// share.
pub struct Runtimes {
    source: Arc<dyn RuntimeSource>,
    /// Whether each storefront's Game sessions may run a game now.
    sessions: Mutex<Vec<Box<dyn Fn() -> bool + Send + Sync>>>,
}

impl Runtimes {
    /// Gets the runtimes into the places `locations` names, where launches
    /// read them.
    pub fn new(locations: &crate::Locations) -> Arc<Self> {
        Self::from_source(Arc::new(Downloads(locations.clone())))
    }

    pub(crate) fn from_source(source: Arc<dyn RuntimeSource>) -> Arc<Self> {
        Arc::new(Self {
            source,
            sessions: Mutex::new(Vec::new()),
        })
    }

    /// Counts a storefront's Game sessions, which `may_run` tells about,
    /// before a launch removes a replaced release.
    pub(crate) fn watch_sessions(&self, may_run: impl Fn() -> bool + Send + Sync + 'static) {
        self.sessions.lock().unwrap().push(Box::new(may_run));
    }

    /// Reads anew what a launch decides on, before its game's profile is
    /// read: Voile's feed and the game profiles.
    pub(crate) async fn refresh_before_launch(&self) {
        self.source.refresh().await;
    }

    /// Gets each runtime a build of `platform` from P needs that is missing,
    /// in order, the newest Voile release offered when Voile is. `shown` gets
    /// each download's progress when it starts and every `PACE`. Cancelling
    /// `cancel` stops the download under way and any after it. Never removes
    /// a release.
    pub(crate) async fn get<P: sessions::Storefront>(
        &self,
        platform: Platform,
        cancel: &CancelToken,
        shown: impl Fn(Progress) + Send,
    ) -> Result<(), Failure> {
        for runtime in needed::<P>(platform) {
            if matches!(self.standing(runtime, None).await?, Standing::Missing) {
                self.download(runtime, cancel, &shown).await?;
            }
        }
        match cancel.is_cancelled() {
            true => Err(Failure::Cancelled),
            false => Ok(()),
        }
    }

    /// Gets the runtimes as `get` does, for a game whose profile needs
    /// `voile` or a newer Voile release. A newer release than the one in
    /// place goes to `ask` first, and downloads only once the answer its
    /// receiver gets takes it; Cancel, or no answer, cancels the launch.
    /// Then, for a build that needs Voile, removes the releases a
    /// newer one replaced, unless the Game sessions of a storefront may run a
    /// game. A failed or cancelled launch removes nothing.
    pub(crate) async fn get_for_launch<P: sessions::Storefront>(
        &self,
        platform: Platform,
        voile: Option<&'static str>,
        cancel: &CancelToken,
        shown: impl Fn(Progress) + Send,
        ask: impl Fn(Update) -> oneshot::Receiver<Answer> + Send,
    ) -> Result<(), Failure> {
        for runtime in needed::<P>(platform) {
            let take = match self.standing(runtime, voile).await? {
                Standing::Current => false,
                Standing::Missing => true,
                Standing::Outdated(update) => {
                    answered(ask(update), cancel).await? == Answer::Update
                }
            };
            if take {
                self.download(runtime, cancel, &shown).await?;
            }
        }
        if cancel.is_cancelled() {
            return Err(Failure::Cancelled);
        }
        if needed::<P>(platform).contains(&Runtime::Voile) && !self.may_run() {
            let source = self.source.clone();
            let _ = tokio::task::spawn_blocking(move || source.remove_replaced()).await;
        }
        Ok(())
    }

    async fn standing(
        &self,
        runtime: Runtime,
        voile: Option<&'static str>,
    ) -> Result<Standing, Failure> {
        let source = self.source.clone();
        blocking(storage_failed(runtime), move || {
            source.standing(runtime, voile)
        })
        .await
    }

    /// Downloads `runtime`, showing its progress through `shown`, until it
    /// ends or `cancel` stops it.
    async fn download(
        &self,
        runtime: Runtime,
        cancel: &CancelToken,
        shown: &(impl Fn(Progress) + Send),
    ) -> Result<(), Failure> {
        if cancel.is_cancelled() {
            return Err(Failure::Cancelled);
        }
        let download = self.source.download(runtime);
        let mut run = download.run();
        let outcome = loop {
            let (received, total) = download.progress();
            shown(Progress {
                runtime,
                received,
                total,
            });
            tokio::select! {
                outcome = &mut run => break outcome,
                () = cancel.cancelled() => {
                    download.cancel();
                    // The install cleans up before the wait ends.
                    break (&mut run).await;
                }
                () = tokio::time::sleep(PACE) => {}
            }
        };
        match outcome {
            Err(_) if cancel.is_cancelled() => Err(Failure::Cancelled),
            outcome => outcome,
        }
    }

    /// Whether the Game sessions of a storefront may run a game now.
    fn may_run(&self) -> bool {
        self.sessions
            .lock()
            .unwrap()
            .iter()
            .any(|may_run| may_run())
    }
}

/// The player's answer `answer` gets, unless `cancel` stops the launch first.
/// Cancel, or a launch that ended without an answer, cancels it.
async fn answered(
    answer: oneshot::Receiver<Answer>,
    cancel: &CancelToken,
) -> Result<Answer, Failure> {
    match cancel.race(answer).await {
        Ok(Ok(answer)) if answer != Answer::Cancel => Ok(answer),
        _ => Err(Failure::Cancelled),
    }
}

/// The runtimes a build of `platform` from P needs, in the order they are
/// got: the storefront's own, then Voile for a build that runs in Wine.
fn needed<P: sessions::Storefront>(platform: Platform) -> Vec<Runtime> {
    let mut needed = P::RUNTIMES.to_vec();
    if platform == Platform::Windows {
        needed.push(Runtime::Voile);
    }
    needed
}

fn storage_failed(runtime: Runtime) -> String {
    match runtime {
        Runtime::Voile => crate::voile::Failure::Storage.to_string(),
        Runtime::SteamComponents => crate::valve::Failure::Storage.to_string(),
    }
}

/// The runtimes Obelisk downloads, in the places its launches read them.
struct Downloads(crate::Locations);

impl RuntimeSource for Downloads {
    fn standing(&self, runtime: Runtime, voile: Option<&str>) -> Standing {
        match runtime {
            Runtime::Voile => crate::voile::standing(&self.0.voile, voile),
            Runtime::SteamComponents => {
                match crate::valve::installed(&self.0.steam_runtime_package) {
                    true => Standing::Current,
                    false => Standing::Missing,
                }
            }
        }
    }
    fn download(&self, runtime: Runtime) -> Arc<dyn RuntimeDownload> {
        match runtime {
            Runtime::Voile => Arc::new(Download {
                install: crate::voile::Install::new(),
                path: self.0.voile.clone(),
            }),
            Runtime::SteamComponents => Arc::new(Download {
                install: crate::valve::Install::new(),
                path: self.0.steam_runtime_package.clone(),
            }),
        }
    }
    fn refresh(&self) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            tokio::join!(
                crate::voile::refresh(&self.0.voile),
                crate::profile::refresh(&self.0.library),
            );
        })
    }
    fn remove_replaced(&self) {
        crate::voile::remove_replaced(&self.0.voile);
    }
}

/// One runtime's install into `path`.
struct Download<I> {
    install: I,
    path: PathBuf,
}

impl RuntimeDownload for Download<crate::voile::Install> {
    fn progress(&self) -> (u64, u64) {
        self.install.progress()
    }
    fn cancel(&self) {
        self.install.cancel();
    }
    fn run(&self) -> BoxFuture<'_, Result<(), Failure>> {
        Box::pin(async move {
            let archive = development_archive();
            self.install
                .run(&self.path, archive.as_deref())
                .await
                .map(drop)
                .map_err(Into::into)
        })
    }
}

impl RuntimeDownload for Download<crate::valve::Install> {
    fn progress(&self) -> (u64, u64) {
        self.install.progress()
    }
    fn cancel(&self) {
        self.install.cancel();
    }
    fn run(&self) -> BoxFuture<'_, Result<(), Failure>> {
        Box::pin(async move { self.install.run(&self.path).await.map_err(Into::into) })
    }
}

/// A local copy of the Voile release Obelisk was built with, which replaces
/// its download in development builds.
fn development_archive() -> Option<PathBuf> {
    if !cfg!(debug_assertions) {
        return None;
    }
    std::env::var_os("OBELISK_VOILE_ARCHIVE")
        .filter(|archive| !archive.is_empty())
        .map(Into::into)
}

impl From<crate::voile::Failure> for Failure {
    fn from(failure: crate::voile::Failure) -> Self {
        match failure {
            crate::voile::Failure::Cancelled => Self::Cancelled,
            failure => failed(failure),
        }
    }
}

impl From<crate::valve::Failure> for Failure {
    fn from(failure: crate::valve::Failure) -> Self {
        match failure {
            crate::valve::Failure::Cancelled => Self::Cancelled,
            failure => failed(failure),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;
