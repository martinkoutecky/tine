//! M3 bounded native replay. Native α resolves every parent using std::fs,
//! independently of checked_write_target and ModelFs. These are API/namespace
//! witnesses, not native power-loss refinement or arbitrary syscall schedules.
use super::*;
use crate::page_host::io::Phase;
use crate::page_host::model_fs::EnvStep;
use crate::page_host::production::ProductionIo;
use std::{fs, io as stdio, path::PathBuf};

const PAGE: &str = "a.md";
const OLD: &[u8] = b"original";
const INPUT: &[u8] = b"admitted input";
const SPELLING: &str = "pages/sub/deep/a.md";

fn bytes(value: &[u8]) -> Text {
    Some(Arc::from(value))
}

fn host<F: HostIo>(fs: F) -> Host<F> {
    Host::new(fs, [(PAGE.into(), Arc::new(Mutex::new(())))].into())
}

fn send<F: HostIo>(h: &mut Host<F>, kind: RequestKind) {
    assert_eq!(
        h.admit(Request {
            id: h.last_admitted + 1,
            generation: h.generation,
            page: PAGE.into(),
            kind
        }),
        Disposition::Applied
    );
    assert_eq!(h.dequeue(), Disposition::Pending);
    assert!(matches!(
        h.apply_request(),
        Disposition::Applied | Disposition::Pending
    ));
}

fn edit<F: HostIo>(h: &mut Host<F>) {
    send(h, RequestKind::Open);
    let version = h.pages[PAGE].version;
    send(
        h,
        RequestKind::Submit {
            bytes: bytes(INPUT),
            version,
            resolve: None,
        },
    );
}

fn at<F: HostIo>(h: &mut Host<F>, phase: SavePhase) {
    assert_eq!(h.start_save(PAGE), Disposition::Pending);
    for _ in 0..10 {
        if h.job.as_ref().unwrap().phase == phase {
            return;
        }
        assert_eq!(h.advance_save(), Disposition::Pending);
    }
    panic!("save never reached {phase:?}");
}

fn finish<F: HostIo>(h: &mut Host<F>) {
    for _ in 0..15 {
        if h.job.is_none() {
            return;
        }
        h.advance_save();
    }
    panic!("save did not finish");
}

fn keep_draft<F: HostIo>(h: &mut Host<F>) {
    assert_eq!(h.pages[PAGE].buf, bytes(INPUT));
    assert!(!h.pages[PAGE].clean());
    assert!(h.pages[PAGE].risk);
    assert_eq!(h.begin_draft(PAGE), Disposition::Pending);
    for _ in 0..30 {
        if h.worker.is_none() {
            break;
        }
        h.advance_draft();
    }
    assert!(h.worker.is_none());
    assert_eq!(
        drafts::scan(h.fs.draft_files(true)).logical[PAGE].bytes,
        bytes(INPUT)
    );
}

fn model(create: bool) -> Host<ModelFs> {
    model_fixture(create, true)
}

fn model_fixture(create: bool, dirty: bool) -> Host<ModelFs> {
    let mut fs = ModelFs::default();
    fs.spell(PAGE, SPELLING);
    // Keep ancestors present for C2 even when the final page is absent.
    fs.env.seed_dirs("pages/sub/deep");
    if !create {
        fs.external(PAGE, bytes(OLD), true);
    }
    let mut h = host(fs);
    if dirty {
        edit(&mut h);
    } else {
        send(&mut h, RequestKind::Open);
    }
    h
}

struct Native {
    dir: tempfile::TempDir,
    graph: PathBuf,
    outside: PathBuf,
    app: PathBuf,
    bound: PathBuf,
    h: Host<ProductionIo>,
}

impl Native {
    fn new(create: bool) -> Self {
        Self::fixture(create, true)
    }

