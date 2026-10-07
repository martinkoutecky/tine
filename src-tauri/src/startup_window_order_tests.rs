//! The first window the process creates is `main`.
//!
//! A WebDriver session (every native real-app journey) attaches to the first
//! window the app creates. `main` is built in `setup` so the workspace-window
//! door can attach, and once built after the Quick Capture window every journey
//! that does not select its window by URL drove the hidden capture webview:
//! "Blocks: 1", no journal, 35 of 45 hosted catalog journeys red. The order is
//! pinned here because no unit test of either window can see it.

const LIB: &str = include_str!("lib.rs");

#[test]
fn main_is_built_before_the_other_startup_windows() {
    let main = LIB
        .find("workspace_windows::create_main(app, config, start_hidden)")
        .expect("setup must build main through workspace_windows::create_main");
    let others = LIB
        .find("youtube_identity::create_windows(app, &youtube_windows)")
        .expect("setup must build the other startup windows through youtube_identity::create_windows");
    assert!(
        main < others,
        "the first window created is the one a WebDriver session attaches to; build `main` before \
         the capture/about windows (the order every native E2E journey assumes; exemplar: \
         scripts/e2e-caret.mjs, which does not select a window)"
    );
}
