use super::*;

#[test]
fn l9_input_typed_after_root_moves_reaches_durable_custody() {
    for replaced in [false, true] {
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