    fn fixture(create: bool, dirty: bool) -> Self {
        let dir = match std::env::var_os("TINE_HOST_FS_ROOT") {
            Some(base) => tempfile::tempdir_in(base).unwrap(),
            None => tempfile::tempdir().unwrap(),
        };
        let graph = dir.path().join("graph");
        let outside = dir.path().join("outside");
        let app = dir.path().join("app");
        fs::create_dir_all(graph.join("pages/sub/deep")).unwrap();
        fs::create_dir(&outside).unwrap();
        if !create {
            fs::write(graph.join(SPELLING), OLD).unwrap();
        }
        let mut fs =
            ProductionIo::attach(&graph, &app, "m3", &graph.join("logseq/.tine-trash/pages"));
        fs.spell(PAGE, SPELLING);
        let mut h = host(fs);
        if dirty {
            edit(&mut h);
        } else {
            send(&mut h, RequestKind::Open);
        }
        Self {
            dir,
            bound: graph.clone(),
            graph,
            outside,
            app,
            h,
        }
    }

    fn apply_env(&mut self, step: &EnvStep) {
        match step {
            EnvStep::RootGone | EnvStep::RootReplaced => {
                self.bound = self.dir.path().join("detached-root");
                fs::rename(&self.graph, &self.bound).unwrap();
                if matches!(step, EnvStep::RootReplaced) {
                    fs::create_dir(&self.graph).unwrap();
                }
            }
            EnvStep::SymlinkSwap(dir) => {
                fs::rename(
                    self.graph.join(dir),
                    self.dir.path().join("detached-parent"),
                )
                .unwrap();
                symlink(&self.outside, &self.graph.join(dir));
            }
            EnvStep::DirRename(from, to) => {
                fs::rename(self.graph.join(from), self.graph.join(to)).unwrap()
            }
            EnvStep::InodeChange(dir) => {
                fs::rename(
                    self.graph.join(dir),
                    self.dir.path().join("detached-parent"),
                )
                .unwrap();
                fs::create_dir(self.graph.join(dir)).unwrap();
            }
        }
    }

    fn alpha(&self, spelling: &str) -> Text {
        if !same_file::is_same_file(&self.graph, &self.bound).unwrap_or(false) {
            return None;
        }
        let root = fs::canonicalize(&self.bound).unwrap();
        let path = self.graph.join(spelling);
        let mut parent = path.parent().unwrap();
        while parent != self.graph {
            let metadata = fs::symlink_metadata(parent).ok()?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return None;
            }
            if !fs::canonicalize(parent).ok()?.starts_with(&root) {
                return None;
            }
            parent = parent.parent().unwrap();
        }
        let metadata = fs::symlink_metadata(&path).ok()?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return None;
        }
        fs::read(path).ok().map(Arc::from)
    }

    fn escapes(&self) {
        assert_eq!(
            fs::read_dir(&self.outside).unwrap().count(),
            0,
            "production escaped into sentinel"
        );
        if self.bound != self.graph {
            assert!(
                !self.graph.exists() || fs::read_dir(&self.graph).unwrap().count() == 0,
                "production recreated a detached graph"
            );
        }
        let mut siblings: Vec<_> = fs::read_dir(self.dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        siblings.sort();
        assert!(
            siblings.iter().all(|name| [
                "graph",
                "app",
                "outside",
                "detached-root",
                "detached-parent"
            ]
            .iter()
            .any(|allowed| name == allowed)),
            "unexpected mutation above bound: {siblings:?}"
        );
    }

    fn reattach(&mut self) {
        self.h.fs = ProductionIo::attach(
            &self.graph,
            &self.app,
            "m3",
            &self.graph.join("logseq/.tine-trash/pages"),
        );
        self.h.fs.spell(PAGE, SPELLING);
    }
}

fn finish_pair(n: &mut Native, m: &mut Host<ModelFs>) {
    for _ in 0..15 {
        assert_eq!(
            n.h.job.is_some(),
            m.job.is_some(),
            "native/model phase completion"
        );
        if n.h.job.is_none() {
            return;
        }
        assert_eq!(n.h.advance_save(), m.advance_save());
        assert_eq!(
            n.alpha(SPELLING),
            m.fs.alpha(PAGE, false),
            "independent native α at every phase barrier"
        );
        assert_eq!(n.h.pages[PAGE], m.pages[PAGE]);
        n.escapes();
        m.fs.assert_no_escapes();
    }
    panic!("paired save did not finish");
}

