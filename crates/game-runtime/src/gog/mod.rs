//! GOG game ownership and Game sessions. GOG's builds are DRM free: a launch
//! needs no account credential, only its installation. No Cloud sync runs
//! yet: GOG Cloud saves need the game's own Galaxy credentials.
mod client;
pub mod content;
mod launch;
mod windows;

use crate::runtimes::Runtime;
use crate::supervisor::{self, Record};
pub use crate::supervisor::{FailureReason, Phase};
pub use client::start;
pub use launch::serve;
use obelisk_core::Platform;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("backend.gog.session")]
    Session,
    #[error("backend.gog.busy")]
    Busy,
    #[error("backend.gog.cancelled")]
    Cancelled,
    #[error("backend.gog.storage")]
    Storage,
    #[error("backend.gog.unsupported")]
    Unsupported,
    #[error("backend.runtime.macBuildNotNative")]
    NotNative,
    #[error("backend.runtime.rosettaUnavailable")]
    RosettaUnavailable,
    #[error("backend.runtime.wineUnavailable")]
    WineUnavailable,
    #[error("backend.runtime.prefixesUnavailable")]
    PrefixesUnavailable,
    #[error("{0}")]
    Gog(#[from] gog_kit::GogError),
    #[error("backend.gog.storage")]
    Io(#[from] std::io::Error),
}
supervisor::session_error_conversions!(Error);
pub type Result<T> = std::result::Result<T, Error>;

/// What a GOG Game session starts: one installed build.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    pub home: PathBuf,
    pub install: PathBuf,
    pub session_directory: PathBuf,
    pub helper: PathBuf,
    pub product_id: String,
    #[serde(serialize_with = "gog_kit::client::write_platform")]
    pub platform: Platform,
    /// The game's name in the store, which the Linux engine shows in the
    /// menu bar, the Dock and Cmd-Tab.
    #[serde(default)]
    pub title: String,
    /// Set for a Windows build, which runs in Wine.
    #[serde(default)]
    pub prefix: Option<PathBuf>,
}
impl Configuration {
    /// The game as Obelisk's profiles and logs name it.
    pub(crate) fn profile_game(&self) -> crate::profile::Game<'_> {
        crate::profile::Game::Gog {
            product_id: &self.product_id,
        }
    }
    /// The game as Wine starts a Windows build.
    pub(crate) fn game(&self) -> crate::wine::Game<'_> {
        crate::wine::Game {
            install: &self.install,
            title: &self.title,
        }
    }
    /// Whether `wine` runs this build in its own prefix, and only a Windows
    /// build.
    fn runs_in(&self, wine: Option<&crate::wine::Config>) -> bool {
        self.prefix.as_ref() == wine.map(|wine| &wine.prefix)
    }
    fn validate(&self, account: &str) -> Result<()> {
        // A Mac build runs natively; a Windows build in its prefix.
        let prefix = match (self.platform, &self.prefix) {
            (Platform::Mac, None) => None,
            (Platform::Windows, Some(prefix)) => Some(prefix),
            _ => return Err(Error::Unsupported),
        };
        if [
            &self.home,
            &self.install,
            &self.session_directory,
            &self.helper,
        ]
        .into_iter()
        .chain(prefix)
        .any(|path| !supervisor::plain_absolute(path))
            || !gog_kit::client::valid_product_id(&self.product_id)
            || !gog_kit::auth::valid_user_id(account)
        {
            return Err(Error::Unsupported);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gog;

/// The game a GOG session runs: its product.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Store {
    pub product_id: String,
}

pub type Snapshot = Record<Gog>;
pub type Client = supervisor::Client<Gog>;

impl supervisor::Provider for Gog {
    type Account = String;
    type Store = Store;
    type Control = supervisor::NoControl;
    type Error = Error;
    const SUPERVISE: &'static str = "gog-supervise";
    const CANCELLED: &'static [&'static str] = &["backend.gog.cancelled"];

    fn valid(record: &Record<Self>) -> bool {
        gog_kit::auth::valid_user_id(&record.account_id)
            && gog_kit::client::valid_product_id(&record.store.product_id)
            && record.socket.is_absolute()
    }

    fn cancelled(error: &Error) -> bool {
        matches!(
            error,
            Error::Cancelled | Error::Gog(gog_kit::GogError::Cancelled)
        )
    }

    fn same_game(a: &Store, b: &Store) -> bool {
        a == b
    }

    /// A supervisor that is gone left a game that may still run.
    fn orphaned(record: &mut Record<Self>, owned: bool) {
        if owned {
            record.reconnecting = true;
        }
        record.cloud.allowed_choices.clear();
    }
}

impl crate::sessions::Storefront for Gog {
    type Game = String;
    const BEGINS: bool = false;
    const INVALID_SESSION: &'static str = "backend.gog.invalidSession";
    const SESSION_FAILED: &'static str = "backend.gog.session";
    const RUNTIMES: &'static [Runtime] = &[];
    fn game(store: &Store) -> String {
        store.product_id.clone()
    }
}

/// What the handoff pipe carries to a GOG supervisor. It holds no credential.
#[derive(Serialize, Deserialize)]
struct Request {
    config: Configuration,
    id: String,
    account_id: String,
    /// Built in Obelisk's process: a supervisor starts without any environment.
    #[serde(default)]
    environment: std::collections::HashMap<String, String>,
    /// How a Windows build runs.
    #[serde(default)]
    wine: Option<crate::wine::Config>,
}

/// What a GOG installation names of its game: its product.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Product {
    pub product_id: String,
}

impl crate::installations::Storefront for Gog {
    type Product = Product;
    /// Nothing joins a game through this storefront.
    type Join = std::convert::Infallible;
    const PLATFORM: obelisk_core::platform::Spelling = gog_kit::client::PLATFORM;
    const RECEIPTS_UNAVAILABLE: &'static str = "model.gog.receiptsUnavailable";
    const RECEIPTS_NOT_SAVED: &'static str = "model.gog.receiptsNotSaved";
    const GAMES_DIRECTORY: &'static str = "GOG";
    const QUEUE: crate::queue::Storefront = crate::queue::Storefront::Gog;
    fn product_game(product: &Product) -> String {
        product.product_id.clone()
    }
    fn profile_game(product: &Product) -> crate::profile::Game<'_> {
        crate::profile::Game::Gog {
            product_id: &product.product_id,
        }
    }
    fn receipt_id(product_id: &String, platform: Platform) -> String {
        format!("{product_id}:{}", Self::PLATFORM.name(platform))
    }
    fn transfer_name(product_id: &String) -> String {
        product_id.clone()
    }
}

#[cfg(test)]
mod tests;
