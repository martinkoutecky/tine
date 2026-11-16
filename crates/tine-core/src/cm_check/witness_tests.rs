//! `expected` against the r6 witness matrix, read through Tine's parser.
//!
//! The fixture is a byte copy of
//! `tine-agents/runtime/research-bulk/cm-cut6k/work/witness-fixtures-r6-headerkey.json`, generated from the
//! Lean model (`CutFixtureData.lean`); it is never edited by hand (its SHA-256 is pinned below).
//! The matrix holds texts, not observations: each row's before files are observed with
//! [`observe`], the expected observation of each kept file is computed with [`expected`] and
//! compared with [`observe`] of the row's after text, and each after path with [`expected_path`].

use super::*;
use crate::config::FileNameFormat;
use crate::date::JournalFormat;
use serde_json::Value;

const MATRIX: &str = include_str!("witness-fixtures-r6.json");
const NAMES: FileNameFormat = FileNameFormat::Legacy;

/// Rows whose text Tine reads differently from the observation the row means
/// (manager rulings Q-a/Q-b, 2026-11-16: the model's observations are what the
/// parser reports; the toy reader that generated the texts is not the model,
/// and Tine's readers are not changed). Each is re-expressed: a text Tine reads
/// as the intended observation replaces the row's text of that file.
const REEXPRESSED: &[(&str, &str, &str)] = &[(
    // The row means a header (title and alias) with a child and leftover
    // lines. Tine reads `title:: A\n- alias:: X\n  - kid` as a pre-block
    // header whose first bullet is an ordinary block (ruling Q-b); a first
    // root holding only properties is Tine's header with children.
    "header_children_leftover",
    "pages/A.md",
    "- title:: A\n  alias:: X\n  - kid\n- source\n",
)];

/// Rows no Tine text expresses, with the reason. They fail as written.
const NOT_EXPRESSIBLE: &[(&str, &str)] = &[(
    "bare_bullet_header",
    "the row means a header entry that is not meaningful, which then vanishes; \
     Tine reads a bare first bullet as an ordinary empty block that a merge keeps \
     (ruling Q-a, P4), and every header entry Tine reads holds a property",
)];

fn days() -> JournalFormat {
    // The fixture's days: `2026-11-02` (a default format) and `2026/10/26`.
    JournalFormat::new(Some("yyyy/MM/dd"), None)
}

fn format_of(file: &Value) -> Format {
    match file["format"].as_str().unwrap() {
        "org" => Format::Org,
        _ => Format::Md,
    }
}

/// Lean `P.name f`: the title, else the decoded file stem.
fn page_name(file: &Value) -> String {
    let (path, text) = (
        file["path"].as_str().unwrap(),
        file["text"].as_str().unwrap(),
    );
    crate::model::page_title_from_preamble(text, format_of(file)).unwrap_or_else(|| {
        let stem = path.rsplit('/').next().unwrap();
        let stem = stem.rsplit_once('.').map_or(stem, |(s, _)| s);
        crate::model::decode_page_name(stem, NAMES)
    })
}

/// `row` with its re-expressed file texts (see [`REEXPRESSED`]).
fn reexpressed(row: &Value) -> Value {
    let mut row = row.clone();
    let name = row["name"].as_str().unwrap().to_owned();
    for (_, path, text) in REEXPRESSED.iter().filter(|r| r.0 == name) {
        for file in row["before"].as_array_mut().unwrap() {
            if file["path"] == *path {
                file["text"] = Value::from(*text);
            }
        }
    }
    row
}

