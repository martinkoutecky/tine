use std::fs;
use std::path::Path;

const TARGETS: [&str; 5] = ["linux", "android", "macos", "ios", "windows"];

/// A `#[cfg]` predicate over the shipped targets. `Other` (test, feature, …)
/// makes the attribute a non-platform one, which the family check ignores.
enum Cfg {
    Os(String),
    Unix,
    Windows,
    Any(Vec<Cfg>),
    All(Vec<Cfg>),
    Not(Box<Cfg>),
    Other,
}

fn parse_cfg(text: &str) -> Cfg {
    fn list(text: &str) -> Vec<Cfg> {
        let (mut depth, mut start, mut items) = (0, 0, vec![]);
        for (i, c) in text.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => depth -= 1,
                ',' if depth == 0 => {
                    items.push(parse_cfg(&text[start..i]));
                    start = i + 1;
                }
                _ => {}
            }
        }
        if !text[start..].trim().is_empty() {
            items.push(parse_cfg(&text[start..]));
        }
        items
    }
    let text = text.trim();
    let inner = |name: &str| {
        text.strip_prefix(name)
            .and_then(|t| t.trim_start().strip_prefix('('))
            .and_then(|t| t.strip_suffix(')'))
    };
    if let Some(t) = inner("any") {
        Cfg::Any(list(t))
    } else if let Some(t) = inner("all") {
        Cfg::All(list(t))
    } else if let Some(t) = inner("not") {
        Cfg::Not(Box::new(parse_cfg(t)))
    } else if let Some(os) = text
        .strip_prefix("target_os")
        .and_then(|t| t.split('"').nth(1))
    {
        Cfg::Os(os.into())
    } else if text == "unix" || text == r#"target_family = "unix""# {
        Cfg::Unix
    } else if text == "windows" || text == r#"target_family = "windows""# {
        Cfg::Windows
    } else {
        Cfg::Other
    }
}

/// `None`: the predicate is not a platform predicate.
fn selects(cfg: &Cfg, target: &str) -> Option<bool> {
    Some(match cfg {
        Cfg::Os(os) => os == target,
        Cfg::Unix => target != "windows",
        Cfg::Windows => target == "windows",
        Cfg::Any(items) => items
            .iter()
            .map(|c| selects(c, target))
            .collect::<Option<Vec<_>>>()?
            .contains(&true),
        Cfg::All(items) => !items
            .iter()
            .map(|c| selects(c, target))
            .collect::<Option<Vec<_>>>()?
            .contains(&false),
        Cfg::Not(item) => !selects(item, target)?,
        Cfg::Other => return None,
    })
}