#[cfg(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "ios",
    target_os = "android"
))]
fn symlink(target: &std::path::Path, path: &std::path::Path) {
    std::os::unix::fs::symlink(target, path).unwrap();
}

// Windows symlink privilege/junction creation is not assumed by this harness.
// The explicit ModelFs-only ledger below owns this unsupported native step.
#[cfg(target_os = "windows")]
fn symlink(_: &std::path::Path, _: &std::path::Path) {
    panic!("C2 is NATIVE_MODELFS_ONLY on Windows");
}

// Separate from the frozen s3.5 corpus exclusions/counts in
// native_conformance_tests.rs. Every exclusion here is asserted by a ledger test.
const NATIVE_MODELFS_ONLY: &[(&str, &str)] = &[
    ("windows", "C2: no unprivileged junction/symlink fixture"),
    (
        "android",
        "C1 RootReplaced: production policy is vanish-only",
    ),
    (
        "all",
        "interior syscall slots: no native scheduler; M2-1/M2-2 use direct plan probes",
    ),
    (
        "all",
        "power loss: native API witnesses do not model a machine power cut",
    ),
];

#[test]
fn m3_native_environment_exclusion_ledger() {
    assert_eq!(NATIVE_MODELFS_ONLY.len(), 4);
    assert_eq!(
        NATIVE_MODELFS_ONLY.iter().map(|r| r.0).collect::<Vec<_>>(),
        ["windows", "android", "all", "all"]
    );
    let ledger = include_str!("../../tests/write_sites.toml");
    for owner in ["PagePlan::apply", "PagePlan::apply_move"] {
        let row = ledger
            .split("[[site]]")
            .find(|row| row.contains(&format!("fn = \"{owner}\"")))
            .unwrap();
        assert!(
            row.lines().any(|line| line == "oracle = false"),
            "{owner} is escape-only"
        );
        assert!(row.contains("transaction_environment_tests.rs#m3_transaction_apply_and_apply_move_c2_c3_escape_only"));
    }
}

#[test]
fn m3_c1_c2_native_replay_matches_model_alpha_and_keeps_input() {
    let mut counts = [0, 0];
    for create in [false, true] {
        for phase in [
            SavePhase::Temp,
            SavePhase::Check,
            SavePhase::Rename,
            SavePhase::DirectorySync,
        ] {
            let mut steps = vec![EnvStep::RootGone];
            if !cfg!(target_os = "android") {
                steps.push(EnvStep::RootReplaced);
            }
            if !cfg!(target_os = "windows") {
                for parent in ["pages", "pages/sub/deep"] {
                    steps.push(EnvStep::SymlinkSwap(parent.into()));
                }
            }
            for step in steps {
                let mut n = Native::new(create);
                let mut m = model(create);
                at(&mut n.h, phase);
                at(&mut m, phase);
                n.apply_env(&step);
                // k=0 is the native phase boundary; no guard-per-slot claim.
                m.fs.inject_env(save_phase(phase), 0, step.clone());
                finish_pair(&mut n, &mut m);
                assert!(m.fs.env_fault.is_none());
                assert_eq!(
                    n.alpha(SPELLING),
                    m.fs.alpha(PAGE, false),
                    "{create}/{phase:?}/{step:?}"
                );
                assert!(n.h.events.iter().any(|e| matches!(
                    e,
                    Event::SaveOutcome {
                        outcome: Outcome::Failed | Outcome::Uncertain,
                        cause: Some(_),
                        ..
                    }
                )));
                keep_draft(&mut n.h);
                keep_draft(&mut m);
                n.escapes();
                m.fs.assert_no_escapes();
                n.reattach();
                n.h.fs.draft_sync().unwrap();
                assert_eq!(
                    drafts::scan(n.h.fs.draft_files(true)).logical[PAGE].bytes,
                    bytes(INPUT)
                );
                counts[usize::from(matches!(step, EnvStep::SymlinkSwap(_)))] += 1;
            }
        }
    }
    assert_eq!(
        counts,
        [
            if cfg!(target_os = "android") { 8 } else { 16 },
            if cfg!(target_os = "windows") { 0 } else { 16 }
        ]
    );
    eprintln!("M3 C1/C2 native + model phase-boundary samples: {counts:?}");
}

