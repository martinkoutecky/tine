use std::{collections::BTreeMap, fs, path::Path};
const RULE:&str="I-12: ask lsdoc through the block-region door; no regex over content structure. Exemplar: crates/tine-core/src/block_regions.rs";
fn count(source: &str, rust: bool) -> usize {
    let tests = regex::Regex::new(r"(?ms)^#\[cfg\(test\)\][\s\S]*?^}\s*$").unwrap();
    let source = if rust {
        tests.replace_all(source, "").into_owned()
    } else {
        source.to_string()
    };
    let recognizer=regex::Regex::new(r#"starts_with|contains\(|strip_prefix|eq_ignore_ascii_case|startswith|includes\(|\.test\(|\.exec\(|regexp|regex|=\s*/|=\s*r#?\""#).unwrap();
    source
        .lines()
        .filter(|line| {
            let s = line.to_lowercase().replace('\\', "");
            [
                "id::",
                ":id:",
                "scheduled",
                "deadline",
                "closed",
                ":logbook:",
                "clock:",
                ":properties:",
                ":end:",
                "```",
                "~~~",
                "#+begin_",
            ]
            .iter()
            .any(|t| s.contains(t))
                && recognizer.is_match(&s)
        })
        .count()
}
fn scan(root: &Path, dir: &Path, out: &mut BTreeMap<String, usize>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap();
        if path.is_dir() {
            if ![
                "target",
                "node_modules",
                "tests",
                "examples",
                "fixtures",
                "wasm",
            ]
            .contains(&name)
            {
                scan(root, &path, out);
            }
        } else {
            let file = path
                .strip_prefix(root)
                .unwrap()
                .to_str()
                .unwrap()
                .replace('\\', "/");
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if !["rs", "ts", "tsx", "js"].contains(&ext)
                || file.contains(".test.")
                || file.ends_with("_tests.rs")
                || file.contains("testSetup")
                || file == "crates/tine-core/src/block_regions.rs"
            {
                continue;
            }
            let n = count(&fs::read_to_string(&path).unwrap(), ext == "rs");
            if n > 0 {
                out.insert(file.into(), n);
            }
        }
    }
}
#[test]
fn structural_recognizers_only_fall() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let mut current = BTreeMap::new();
    for dir in ["src", "crates"] {
        scan(root, &root.join(dir), &mut current);
    }
    let baseline: BTreeMap<String, usize> =
        serde_json::from_str(include_str!("fixtures/structural-recognizers.json")).unwrap();
    for (file, n) in current {
        assert!(
            n <= *baseline.get(&file).unwrap_or(&0),
            "{RULE}; {file}: {n}"
        );
    }
}
#[test]
fn detects_prohibited_recognizers() {
    assert_eq!(
        count("if line.starts_with(\"id::\") { rewrite(); }", true),
        1
    );
    assert_eq!(count("const bad = /^SCHEDULED:/m;", false), 1);
}