/// I-16 per cfg family. A family is the set of platform-`cfg` arms that
/// attribute the same item (an anonymous block, `let x`, `fn f`, …) in the same
/// enclosing block. Every family of two or more arms must select each target
/// of its domain (the targets its enclosing cfg'd item compiles for) exactly
/// once, and a `not(any(target_os = …))` stub must select none of them: it
/// means "not a Tine platform", never a shipped target's Unsupported outage.
fn family_errors(file: &str, source: &str) -> (usize, Vec<String>) {
    let bytes = source.as_bytes();
    // (enclosing block, item) -> [(domain, predicate text, predicate)]
    type Arm<'a> = (Vec<&'a str>, String, Cfg);
    let mut families: std::collections::BTreeMap<(usize, String), Vec<Arm>> = Default::default();
    let mut blocks = vec![(0usize, TARGETS.to_vec())]; // (id, domain)
    let mut next_block = 1;
    let mut pending: Option<Vec<&str>> = None; // domain for the next `{`
    let mut i = 0;
    while i < bytes.len() {
        let rest = &source[i..];
        if rest.starts_with("//") {
            i += rest.find('\n').unwrap_or(rest.len());
        } else if rest.starts_with("/*") {
            i += rest.find("*/").unwrap() + 2;
        } else if rest.starts_with('"') {
            let mut j = 1;
            while bytes[i + j] != b'"' {
                j += if bytes[i + j] == b'\\' { 2 } else { 1 };
            }
            i += j + 1;
        } else if rest.starts_with("r#\"") {
            i += rest.find("\"#").unwrap() + 2;
        } else if rest.len() > 2 && bytes[i] == b'\'' && bytes[i + 2] == b'\'' {
            i += 3;
        } else if rest.starts_with("#[cfg(") {
            let mut depth = 0;
            let close = rest
                .char_indices()
                .skip(5)
                .find(|&(_, c)| {
                    depth += (c == '(') as i32 - (c == ')') as i32;
                    depth == 0
                })
                .unwrap()
                .0;
            let text = rest[6..close].to_string();
            let cfg = parse_cfg(&text);
            i += close + 2;
            if selects(&cfg, "linux").is_none() {
                continue;
            }
            // The attributed item: skip further attributes, then name it.
            let mut item = source[i..].trim_start();
            while item.starts_with("#[") {
                item = item[item.find(']').unwrap() + 1..].trim_start();
            }
            for vis in ["pub(crate) ", "pub(super) ", "pub "] {
                item = item.strip_prefix(vis).unwrap_or(item);
            }
            let name = if item.starts_with('{') {
                "block".to_string()
            } else {
                item[..item.find(['(', '=', ':', '{', '<', ';']).unwrap()]
                    .trim()
                    .to_string()
            };
            let (block, domain) = blocks.last().unwrap().clone();
            let narrowed = domain
                .iter()
                .copied()
                .filter(|t| selects(&cfg, t) == Some(true))
                .collect();
            pending = Some(narrowed);
            families
                .entry((block, name))
                .or_default()
                .push((domain, text, cfg));
        } else {
            match bytes[i] {
                b'{' => {
                    let domain = pending
                        .take()
                        .unwrap_or_else(|| blocks.last().unwrap().1.clone());
                    blocks.push((next_block, domain));
                    next_block += 1;
                }
                b'}' => {
                    blocks.pop();
                }
                b';' => pending = None,
                _ => {}
            }
            i += rest.chars().next().unwrap().len_utf8();
        }
    }
    let mut checked = 0;
    let mut errors = vec![];
    for ((_, name), arms) in families.iter().filter(|(_, arms)| arms.len() >= 2) {
        checked += 1;
        let domain = &arms[0].0;
        for target in domain {
            let count = arms
                .iter()
                .filter(|(_, _, c)| selects(c, target) == Some(true))
                .count();
            if count != 1 {
                errors.push(format!(
                    "{file} `{name}`: {target} selected by {count} arms"
                ));
            }
        }
        for (_, text, cfg) in arms {
            let stub = matches!(cfg, Cfg::Not(inner) if matches!(&**inner, Cfg::Any(items)
                if items.iter().all(|c| matches!(c, Cfg::Os(_)))));
            if stub && domain.iter().any(|t| selects(cfg, t) == Some(true)) {
                errors.push(format!(
                    "{file} `{name}`: fallback `{text}` selects a shipped target"
                ));
            }
        }
    }
    (checked, errors)
}

#[test]
fn host_publication_witnesses_cover_exactly_the_five_shipped_targets() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut checked = 0;
    for entry in walk(&root.join("src")) {
        let source = fs::read_to_string(&entry).unwrap();
        let (count, errors) = family_errors(&entry.display().to_string(), &source);
        checked += count;
        assert!(
            errors.is_empty(),
            "I-16: every platform cfg family names all five shipped targets once; \
             exemplar atomic_file.rs rename_replace: {errors:#?}"
        );
    }
    // Positive property: the families named in I-16's exemplars are checked.
    for (file, family) in [
        ("src/atomic_file.rs", 2),
        ("src/directory_durability.rs", 2),
        ("src/no_replace.rs", 4),
    ] {
        let source = fs::read_to_string(root.join(file)).unwrap();
        assert_eq!(family_errors(file, &source), (family, vec![]), "{file}");
    }
    assert!(checked >= 8);
    let atomic = fs::read_to_string(root.join("src/atomic_file.rs")).unwrap();
    assert!(atomic.contains("MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH"));
    let directory = fs::read_to_string(root.join("src/directory_durability.rs")).unwrap();
    assert!(directory.contains("!private") && directory.contains("DirectoryWitness::Unsupported"));
}

/// REVIEW-2b F8: the guard must catch a family that drops a shipped target,
/// even when the target is still spelled elsewhere in the file.
#[test]
fn platform_family_guard_rejects_a_dropped_ios_arm() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = fs::read_to_string(root.join("src/atomic_file.rs")).unwrap();
    let positive = "#[cfg(any(\n        target_os = \"linux\",\n        target_os = \"android\",\n        target_os = \"macos\",\n        target_os = \"ios\"\n    ))]\n    {\n        fs::rename";
    assert_eq!(source.matches(positive).count(), 1);
    let dropped = source.replace(
        positive,
        &positive.replace("        target_os = \"ios\"\n", ""),
    );
    let (_, errors) = family_errors("mutant", &dropped);
    assert_eq!(errors, ["mutant `block`: ios selected by 0 arms"]);
    // Dropped from the stub too: iOS would silently get the Unsupported arm.
    let stub = "target_os = \"ios\",\n        target_os = \"windows\"\n    )))]\n    {\n        let _ = (src, dst);";
    assert_eq!(dropped.matches(stub).count(), 1);
    let outage = dropped.replace(stub, &stub.replace("target_os = \"ios\",\n        ", ""));
    let (_, errors) = family_errors("mutant", &outage);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("fallback"), "{errors:?}");
}