fn save_phase(phase: SavePhase) -> Phase {
    match phase {
        SavePhase::Temp => Phase::PageTemp,
        SavePhase::Check => Phase::Read,
        SavePhase::Rename => Phase::PageRename,
        SavePhase::DirectorySync => Phase::PageSync,
        _ => unreachable!(),
    }
}

#[test]
fn m3_c3_c4_native_and_model_do_not_publish_a_stale_recreation() {
    let mut counts = [0, 0];
    for (family, replace) in [false, true].into_iter().enumerate() {
        for parent in ["pages", "pages/sub", "pages/sub/deep"] {
            for phase in [SavePhase::Temp, SavePhase::Check, SavePhase::Rename] {
                let mut n = Native::new(false);
                let mut m = model(false);
                at(&mut n.h, phase);
                at(&mut m, phase);
                let to = format!("{parent}-moved");
                let step = if replace {
                    EnvStep::InodeChange(parent.into())
                } else {
                    EnvStep::DirRename(parent.into(), to.clone())
                };
                n.apply_env(&step);
                m.fs.apply_env(step);
                assert_eq!(n.alpha(SPELLING), None);
                assert_eq!(m.fs.alpha(PAGE, false), None);
                if !replace {
                    let moved = SPELLING.replacen(parent, &to, 1);
                    assert_eq!(n.alpha(&moved), bytes(OLD));
                    assert_eq!(m.fs.alpha(&moved, false), bytes(OLD));
                }
                finish_pair(&mut n, &mut m);
                for events in [&n.h.events, &m.events] {
                    assert!(
                        !events.iter().any(|e| matches!(e, Event::Published { .. })),
                        "{replace}/{parent}/{phase:?}"
                    );
                }
                if phase != SavePhase::Rename {
                    assert!(
                        n.h.pages[PAGE].conflict && m.pages[PAGE].conflict,
                        "base guard must see parent loss"
                    );
                }
                assert_eq!(n.alpha(SPELLING), None);
                assert_eq!(m.fs.alpha(PAGE, false), None);
                keep_draft(&mut n.h);
                keep_draft(&mut m);
                n.escapes();
                m.fs.assert_no_escapes();
                counts[family] += 1;
            }
        }
    }
    assert_eq!(counts, [9, 9]);
}

#[test]
fn m3_native_alpha_and_escape_witness_negative_controls() {
    let mut n = Native::new(false);
    assert_eq!(n.alpha(SPELLING), bytes(OLD));
    if !cfg!(target_os = "windows") {
        n.apply_env(&EnvStep::SymlinkSwap("pages".into()));
        assert_eq!(n.alpha(SPELLING), None);
    }
    fs::write(n.outside.join("unguarded-write"), INPUT).unwrap();
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| n.escapes())).is_err());
}

#[test]
#[ignore = "M3-1: C3/C4 parent loss after rename is reported current/published"]
fn m3_finding_parent_loss_after_publish_must_not_report_current_durable() {
    let mut stale = vec![];
    for replace in [false, true] {
        for native in [false, true] {
            let step = if replace {
                EnvStep::InodeChange("pages/sub/deep".into())
            } else {
                EnvStep::DirRename("pages/sub/deep".into(), "pages/sub/moved".into())
            };
            if native {
                let mut n = Native::new(false);
                at(&mut n.h, SavePhase::DirectorySync);
                n.apply_env(&step);
                finish(&mut n.h);
                n.escapes();
                assert_eq!(n.alpha(SPELLING), None);
                if n.h.pages[PAGE].clean() {
                    stale.push(format!("NativeFs replace={replace}"));
                }
            } else {
                let mut m = model(false);
                at(&mut m, SavePhase::DirectorySync);
                m.fs.apply_env(step);
                finish(&mut m);
                assert_eq!(m.fs.alpha(PAGE, false), None);
                if m.pages[PAGE].clean() {
                    stale.push(format!("ModelFs replace={replace}"));
                }
            }
        }
    }
    assert!(
        stale.is_empty(),
        "C3/C4 stale directory sync reported current/durable: {stale:?}"
    );
}

