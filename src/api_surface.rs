//! The crate's public surface, recorded as a reviewable artifact.
//!
//! `tine-storage` is on its way to being an independently versioned package
//! with an exact Tine pin. A version number only means something if a change to
//! what the package exports is a visible event, so this module derives the
//! export list from the source and compares it against the checked-in
//! `api.txt`. Adding, removing, renaming or re-homing a public name fails the
//! test until `api.txt` is regenerated in the same commit, which puts the
//! surface change in the diff a reviewer reads.
//!
//! # What it covers, and what it does not
//!
//! It records **names, their export path, and whether they are gated behind
//! `test-support`** — the axis a version freeze commits to and the axis
//! `formats` exists to keep single-valued. It does **not** record signatures:
//! rustdoc JSON, which would, is nightly-only, and this crate builds on the
//! pinned stable toolchain. So a parameter type changed in place still passes
//! here. Treat `api.txt` as the inventory, not as a semver oracle; a signature
//! change is caught by `tine-core` failing to compile against it.
//!
//! Regenerate with:
//!
//! ```text
//! TINE_STORAGE_BLESS_API=1 cargo test -p tine-storage api_surface
//! ```

/// Which public path reaches an exported name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ExportPath {
    /// `tine_storage::NAME`
    Root,
    /// `tine_storage::sqlite::NAME`
    Sqlite,
    /// `tine_storage::formats::NAME`
    Formats,
    /// `tine_storage::api_surface::NAME`
    ApiSurface,
}

impl ExportPath {
    const fn as_str(self) -> &'static str {
        match self {
            ExportPath::Root => "tine_storage",
            ExportPath::Sqlite => "tine_storage::sqlite",
            ExportPath::Formats => "tine_storage::formats",
            ExportPath::ApiSurface => "tine_storage::api_surface",
        }
    }
}

/// One publicly reachable name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ExportedName {
    pub path: ExportPath,
    pub name: String,
    /// True when the export is behind `#[cfg(feature = "test-support")]`, so it
    /// is absent from an ordinary release build.
    pub test_support_only: bool,
}

impl ExportedName {
    fn row(&self) -> String {
        format!(
            "{}::{}{}",
            self.path.as_str(),
            self.name,
            if self.test_support_only {
                "  [test-support]"
            } else {
                ""
            }
        )
    }
}

/// Everything before the file's `#[cfg(test)]` module.
///
/// Test code is not API, and it is actively hostile to this parser: the
/// export-path guard in `formats` contains the literal `"pub use "` inside its
/// own source-scanning code, which an unrestricted scan happily parses into a
/// nonexistent export. `#[cfg(feature = "test-support")]` is deliberately not
/// matched — those exports are real API, just gated.
fn production_region(source: &str) -> &str {
    match source.find("\n#[cfg(test)]") {
        Some(offset) => &source[..offset],
        None => source,
    }
}

/// Parse the `pub use` items of one source region into exported names.
///
/// `region` selects either a file's top level or the `pub mod sqlite { ... }`
/// body inside `lib.rs`, so the two export paths are distinguished without a
/// second parser.
fn parse_exports(path: ExportPath, region: &str) -> Vec<ExportedName> {
    let mut out = Vec::new();
    let rest = production_region(region);

    // `#[cfg(feature = "test-support")]` applies to the `pub use` that follows
    // it, so track whether the most recent attribute was that gate.
    let mut cursor = 0usize;
    while let Some(found) = rest[cursor..].find("pub use ") {
        let start = cursor + found;
        let preceding = &rest[..start];
        // Look only at the text since the previous item ended, so a gate does
        // not leak forward past an intervening ungated export.
        let since_previous = preceding.rsplit(';').next().unwrap_or("");
        let test_support_only = since_previous.contains("feature = \"test-support\"");

        let after = start + "pub use ".len();
        let Some(end_offset) = rest[after..].find(';') else {
            break;
        };
        let item = &rest[after..after + end_offset];
        cursor = after + end_offset + 1;

        for name in leaf_names(item) {
            out.push(ExportedName {
                path,
                name,
                test_support_only,
            });
        }
    }
    out
}

/// Extract the imported leaf identifiers from one `use` item's body,
/// resolving `as` renames to the name a consumer actually writes.
fn leaf_names(item: &str) -> Vec<String> {
    let body = match (item.find('{'), item.rfind('}')) {
        (Some(open), Some(close)) if open < close => &item[open + 1..close],
        _ => item,
    };
    body.split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| {
            let visible = part.rsplit(" as ").next().unwrap_or(part).trim();
            visible
                .rsplit("::")
                .next()
                .unwrap_or(visible)
                .trim()
                .to_string()
        })
        .filter(|name| !name.is_empty())
        .collect()
}

/// Isolate the body of `pub mod sqlite { ... }` by brace matching.
fn sqlite_module_body(lib_rs: &str) -> &str {
    let start = lib_rs
        .find("pub mod sqlite {")
        .expect("lib.rs must declare `pub mod sqlite`");
    let body_start = start + "pub mod sqlite {".len();
    let mut depth = 1usize;
    for (offset, byte) in lib_rs[body_start..].bytes().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return &lib_rs[body_start..body_start + offset];
                }
            }
            _ => {}
        }
    }
    panic!("`pub mod sqlite` is not brace-balanced");
}

