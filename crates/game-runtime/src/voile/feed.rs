//! The releases Voile offers, as `voile-feed.json` on Voile's `main` lists
//! them: each one's archive, the contract it keeps with Obelisk, what it
//! brings, and whether players should leave it. Voile's Feed workflow writes
//! it when a release is published and signs it. Obelisk reads it as a
//! Windows game launches, and keeps the last one whose signature it checked
//! beside the runtimes.
use super::Release;
use aws_lc_rs::signature::{ED25519, UnparsedPublicKey};
use base64::Engine as _;
use serde::Deserialize;
use std::{fs, path::Path};

/// The feed, and its signature beside it with `.sig` added.
const URL: &str = "https://raw.githubusercontent.com/Zagrios/Voile/main/voile-feed.json";
/// The public key the feed is signed with, made by Voile's
/// scripts/feed-key.sh: the base64 of its 32 bytes.
const KEY: &str = "ZUQNaLpt7otj1aDIN1adX8VxRSGoidNqCUbILs431oE=";
/// The feed's format this Obelisk reads. Another one is a feed it ignores.
const FORMAT: u32 = 1;
/// The last feed Obelisk checked, in the runtimes' folder.
pub(super) const KEPT: &str = "feed.json";

#[derive(Debug, Deserialize)]
struct Feed {
    format: u32,
    releases: Vec<Offered>,
}

/// A release the feed offers.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub(super) struct Offered {
    /// What it keeps of what Obelisk relies on: Obelisk runs only a release
    /// of its own contract.
    pub contract: u32,
    #[serde(flatten)]
    pub release: Release,
    /// What it brings, in Markdown, as its GitHub release tells it. The
    /// entries written before the feed carried them have none.
    #[serde(default)]
    pub notes: String,
    /// Why players should leave it, once it is withdrawn.
    pub withdrawn: Option<String>,
}

/// The releases the last checked feed in `root` offers, or none.
pub(super) fn offered(root: &Path) -> Vec<Offered> {
    fs::read(root.join(KEPT))
        .ok()
        .and_then(|feed| releases(&feed))
        .unwrap_or_default()
}

/// Reads the feed again and keeps it in `root` once its signature holds.
/// Without the network, or with a feed it cannot check, the last one stays.
pub(crate) async fn refresh(root: &Path) {
    let signature = format!("{URL}.sig");
    let (feed, signature) = tokio::join!(
        crate::remote::fetched(URL),
        crate::remote::fetched(&signature)
    );
    if let (Some(feed), Some(signature)) = (feed, signature)
        && signed(&feed, &signature, KEY)
        && releases(&feed).is_some()
    {
        let _ = crate::remote::keep(&root.join(KEPT), &feed);
    }
}

/// Whether `signature`, the base64 of an Ed25519 signature, signs `feed`
/// with `key`.
fn signed(feed: &[u8], signature: &[u8], key: &str) -> bool {
    let base64 = base64::engine::general_purpose::STANDARD;
    let (Ok(key), Ok(signature)) = (base64.decode(key), base64.decode(signature.trim_ascii()))
    else {
        return false;
    };
    UnparsedPublicKey::new(&ED25519, key)
        .verify(feed, &signature)
        .is_ok()
}

/// The releases `feed` offers, when it is a feed of the format Obelisk
/// reads.
fn releases(feed: &[u8]) -> Option<Vec<Offered>> {
    let feed: Feed = serde_json::from_slice(feed).ok()?;
    (feed.format == FORMAT).then_some(feed.releases)
}

#[cfg(test)]
mod tests;
