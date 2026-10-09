use super::*;
use sha2::{Digest, Sha256};

#[test]
fn review_f7_missing_backend_refusal_guards_are_called_directly() {
    // Neither the scenario decoder nor the oracle is involved in these calls.
    let mut h = host();
    h.fs.inject(Phase::Read, [Fault::Before]);
    assert_eq!(h.load("a.md"), Disposition::Refused);
    assert!(h.pages.is_empty());
    open(&mut h, "a.md");
    let pages = h.pages.clone();
    h.fs.inject(Phase::Read, [Fault::Before]);
    assert_eq!(h.observe("a.md"), Disposition::Refused);
    assert_eq!(h.pages, pages);
    assert_eq!(
        h.reserve(&BTreeSet::from(["unknown.md".into()])),
        Disposition::Refused
    );
    assert_eq!(
        h.release(&BTreeSet::from(["a.md".into()])),
        Disposition::Refused
    );
    assert!(h.retained.is_empty());

    let mut h = host();
    assert_eq!(
        send(
            &mut h,
            "a.md",
            RequestKind::Submit {
                bytes: text("unheld"),
                version: 0,
                resolve: None
            }
        ),
        Disposition::Refused
    );
    assert!(!h.receive("a.md").unwrap().answer.unwrap().took);
    assert!(h.pages.is_empty());

    for missing in ["a.md", "b.md"] {
        let mut h = host();
        open(&mut h, if missing == "a.md" { "b.md" } else { "a.md" });
        let files = h.fs.files.clone();
        let source_version = h_version(&h, "a.md");
        let receiver_version = h_version(&h, "b.md");
        assert_eq!(
            send(
                &mut h,
                "a.md",
                RequestKind::Move {
                    receiver: "b.md".into(),
                    source_text: text("new source"),
                    receiver_text: text("new receiver"),
                    source_version,
                    receiver_version
                }
            ),
            Disposition::Refused
        );
        assert!(h.worker.is_none());
        assert_eq!(h.fs.files, files);
        for key in ["a.md", "b.md"] {
            assert!(!h.receive(key).unwrap().answer.unwrap().took);
        }
    }
    for stale_source in [true, false] {
        let mut h = host();
        open(&mut h, "a.md");
        open(&mut h, "b.md");
        let pages = h.pages.clone();
        let sv = h.pages["a.md"].version;
        let rv = h.pages["b.md"].version;
        assert_eq!(
            send(
                &mut h,
                "a.md",
                RequestKind::Move {
                    receiver: "b.md".into(),
                    source_text: text("new source"),
                    receiver_text: text("new receiver"),
                    source_version: if stale_source { sv + 1 } else { sv },
                    receiver_version: if stale_source { rv } else { rv + 1 }
                }
            ),
            Disposition::Refused
        );
        assert_eq!(h.pages, pages);
        assert!(h.worker.is_none());
    }

    let mut h = host();
    assert_eq!(h.load("c.md"), Disposition::Applied);
    assert_eq!(h.delete("c.md"), Disposition::Refused);
    assert_eq!(h.delete("unknown.md"), Disposition::Disabled);
    assert_eq!(
        h.rename_with("c.md", "a.md", &BTreeSet::new(), |b, _, _| Ok(b.clone())),
        Disposition::Refused
    );
    assert!(h.worker.is_none());

    for failure_site in ["discovery", "moving", "referrer"] {
        let mut h = host();
        if failure_site == "discovery" {
            open(&mut h, "b.md");
        }
        assert_eq!(
            h.rename_with(
                "a.md",
                "c.md",
                &BTreeSet::from(["b.md".into()]),
                |b, key, moving| {
                    if (failure_site == "moving" && moving)
                        || (failure_site != "moving" && key == "b.md")
                    {
                        Err(())
                    } else {
                        Ok(b.clone())
                    }
                }
            ),
            Disposition::Refused
        );
        assert!(h.worker.is_none());
        assert_eq!(h.fs.files["graph/a.md"].as_ref(), b"A");
        assert!(h.pages.values().all(Page::clean));
    }
    for fail_source in [true, false] {
        let mut h = host();
        if !fail_source {
            assert_eq!(h.load("a.md"), Disposition::Applied);
        }
        h.fs.inject(Phase::Read, [Fault::Before]);
        assert_eq!(
            h.rename_with(
                "a.md",
                "c.md",
                &BTreeSet::from(["b.md".into()]),
                |b, _, _| Ok(b.clone())
            ),
            Disposition::Refused
        );
        assert!(h.worker.is_none());
        assert!(h.pages.values().all(Page::clean));
    }

    let mut h = host();
    assert_eq!(h.switch_ready(0), Disposition::Applied);
    assert_eq!(
        h.admit(Request {
            id: 1,
            generation: h.generation,
            page: "a.md".into(),
            kind: RequestKind::Open
        }),
        Disposition::Refused
    );
    assert!(h.queue.is_empty());

    let mut h = host();
    h.stop();
    let record = Record {
        page: "unknown.md".into(),
        wseq: 1,
        version: 1,
        base: Base::Known(None),
        bytes: text("recover"),
    };
    h.fs.draft_temp("p-unknown.draft", &drafts::encode(&[record]))
        .unwrap();
    h.fs.draft_rename("p-unknown.draft").unwrap();
    assert_eq!(h.launch(), Disposition::Refused);
    assert!(!h.alive);
    assert!(h.pages.is_empty());
    assert!(h.fs.files.contains_key("draft/p-unknown.draft"));
}

