//! Personal glossary: spoken phrases → exact replacement text, loaded from a
//! separate `glossary.toml` next to `config.toml`.
//!
//! Unlike `[[llm_commands]]` (which routes through the LLM) and
//! `[general] vocabulary` (which only biases transcription), a glossary entry
//! is a *deterministic* rewrite: when the transcript (normalized) exactly
//! equals `say`, the fixed `type` text is typed instead — no LLM involved, so
//! "la mia email" always types the exact address, fast and never mangled.
//!
//! Keeping it in its own file (not inside config.toml) means backend-switch
//! scripts (e.g. `whisrs-switch` rewriting `[general] backend`) never touch
//! the glossary, and the file can be edited freely without the config's
//! `unknown keys` warning.

use std::io;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// One glossary entry: `say` is the spoken phrase (normalized before
/// comparison), `type` is the exact text typed at the cursor on a match.
#[derive(Debug, Clone, Deserialize)]
pub struct GlossaryEntry {
    /// The spoken phrase to recognize. Normalized (trimmed, lowercased,
    /// whitespace collapsed) before comparison, so "La Mia Email" and
    /// "la mia  email" both match an entry with `say = "la mia email"`.
    pub say: String,
    /// The exact text to type at the cursor when the transcript matches.
    #[serde(alias = "text")]
    pub r#type: String,
}

/// The parsed glossary file: a `[[glossary]]` array-of-tables.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Glossary {
    #[serde(default, rename = "glossary")]
    pub entries: Vec<GlossaryEntry>,
}

impl Glossary {
    /// Look up a transcript (normalized) and return the replacement text, or
    /// `None` when nothing matches.
    pub fn lookup(&self, transcript: &str) -> Option<String> {
        if self.entries.is_empty() {
            return None;
        }
        let normalized = normalize_glossary_phrase(transcript);
        self.entries
            .iter()
            .find(|e| normalize_glossary_phrase(&e.say) == normalized)
            .map(|e| e.r#type.clone())
    }
}

/// The glossary file lives next to `config.toml`, like `vocabulary.txt`.
pub fn glossary_path() -> PathBuf {
    crate::config_path().with_file_name("glossary.toml")
}

/// Load the glossary file. A missing file is the opt-out (`Ok(Glossary::default())`).
/// A malformed file is an error — the caller decides whether to warn or fail.
pub fn load_glossary_file(path: &Path) -> io::Result<Glossary> {
    match std::fs::read_to_string(path) {
        Ok(contents) => {
            let glossary: Glossary = toml::from_str(&contents).map_err(|e| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("failed to parse glossary at {}: {e}", path.display()),
                )
            })?;
            Ok(glossary)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Glossary::default()),
        Err(e) => Err(e),
    }
}

/// Collapse whitespace, trim, and lowercase — the same normalization applied
/// to transcripts before glossary matching.
pub fn normalize_glossary_phrase(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Serialize the glossary back to TOML, one `[[glossary]]` block per entry.
///
/// The file is rewritten whole (not appended) so `whisrs glossary add/remove`
/// can never leave a half-written entry or duplicate a block. Comments in a
/// hand-edited file are lost on the first command-driven write — acceptable
/// for a file the CLI manages, and the alternative (append) corrupts the
/// file on concurrent edits.
pub fn serialize_glossary(glossary: &Glossary) -> String {
    let mut out = String::from("# Personal glossary: say a phrase, whisrs types the exact text.\n");
    out.push_str("# Managed by `whisrs glossary` — edit entries there.\n");
    for entry in &glossary.entries {
        out.push_str("\n[[glossary]]\n");
        out.push_str(&format!("say = {:?}\n", entry.say));
        out.push_str(&format!("type = {:?}\n", entry.r#type));
    }
    out
}

/// Write the glossary file (0600, like config.toml). Creates parents if needed.
pub fn save_glossary_file(path: &Path, glossary: &Glossary) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::config::setup::write_private_file(path, &serialize_glossary(glossary))
}

/// Add an entry and persist. Returns the new entry's 0-based index.
pub fn add_entry(
    path: &Path,
    glossary: &mut Glossary,
    say: String,
    r#type: String,
) -> io::Result<usize> {
    let idx = glossary.entries.len();
    glossary.entries.push(GlossaryEntry { say, r#type });
    save_glossary_file(path, glossary)?;
    Ok(idx)
}

/// Remove an entry by 0-based index, persisting the change. Returns the
/// removed entry, or `None` when the index is out of range (nothing written).
pub fn remove_entry(
    path: &Path,
    glossary: &mut Glossary,
    idx: usize,
) -> io::Result<Option<GlossaryEntry>> {
    if idx >= glossary.entries.len() {
        return Ok(None);
    }
    let removed = glossary.entries.remove(idx);
    save_glossary_file(path, glossary)?;
    Ok(Some(removed))
}
