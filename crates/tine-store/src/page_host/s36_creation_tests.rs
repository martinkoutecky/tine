//! JA5/s3.6: real binding admission, independently observed claimant files,
//! production refusal and physical durable input. Never an inner Host::apply.
use super::*;
use serde_json::Value;

fn endpoint(fixture: &Value, name: &str) -> Value {
    fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == name && c["profile"] == "base")
        .unwrap()["endpoint"]
        .clone()
}

fn retained(live: &Live, key: &str, version: u64, expected: &Value) {
    assert_eq!(expected["disk"][2], -1);
    assert_eq!(expected["pages"][2]["buf"], 3);
    assert_eq!(expected["drafts"][2]["bytes"], 3);
    assert!(!live.root.join(key).exists(), "claimed create published");
    let key = key.to_owned();
    live.host
        .driver
        .shared
        .read(move |state| {
            let page = &state.progress.host.pages[&key];
            assert_eq!(page.buf.as_deref(), Some(b"- 3\n".as_slice()));
            assert_eq!(page.version, version);
            assert_eq!(page.base, Base::Known(None));
            assert!(!page.clean());
        })
        .unwrap();
    // Physical survivor scan, independent of ProductionIo's durable census.
    let dir = live._dir.path().join("app/drafts-v2/test-graph");
    let files = fs::read_dir(dir)
        .unwrap()
        .filter_map(|entry| {
            let entry = entry.unwrap();
            if !entry.file_type().unwrap().is_file() {
                return None;
            }
            let path = entry.path();
            Some((
                path.file_name().unwrap().to_str().unwrap().to_owned(),
                fs::read(path).unwrap(),
            ))
        })
        .collect();
    let records = crate::page_host::drafts::scan(files).logical;
    let record = &records["pages/0002.md"];
    assert_eq!(record.bytes.as_deref(), Some(b"- 3\n".as_slice()));
    assert_eq!(record.base, Base::Known(None));
    assert_eq!(record.version, version);
}

#[test]
fn s36_plain_creation_replays_binding_admission_and_retains_input() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/s2/s36-policy.json")).unwrap();
    let expected = endpoint(&fixture, "ownPlainSlotRetained");
    // Occupancy's vehicles are inputs, never supplied by an expected refusal.
    for (claim, body) in [
        ("pages/claim.md", b"title:: 0002\n- claimant\n".as_slice()),
        ("pages/0002.org", b"* claimant\n".as_slice()),
        (
            "pages/claim.md",
            b"title:: 0002\ntags:: caf\xe9\n".as_slice(),
        ),
    ] {
        let live = Live::new(&[]);
        let (key, page) = live.open("pages/0002.md");
        // Arrival is after open, before input is sent. The ordinary submit
        // remains admitted; publication is refused and its input is drafted.
        fs::write(live.root.join(claim), body).unwrap();
        assert!(live
            .store
            .watcher_drained_for_tests(Duration::from_secs(20)));
        assert_eq!(fs::read(live.root.join(claim)).unwrap(), body);
        assert!(!live.root.join(&key).exists());
        let id = live.submit(&key, "- 3\n", page.version, None).unwrap();
        let answer = live.answer(&key, id);
        let version = answer.answer.unwrap().version;
        live.until_recoverable(&[(key.clone(), version)]);
        retained(&live, &key, version, &expected);
        live.wait(&key, "s3.6 occupied slot refuses publication", |mail| {
            mail.notice.twin.as_deref() == Some(claim) && mail.notice.save_error
        });
        assert_eq!(
            fs::read(live.root.join(claim)).unwrap(),
            body,
            "claimant overwritten"
        );

        // Departure permits the same retained bytes, through a fresh ordinary
        // submit, not an operation or a raw production write.
        fs::remove_file(live.root.join(claim)).unwrap();
        assert!(live
            .store
            .watcher_drained_for_tests(Duration::from_secs(20)));
        let id = live.submit(&key, "- 3\n", version, None).unwrap();
        let answer = live.answer(&key, id);
        live.until_published(&[(key.clone(), answer.answer.unwrap().version, None)]);
        assert_eq!(endpoint(&fixture, "ownPlainSlotLeaves")["disk"][2], 3);
        assert_eq!(fs::read(live.root.join(&key)).unwrap(), b"- 3\n");
        live.host.stop();
    }
}

#[test]
fn s36_unrelated_claim_does_not_block_plain_creation() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/s2/s36-policy.json")).unwrap();
    let live = Live::new(&[]);
    let (key, page) = live.open("pages/0002.md");
    fs::write(live.root.join("pages/claim.md"), "title:: Other\n- claim\n").unwrap();
    assert!(live
        .store
        .watcher_drained_for_tests(Duration::from_secs(20)));
    let id = live.submit(&key, "- 3\n", page.version, None).unwrap();
    let answer = live.answer(&key, id);
    live.until_published(&[(key.clone(), answer.answer.unwrap().version, None)]);
    assert_eq!(endpoint(&fixture, "ownPlainSlotUnrelated")["disk"][2], 3);
    assert_eq!(fs::read(live.root.join(&key)).unwrap(), b"- 3\n");
    assert_eq!(
        fs::read(live.root.join("pages/claim.md")).unwrap(),
        b"title:: Other\n- claim\n"
    );
    live.host.stop();
}
