//! M2's hand-written environment schedules. Syscall indices count the existing
//! ModelFs syscall abstraction, not native OS calls (native replay is M3).
use super::*;
use model_fs::{DirKind, EnvStep, RootState};

impl Driver<ModelFs> {
    /// The environment changed path resolution, not bytes. Replay its α
    /// projection as extWriteD; never call external(), which would erase the
    /// detached physical evidence. Intermediate projections precede the host
    /// effect, so host/start-save probes run only at the final host barrier.
    fn environment_projection(&mut self) {
        for p in 0..self.windows.len() {
            let disk = label(&self.host.fs.alpha(&key(p), false));
            let stable = label(&self.host.fs.alpha(&key(p), true));
            if self.observed["s"]["disk"][p] == json!(disk) {
                assert_eq!(
                    self.observed["s"]["stable"][p], stable,
                    "environment changed stable without disk: M2 projection limit"
                );
                continue;
            }
            assert_eq!(disk, ABSENT, "C1/C2 only remove pages from α");
            let args = [json!(p), json!(ABSENT), json!(true)];
            let next = self
                .oracle
                .next("extWriteD", &args)
                .expect("schedule must enable the environment step (R1 inside guard window)");
            let before = self.observed.clone();
            let mut after = before.clone();
            after["s"]["disk"][p] = json!(disk);
            after["s"]["stable"][p] = json!(stable);
            after["g"]["ext"][p] = json!(self.host.fs.epochs[&key(p)]);
            after["g"]["mine"] = json!([]);
            let mut renamed: BTreeSet<(usize, usize)> =
                serde_json::from_value(after["g"]["renamed"].clone()).unwrap();
            renamed.retain(|r| r.0 != p && r.1 != p);
            after["g"]["renamed"] = json!(renamed);
            after["g"] = self.oracle.observed_commit(&before, &after, "extWrite")["g"].clone();
            self.oracle = next;
            self.compare_projection(&after["s"], "environment/extWriteD");
            assert_eq!(after["g"], self.oracle.state()["g"]);
            assert!(self.oracle.observed_guarantee(&after));
            self.observed = after;
        }
    }

    fn environment_failure(&mut self, phase: Phase, k: usize, step: EnvStep) {
        ConformanceIo::inject_env(&mut self.host.fs, phase, k, step);
        let old_inc = self.host.incarnation;
        assert_eq!(self.host.advance_save(), Disposition::Applied);
        assert!(
            self.host.fs.env_fault.is_none(),
            "every scheduled environment step must land"
        );
        assert!(
            self.host.events.iter().any(|event| matches!(
                event,
                Event::SaveOutcome {
                    cause: Some(_),
                    outcome: Outcome::Failed | Outcome::Uncertain,
                    ..
                }
            )),
            "failed graph I/O must report its error"
        );
        self.environment_projection();
        let (name, args) = if phase == Phase::PageSync {
            ("dirSync", vec![json!(false)])
        } else {
            ("saveFail", vec![])
        };
        self.register_versions(name, &args, old_inc);
        self.finish(name, &args);
        self.host.fs.assert_no_escapes();
    }
}

fn dirty(profile: &str, create: bool) -> Driver {
    let mut d = Driver::new(profile, 3);
    let p = if create { 2 } else { 0 };
    // Existing pages exercise retargeting. The creating page has two missing
    // parents, exercising graph_directory rather than only the flat map.
    d.host.fs.spell(&key(0), "pages/sub/0000.md");
    d.host.fs.spell(&key(2), "pages/new/deep/0002.md");
    run(
        &mut d,
        &[
            ("wOpen", json!([p])),
            ("deliverUp", json!([true])),
            ("wRecv", json!([p])),
            ("wEdit", json!([p, 3])),
            ("wSend", json!([p])),
            ("deliverUp", json!([true])),
        ],
    );
    d
}

#[test]
fn m3_c3_c4_model_parent_loss_refines_external_absence_and_conflict() {
    let mut counts = [0, 0];
    for profile in ["base", "R1", "weak", "all"] {
        for (family, step) in [
            EnvStep::DirRename("pages/sub".into(), "pages/moved".into()),
            EnvStep::InodeChange("pages/sub".into()),
        ]
        .into_iter()
        .enumerate()
        {
            for before_temp in [false, true] {
                let mut d = dirty(profile, false);
                if before_temp {
                    d.stepped = true;
                }
                run(&mut d, &[("flush", json!([0]))]);
                d.host.fs.apply_env(step.clone());
                d.environment_projection();
                if before_temp {
                    assert_eq!(d.host.advance_save(), Disposition::Pending);
                }
                run(&mut d, &[("check", json!([]))]);
                assert!(d.host.pages[&key(0)].conflict);
                d.stepped = false;
                run(&mut d, &[("draftSync", json!([0]))]);
                assert_eq!(
                    drafts::scan(d.host.fs.draft_files(true)).logical[&key(0)].bytes,
                    text(3)
                );
                d.host.fs.assert_no_escapes();
                counts[family] += 1;
            }
        }
    }
    assert_eq!(counts, [8, 8]);
}

