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
    #[serde(default)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glossary_file_lives_next_to_config_toml() {
        let path = glossary_path();
        assert_eq!(path.file_name().unwrap(), "glossary.toml");
        assert_eq!(path.parent(), crate::config_path().parent());
    }

    #[test]
    fn missing_file_is_not_an_error() {
        let dir = std::env::temp_dir().join("whisrs-glossary-test-missing");
        let _ = std::fs::remove_dir_all(&dir);
        let glossary = load_glossary_file(&dir.join("glossary.toml")).unwrap();
        assert!(glossary.entries.is_empty());
    }

    #[test]
    fn parses_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("glossary.toml");
        std::fs::write(
            &path,
            r#"
[[glossary]]
say = "la mia email"
type = "nome.cognome@example.com"

[[glossary]]
say = "il mio numero"
text = "+39 333 1234567"
"#,
        )
        .unwrap();
        let glossary = load_glossary_file(&path).unwrap();
        assert_eq!(glossary.entries.len(), 2);
        assert_eq!(glossary.entries[0].say, "la mia email");
        assert_eq!(glossary.entries[0].r#type, "nome.cognome@example.com");
        // `text` alias works too.
        assert_eq!(glossary.entries[1].r#type, "+39 333 1234567");
    }

    #[test]
    fn lookup_matches_normalized_transcript() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("glossary.toml");
        std::fs::write(
            &path,
            r#"
[[glossary]]
say = "la mia email"
type = "nome.cognome@example.com"
"#,
        )
        .unwrap();
        let glossary = load_glossary_file(&path).unwrap();

        // Exact match.
        assert_eq!(
            glossary.lookup("la mia email"),
            Some("nome.cognome@example.com".to_string())
        );
        // Case-insensitive.
        assert_eq!(
            glossary.lookup("La Mia Email"),
            Some("nome.cognome@example.com".to_string())
        );
        // Whitespace collapsed.
        assert_eq!(
            glossary.lookup("  la   mia  email  "),
            Some("nome.cognome@example.com".to_string())
        );
        // No match.
        assert_eq!(glossary.lookup("il mio numero"), None);
    }

    #[test]
    fn lookup_on_empty_glossary_returns_none() {
        let glossary = Glossary::default();
        assert_eq!(glossary.lookup("la mia email"), None);
    }
}
