use serde::{Deserialize, Serialize};
use std::path::Path;

/// On-disk file format of a page. Markdown (`.md` or `.markdown`) is the default; Logseq org
/// graphs use `.org`. Existing extensions are matched without case sensitivity.
/// A graph may mix the two — format is decided per file by
/// extension, never graph-wide (matching OG, which stores `:block/format` per
/// page). The graph's `:preferred-format` only chooses the extension for NEW
/// files through `Config::preferred_format`.
#[deny(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    /// Markdown page file.
    #[default]
    Md,
    /// Org page file.
    Org,
}

impl Format {
    /// Format of a page file by its extension (`.org` → Org, else Md).
    pub fn from_path(p: &Path) -> Format {
        match p.extension().and_then(|e| e.to_str()) {
            Some(extension) if extension.eq_ignore_ascii_case("org") => Format::Org,
            _ => Format::Md,
        }
    }
    /// File extension (no dot) for this format.
    pub fn ext(self) -> &'static str {
        match self {
            Format::Md => "md",
            Format::Org => "org",
        }
    }
}