fn h_version(h: &Host<ModelFs>, key: &str) -> u64 {
    h.pages.get(key).map_or(0, |p| p.version)
}

#[test]
fn clean_and_conflicted_pages_independently_disable_saving() {
    let mut h = host();
    open(&mut h, "a.md");
    assert_eq!(h.start_save("a.md"), Disposition::Disabled);
    edit(&mut h, "a.md", "mine");
    h.fs.external("a.md", text("theirs"), true);
    assert_eq!(h.observe("a.md"), Disposition::Applied);
    assert!(h.pages["a.md"].conflict);
    assert_eq!(h.start_save("a.md"), Disposition::Disabled);
    assert!(h.job.is_none());
}

#[test]
fn draft_envelopes_reject_each_invalid_header_record_and_vehicle_name() {
    fn checksum(bytes: &mut Vec<u8>) {
        bytes.truncate(bytes.len() - 32);
        bytes.extend_from_slice(&Sha256::digest(&*bytes));
    }
    let record = Record {
        page: "a.md".into(),
        wseq: 1,
        version: 1,
        base: Base::Known(text("A")),
        bytes: text("A"),
    };
    let encoded = drafts::encode(std::slice::from_ref(&record));
    assert_eq!(drafts::decode(&encoded).unwrap(), vec![record.clone()]);
    assert!(drafts::decode(&encoded[..8]).is_err());
    assert!(drafts::decode(&encoded[..9]).is_err());
    for index in [0, 8] {
        let mut bytes = encoded.clone();
        bytes[index] ^= 1;
        checksum(&mut bytes);
        assert!(drafts::decode(&bytes).is_err());
    }
    for records in [
        vec![],
        vec![Record {
            wseq: 0,
            ..record.clone()
        }],
        vec![
            record.clone(),
            Record {
                wseq: 2,
                ..record.clone()
            },
        ],
    ] {
        assert!(drafts::decode(&drafts::encode(&records)).is_err());
    }
    let other = Record {
        page: "b.md".into(),
        wseq: 2,
        ..record.clone()
    };
    for (name, records) in [
        ("alien.draft", vec![record.clone()]),
        ("p-a.tmp", vec![record.clone()]),
        ("p-many.draft", vec![record.clone(), other.clone()]),
    ] {
        let scan = drafts::scan(vec![(name.into(), drafts::encode(&records))]);
        assert!(scan.logical.is_empty());
        assert!(scan.unreadable.contains(name));
    }
    assert_eq!(
        drafts::scan(vec![(
            "op-a.draft".into(),
            drafts::encode(&[record, other])
        )])
        .logical
        .len(),
        2
    );
}

#[test]
fn retirement_selects_only_older_single_page_vehicles_in_order() {
    let mut files = vec![];
    for seq in [3, 1, 2] {
        let record = Record {
            page: "a.md".into(),
            wseq: seq,
            version: seq,
            base: Base::Unknown,
            bytes: text("A"),
        };
        files.push((format!("p-{seq}.draft"), drafts::encode(&[record])));
    }
    let scan = drafts::scan(files);
    assert_eq!(
        drafts::older_vehicles(&scan, "a.md", Some(2)),
        vec!["p-1.draft"]
    );
    assert_eq!(
        drafts::older_vehicles(&scan, "a.md", None),
        vec!["p-1.draft", "p-2.draft", "p-3.draft"]
    );
    assert!(drafts::older_vehicles(&scan, "b.md", None).is_empty());
}