#[test]
fn m3_c3_moves_multiple_physical_pages_and_temps_without_aliasing_them() {
    let mut fs = ModelFs::default();
    for (page, spelling, value) in [("a.md", "pages/sub/a.md", 1), ("b.md", "pages/sub/b.md", 2)] {
        fs.spell(page, spelling);
        fs.external(page, text(value), true);
        fs.page_temp(page, &text(value + 2)).unwrap();
    }
    fs.apply_env(EnvStep::DirRename("pages/sub".into(), "pages/moved".into()));
    for (page, value) in [("a.md", 1), ("b.md", 2)] {
        assert_eq!(fs.alpha(page, false), None);
        assert_eq!(fs.alpha(&format!("pages/moved/{page}"), false), text(value));
        assert_eq!(
            fs.files[&format!("temp/page/pages/moved/{page}")],
            text(value + 2).unwrap()
        );
    }
    fs.assert_no_escapes();
}

#[test]
fn m3_c6_classified_errors_refine_save_fail_and_failed_dirsync() {
    let mut count = 0;
    for profile in ["R1", "all"] {
        for create in [false, true] {
            for kind in [
                std::io::ErrorKind::StorageFull,
                std::io::ErrorKind::Other,
                std::io::ErrorKind::PermissionDenied,
                std::io::ErrorKind::ReadOnlyFilesystem,
            ] {
                for phase in [
                    Phase::PageTemp,
                    Phase::Read,
                    Phase::PageRename,
                    Phase::PageSync,
                ] {
                    let mut d = dirty(profile, create);
                    let p = if create { 2 } else { 0 };
                    if phase == Phase::PageTemp {
                        d.stepped = true;
                    }
                    run(&mut d, &[("flush", json!([p]))]);
                    if matches!(phase, Phase::PageRename | Phase::PageSync) {
                        run(&mut d, &[("check", json!([]))]);
                    }
                    if phase == Phase::PageSync {
                        run(&mut d, &[("rename", json!([]))]);
                    }
                    d.host.fs.inject(phase, [Fault::Errno(kind, None)]);
                    let inc = d.host.incarnation;
                    assert_eq!(d.host.advance_save(), Disposition::Applied);
                    assert_eq!(d.host.fs.observed_errno, Some(kind));
                    let (name, args) = if phase == Phase::PageSync {
                        ("dirSync", vec![json!(false)])
                    } else {
                        ("saveFail", vec![])
                    };
                    d.register_versions(name, &args, inc);
                    d.finish(name, &args);
                    assert!(d.host.pages[&key(p)].risk);
                    d.stepped = false;
                    run(&mut d, &[("draftSync", json!([p]))]);
                    assert_eq!(
                        drafts::scan(d.host.fs.draft_files(true)).logical[&key(p)].bytes,
                        text(3)
                    );
                    count += 1;
                }
            }
        }
    }
    assert_eq!(count, 64);
}