fn walk(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut files = vec![];
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files.extend(walk(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            files.push(path);
        }
    }
    files
}

#[test]
fn one_no_replace_owner_covers_all_shipped_targets() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let owner = fs::read_to_string(root.join("src/no_replace.rs")).unwrap();
    for target in ["linux", "android", "macos", "ios", "windows"] {
        assert!(owner.contains(&format!("target_os = \"{target}\"")) ||
            (target == "windows" && owner.contains("#[cfg(windows)]")),
            "I-16: every shipped target needs a deliberate no-replace arm; exemplar no_replace.rs move_at/move_windows; missing {target}");
    }
    assert!(
        owner.contains("libc::SYS_renameat2")
            && owner.contains("libc::renameatx_np")
            && owner.contains("MoveFileExW")
            && owner.contains("MOVEFILE_WRITE_THROUGH")
            && !owner.contains("MOVEFILE_REPLACE_EXISTING)"),
        "I-16: no-replace owner must use non-replacing native calls; exemplar no_replace.rs"
    );
    for (path, source) in [
        (
            "src/model.rs",
            fs::read_to_string(root.join("src/model.rs")).unwrap(),
        ),
        (
            "src/restore.rs",
            fs::read_to_string(root.join("src/restore.rs")).unwrap(),
        ),
        (
            "src-tauri/src/device_io.rs",
            fs::read_to_string(root.join("../../src-tauri/src/device_io.rs")).unwrap(),
        ),
    ] {
        assert_eq!(second_owner(&source), None,
            "I-16/I-12: only no_replace.rs owns no-replace rename; exemplar no_replace.rs; in {path}");
    }
    assert!(!fs::read_to_string(root.join("src/restore.rs")).unwrap().contains("from_dir.rename("),
        "I-16: restore may not fall back to replacing Dir::rename; exemplar no_replace.rs rename_noreplace_dir");
}

/// The owner scan, shared by the real check and its negative test.
fn second_owner(source: &str) -> Option<&'static str> {
    [
        "SYS_renameat2",
        "renameatx_np",
        "renamex_np",
        "MoveFileW",
        "MoveFileExW",
    ]
    .into_iter()
    .find(|primitive| source.contains(primitive))
}

#[test]
fn no_replace_guard_rejects_a_second_owner() {
    let fake = "fn another() { MoveFileExW(src, dest, 0); }";
    assert_eq!(
        second_owner(fake),
        Some("MoveFileExW"),
        "I-16/I-12: a second native rename must fail the owner scan; exemplar no_replace.rs"
    );
}

#[cfg(windows)]
#[test]
fn windows_no_replace_keeps_the_existing_destination() {
    let root = std::env::temp_dir().join(format!("tine-i16-windows-{}", std::process::id()));
    fs::create_dir_all(root.join("pages")).unwrap();
    let source = root.join("pages/source.md");
    let dest = root.join("pages/dest.md");
    fs::write(&source, b"source").unwrap();
    fs::write(&dest, b"destination").unwrap();
    let store = tine_store::Store::open(&root, Default::default())
        .unwrap()
        .0;
    let src_id = store.file_id(tine_store::Area::Pages, "source.md").unwrap();
    let dest_id = store.file_id(tine_store::Area::Pages, "dest.md").unwrap();
    let rev = store.read(&src_id, None).unwrap().1;
    let mut tx = store.transaction(Some(tine_store::EditKind::ReplacePage));
    tx.move_file(&src_id, rev, &dest_id, None);
    assert!(matches!(tx.commit(), tine_store::TxOutcome::NotCommitted { .. }),
        "I-16: Windows no-replace must refuse an existing destination; exemplar no_replace.rs move_windows");
    assert_eq!(fs::read(&source).unwrap(), b"source");
    assert_eq!(fs::read(&dest).unwrap(), b"destination");
    drop(store);
    fs::remove_dir_all(root).unwrap();
}
