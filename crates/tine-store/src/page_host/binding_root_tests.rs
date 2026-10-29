use super::*;

fn create_nested(live: &Live) -> Result<(), ApplyRefusal> {
    live.host.apply(HostOp {
        pages: vec![(
            PageId::from("pages/sub/New.md"),
            Effect::Write(Arc::from(&b"- nested input\n"[..])),
            Expected::Absent,
        )],
        kinds: vec![EditKind::SaveBlock],
        policy: Input::Refuse,
        moves: vec![],
        publications: None,
        creation_checks: vec![],
        view: None,
    })
}

#[cfg(target_os = "linux")]
#[test]
fn dircreate_failed_mkdir_discard_close_releases_graph_handles() {
    use std::os::unix::fs::PermissionsExt;
    let live = Live::new(&[]);
    let pages = live.root.join("pages");
    fs::set_permissions(&pages, fs::Permissions::from_mode(0o500)).unwrap();
    let result = create_nested(&live);
    assert!(
        matches!(result, Err(ApplyRefusal::Unwritten(_))),
        "{result:?}"
    );
    assert!(!pages.join("sub").exists());
    let key = "pages/sub/New.md";
    let id = live.id();
    live.host.discard(live.host.session(), id, key).unwrap();
    assert_eq!(
        live.answer(key, id).answer.unwrap().outcome,
        AnswerOutcome::Applied
    );
    live.close(key);
    assert!(live.host.quiesce_for_tests(Duration::from_secs(20)));
    fs::set_permissions(&pages, fs::Permissions::from_mode(0o700)).unwrap();
    let held: Vec<_> = fs::read_dir("/proc/self/fd")
        .unwrap()
        .flatten()
        .filter_map(|fd| fs::read_link(fd.path()).ok())
        .filter(|path| path.starts_with(&live.root))
        .collect();
    assert!(
        held.is_empty(),
        "abandoned graph directory handles: {held:?}"
    );
    live.host.stop();
}

#[cfg(unix)]
#[test]
fn dircreate_new_folder_save_needs_no_root_read_permission() {
    use std::os::unix::fs::PermissionsExt;
    let live = Live::new(&[]);
    live.open("pages/sub/New.md");
    fs::set_permissions(&live.root, fs::Permissions::from_mode(0o300)).unwrap();
    let result = create_nested(&live);
    fs::set_permissions(&live.root, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(result, Ok(()));
    assert_eq!(
        fs::read(live.root.join("pages/sub/New.md")).unwrap(),
        b"- nested input\n"
    );
    live.host.stop();
}

#[test]
fn l9_input_typed_after_root_moves_reaches_durable_custody() {
    for replaced in [false, true] {
        if replaced && cfg!(target_os = "android") {
            continue; // Approved vanish-only fallback on shared-storage FUSE.
        }
        let live = Live::new(&[("pages/a.md", "- before\n")]);
        let (key, page) = live.open("pages/a.md");
        let dto = live.dto(&key, "- typed after move\n");
        let moved = live._dir.path().join("moved");
        fs::rename(&live.root, &moved).unwrap();
        if replaced {
            fs::create_dir(&live.root).unwrap();
        }
        let id = live.id();
        live.host
            .submit(
                live.host.session(),
                id,
                &key,
                &dto,
                page.version,
                None,
                &[EditKind::SaveBlock],
            )
            .expect("input must reach custody before graph I/O fails");
        let answer = live.answer(&key, id);
        assert!(answer.answer.as_ref().unwrap().took);
        live.until_recoverable(&[(key.clone(), answer.answer.as_ref().unwrap().version)]);
        assert_eq!(fs::read(moved.join(&key)).unwrap(), b"- before\n");
        if replaced {
            assert_eq!(fs::read_dir(&live.root).unwrap().count(), 0);
        } else {
            assert!(!live.root.exists());
        }
        live.host.stop();
    }
}