#[test]
fn m2_c1_c2_save_schedules_refine_alpha_and_keep_durable_input() {
    let mut counts = [0, 0];
    // R1 is required for environment changes after check and before rename.
    // Other schedules also run in base/weak in the boundary test below.
    for profile in ["R1", "all"] {
        for create in [false, true] {
            for phase in [
                Phase::PageTemp,
                Phase::Read,
                Phase::PageRename,
                Phase::PageSync,
            ] {
                let calls = if phase == Phase::PageTemp {
                    if create {
                        6
                    } else {
                        2
                    }
                } else {
                    1
                };
                for k in 0..calls {
                    for step in [
                        EnvStep::RootGone,
                        EnvStep::RootReplaced,
                        EnvStep::SymlinkSwap(if create { "pages/new" } else { "pages/sub" }.into()),
                    ] {
                        let mut d = dirty(profile, create);
                        let p = if create { 2 } else { 0 };
                        if phase == Phase::PageTemp {
                            d.stepped = true;
                        }
                        run(&mut d, &[("flush", json!([p]))]);
                        if matches!(phase, Phase::PageRename | Phase::PageSync) {
                            run(&mut d, &[("check", json!([]))]);
                        }
                        if phase == Phase::PageSync {
                            run(&mut d, &[("rename", json!([]))]);
                        }
                        let family = usize::from(matches!(step, EnvStep::SymlinkSwap(_)));
                        d.environment_failure(phase, k, step.clone());
                        assert!(
                            !d.host.pages[&key(p)].clean(),
                            "{profile}/{create}/{phase:?}/{k}/{step:?}"
                        );
                        assert!(d.host.pages[&key(p)].risk);
                        d.stepped = false;
                        run(&mut d, &[("draftSync", json!([p]))]);
                        assert_eq!(
                            drafts::scan(d.host.fs.draft_files(true)).logical[&key(p)].bytes,
                            text(3)
                        );
                        // Draft persistence is independent of graph resolution,
                        // including a process crash after the failing syscall.
                        d.host.fs.crash();
                        assert_eq!(
                            drafts::scan(d.host.fs.draft_files(true)).logical[&key(p)].bytes,
                            text(3)
                        );
                        d.host.fs.assert_no_escapes();
                        counts[family] += 1;
                    }
                }
            }
        }
    }
    assert_eq!(counts, [56, 28]);
}

#[test]
fn m2_c1_c2_environment_boundaries_in_all_four_profiles() {
    let mut count = 0;
    for profile in ["base", "R1", "weak", "all"] {
        for step in [
            EnvStep::RootGone,
            EnvStep::RootReplaced,
            EnvStep::SymlinkSwap("pages/sub".into()),
        ] {
            let mut d = dirty(profile, false);
            d.host.fs.apply_env(step);
            d.environment_projection();
            d.compare("environment boundary");
            // The subsequent ordinary host step is the unchanged saveFail
            // relation, checked with the full observed-state/ghost oracle.
            d.stepped = true;
            run(&mut d, &[("flush", json!([0]))]);
            assert_eq!(d.host.advance_save(), Disposition::Applied);
            let inc = d.host.incarnation;
            d.register_versions("saveFail", &[], inc);
            d.finish("saveFail", &[]);
            count += 1;
        }
    }
    assert_eq!(count, 12);
}

#[test]
fn m2_nested_page_creation_and_deletion_refine_the_unchanged_oracle() {
    for profile in ["base", "R1", "weak", "all"] {
        let mut d = dirty(profile, true);
        assert!(!d.host.fs.env.dirs.contains_key("pages/new"));
        run(
            &mut d,
            &[
                ("flush", json!([2])),
                ("check", json!([])),
                ("rename", json!([])),
                ("dirSync", json!([true])),
                ("wRecv", json!([2])),
            ],
        );
        assert_eq!(d.host.fs.alpha(&key(2), false), text(3));
        assert_eq!(d.host.fs.env.dirs["pages/new/deep"].kind, DirKind::Real);
        assert!(
            d.host.fs.owed.is_empty(),
            "nested creation owes no parent sync"
        );
        if matches!(profile, "base" | "R1") {
            for dir in ["dir/pages/new", "dir/pages/new/deep"] {
                assert!(d.host.fs.stable.contains_key(dir));
            }
        }
        run(
            &mut d,
            &[
                ("opDelete", json!([2])),
                ("flushDel", json!([2])),
                ("check", json!([])),
                ("rename", json!([])),
                ("dirSync", json!([true])),
            ],
        );
        assert_eq!(d.host.fs.alpha(&key(2), false), None);
        d.host.fs.assert_no_escapes();
    }
}

#[test]
fn m2_stable_only_parent_loss_is_an_explicit_s35_projection_limit() {
    let mut d = Driver::new("R1", 3);
    d.host.fs.spell(&key(0), "pages/sub/0000.md");
    run(
        &mut d,
        &[
            ("load", json!([0])),
            ("opDelete", json!([0])),
            ("flushDel", json!([0])),
            ("check", json!([])),
            ("rename", json!([])),
        ],
    );
    assert_eq!(d.observed["s"]["disk"][0], ABSENT);
    assert_eq!(d.observed["s"]["stable"][0], 1);
    d.host
        .fs
        .apply_env(EnvStep::SymlinkSwap("pages/sub".into()));
    assert_eq!(d.host.fs.alpha(&key(0), false), None);
    assert_eq!(d.host.fs.alpha(&key(0), true), None);
    assert!(
        d.oracle
            .next("extWriteD", &[json!(0), json!(ABSENT), json!(true)])
            .is_none(),
        "s3.5 forbids an extWriteD whose value already equals disk; no full-refinement claim here"
    );
    d.host.fs.assert_no_escapes();
}

