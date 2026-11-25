//! State setup for `scripts/e2e-launch-custody.mjs`: a decided superseding
//! pair left for the shipped app's next launch to judge (E1). Run once, in its
//! own process, with `--ignored --exact`, against the journey's fixture graph
//! and the app's own draft directory:
//!
//! - `TINE_JOURNEY_GRAPH`: the graph root, holding `pages/Source.md`;
//! - `TINE_JOURNEY_APP_DATA`: the app's data directory;
//! - `TINE_JOURNEY_GRAPH_ID`: the draft namespace the app created for that
//!   graph (`drafts-v2/<id>`), observed by the journey, not recomputed.
//!
//! The journey: the rename of Source to Receiver is interrupted before the
//! receiver publishes; the relaunch recovers it, the receiver publishes its
//! own text, and Use Disk on the source decides the pair; the process then
//! dies before the decision drafts retire. Uses only host interface develop
//! also has, so the same file prepares the state for a develop build.
use super::*;

const SOURCE: &str = "pages/Source.md";
const RECEIVER: &str = "pages/Receiver.md";
/// The receiver's own text, distinct from the source's.
const RECEIVER_TEXT: &str = "- receiver text\n";

fn var(name: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("{name} is not set")))
}

/// A process start on the journey's graph and app data: the same production
/// IO and launch the app's host runs.
fn start(graph: &Path, app: &Path, id: &str, root: tempfile::TempDir) -> Fixture {
    let trash = crate::model::trash_root(graph).join("pages");
    let io = ProductionIo::attach(graph, app, id, &trash);
    let locks = [SOURCE, RECEIVER]
        .into_iter()
        .map(|key| (key.into(), Arc::new(Mutex::new(()))))
        .collect();
    let mut host = shared_counters(Host::new(io, locks));
    host.stop();
    assert!(matches!(
        host.launch(),
        Disposition::Applied | Disposition::Pending
    ));
    let mut f = Fixture {
        graph: graph.into(),
        app: app.into(),
        trash,
        host,
        root,
    };
    f.drain();
    f
}

#[test]
#[ignore = "native journey setup: run by scripts/e2e-launch-custody.mjs"]
fn launch_custody_journey_setup() {
    let (graph, app) = (var("TINE_JOURNEY_GRAPH"), var("TINE_JOURNEY_APP_DATA"));
    let id = std::env::var("TINE_JOURNEY_GRAPH_ID").expect("TINE_JOURNEY_GRAPH_ID");
    assert!(
        app.join("drafts-v2").join(&id).is_dir(),
        "the app's draft namespace"
    );
    let source_text = fs::read_to_string(graph.join(SOURCE)).unwrap();
    // Unused sandbox: the fixture owns no directory of the journey's.
    let sandbox = || tempfile::tempdir().unwrap();
    let mut f = start(&graph, &app, &id, sandbox());
    assert_eq!(
        f.host.rename(
            SOURCE,
            RECEIVER,
            &BTreeSet::new(),
            "Source",
            "Receiver",
            tine_core::config::FileNameFormat::TripleLowbar
        ),
        Disposition::Pending
    );
    f.drain();
    assert!(
        !graph.join(RECEIVER).exists(),
        "the receiver is not published"
    );
    // First interruption: the process dies before the receiver publishes.
    drop(f);
    let mut f = start(&graph, &app, &id, sandbox());
    assert_eq!(
        f.host.pages[SOURCE].buf, None,
        "the source's removal is recovered"
    );
    f.edit(RECEIVER, RECEIVER_TEXT);
    assert_eq!(f.save(RECEIVER), Outcome::Published);
    f.send(SOURCE, RequestKind::Discard);
    f.drain();
    assert_eq!(fs::read_to_string(graph.join(SOURCE)).unwrap(), source_text);
    assert_eq!(
        fs::read_to_string(graph.join(RECEIVER)).unwrap(),
        RECEIVER_TEXT
    );
    let drafts = f.host.fs.draft_files(true).len();
    assert!(drafts >= 1, "the decided pair's drafts are not retired");
    // Second interruption: the process dies before the decision retires.
    drop(f);
    eprintln!("SETUP decided pair left with {drafts} draft files");
}