/// Public items declared directly in a module, as opposed to re-exported into
/// it. `pub use` parsing alone would miss `formats`' manifest types and this
/// module's own accessors, leaving holes in an inventory whose whole job is to
/// have none.
fn declared_items(path: ExportPath, source: &str) -> Vec<ExportedName> {
    let mut out = Vec::new();
    for line in production_region(source).lines() {
        let Some(rest) = line.strip_prefix("pub ") else {
            continue;
        };
        let rest = rest
            .strip_prefix("const fn ")
            .or_else(|| rest.strip_prefix("enum "))
            .or_else(|| rest.strip_prefix("struct "))
            .or_else(|| rest.strip_prefix("trait "))
            .or_else(|| rest.strip_prefix("type "))
            .or_else(|| rest.strip_prefix("const "))
            .or_else(|| rest.strip_prefix("fn "));
        let Some(rest) = rest else { continue };
        let name: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if !name.is_empty() {
            out.push(ExportedName {
                path,
                name,
                test_support_only: false,
            });
        }
    }
    out
}

/// Everything `tine-storage` exports, sorted and deduplicated.
pub fn exported_names() -> Vec<ExportedName> {
    const LIB_RS: &str = include_str!("lib.rs");
    const FORMATS_RS: &str = include_str!("formats.rs");

    let sqlite_body = sqlite_module_body(LIB_RS);
    // The root region is lib.rs with the sqlite module body removed, so an
    // export inside that module is not counted twice.
    let root_region = LIB_RS.replace(sqlite_body, "");

    let mut names = Vec::new();
    names.extend(parse_exports(ExportPath::Root, &root_region));
    names.extend(parse_exports(ExportPath::Sqlite, sqlite_body));
    names.extend(parse_exports(ExportPath::Formats, FORMATS_RS));

    // Items *declared* in a public module rather than re-exported into it.
    // `formats` and this module both have them, and they are as much of the
    // surface as anything in a `pub use`.
    names.extend(declared_items(ExportPath::Formats, FORMATS_RS));
    names.extend(declared_items(
        ExportPath::ApiSurface,
        include_str!("api_surface.rs"),
    ));

    // The public modules are themselves public paths.
    for module in ["api_surface", "formats", "sqlite"] {
        names.push(ExportedName {
            path: ExportPath::Root,
            name: module.to_string(),
            test_support_only: false,
        });
    }

    names.sort();
    names.dedup();
    names
}

/// Render the surface in the form stored in `api.txt`.
pub fn render() -> String {
    let names = exported_names();
    let production = names.iter().filter(|n| !n.test_support_only).count();
    let gated = names.len() - production;

    let mut out = String::new();
    out.push_str("# tine-storage public API surface\n");
    out.push_str("# Generated by `api_surface::render`; regenerate with\n");
    out.push_str("#   TINE_STORAGE_BLESS_API=1 cargo test -p tine-storage api_surface\n");
    out.push_str("# Names only -- see the module docs for why signatures are not covered.\n");
    out.push_str(&format!(
        "# {} public names: {production} in ordinary release builds, {gated} behind `test-support`.\n\n",
        names.len()
    ));
    for name in &names {
        out.push_str(&name.row());
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOLDEN: &str = include_str!("../api.txt");

    /// The recorded surface must match the source. A failure here is not a
    /// broken test: the crate's public API changed, and that belongs in the
    /// diff a reviewer sees before a version is cut.
    #[test]
    fn api_surface_matches_the_recorded_golden() {
        let rendered = render();
        if std::env::var_os("TINE_STORAGE_BLESS_API").is_some() {
            let path = concat!(env!("CARGO_MANIFEST_DIR"), "/api.txt");
            std::fs::write(path, &rendered).expect("could not write api.txt");
            return;
        }
        assert_eq!(
            rendered.trim_end(),
            GOLDEN.trim_end(),
            "tine-storage's public API differs from api.txt.\n\
             If the change is intended, regenerate it in this commit:\n\
             \x20 TINE_STORAGE_BLESS_API=1 cargo test -p tine-storage api_surface"
        );
    }

    /// Guard the guard: the parse must actually find the surface, or the
    /// comparison above would be a comparison of two empty things.
    #[test]
    fn the_parse_finds_a_plausible_surface() {
        let names = exported_names();
        assert!(
            names.len() > 100,
            "parsed only {} exported names; the parser has stopped seeing the surface",
            names.len()
        );
        for expected in ["ContentDigest", "OperationBatch", "ScratchRun"] {
            assert!(
                names
                    .iter()
                    .any(|n| n.name == expected && n.path == ExportPath::Root),
                "{expected} is missing from the parsed root surface"
            );
        }
        assert!(
            names
                .iter()
                .any(|n| n.name == "SQLITE_SCHEMA_VERSION" && n.path == ExportPath::Formats),
            "format constants are not being attributed to `formats`"
        );
        assert!(
            names.iter().any(|n| n.test_support_only),
            "no export was recognized as test-support-gated; the cfg parse is broken"
        );
        // `formats`' own export-path guard writes the literal `"pub use "` in
        // its source. A scan that reaches the test module invents an export
        // from it, which is exactly how this parser was wrong when first
        // written, so assert the cut rather than trusting it.
        assert!(
            names
                .iter()
                .all(|n| n.name.chars().all(|c| c.is_alphanumeric() || c == '_')),
            "a parsed name is not an identifier; the parser is reading past the API into code: {:?}",
            names
                .iter()
                .filter(|n| !n.name.chars().all(|c| c.is_alphanumeric() || c == '_'))
                .collect::<Vec<_>>()
        );
    }

    /// The `test-support` seams are what must not reach a release build, so the
    /// inventory has to keep telling them apart from production API.
    #[test]
    fn test_support_seams_stay_distinguishable() {
        let names = exported_names();
        let gated: Vec<&str> = names
            .iter()
            .filter(|n| n.test_support_only)
            .map(|n| n.name.as_str())
            .collect();
        assert!(
            gated
                .iter()
                .all(|name| name.contains("for_test") || name.contains("ForTest")),
            "a test-support export is not named as a test seam: {gated:?}"
        );
    }
}
