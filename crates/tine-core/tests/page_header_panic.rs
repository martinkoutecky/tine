//! A page-header read must degrade, never panic, on text lsdoc v2 does not own.
//!
//! In the wasm build (`panic = "abort"`) a panic is an `unreachable` trap with
//! no message; the user saw "Unreachable code should not be executed
//! (evaluating 'page_header_json')" and one page refused to display. Native
//! tine-core hid the same bug behind `catch_unwind`. Invariants: I-2 (one bad
//! file must not refuse the graph), I-22 (every lsdoc call goes through
//! `crates/lsdoc-block-parse.rs`). Exemplar: `parse_text_bounded`.

use tine_core::block_regions::page_header;

#[test]
fn page_header_survives_bare_cr_after_empty_property() {
    for raw in ["- s::\r", "- tags:: x\rid:: ::}}"] {
        let result = std::panic::catch_unwind(|| page_header(raw));
        let header = result.unwrap_or_else(|_| panic!("page_header panicked on {raw:?}"));
        // An unowned page is quarantined as literal: no header, no entries.
        assert!(header.entries.is_empty(), "{raw:?} yielded entries");
        assert!(header.end <= raw.len());
    }
}