#[test]
#[ignore = "M3-2: retained DirectoryCreation sync debt accepts a replaced directory epoch"]
fn m3_finding_c4_retained_directory_sync_must_not_acknowledge_a_new_inode() {
    let n = Native::new(true);
    let target = n.graph.join("new/deep");
    crate::model::checked_write_target(&n.graph, &target, "graph root", None).unwrap();
    crate::model::assert_checked_target(&target);
    let binding = crate::root_binding::RootBinding::new(&n.graph);
    let mut plan = crate::directory_durability::DirectoryCreation::new(&target, &n.graph).unwrap();
    assert!(plan
        .finish(&binding, |_| Err(stdio::ErrorKind::Other.into()))
        .is_err());
    fs::rename(n.graph.join("new"), n.graph.join("old-new")).unwrap();
    fs::create_dir_all(&target).unwrap();
    assert!(
        plan.finish(&binding, crate::directory_durability::sync_directory_entry)
            .is_err(),
        "C4 retained plan synced replacement paths and retired old epoch's sync debt"
    );
}

#[test]
#[ignore = "M3-2: ModelFs retained directory sync debt has no epoch witness, matching the native finding"]
fn m3_finding_c4_model_retained_directory_sync_must_not_acknowledge_a_new_inode() {
    let mut fs = ModelFs::default();
    fs.spell(PAGE, "pages/new/deep/a.md");
    fs.env.seed_dirs("pages");
    // mkdir new, mkdir deep, then fail the first owed sync.
    fs.inject(
        Phase::PageTemp,
        [Fault::Errno(stdio::ErrorKind::Other, Some(2))],
    );
    assert!(fs.page_temp(PAGE, &bytes(INPUT)).is_err());
    assert_eq!(fs.owed, ["dir/pages/new", "dir/pages/new/deep"]);
    fs.apply_env(EnvStep::InodeChange("pages/new".into()));
    let result = fs.page_temp(PAGE, &bytes(INPUT));
    assert!(
        result.is_err(),
        "C4 ModelFs retired old epoch debt by syncing replacement paths: {result:?}, owed={:?}",
        fs.owed
    );
}

// Approved Android vanish-only binding cannot detect C1 replacement.
#[cfg(not(target_os = "android"))]
#[test]
fn m3_native_m2_1_replaced_root_between_plan_and_finish() {
    let mut n = Native::new(true);
    let target = n.graph.join("new/deep");
    crate::model::checked_write_target(&n.graph, &target, "graph root", None).unwrap();
    crate::model::assert_checked_target(&target);
    let binding = crate::root_binding::RootBinding::new(&n.graph);
    let mut plan = crate::directory_durability::DirectoryCreation::new(&target, &n.graph).unwrap();
    n.apply_env(&EnvStep::RootReplaced);
    let result = plan.finish(&binding, crate::directory_durability::sync_directory_entry);
    n.escapes();
    assert!(result.is_err());
}

#[cfg(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "ios",
    target_os = "android"
))]
#[test]
fn m3_native_m2_2_symlink_between_plan_and_finish() {
    let mut n = Native::new(true);
    let target = n.graph.join("pages/new/deep");
    crate::model::checked_write_target(&n.graph, &target, "graph root", None).unwrap();
    crate::model::assert_checked_target(&target);
    let binding = crate::root_binding::RootBinding::new(&n.graph);
    let mut plan = crate::directory_durability::DirectoryCreation::new(&target, &n.graph).unwrap();
    n.apply_env(&EnvStep::SymlinkSwap("pages".into()));
    let result = plan.finish(&binding, crate::directory_durability::sync_directory_entry);
    n.escapes();
    assert!(result.is_err());
}

