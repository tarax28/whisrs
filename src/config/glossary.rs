//! Personal glossary: spoken phrases → exact replacement text, loaded from a
//! separate `glossary.toml` next to `config.toml`.
//!
//! Unlike `[[llm_commands]]` (which routes through the LLM) and
//! `[general] vocabulary` (which only biases transcription), a glossary entry
//! is a *deterministic* rewrite: when a spoken phrase appears in the
//! transcript, the fixed `type` text is typed instead — no LLM involved.
//!
//! Keeping it in its own file (not inside config.toml) means backend-switch
//! scripts (e.g. `whisrs-switch` rewriting `[general] backend`) never touch
//! the glossary, and the file can be edited freely without the config's
//! `unknown keys` warning.

use std::io;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// One glossary entry: `says` are the spoken phrases (normalized before
/// comparison), `type` is the exact text typed at the cursor on a match.
#[derive(Debug, Clone, Deserialize)]
pub struct GlossaryEntry {
    /// The spoken phrases to recognize. Normalized (trimmed, lowercased,
    /// whitespace collapsed) before comparison. Accepts either a list
    /// (`says = [...]`) or a single string (`say = "..."`) for backward
    /// compatibility.
    #[serde(default, alias = "say", deserialize_with = "de_says")]
    pub says: Vec<String>,
    /// The exact text to type at the cursor when the transcript matches.
    #[serde(alias = "text")]
    pub r#type: String,
}

impl GlossaryEntry {
    /// The first phrase, kept for display (matches old `say` semantics).
    pub fn primary_say(&self) -> &str {
        self.says.first().map(String::as_str).unwrap_or("")
    }
}

/// Deserialize `says` from either a single string (`say = "..."`) or a list
/// (`says = [...]`). Backward compatible with the pre-alias `say` field.
fn de_says<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<String>),
    }
    match OneOrMany::deserialize(deserializer)? {
        OneOrMany::One(s) => Ok(vec![s]),
        OneOrMany::Many(v) => Ok(v),
    }
}

/// The parsed glossary file: a `[[glossary]]` array-of-tables.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Glossary {
    #[serde(default, rename = "glossary")]
    pub entries: Vec<GlossaryEntry>,
}

impl Glossary {
    /// Substitute glossary phrases inside a transcript.
    ///
    /// A phrase that is exactly the whole transcript (trailing periods
    /// stripped — the transcription model appends one to a bare utterance)
    /// yields exactly the replacement, without the stray period. Otherwise
    /// every occurrence of a phrase is replaced case-insensitively at word
    /// boundaries, preserving surrounding words and punctuation.
    pub fn lookup(&self, transcript: &str) -> Option<String> {
        if self.entries.is_empty() {
            return None;
        }
        let trimmed = transcript.trim_end_matches('.');
        let trimmed_norm = normalize_glossary_phrase(trimmed);
        if !trimmed_norm.is_empty() {
            for entry in &self.entries {
                if entry
                    .says
                    .iter()
                    .any(|s| normalize_glossary_phrase(s) == trimmed_norm)
                {
                    return Some(entry.r#type.clone());
                }
            }
            for entry in &self.entries {
                if normalize_glossary_phrase(&entry.r#type) == trimmed_norm {
                    return Some(entry.r#type.clone());
                }
            }
        }

        let lower = transcript.to_lowercase();
        for entry in &self.entries {
            let mut replaced: Option<String> = None;
            for phrase in &entry.says {
                let phrase_lower = phrase.to_lowercase();
                if phrase_lower.is_empty() {
                    continue;
                }
                if lower.contains(&phrase_lower) {
                    let text = replaced.get_or_insert_with(|| transcript.to_string());
                    *text = replace_all_case_insensitive(text, phrase, &entry.r#type);
                }
            }
            if replaced.is_some() {
                return replaced;
            }
        }
        None
    }
}

/// Replace every case-insensitive occurrence of `from` in `text` with `to`,
/// matching only at word boundaries.
fn replace_all_case_insensitive(text: &str, from: &str, to: &str) -> String {
    if from.is_empty() {
        return text.to_string();
    }
    let lower = text.to_lowercase();
    let from_lower = from.to_lowercase();
    let mut result = String::with_capacity(text.len() + to.len());
    let mut cursor = 0;
    let mut search_from = 0;
    while let Some(rel) = lower[search_from..].find(&from_lower) {
        let idx = search_from + rel;
        let end = idx + from.len();
        let is_boundary = (idx == 0
            || !lower[..idx]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric()))
            && (end >= lower.len()
                || !lower[end..]
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_alphanumeric()));
        if is_boundary {
            result.push_str(&text[cursor..idx]);
            result.push_str(to);
            cursor = end;
        }
        search_from = end;
    }
    result.push_str(&text[cursor..]);
    result
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
pub fn serialize_glossary(glossary: &Glossary) -> String {
    let mut out = String::from("# Personal glossary: say a phrase, whisrs types the exact text.\n");
    out.push_str("# Managed by `whisrs glossary` — edit entries there.\n");
    for entry in &glossary.entries {
        out.push_str("\n[[glossary]]\n");
        if entry.says.len() == 1 {
            out.push_str(&format!("say = {:?}\n", entry.says[0]));
        } else {
            let quoted: Vec<String> = entry.says.iter().map(|s| format!("{s:?}")).collect();
            out.push_str(&format!("says = [{}]\n", quoted.join(", ")));
        }
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
    says: Vec<String>,
    r#type: String,
) -> io::Result<usize> {
    let idx = glossary.entries.len();
    glossary.entries.push(GlossaryEntry { says, r#type });
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