fn physical_fixture(phase: Phase) -> ModelFs {
    let mut fs = ModelFs::default();
    fs.spell("a.md", "pages/sub/a.md");
    fs.external("a.md", text(1), true);
    fs.spell("new.md", "pages/new/deep/new.md");
    fs.draft_temp("draft", b"device input").unwrap();
    fs.draft_rename("draft").unwrap();
    fs.draft_sync().unwrap();
    if phase == Phase::PageRename {
        fs.page_temp("a.md", &text(3)).unwrap();
    }
    if phase == Phase::TrashSync {
        fs.trash_move("a.md", "payload").result.unwrap();
    }
    fs
}

fn physical_step(fs: &mut ModelFs, phase: Phase) -> io::IoResult<()> {
    match phase {
        Phase::Read => fs.read_page("a.md").map(|_| ()),
        Phase::PageTemp => fs.page_temp("new.md", &text(3)),
        Phase::PageRename => fs.page_rename("a.md"),
        Phase::PageSync => fs.page_sync("a.md").map(|_| ()),
        Phase::TrashMove => fs.trash_move("a.md", "payload").result,
        Phase::TrashSync => fs.trash_sync("a.md", "payload").map(|_| ()),
        Phase::CustodyWrite => fs.custody_write("marker", b"custody"),
        Phase::CustodyRetire => fs.custody_retire("marker"),
        Phase::CustodyList => fs.custody_markers().map(|_| ()).map_err(|_| unreachable!()),
        Phase::DraftTemp => fs.draft_temp("next", b"next input"),
        Phase::DraftRename => fs.draft_rename("next"),
        Phase::DraftUnlink => fs.draft_unlink("draft"),
        Phase::DraftSync => fs.draft_sync().map(|_| ()),
        Phase::Quarantine => fs.quarantine("draft"),
    }
}

#[test]
fn m2_c1_c2_syscall_cross_product_including_trash_and_app_data() {
    let phases = [
        (Phase::Read, 1),
        (Phase::PageTemp, 6),
        (Phase::PageRename, 1),
        (Phase::PageSync, 1),
        (Phase::TrashMove, 8),
        (Phase::TrashSync, 5),
        (Phase::CustodyWrite, 3),
        (Phase::CustodyRetire, 2),
        (Phase::CustodyList, 1),
        (Phase::DraftTemp, 1),
        (Phase::DraftRename, 1),
        (Phase::DraftUnlink, 1),
        (Phase::DraftSync, 1),
        (Phase::Quarantine, 1),
    ];
    let mut counts = [0, 0];
    for (phase, calls) in phases {
        for k in 0..calls {
            let dir = if phase == Phase::PageTemp {
                "pages/new"
            } else {
                "pages/sub"
            };
            for step in [
                EnvStep::RootGone,
                EnvStep::RootReplaced,
                EnvStep::SymlinkSwap(dir.into()),
            ] {
                let mut fs = physical_fixture(phase);
                if phase == Phase::DraftRename {
                    fs.draft_temp("next", b"next input").unwrap();
                }
                fs.inject_env(phase, k, step.clone());
                let result = physical_step(&mut fs, phase);
                // A source-parent swap need not stop a trash-destination sync.
                let graph = matches!(
                    phase,
                    Phase::Read
                        | Phase::PageTemp
                        | Phase::PageRename
                        | Phase::PageSync
                        | Phase::TrashMove
                        | Phase::TrashSync
                );
                let must_refuse = graph
                    && (phase != Phase::TrashSync || !matches!(step, EnvStep::SymlinkSwap(_)));
                assert_eq!(result.is_err(), must_refuse, "{phase:?}/{k}/{step:?}");
                assert!(fs.env_fault.is_none(), "every scheduled step must land");
                let expected_a = if matches!(&step, EnvStep::SymlinkSwap(dir) if dir == "pages/new")
                {
                    text(1)
                } else {
                    None
                };
                assert_eq!(
                    fs.alpha("a.md", false),
                    expected_a,
                    "{phase:?}/{k}/{step:?}"
                );
                if !matches!(phase, Phase::DraftUnlink | Phase::Quarantine) {
                    assert_eq!(
                        fs.files.get("draft/draft").unwrap().as_ref(),
                        b"device input"
                    );
                }
                fs.assert_no_escapes();
                counts[usize::from(matches!(step, EnvStep::SymlinkSwap(_)))] += 1;
            }
        }
    }
    assert_eq!(counts, [66, 33]);
}