#[test]
fn draft_phase_failures_preserve_whether_the_file_step_completed() {
    for initial in [
        Stage::Temp,
        Stage::Rename,
        Stage::CleanupUnlink,
        Stage::Unlink,
    ] {
        for fault in [Fault::Before, Fault::After] {
            let mut fs = ModelFs::default();
            let record = Record {
                page: "a.md".into(),
                wseq: 1,
                version: 1,
                base: Base::Unknown,
                bytes: text("A"),
            };
            let mut v = Vehicle::write("p-a.draft".into(), &[record]);
            if initial != Stage::Temp {
                v.advance(&mut fs);
            }
            if matches!(initial, Stage::CleanupUnlink | Stage::Unlink) {
                v.advance(&mut fs);
            }
            if initial == Stage::CleanupUnlink {
                fs.inject(Phase::DraftSync, [Fault::Before; 3]);
                for _ in 0..3 {
                    v.advance(&mut fs);
                }
            }
            if initial == Stage::Unlink {
                v = Vehicle::remove("p-a.draft".into());
            }
            assert_eq!(v.stage, initial);
            let phase = if initial == Stage::Temp {
                Phase::DraftTemp
            } else if initial == Stage::Rename {
                Phase::DraftRename
            } else {
                Phase::DraftUnlink
            };
            fs.inject(phase, [fault]);
            let failures = v.failures;
            v.advance(&mut fs);
            let expected = match (initial, fault) {
                (Stage::Temp, _) | (Stage::Rename, Fault::Before) => Stage::Absent,
                (Stage::Rename, Fault::After) => Stage::CleanupUnlink,
                (Stage::CleanupUnlink, Fault::After) => Stage::CleanupSync,
                (Stage::Unlink, Fault::After) => Stage::UnlinkSync,
                _ => initial,
            };
            assert_eq!(v.stage, expected);
            assert_eq!(v.failures, failures + 1);
        }
    }
}

#[test]
fn operation_envelopes_and_unrelated_worker_are_refused_without_changes() {
    for (source, target, refs, dead) in [
        ("unknown.md", "c.md", BTreeSet::new(), false),
        ("a.md", "unknown.md", BTreeSet::new(), false),
        ("a.md", "c.md", BTreeSet::from(["unknown.md".into()]), false),
        ("c.md", "c.md", BTreeSet::from(["b.md".into()]), false),
        ("a.md", "c.md", BTreeSet::from(["a.md".into()]), false),
        ("a.md", "c.md", BTreeSet::from(["c.md".into()]), false),
        ("a.md", "c.md", BTreeSet::new(), true),
    ] {
        let mut h = host();
        if dead {
            h.stop();
        }
        let files = h.fs.files.clone();
        assert_eq!(
            h.rename_with(source, target, &refs, |bytes, _, _| Ok(bytes.clone())),
            Disposition::Refused
        );
        assert!(h.pages.is_empty());
        assert!(h.worker.is_none());
        assert_eq!(h.fs.files, files);
    }
    let mut h = host();
    h.stop();
    assert_eq!(h.delete("a.md"), Disposition::Disabled);
    let mut h = host();
    for key in ["a.md", "b.md", "c.md"] {
        open(&mut h, key);
    }
    edit(&mut h, "a.md", "dirty");
    risk(&mut h, "a.md");
    assert_eq!(h.begin_draft("a.md"), Disposition::Pending);
    let name = h.worker.as_ref().unwrap().task.name.clone();
    assert_eq!(h.delete("b.md"), Disposition::Waiting);
    assert_eq!(
        h.rename_with(
            "c.md",
            "a.md",
            &BTreeSet::from(["b.md".into()]),
            |bytes, _, _| Ok(bytes.clone())
        ),
        Disposition::Waiting
    );
    assert_eq!(h.worker.as_ref().unwrap().task.name, name);
}

#[test]
fn refs_only_rerun_excludes_a_held_target_that_references_the_source() {
    let mut h = host();
    h.fs.external("a.md", text("[[C]]"), true);
    for key in ["a.md", "b.md", "c.md"] {
        open(&mut h, key);
    }
    assert_eq!(
        h.rename(
            "c.md",
            "a.md",
            &BTreeSet::from(["b.md".into()]),
            "C",
            "A",
            tine_core::config::FileNameFormat::TripleLowbar
        ),
        Disposition::Pending
    );
    drain(&mut h);
    assert_eq!(h.pages["a.md"].buf, text("[[C]]"));
}

