//! og C3Y guard (C3W W1 class): every file name tine-store derives from a
//! user's file name — a same-directory temp, a trash or conflict copy, a
//! collision candidate — is composed by `src/atomic_file.rs`
//! (`temp_path`, `prefixed_name`, `marked_name`), which keeps it within the
//! 255-byte name limit whenever the user's own name fits. A hand-written
//! format appended ~20 bytes to a 231–255-byte name and every save, import,
//! delete or marker resolution of such a file failed with ENAMETOOLONG.
use std::fs;
use std::path::{Path, PathBuf};

const EXEMPLAR: &str = "crates/tine-store/src/atomic_file.rs";

/// Temp literals whose interpolations are a pid, sequence or stamp only —
/// never a user's file name — so their length is fixed.
const NAME_FREE_TEMPS: &[&str] = &[".tine-restore-{}-{}.tmp", ".tine-tx-{}.tmp", ".tmp"];

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if path.is_dir() {
            sources(&path, out);
        } else if name.ends_with(".rs")
            && !name.ends_with("_tests.rs")
            && !name.starts_with("test_")
        {
            out.push(path);
        }
    }
}

/// Production lines: a `#[cfg(test)]` item (a test module or helper) is skipped
/// to the end of its braces.
fn production_lines(text: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut skip = false;
    let mut depth = 0i64;
    let mut opened = false;
    for (index, line) in text.lines().enumerate() {
        if !skip && line.trim() == "#[cfg(test)]" {
            skip = true;
            depth = 0;
            opened = false;
            continue;
        }
        if skip {
            depth += line.matches('{').count() as i64 - line.matches('}').count() as i64;
            opened |= line.contains('{');
            if (opened && depth <= 0) || (!opened && line.trim_end().ends_with(';')) {
                skip = false;
            }
            continue;
        }
        out.push((index + 1, line));
    }
    out
}

/// String literals on one line (no escapes in the formats this guard polices).
fn literals(line: &str) -> Vec<&str> {
    let code = line.split("//").next().unwrap_or("");
    code.split('"').skip(1).step_by(2).collect()
}

#[test]
fn c3y_every_derived_file_name_goes_through_atomic_file() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    sources(&root.join("src"), &mut files);
    assert!(
        files.len() > 20,
        "guard scanned too few files: {}",
        files.len()
    );
    let mut violations = Vec::new();
    let mut exemplar_seen = false;
    for file in files {
        let rel = file
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let is_exemplar = rel == "src/atomic_file.rs";
        let text = fs::read_to_string(&file).unwrap();
        for (line_no, line) in production_lines(&text) {
            for literal in literals(line) {
                let temp = literal.contains(".tmp") && !NAME_FREE_TEMPS.contains(&literal);
                // `…__{name}"`: a user name appended after a trash stamp/reason.
                let trash = literal.contains("__{") && literal.ends_with('}');
                // `{stem}_{n}{ext}`: a collision mark spliced into a user name.
                let collision = literal.contains("{stem}_{");
                if is_exemplar {
                    exemplar_seen |= temp;
                    continue;
                }
                if temp || trash || collision {
                    violations.push(format!("{rel}:{line_no}: \"{literal}\""));
                }
            }
        }
    }
    assert!(
        exemplar_seen,
        "the guard no longer sees the exemplar temp format in {EXEMPLAR}"
    );
    assert!(
        violations.is_empty(),
        "C3Y (I-2, I-11): a file name derived from a user's file name must be composed by \
         {EXEMPLAR} (`temp_path`, `prefixed_name`, `marked_name`) so it fits the 255-byte name \
         limit whenever the user's name does; imitate `transaction.rs::trash_id`. Found:\n{}",
        violations.join("\n")
    );
}
