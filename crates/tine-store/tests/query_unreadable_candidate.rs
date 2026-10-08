//! Master 64c060160 (GH #594): a reference candidate the graph can no longer
//! parse, or one deleted outside Tine with no watcher event, made candidate
//! hydration decline as a whole and the reference panel fall through to
//! parsing EVERY page. og has no candidate hydration to decline (references are
//! answered from the parsed in-memory snapshot), so the outcome to pin is:
//! damage to a candidate's file on disk after the snapshot was taken never
//! makes a reference question read or parse the graph again (I-13/I-15).

use std::fs;
use std::sync::Mutex;

use tine_store::cost_counters;
use tine_store::{OpenOptions, Store};

static CASE_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn backlinks_after_a_candidate_file_is_damaged_or_deleted_on_disk_parse_nothing() {
    let _case = CASE_LOCK.lock().unwrap();
    let dir = std::env::temp_dir().join(format!("tine-594-candidate-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("pages")).unwrap();
    fs::create_dir_all(dir.join("journals")).unwrap();
    fs::write(dir.join("pages/Target.md"), "- the target\n").unwrap();
    fs::write(dir.join("pages/Damaged.md"), "- see [[Target]]\n").unwrap();
    fs::write(dir.join("pages/Deleted.md"), "- also [[Target]]\n").unwrap();
    fs::write(dir.join("pages/Intact.md"), "- and [[Target]]\n").unwrap();
    for n in 0..30 {
        fs::write(dir.join(format!("pages/Filler{n:02}.md")), "- filler\n").unwrap();
    }
    let (store, _, _) = Store::open(&dir, OpenOptions::default()).expect("open");
    let view = store.whole_graph().expect("load");

    // Damage two candidates behind the snapshot's back (no watcher event awaited).
    fs::write(dir.join("pages/Damaged.md"), [0xff, 0xfe, 0x00, 0x9f, 0x92]).unwrap();
    fs::remove_file(dir.join("pages/Deleted.md")).unwrap();

    cost_counters::reset();
    let groups = view.backlinks("Target").expect("backlinks answer");
    let counts = cost_counters::snapshot();
    let mut pages: Vec<&str> = groups.iter().map(|group| group.page.as_str()).collect();
    pages.sort();
    assert_eq!(
        pages,
        vec!["Damaged", "Deleted", "Intact"],
        "answers from the snapshot"
    );
    assert_eq!(
        counts.parses, 0,
        "I-13/I-15: a reference question must not parse (let alone parse the graph); observed {counts:?}"
    );
    store.close();
    let _ = fs::remove_dir_all(&dir);
}