#[test]
fn switch_checks_job_representation_worker_and_reservation_after_confirmation() {
    let mut h = host();
    open(&mut h, "a.md");
    edit(&mut h, "a.md", "dirty");
    risk(&mut h, "a.md");
    draft(&mut h, "a.md");
    assert_eq!(h.switch_ready(h.last_admitted), Disposition::Applied);
    assert_eq!(h.start_save("a.md"), Disposition::Pending);
    assert_eq!(h.switch_finish(), Disposition::Waiting);

    let mut h = host();
    assert_eq!(
        h.rename_with("a.md", "c.md", &BTreeSet::new(), |bytes, _, _| Ok(
            bytes.clone()
        )),
        Disposition::Pending
    );
    for _ in 0..4 {
        h.advance_draft();
    }
    assert!(h.worker.is_some());
    assert!(h.worker.as_ref().unwrap().application.is_none());
    assert_eq!(h.switch_ready(0), Disposition::Applied);
    assert_eq!(h.switch_finish(), Disposition::Waiting);

    let mut h = host();
    assert_eq!(
        h.reserve(&BTreeSet::from(["a.md".into()])),
        Disposition::Applied
    );
    assert_eq!(h.switch_ready(0), Disposition::Applied);
    assert_eq!(h.switch_finish(), Disposition::Waiting);
}

#[test]
fn switch_checks_same_version_base_refresh_and_later_same_bytes() {
    for changed_base in [false, true] {
        let mut h = host();
        open(&mut h, "a.md");
        edit(&mut h, "a.md", "dirty");
        risk(&mut h, "a.md");
        draft(&mut h, "a.md");
        if changed_base {
            h.fs.external("a.md", text("dirty"), true);
            assert_eq!(h.observe("a.md"), Disposition::Applied);
        } else {
            edit(&mut h, "a.md", "different");
            edit(&mut h, "a.md", "dirty");
        }
        assert_eq!(h.switch_ready(h.last_admitted), Disposition::Applied);
        assert_eq!(h.switch_finish(), Disposition::Waiting);
        draft(&mut h, "a.md");
        assert_eq!(h.switch_finish(), Disposition::Applied);
    }
}

#[test]
fn s31_operation_loads_once_and_failed_read_has_no_operation_effect() {
    let mut h = host();
    assert_eq!(h.delete("a.md"), Disposition::Pending);
    assert_eq!(h.fs.calls.iter().filter(|p| **p == Phase::Read).count(), 1);
    assert!(h.pages["a.md"].clean());
    drain(&mut h);
    assert_eq!(h.pages["a.md"].buf, None);

    let mut h = host();
    h.fs.inject(Phase::Read, [Fault::Before]);
    let files = h.fs.files.clone();
    assert_eq!(h.delete("a.md"), Disposition::Refused);
    assert!(h.pages.is_empty());
    assert!(h.worker.is_none());
    assert_eq!(h.version, 0);
    assert_eq!(h.fs.files, files);

    let mut h = host();
    assert_eq!(
        h.rename_with(
            "a.md",
            "c.md",
            &BTreeSet::from(["b.md".into()]),
            |bytes, _, _| Ok(bytes.clone())
        ),
        Disposition::Pending
    );
    assert_eq!(h.fs.calls.iter().filter(|p| **p == Phase::Read).count(), 3);
    assert!(h.pages.values().all(Page::clean));
    drain(&mut h);
    assert_eq!(h.pages["c.md"].buf, text("A"));
}

#[test]
fn operation_guard_refusals_keep_held_pages_and_files() {
    let mut h = host();
    assert_eq!(h.delete_loaded("a.md"), Disposition::Refused);
    assert!(h.pages.is_empty());
    assert!(h.fs.calls.is_empty());
    for key in ["a.md", "b.md", "c.md"] {
        assert_eq!(h.load(key), Disposition::Applied);
    }
    let before = h.pages.clone();
    let files = h.fs.files.clone();
    assert_eq!(
        h.rename_with("a.md", "b.md", &BTreeSet::new(), |bytes, _, _| Ok(
            bytes.clone()
        )),
        Disposition::Refused
    );
    assert_eq!(h.pages, before);
    assert_eq!(h.fs.files, files);
    edit(&mut h, "a.md", "dirty source");
    let before = h.pages.clone();
    assert_eq!(
        h.rename_with("a.md", "c.md", &BTreeSet::new(), |bytes, _, _| Ok(
            bytes.clone()
        )),
        Disposition::Refused
    );
    assert_eq!(h.delete("a.md"), Disposition::Refused);
    assert_eq!(h.pages, before);
    assert_eq!(h.fs.files, files);
    assert!(h.worker.is_none());
}