#[cfg(feature = "test-faults")]
#[test]
fn m3_c6_errno_classes_keep_sticky_failure_and_admitted_input() {
    let kinds = [
        stdio::ErrorKind::StorageFull,
        stdio::ErrorKind::Other,
        stdio::ErrorKind::PermissionDenied,
        stdio::ErrorKind::ReadOnlyFilesystem,
    ];
    let mut count = 0;
    for kind in kinds {
        for phase in [
            Phase::PageTemp,
            Phase::Read,
            Phase::PageRename,
            Phase::PageSync,
        ] {
            let mut n = Native::new(false);
            let mut m = model(false);
            // Reuse the public native fault seam, with its actual ErrorKind.
            crate::page_host::faults::fail(phase, kind, 1);
            n.reattach();
            m.fs.inject(phase, [Fault::Errno(kind, None)]);
            at(&mut n.h, SavePhase::Temp);
            at(&mut m, SavePhase::Temp);
            finish_pair(&mut n, &mut m);
            assert_eq!(m.fs.observed_errno, Some(kind));
            assert_eq!(n.alpha(SPELLING), m.fs.alpha(PAGE, false));
            assert!(n.h.pages[PAGE].risk && m.pages[PAGE].risk);
            for events in [&n.h.events, &m.events] {
                assert_eq!(
                    events
                        .iter()
                        .filter(|e| matches!(
                            e,
                            Event::SaveOutcome {
                                outcome: Outcome::Failed | Outcome::Uncertain,
                                cause: Some(_),
                                ..
                            }
                        ))
                        .count(),
                    1
                );
                assert!(!events.iter().any(|e| matches!(e, Event::Published { .. })));
            }
            keep_draft(&mut n.h);
            keep_draft(&mut m);
            n.escapes();
            n.reattach();
            n.h.fs.draft_sync().unwrap();
            assert_eq!(
                drafts::scan(n.h.fs.draft_files(true)).logical[PAGE].bytes,
                bytes(INPUT)
            );
            // A repaired filesystem permits retry; no success before then.
            // After an uncertain publication the first retry adopts equal
            // disk bytes as its base; the following save pays the sync.
            for _ in 0..2 {
                if !n.h.pages[PAGE].clean() {
                    at(&mut n.h, SavePhase::Temp);
                    finish(&mut n.h);
                }
                if !m.pages[PAGE].clean() {
                    at(&mut m, SavePhase::Temp);
                    finish(&mut m);
                }
            }
            assert!(n.h.pages[PAGE].clean() && m.pages[PAGE].clean());
            count += 1;
        }
        // Interior errno classes use the same Fault mechanism as pre-phase
        // errors. Model slots are explicitly not native syscall evidence.
        for k in 0..4 {
            let mut m = model(true);
            m.fs.spell(PAGE, "pages/new/deep/a.md");
            m.fs.inject(Phase::PageTemp, [Fault::Errno(kind, Some(k))]);
            at(&mut m, SavePhase::Temp);
            finish(&mut m);
            assert_eq!(m.fs.observed_errno, Some(kind));
            keep_draft(&mut m);
            m.fs.assert_no_escapes();
        }
    }
    assert_eq!(count, 16);
}

