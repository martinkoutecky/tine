//! Re-share protocol and recovery against the same loopback transport as create.
use super::*;

fn payload(generation: u32) -> Vec<u8> {
    use std::io::Write;
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let document = match generation {
        1 => include_bytes!("../../../tests/fixtures/quick-share/g1/one-page/document.json")
            .as_slice(),
        2 => include_bytes!("../../../tests/fixtures/quick-share/g2/reduced-config/document.json")
            .as_slice(),
        _ => unreachable!(),
    };
    gzip.write_all(document).unwrap();
    gzip.finish().unwrap()
}

#[test]
fn reshare_preserves_link_and_key_uses_fresh_nonce_and_previews_membership() {
    run_update_case(200, false, share::GENERATION);
}

#[test]
fn reshare_of_generation_one_creates_a_new_gist_and_keeps_the_old_link() {
    run_update_case(200, false, 1);
}

#[test]
fn membership_compares_page_identity_not_spelling() {
    let old = [
        ("project".into(), "Project".into()),
        ("old note".into(), "Old note".into()),
    ]
    .into();
    let (added, removed) = super::reshare::membership_diff(&old, ["PROJECT", "New member"]);
    assert_eq!(added, ["New member"]);
    assert_eq!(removed, ["Old note"]);
}

#[test]
fn reshare_refusals_and_unknown_outcomes_never_retry_or_replace_records() {
    for code in [401, 403, 404, 410, 429, 500, 596, 597, 598, 599, 201] {
        run_update_case(code, false, share::GENERATION);
    }
    run_update_case(200, true, share::GENERATION);
}