#[test]
fn switch_requires_current_draft_even_without_a_pending_worker() {
    let mut h = host();
    open(&mut h, "a.md");
    edit(&mut h, "a.md", "edited");
    risk(&mut h, "a.md");
    assert_eq!(h.switch_ready(h.last_admitted), Disposition::Applied);
    assert_eq!(h.switch_finish(), Disposition::Waiting);
    assert!(h.alive);
    draft(&mut h, "a.md");
    assert_eq!(h.switch_finish(), Disposition::Applied);
}

#[test]
fn admission_rejects_each_invalid_envelope_without_custody_changes() {
    let mut h = host();
    let valid = Request {
        id: 1,
        generation: h.generation,
        page: "a.md".into(),
        kind: RequestKind::Open,
    };
    for request in [
        Request {
            generation: 0,
            ..valid.clone()
        },
        Request {
            id: 0,
            ..valid.clone()
        },
        Request {
            page: "unknown.md".into(),
            ..valid.clone()
        },
        Request {
            kind: RequestKind::Move {
                receiver: "a.md".into(),
                source_text: text("x"),
                receiver_text: text("y"),
                source_version: 0,
                receiver_version: 0,
            },
            ..valid.clone()
        },
        Request {
            kind: RequestKind::Move {
                receiver: "unknown.md".into(),
                source_text: text("x"),
                receiver_text: text("y"),
                source_version: 0,
                receiver_version: 0,
            },
            ..valid.clone()
        },
    ] {
        assert_eq!(h.admit(request), Disposition::Refused);
        assert!(h.queue.is_empty());
        assert!(h.subscriptions.is_empty());
        assert_eq!(h.last_admitted, 0);
    }
    assert_eq!(h.admit(valid.clone()), Disposition::Applied);
    assert_eq!(h.admit(valid.clone()), Disposition::Refused);
    assert_eq!(h.queue.len(), 1);
    h.stop();
    assert_eq!(h.admit(Request { id: 2, ..valid }), Disposition::Refused);
    assert!(h.queue.is_empty());
}

#[test]
fn load_observe_and_draft_guard_dispositions_are_independent() {
    let mut h = host();
    assert_eq!(h.load("unknown.md"), Disposition::Disabled);
    assert_eq!(h.observe("a.md"), Disposition::Disabled);
    assert_eq!(h.load("a.md"), Disposition::Applied);
    assert_eq!(h.load("a.md"), Disposition::Disabled);
    h.stop();
    assert_eq!(h.load("a.md"), Disposition::Disabled);
    assert_eq!(h.observe("a.md"), Disposition::Disabled);
    assert_eq!(h.begin_draft("a.md"), Disposition::Waiting);

    let mut h = host();
    let keys = BTreeSet::from(["c.md".into()]);
    assert_eq!(h.reserve(&keys), Disposition::Applied);
    assert_eq!(h.load("c.md"), Disposition::Waiting);
    assert_eq!(h.begin_draft("c.md"), Disposition::Waiting);
    assert_eq!(h.release(&keys), Disposition::Applied);
    assert_eq!(h.delete("a.md"), Disposition::Pending);
    assert_eq!(h.load("b.md"), Disposition::Waiting);
    assert_eq!(h.begin_draft("b.md"), Disposition::Waiting);
}

#[test]
fn dequeue_cannot_overwrite_custody_and_apply_waits_for_later_save() {
    let mut h = host();
    open(&mut h, "a.md");
    edit(&mut h, "a.md", "edited");
    let request = Request {
        id: h.last_admitted + 1,
        generation: h.generation,
        page: "a.md".into(),
        kind: RequestKind::Close,
    };
    assert_eq!(h.admit(request.clone()), Disposition::Applied);
    assert_eq!(
        h.admit(Request {
            id: request.id + 1,
            ..request
        }),
        Disposition::Applied
    );
    assert_eq!(h.dequeue(), Disposition::Pending);
    let custody = h.abstract_queue();
    assert_eq!(h.dequeue(), Disposition::Disabled);
    assert_eq!(h.abstract_queue(), custody);
    assert_eq!(h.start_save("a.md"), Disposition::Pending);
    assert_eq!(h.apply_request(), Disposition::Waiting);
    assert_eq!(h.abstract_queue(), custody);
}

