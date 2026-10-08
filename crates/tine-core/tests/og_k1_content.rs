use std::collections::HashMap;
use tine_core::{doc, org, refs, sync_diff};

fn keep_both(mine: doc::Document, theirs: doc::Document) -> Vec<doc::DocBlock> {
    let diff = sync_diff::diff_docs(&mine, &theirs);
    let decisions: HashMap<_, _> = diff
        .rows
        .iter()
        .map(|row| (row.id.clone(), "both".into()))
        .collect();
    sync_diff::merge_blocks(&mine.roots, &theirs.roots, &decisions).unwrap()
}

#[test]
fn k1_keep_both_handles_unicode_org_first_line() {
    let id = "aaaaaaaa-0000-0000-0000-0000000000cd";
    let mine = org::parse_org(&format!("* winner\n:PROPERTIES:\n:id: {id}\n:END:\n"));
    for text in ["café", "猫猫", "a💡", "á猫", "💡x"] {
        let theirs = org::parse_org(&format!("* {text}\n:PROPERTIES:\n:id: {id}\n:END:\n"));
        let merged = keep_both(mine.clone(), theirs);
        assert_eq!(merged[1].raw(), text);
        assert_eq!(merged[1].property("id"), None);
    }
}

#[test]
fn k1_org_reference_rename_handles_unicode_directives() {
    for directive in ["#+aaaaaé", "#+aaaé", "#+猫猫", "#+💡💡"] {
        let raw = format!("* Example [[Old]]\n{directive}\n");
        let renames = HashMap::from([(refs::normalize("Old"), "New".to_string())]);
        assert_eq!(
            refs::rename_refs_multi(
                &raw,
                &renames,
                true,
                tine_core::config::FileNameFormat::TripleLowbar
            ),
            format!("* Example [[New]]\n{directive}\n")
        );
    }
}