#[cfg(feature = "test-faults")]
#[test]
fn m3_c6_draft_failed_matches_i8_host_apply_row() {
    let mut count = 0;
    for kind in [
        stdio::ErrorKind::StorageFull,
        stdio::ErrorKind::Other,
        stdio::ErrorKind::PermissionDenied,
        stdio::ErrorKind::ReadOnlyFilesystem,
    ] {
        for phase in [Phase::DraftTemp, Phase::DraftRename, Phase::DraftSync] {
            // This operation has not admitted its new input until its draft
            // is durable. Start from the clean, observed original on both.
            let mut n = Native::fixture(false, false);
            let mut m = model_fixture(false, false);
            crate::page_host::faults::fail(phase, kind, 3);
            m.fs.inject(phase, [Fault::Errno(kind, None); 3]);
            if phase != Phase::DraftSync {
                crate::page_host::faults::fail(Phase::DraftSync, kind, 3);
                m.fs.inject(Phase::DraftSync, [Fault::Errno(kind, None); 3]);
            }
            n.reattach();
            let effects = [(PAGE.into(), (bytes(INPUT), bytes(OLD)))].into();
            fn refusal<F: HostIo>(h: &mut Host<F>, effects: &BTreeMap<String, (Text, Text)>) {
                let before = h.pages[PAGE].clone();
                assert_eq!(h.apply(effects, &[]), Disposition::Pending);
                let id = h.register_reply().unwrap();
                for _ in 0..100 {
                    if h.worker.is_none() {
                        break;
                    }
                    h.advance_draft();
                }
                assert!(h.worker.is_none());
                assert_eq!(h.order.slot(id).unwrap().reply, Some(OperationReply::DraftFailed), "I-8 row tine-store::page_host::apply: full/read-only/denied/broken app-data disk refuses whole");
                assert_eq!(h.pages[PAGE], before, "I-8 DraftFailed changes no input");
                assert!(h.order.view().next().is_none());
            }
            refusal(&mut n.h, &effects);
            refusal(&mut m, &effects);
            assert_eq!(n.alpha(SPELLING), bytes(OLD));
            assert_eq!(m.fs.alpha(PAGE, false), bytes(OLD));
            n.escapes();
            m.fs.assert_no_escapes();
            count += 1;
        }
    }
    assert_eq!(count, 12);
}

#[cfg(feature = "test-faults")]
#[test]
fn m3_c6_errno_notice_stays_sticky_until_publication_on_both_backends() {
    use crate::page_host::progress::{Clock, Progress};
    struct Now;
    impl Clock for Now {
        fn now_ms(&self) -> u64 {
            0
        }
    }
    fn check<F: HostIo>(h: Host<F>) -> Host<F> {
        let mut p = Progress::new(h, Now);
        // A post-rename failure can need an adopt-base attempt between two
        // failed syncs. Stop by observed failures, with a fixed upper bound.
        for _ in 0..6 {
            if p.notice(PAGE).failures == 3 {
                break;
            }
            p.with_host(|h| {
                at(h, SavePhase::Temp);
                finish(h);
            });
            assert!(!p.host.pages[PAGE].clean());
        }
        assert_eq!(p.notice(PAGE).failures, 3);
        assert!(
            p.notice(PAGE).save_error,
            "C6 error must reach the actual persistent notice policy"
        );
        p.with_host(keep_draft);
        assert!(
            p.notice(PAGE).save_error,
            "a durable draft does not clear the save error"
        );
        let notice = p.notice(PAGE);
        p.with_host(|_| ());
        assert_eq!(
            p.notice(PAGE),
            notice,
            "idle reconciliation must retain the notice"
        );
        for _ in 0..2 {
            if p.host.pages[PAGE].clean() {
                break;
            }
            p.with_host(|h| {
                at(h, SavePhase::Temp);
                finish(h);
            });
        }
        assert!(p.host.pages[PAGE].clean());
        assert!(!p.notice(PAGE).save_error);
        assert_eq!(p.notice(PAGE).failures, 0);
        p.host
    }
    let mut count = 0;
    for kind in [
        stdio::ErrorKind::StorageFull,
        stdio::ErrorKind::Other,
        stdio::ErrorKind::PermissionDenied,
        stdio::ErrorKind::ReadOnlyFilesystem,
    ] {
        for phase in [
            Phase::PageTemp,
            Phase::Read,
            Phase::PageRename,
            Phase::PageSync,
        ] {
            let mut n = Native::new(false);
            let mut m = model(false);
            crate::page_host::faults::fail(phase, kind, 3);
            n.reattach();
            m.fs.inject(phase, [Fault::Errno(kind, None); 3]);
            n.h = check(n.h);
            let m = check(m);
            assert_eq!(n.alpha(SPELLING), bytes(INPUT));
            assert_eq!(m.fs.alpha(PAGE, false), bytes(INPUT));
            n.escapes();
            m.fs.assert_no_escapes();
            count += 1;
        }
    }
    assert_eq!(count, 16);
}