#[test]
fn dequeued_move_waits_for_receiver_job_or_unrelated_draft_worker() {
    for unrelated in [false, true] {
        let mut h = host();
        for k in ["a.md", "b.md", "c.md"] {
            open(&mut h, k);
            edit(&mut h, k, "edit");
        }
        let request = Request {
            id: h.last_admitted + 1,
            generation: h.generation,
            page: "a.md".into(),
            kind: RequestKind::Move {
                receiver: "b.md".into(),
                source_text: text("source"),
                receiver_text: text("target"),
                source_version: h.pages["a.md"].version,
                receiver_version: h.pages["b.md"].version,
            },
        };
        assert_eq!(h.admit(request), Disposition::Applied);
        assert_eq!(h.dequeue(), Disposition::Pending);
        if unrelated {
            risk(&mut h, "c.md");
            assert_eq!(h.begin_draft("c.md"), Disposition::Pending);
        } else {
            assert_eq!(h.start_save("b.md"), Disposition::Pending);
        }
        assert_eq!(h.apply_request(), Disposition::Waiting);
        assert!(h.outbox.is_empty());
        assert_eq!(h.abstract_queue().len(), 1);
    }
}

#[test]
fn matching_draft_is_disabled_and_a_second_save_keeps_the_first_job() {
    let mut h = host();
    open(&mut h, "a.md");
    edit(&mut h, "a.md", "edited");
    risk(&mut h, "a.md");
    draft(&mut h, "a.md");
    let sequence = h.wseq;
    assert_eq!(h.begin_draft("a.md"), Disposition::Disabled);
    assert_eq!(h.wseq, sequence);
    open(&mut h, "b.md");
    edit(&mut h, "b.md", "other edit");
    assert_eq!(h.start_save("a.md"), Disposition::Pending);
    let job = h.job.clone();
    assert_eq!(h.start_save("b.md"), Disposition::Disabled);
    assert_eq!(h.job, job);
}

#[test]
fn guard_conflict_never_rewinds_an_unrelated_newer_version() {
    let mut h = host();
    open(&mut h, "a.md");
    edit(&mut h, "a.md", "edited");
    open(&mut h, "b.md");
    let version = h.version;
    h.fs.external("a.md", text("external"), true);
    assert_eq!(h.start_save("a.md"), Disposition::Pending);
    assert_eq!(h.advance_save(0), Disposition::Pending);
    assert_eq!(h.advance_save(0), Disposition::Applied);
    assert!(h.pages["a.md"].conflict);
    assert_eq!(h.version, version);
    assert_eq!(h.load("c.md"), Disposition::Applied);
    assert_eq!(h.pages["c.md"].version, version + 1);
}

#[test]
fn save_phase_errors_keep_the_observed_completion_classification() {
    for (phase, fault, outcome) in [
        (Phase::PageTemp, Fault::Before, Outcome::Failed),
        (Phase::PageRename, Fault::Before, Outcome::Failed),
        (Phase::PageRename, Fault::After, Outcome::Uncertain),
        (Phase::TrashMove, Fault::Before, Outcome::Failed),
    ] {
        let mut h = host();
        if phase == Phase::TrashMove {
            assert_eq!(h.delete("a.md"), Disposition::Pending);
            drain(&mut h);
        } else {
            open(&mut h, "a.md");
            edit(&mut h, "a.md", "edited");
        }
        h.fs.inject(phase, [fault]);
        assert_eq!(h.start_save("a.md"), Disposition::Pending);
        for _ in 0..8 {
            if h.job.is_none() {
                break;
            }
            h.advance_save(0);
        }
        assert!(h.job.is_none());
        assert!(h.events.iter().any(
            |e| matches!(e, Event::SaveOutcome { outcome: actual, .. } if *actual == outcome)
        ));
        assert!(h.pages["a.md"].risk);
        if phase == Phase::PageRename && matches!(fault, Fault::After) {
            assert_eq!(h.fs.files["graph/a.md"].as_ref(), b"edited");
            assert!(h.events.iter().any(|e| matches!(e, Event::Renamed { .. })));
        }
    }
}

