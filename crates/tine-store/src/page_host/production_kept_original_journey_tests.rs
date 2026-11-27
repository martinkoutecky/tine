//! State setup for `scripts/e2e-gh540-kept-original.mjs` (GH #540): a rename
//! of `pages/Draft.md` to `pages/Final.md`, cancelled after the source's
//! removal published (`keep_an_original`: the receiver changed on disk, then
//! Use Disk on it), so the shipped app's next launch has one kept original.
//! Run once, in its own process, with `--ignored --exact`, against the
//! journey's fixture graph and the app's own draft directory:
//!
//! - `TINE_JOURNEY_GRAPH`: the graph root, holding the three pages;
//! - `TINE_JOURNEY_APP_DATA`: the app's data directory;
//! - `TINE_JOURNEY_GRAPH_ID`: the draft namespace the app created for that
//!   graph (`drafts-v2/<id>`), observed by the journey, not recomputed.
use super::*;

fn var(name: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("{name} is not set")))
}

#[test]
#[ignore = "native journey setup: run by scripts/e2e-gh540-kept-original.mjs"]
fn kept_original_journey_setup() {
    let (graph, app) = (var("TINE_JOURNEY_GRAPH"), var("TINE_JOURNEY_APP_DATA"));
    let id = std::env::var("TINE_JOURNEY_GRAPH_ID").expect("TINE_JOURNEY_GRAPH_ID");
    assert!(
        app.join("drafts-v2").join(&id).is_dir(),
        "the app's draft namespace"
    );
    super::keep_an_original(
        &graph,
        &app,
        &id,
        ["pages/Draft.md", "pages/Final.md", "pages/Index.md"],
    );
    eprintln!("SETUP kept original left");
}