#[test]
fn m2_alpha_checks_every_parent_and_escape_history_survives_crash() {
    let mut fs = physical_fixture(Phase::Read);
    fs.spell("peer.md", "pages/sub/peer.md");
    fs.external("peer.md", text(2), true);
    fs.external("flat.md", text(3), true);
    fs.apply_env(EnvStep::SymlinkSwap("pages".into()));
    assert_eq!(fs.env.dirs["pages"].epoch, 1);
    assert_eq!(
        fs.env.dirs["pages"].kind,
        DirKind::Symlink { out_of_root: true }
    );
    assert_eq!(fs.alpha("a.md", false), None);
    assert_eq!(fs.alpha("peer.md", true), None);
    assert_eq!(fs.alpha("flat.md", false), text(3));
    assert_eq!(
        fs.files["graph/a.md"],
        text(1).unwrap(),
        "detached bytes remain physical evidence"
    );
    // Negative control: omit the adapter guard and attempt each mutation
    // domain, including directory creation above the bound root.
    for key in [
        "graph/a.md",
        "temp/page/a.md",
        "dir/pages/sub/new",
        "dir/",
        "dir/../ancestor",
    ] {
        fs.record_mutation(key);
    }
    assert_eq!(fs.escapes.len(), 5);
    fs.crash();
    assert_eq!(fs.escapes.len(), 5);
    assert_eq!(fs.env.root, RootState::Bound);
}

#[test]
#[should_panic(expected = "M2 mutation escaped α")]
fn m2_driver_rejects_escape_even_on_disabled_step() {
    let mut d = Driver::new("base", 3);
    d.host.fs.spell(&key(0), "pages/sub/0000.md");
    d.host.fs.page_temp(&key(0), &text(3)).unwrap();
    d.host
        .fs
        .apply_env(EnvStep::SymlinkSwap("pages/sub".into()));
    assert!(d.host.fs.page_rename(&key(0)).is_err());
    d.host.fs.assert_no_escapes();
    // Same physical mutation as page_rename, deliberately bypassing only its
    // adapter guard. α alone cannot see the changed, out-of-domain bytes.
    d.host.fs.publish_page(&key(0));
    assert_eq!(d.host.fs.files["graph/0000.md"], text(3).unwrap());
    assert_eq!(d.host.fs.alpha(&key(0), false), None);
    d.step("saveFail", &[]);
}

// Focused production regressions, not M3's NativeFs schedule replay.
// Outcome assertions retain M2's untouched replacement/outside requirements.
// Approved Android vanish-only binding cannot detect C1 replacement.
#[cfg(not(target_os = "android"))]
#[test]
fn m2_finding_c1_replaced_root_between_directory_plan_and_finish() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("graph");
    std::fs::create_dir(&root).unwrap();
    let binding = crate::root_binding::RootBinding::new(&root);
    let target = root.join("pages/sub");
    crate::model::checked_write_target(&root, &target, "graph root", Some(&binding)).unwrap();
    crate::model::assert_checked_target(&target);
    let mut creation = crate::directory_durability::DirectoryCreation::new(&target, &root).unwrap();
    std::fs::rename(&root, temp.path().join("detached")).unwrap();
    std::fs::create_dir(&root).unwrap();
    let result = creation.finish(&binding, crate::directory_durability::sync_directory_entry);
    assert!(result.is_err() && !root.join("pages").exists(),
        "C1/graph_directory/after plan: expected untouched replacement; result={result:?}, replacement pages exists={}", root.join("pages").exists());
}

#[cfg(unix)]
#[test]
fn m2_finding_c2_symlink_between_directory_plan_and_finish() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("graph");
    let outside = temp.path().join("outside");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir(&outside).unwrap();
    let binding = crate::root_binding::RootBinding::new(&root);
    let target = root.join("pages/sub/deep");
    crate::model::checked_write_target(&root, &target, "graph root", Some(&binding)).unwrap();
    crate::model::assert_checked_target(&target);
    let mut creation = crate::directory_durability::DirectoryCreation::new(&target, &root).unwrap();
    std::fs::rename(root.join("pages"), temp.path().join("detached-pages")).unwrap();
    std::os::unix::fs::symlink(&outside, root.join("pages")).unwrap();
    let result = creation.finish(&binding, crate::directory_durability::sync_directory_entry);
    assert!(result.is_err() && !outside.join("sub").exists(),
        "C2/graph_directory/after plan: expected untouched outside; result={result:?}, outside sub exists={}", outside.join("sub").exists());
}