#[test]
fn trash_collision_retries_with_a_fresh_name_without_removal() {
    let mut h = host();
    assert_eq!(h.delete("a.md"), Disposition::Pending);
    drain(&mut h);
    assert_eq!(h.start_save("a.md"), Disposition::Pending);
    assert_eq!(h.advance_save(0), Disposition::Pending); // check
    assert_eq!(h.advance_save(0), Disposition::Pending); // marker
    let (marker, payload) = h.job.as_ref().unwrap().marker.clone().unwrap();
    assert_eq!(h.fs.custody_markers().unwrap().len(), 1);
    h.fs.inject(Phase::TrashMove, [Fault::Collision]);
    assert_eq!(h.advance_save(0), Disposition::Pending);
    assert_eq!(h.job.as_ref().unwrap().phase, SavePhase::Marker);
    assert!(
        h.custody.is_empty(),
        "the unused marker {marker} is retired"
    );
    assert!(h.fs.custody_markers().unwrap().is_empty());
    assert!(h.worker.is_none(), "a collision rewrites no draft");
    assert_eq!(h.fs.files["graph/a.md"].as_ref(), b"A");
    assert!(!h.events.iter().any(|e| matches!(e, Event::Removed { .. })));
    for _ in 0..8 {
        if h.job.is_none() {
            break;
        }
        h.advance_save(0);
    }
    assert!(h.pages["a.md"].clean());
    assert!(h.fs.custody_markers().unwrap().is_empty());
    let trash: Vec<_> =
        h.fs.files
            .keys()
            .filter(|k| k.starts_with("trash/"))
            .collect();
    assert_eq!(trash.len(), 1);
    assert!(!trash[0].ends_with(&payload), "fresh payload name");
}

#[test]
fn release_reconciles_fresh_versions_and_failed_reads_keep_reservations() {
    let mut h = host();
    open(&mut h, "a.md");
    let keys = BTreeSet::from(["a.md".into()]);
    let version = h.version;
    assert_eq!(h.reserve(&keys), Disposition::Applied);
    h.fs.external("a.md", text("external"), true);
    h.fs.inject(Phase::Read, [Fault::Before]);
    assert_eq!(h.release(&keys), Disposition::Waiting);
    assert_eq!(h.retained, keys);
    assert_eq!(h.switch_request(), Disposition::Waiting);
    assert_eq!(h.release(&keys), Disposition::Applied);
    assert!(h.retained.is_empty());
    assert_eq!(h.version, version + 1);
    assert_eq!(h.pages["a.md"].version, h.version);
    assert_eq!(h.pages["a.md"].buf, text("external"));
    assert_eq!(h.delete("a.md"), Disposition::Pending);
    assert_eq!(h.switch_request(), Disposition::Waiting);
}

#[test]
fn switch_confirmation_checks_both_admitted_and_applied_watermarks() {
    let mut h = host();
    open(&mut h, "a.md");
    let old = h.last_applied;
    assert_eq!(
        h.admit(Request {
            id: old + 1,
            generation: h.generation,
            page: "a.md".into(),
            kind: RequestKind::Close
        }),
        Disposition::Applied
    );
    assert_eq!(h.switch_ready(old), Disposition::Waiting);
    assert_eq!(h.switch_ready(old + 1), Disposition::Waiting);
    assert!(h.admission_open);
    assert_eq!(h.dequeue(), Disposition::Pending);
    assert_eq!(h.apply_request(), Disposition::Applied);
    assert_eq!(h.switch_ready(old + 1), Disposition::Applied);
}

#[test]
fn draft_error_is_surfaced_on_the_third_failure_and_custody_terminates() {
    let mut h = host();
    open(&mut h, "a.md");
    edit(&mut h, "a.md", "edited");
    risk(&mut h, "a.md");
    assert_eq!(h.begin_draft("a.md"), Disposition::Pending);
    h.advance_draft();
    h.advance_draft();
    h.fs.inject(Phase::DraftSync, [Fault::Before; 3]);
    for _ in 0..2 {
        h.advance_draft();
        assert!(!h
            .events
            .iter()
            .any(|e| matches!(e, Event::DraftError { .. })));
    }
    h.advance_draft();
    assert!(h
        .events
        .iter()
        .any(|e| matches!(e, Event::DraftError { failures: 3, .. })));
    drain(&mut h);
    assert!(h.worker.is_none());
    assert!(h.lock_ownership.is_empty());
}