fn fixture(name: &str) -> Value {
    let matrix: Value = serde_json::from_str(MATRIX).unwrap();
    matrix["fixtures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == name)
        .unwrap()
        .clone()
}

/// One admitted row: `Ok` when every kept file's after text and path match.
fn check_row(row: &Value, days: &JournalFormat) -> Result<(), String> {
    let before = row["before"].as_array().unwrap();
    let after = row["after"].as_array().unwrap();
    let merge = row["merge"].as_bool().unwrap();
    let (old_key, new_key) = (row["old"].as_str().unwrap(), row["new"].as_str().unwrap());
    // A file merge names its pair by path; both files name one page.
    let is = |file: &Value, key: &str| {
        if row["filemerge"].as_bool().unwrap() {
            file["path"] == key
        } else {
            crate::refs::same_page(&page_name(file), key)
        }
    };
    let source = merge
        .then(|| before.iter().find(|f| is(f, old_key)))
        .flatten();
    let survivor = merge
        .then(|| before.iter().find(|f| is(f, new_key)))
        .flatten();
    let (old, destination) = match (row["filemerge"].as_bool().unwrap(), source, survivor) {
        (true, Some(s), Some(t)) => (page_name(s), page_name(t)),
        _ => (old_key.to_owned(), new_key.to_owned()),
    };
    let rename = Rename::new(&old, &destination, Some(days), NAMES);
    let kept: Vec<&Value> = before
        .iter()
        .filter(|f| survivor.is_none() || Some(*f) != source)
        .collect();
    if kept.len() != after.len() {
        return Err(format!("{} kept files, {} after", kept.len(), after.len()));
    }
    for (f, h) in kept.into_iter().zip(after) {
        let (path, text, format) = (
            f["path"].as_str().unwrap(),
            f["text"].as_str().unwrap(),
            format_of(f),
        );
        let is_survivor = Some(f) == survivor;
        let name = page_name(f);
        let journal = days.parse(&name).is_some();
        let want_path = expected_path(&rename, path, &name, journal || is_survivor);
        if h["path"] != want_path.as_str() {
            return Err(format!("path {want_path:?}, after {}", h["path"]));
        }
        let content = observe(text, format, NAMES);
        let absorbed = source.map(|s| observe(s["text"].as_str().unwrap(), format_of(s), NAMES));
        let input = match (is_survivor, &absorbed) {
            (true, Some(s)) => Expect::Survivor {
                source: s,
                target: &content,
            },
            _ => Expect::File {
                content: &content,
                skip: skip_file(text, format, path, &want_path),
            },
        };
        let want = expected(&rename, format, input);
        let got = observe(h["text"].as_str().unwrap(), format_of(h), NAMES);
        if !matches(&want, &got, is_survivor) {
            return Err(format!("{path}: expected {want:#?}\nobserved {got:#?}"));
        }
    }
    Ok(())
}

#[test]
fn the_fixture_is_the_generated_matrix() {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(MATRIX.as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(
        hex,
        "9af5d0f09a4d9cfb9dc8d379ff73555a14a15dc60a5b518c63f8765226c2dd3f"
    );
}

/// Every admitted row passes, re-expressed where Tine reads its text
/// differently; the rows no Tine text expresses fail as written.
#[test]
fn expected_matches_every_admitted_witness_row() {
    let matrix: Value = serde_json::from_str(MATRIX).unwrap();
    let days = days();
    let rows = matrix["fixtures"].as_array().unwrap();
    assert_eq!(rows.len(), 36);
    let unexpressed: Vec<&str> = NOT_EXPRESSIBLE.iter().map(|r| r.0).collect();
    let mut failed = Vec::new();
    for row in rows {
        let name = row["name"].as_str().unwrap();
        if let Err(why) = check_row(&reexpressed(row), &days) {
            if !unexpressed.contains(&name) {
                eprintln!("{name}: {why}");
            }
            failed.push(name);
        }
    }
    assert_eq!(failed, unexpressed, "rows failing the check");
}

/// The re-expressed text reads as the observation the row means (a header of
/// a title and an alias, with a child under it), and the row's own text does
/// not; without the re-expression the row fails.
#[test]
fn the_reexpressed_header_row_reads_as_intended() {
    let roles = |text: &str| -> Vec<HeaderRole> {
        observe(text, Format::Md, NAMES)
            .header
            .iter()
            .map(|h| h.role)
            .collect()
    };
    let (_, _, text) = REEXPRESSED[0];
    assert_eq!(roles(text), [HeaderRole::Title, HeaderRole::Aliases]);
    let kid = &observe(text, Format::Md, NAMES).blocks[0];
    assert!(kid.under_header && kid.depth == 1);
    let row = fixture("header_children_leftover");
    let original = row["before"][0]["text"].as_str().unwrap();
    assert_eq!(roles(original), [HeaderRole::Title]);
    assert!(check_row(&row, &days()).is_err());
}

/// Ruling Q-a: a bare first bullet is an ordinary empty block and the merge
/// keeps it; this pins the survivor the check expects for that row.
#[test]
fn a_bare_first_bullet_is_a_block_the_merge_keeps() {
    let mut row = fixture("bare_bullet_header");
    row["after"][0]["text"] = Value::from("- target\n- \n- source\n");
    assert_eq!(check_row(&row, &days()), Ok(()));
    row["after"][0]["text"] = Value::from("- target\n- source\n");
    assert!(check_row(&row, &days()).is_err());
}

/// The P2 refusals with an output: the check rejects that output. Also the
/// Org leftover whose quote drops a reference (Lean refuses it earlier, as
/// `leftoverReference`; its output fails P2 as well).
#[test]
fn every_p2_refusal_output_fails_the_check() {
    let matrix: Value = serde_json::from_str(MATRIX).unwrap();
    let days = days();
    let mut seen = Vec::new();
    for row in matrix["negative"].as_array().unwrap() {
        let reason = row[4].as_str().unwrap();
        if !(reason == ".observationsP2" || row[0] == "leftover_changed_text") {
            continue;
        }
        let as_row = serde_json::json!({
            "name": row[0], "before": row[1], "old": row[2], "new": row[3],
            "after": row[5], "merge": true, "attended": true, "filemerge": false,
        });
        assert!(check_row(&as_row, &days).is_err(), "{} passed", row[0]);
        seen.push(row[0].as_str().unwrap());
    }
    assert_eq!(
        seen,
        [
            "split_alias",
            "header_missing_entry",
            "leftover_changed_text",
            "leftover_changed_text_md"
        ]
    );
}