fn run_update_case(code: u16, changed: bool, original_generation: u32) {
    let original = share::seal_fresh(&payload(original_generation)).unwrap();
    let envelope = original.envelope.clone();
    let gist_id = "1234567890abcdef1234567890abcdef";
    let new_gist = "fedcba0987654321fedcba0987654321";
    let gets = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = gets.clone();
    let mock = Mock::start(move |req| {
        let revision = if changed && seen.load(std::sync::atomic::Ordering::SeqCst) > 0 {
            "b".repeat(40)
        } else {
            "a".repeat(40)
        };
        let gist = serde_json::json!({"id": gist_id, "owner": {"id": ACCOUNT_ID}, "description": GIST_DESCRIPTION,
            "history": [{"version": revision}], "files": {README_FILE: {}, SHARE_FILE: {"raw_url": format!("http://{}/{LOGIN}/{gist_id}/raw/{revision}/{SHARE_FILE}", req.header("host").unwrap())}}});
        if req.method == "GET" && req.path.starts_with("/gists/") {
            let n = seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if code == 410 && n > 0 {
                return resp(404, "deleted before confirmation");
            }
            return json(200, gist);
        }
        if req.path.contains("/raw/") {
            return resp(200, envelope.clone());
        }
        if req.method == "POST" && req.path == "/gists" {
            return json(201, serde_json::json!({ "id": new_gist }));
        }
        if req.method == "PATCH" {
            assert!(req.header("if-match").is_none());
            if code == 596 {
                return json(
                    200,
                    serde_json::json!({"id": gist_id, "history": [{"version": "a".repeat(40)}]}),
                );
            }
            if code == 598 {
                return json(200, gist).delayed(Duration::from_millis(1700));
            }
            if code == 597 {
                return resp(0, "dropped response");
            }
            return if code == 599 {
                resp(200, "damaged success")
            } else {
                json(code, gist)
            };
        }
        resp(404, "")
    });
    let f = fixture(mock.endpoints());
    f.connect();
    let row = ShareRow {
        share_id: share::b64u(&original.share_id),
        key: share::b64u(&original.key),
        state: ShareState::Live,
        account_id: ACCOUNT_ID,
        login: LOGIN.into(),
        gist_id: Some(gist_id.into()),
        title: "Before".into(),
        pages: 1,
        created_at: now_rfc3339(),
        viewer_generation: original_generation,
    };
    f.service
        .records
        .update_shares(|rows| rows.push(row.clone()))
        .unwrap();
    let next_payload = payload(share::GENERATION);
    let id = f.prepared(&next_payload);
    let share_id = {
        let mut previews = f.service.prepared.lock().unwrap();
        previews[0].generation = share::GENERATION;
        share::b64u(&previews[0].share_id)
    };
    let result = PrepareResult {
        prepare_id: id.clone(),
        share_id: share_id.clone(),
        membership: None,
        preview: SharePreview {
            config_settings_omitted: false,
            include_comments: true,
            title: "After".into(),
            pages: vec![tine_graph_features::publish_query::ExportPage {
                name: "New member".into(),
                path: "pages/new.md".into(),
                journal: false,
            }],
            left_out: vec![],
            payload_bytes: 1,
            envelope_bytes: 1,
        },
    };
    let preview = block_on(f.service.review_update(result, &row.share_id)).unwrap();
    if original_generation == 1 {
        assert_eq!(preview.share_id, share_id);
        assert_eq!(preview.membership.unwrap().removed, ["Project"]);
        let mut link = String::new();
        let created = block_on(f.service.create_share_with_viewer_check(
            &id,
            |value| {
                link = value.to_owned();
                true
            },
            |generation| generation == 2,
        ))
        .unwrap();
        assert_eq!(created.gist_id, new_gist);
        assert_eq!(mock.count("POST", "/gists"), 1);
        assert_eq!(mock.count("PATCH", "/gists/"), 0);
        let rows = f.rows();
        assert_eq!(rows[0], row);
        assert_eq!(rows[1].viewer_generation, 2);
        assert_eq!(
            link,
            share::link(LOGIN, new_gist, &decode_key(&rows[1]).unwrap())
        );
        let mut old_link = String::new();
        f.service
            .copy_link(&row.share_id, |value| {
                old_link = value.to_owned();
                true
            })
            .unwrap();
        assert_eq!(old_link, generation_one_link(LOGIN, gist_id, &original.key));
        return;
    }
    assert_eq!(preview.share_id, row.share_id);
    let diff = preview.membership.unwrap();
    assert_eq!(diff.added, ["New member"]);
    assert_eq!(diff.removed, ["Configured"]);
    assert_eq!(mock.count("PATCH", "/gists/"), 0);
    let mut link = String::new();
    let result = block_on(f.service.create_share_with_viewer_check(
        &id,
        |value| {
            link = value.to_owned();
            true
        },
        |generation| generation == 2,
    ));
    if code == 410 {
        assert_eq!(result.unwrap_err(), QsError::NotFound);
        assert_eq!(mock.count("PATCH", "/gists/"), 0);
        assert_eq!(f.rows(), [row.clone()]);
    } else if changed {
        assert_eq!(result.unwrap_err(), QsError::ShareChanged);
        assert_eq!(mock.count("PATCH", "/gists/"), 0);
    } else if code == 200 {
        assert_eq!(result.unwrap().gist_id, gist_id);
        assert_eq!(link, share::link(LOGIN, gist_id, &original.key));
        let req = mock
            .requests()
            .into_iter()
            .find(|r| r.method == "PATCH")
            .unwrap();
        let body = req.json();
        let updated = body["files"][SHARE_FILE]["content"].as_str().unwrap();
        assert_ne!(updated, original.envelope);
        assert_eq!(envelope_share_id(updated), Some(original.share_id.to_vec()));
        assert_eq!(share::open(updated, &original.key).unwrap(), next_payload);
    } else {
        let error = result.unwrap_err();
        match code {
            401 => assert_eq!(error, QsError::Reconnect),
            404 => assert_eq!(error, QsError::NotFound),
            500 | 596 | 597 | 598 | 599 | 201 => assert_eq!(error, QsError::MaybeUpdated),
            429 => assert!(matches!(error, QsError::RateLimited { .. })),
            _ => assert!(matches!(error, QsError::GitHub { .. })),
        }
        assert_eq!(f.rows(), [row.clone()]);
        assert!(link.is_empty());
        // Unknown PATCH outcomes need no durable new state: Check decrypts
        // whichever complete snapshot GitHub currently serves.
        assert!(
            matches!(block_on(f.service.check_share(&row.share_id)).unwrap(), CheckOutcome::Current { title, pages, .. } if title != "Before" && pages.len() == 1)
        );
        assert_ne!(f.rows()[0].title, "Before");
    }
    assert_eq!(
        block_on(f.service.create_share(&id, |_| true)).unwrap_err(),
        QsError::PreviewExpired
    );
    assert!(mock.count("PATCH", "/gists/") <= 1);
    assert_eq!(f.rows()[0].key, row.key);
}
