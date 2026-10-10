//! The binding's whole restore stop and its relaunch (STEP3 §7), against
//! a real Store.
use super::*;

/// §7 steps 3–6: the whole restore stop saves the drained edit, and the
/// stopped binding relaunches one host, for the same binding and mail
/// sink, on the tree as the restore left it.
#[test]
fn a_restore_stop_relaunches_one_host_on_the_restored_tree() {
    let live = Live::new(&[("pages/a.md", "- one\n")]);
    let (key, page) = live.open("pages/a.md");
    let id = live.submit(&key, "- two\n", page.version, None).unwrap();
    live.answer(&key, id);
    let Live {
        _dir,
        root,
        store,
        host,
        mail,
        ..
    } = live;
    let Ok(stopped) = host.stop_saved(id, StopMode::Restore) else {
        panic!("the restore stop completes");
    };
    assert_eq!(fs::read_to_string(root.join(&key)).unwrap(), "- two\n");
    {
        let _writer = store.writer.lock().unwrap();
        fs::write(root.join(&key), "- restored\n").unwrap();
    }
    let live = Live {
        _dir,
        root,
        store,
        host: stopped.relaunch().unwrap(),
        mail,
        id: std::cell::Cell::new(100),
    };
    // `wait` asserts every mail carries binding 7 on the original sink.
    let (_, page) = live.open("pages/a.md");
    assert_eq!(page.disk, Some(token("- restored\n")));
    live.host.stop();
}

/// §7 step 3: a restore stop that cannot save every edit hands the host
/// back with the page, admission reopened, as today's restore stops when
/// its flush fails.
#[test]
fn a_restore_stop_that_cannot_save_hands_the_host_back() {
    let live = Live::new(&[("pages/a.md", "- one\n")]);
    let (key, page) = live.open("pages/a.md");
    let id = live.submit(&key, "- mine\n", page.version, None).unwrap();
    live.answer(&key, id);
    fs::write(live.root.join(&key), "- theirs\n").unwrap();
    let Live {
        _dir,
        root,
        store,
        host,
        mail,
        id,
    } = live;
    let Err((host, pages)) = host.stop_saved(id.get(), StopMode::Restore) else {
        panic!("a conflict aborts the restore stop");
    };
    assert_eq!(pages, BTreeSet::from([PageId::from(key.clone())]));
    let live = Live {
        _dir,
        root,
        store,
        host: *host,
        mail,
        id,
    };
    assert!(live.submit(&key, "- again\n", page.version, None).is_ok());
    live.host.stop();
}

/// REVIEW-3a5 neighbour: a restore stop that cannot save names the page by
/// its current spelling (the restore error shows it), never by its opaque
/// key.
#[test]
fn a_restore_stop_names_an_unsaved_page_by_its_spelling() {
    let live = Live::new(&[("pages/a.md", "- old\n")]);
    if super::rename::folds_case(&live.root) {
        live.host.stop();
        return;
    }
    super::rename::typed_fresh_entry(&live, "- typed\n");
    let Live { host, id, .. } = live;
    let Err((host, pages)) = host.stop_saved(id.get(), StopMode::Restore) else {
        panic!("an unsaveable edit aborts the restore stop");
    };
    let pages: Vec<String> = pages.iter().map(|page| page.as_str().to_owned()).collect();
    assert_eq!(
        pages,
        ["pages/b.md"],
        "REVIEW-3a5: the restore stop named the page by its opaque key"
    );
    host.stop();
}
