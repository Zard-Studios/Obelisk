//! What Obelisk tells players about particular games on their page, such as
//! what works and what does not yet: a game's note, whatever its storefront
//! or build. The table is `notes.json`, apart from the profiles so that an
//! Obelisk that reads only profiles never meets its keys. Obelisk is built
//! with it, and a release build reads it again from Obelisk's `main` when it
//! starts, keeping the last one beside its library; a development build
//! keeps the one it was built with. A table that does not parse shows no
//! note, and an entry that is not a note Obelisk can show is left out.
use crate::profile::{EpicGame, Game};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    sync::{LazyLock, Mutex},
};

const TABLE: &str = include_str!("../notes.json");
/// Where the table is on Obelisk's `main`.
const URL: &str =
    "https://raw.githubusercontent.com/Zagrios/Obelisk/main/crates/game-runtime/notes.json";
/// The table's format this Obelisk reads. Another one is a table it ignores.
const FORMAT: u32 = 1;
/// The last table read, in Obelisk's library.
const KEPT: &str = "notes.json";
/// The language every note has, which stands in for one it lacks.
const FALLBACK: &str = "en";
/// The longest note shown, in characters: a few sentences beside the
/// game's details, not an article.
const LONGEST: usize = 400;

/// The table, as `notes.json` holds it: each game as written there.
#[derive(Debug, Deserialize)]
struct Table {
    format: u32,
    games: Vec<serde_json::Value>,
}

/// One game of the table: the storefronts' ids for it and its note by
/// language. Other keys, such as the `name` the table's readers go by, are
/// left aside.
#[derive(Debug, Deserialize)]
struct Known {
    steam: Option<u32>,
    epic: Option<EpicGame>,
    gog: Option<String>,
    /// The note in each language it is written in, by language code such
    /// as `en` or `fr`; English always.
    text: BTreeMap<String, String>,
}

/// The notes pages read now: the table Obelisk was built with until a
/// release build reads another.
static NOTES: LazyLock<Mutex<Vec<Known>>> =
    LazyLock::new(|| Mutex::new(parsed(TABLE.as_bytes()).unwrap_or_default()));

/// The notes of `table` Obelisk can show, when it is a table of the format
/// Obelisk reads. Each entry is read alone: one Obelisk cannot show leaves
/// the others in place.
fn parsed(table: &[u8]) -> Option<Vec<Known>> {
    let table: Table = serde_json::from_slice(table).ok()?;
    (table.format == FORMAT).then(|| {
        table
            .games
            .into_iter()
            .filter_map(|entry| serde_json::from_value::<Known>(entry).ok())
            .filter(Known::allowed)
            .collect()
    })
}

impl Known {
    /// Whether Obelisk can show this note: in English, and in each language
    /// short enough to show beside a game.
    fn allowed(&self) -> bool {
        self.text.contains_key(FALLBACK)
            && self
                .text
                .values()
                .all(|note| !note.trim().is_empty() && note.chars().count() <= LONGEST)
    }

    fn names(&self, game: Game<'_>) -> bool {
        game.named_by(self.steam, self.epic.as_ref(), self.gog.as_deref())
    }

    /// The note in `language`, else in English.
    fn text(&self, language: &str) -> &str {
        self.text.get(language).unwrap_or(&self.text[FALLBACK])
    }
}

/// The note on `game` in `language`, else in English. None when the table
/// has none for it.
pub fn of(game: Game<'_>, language: &str) -> Option<String> {
    NOTES
        .lock()
        .unwrap()
        .iter()
        .find(|known| known.names(game))
        .map(|known| known.text(language).to_owned())
}

/// Reads the table again from Obelisk's `main` and keeps it in `library`.
/// When `main` does not answer in time, or with a table Obelisk reads, pages
/// read the last one kept, else the one Obelisk was built with. A
/// development build keeps the one it was built with, as edited.
pub async fn refresh(library: &Path) {
    if cfg!(debug_assertions) {
        return;
    }
    let kept = library.join(KEPT);
    let fetched = crate::remote::fetched(URL).await.and_then(|table| {
        let notes = parsed(&table)?;
        let _ = crate::remote::keep(&kept, &table);
        Some(notes)
    });
    if let Some(notes) = fetched.or_else(|| parsed(&fs::read(&kept).ok()?)) {
        *NOTES.lock().unwrap() = notes;
    }
}

#[cfg(test)]
mod tests;
