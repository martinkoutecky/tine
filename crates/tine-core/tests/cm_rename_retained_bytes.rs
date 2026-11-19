//! PC-9 (CM B4 follow-up F2): what a prepared referrer keeps. A graph rename
//! holds one `RenameProposal` per changed referrer (its text, the complete-text
//! certificate and the model's reading of the input) from planning until its
//! own step's preflight and model check, which free it (tine-store
//! `prepare_steps`), so a referrer's retained bytes are a multiple of its own
//! size, summed over every referrer not yet checked. Measured on a synthetic 5,000-block
//! Markdown page (mixed link forms, 230 KB) on 2026-11-18: 6.96 MB retained
//! (30.3 bytes per input byte) and 14.0 MB peak while building (60.9); a
//! 20,000-block page gives 29.9 and 60.0, so both are linear in the input. The
//! budgets are those plus about 10%; a larger proposal must say why here.
use std::collections::HashMap;
use tine_core::config::FileNameFormat;

fn page(blocks: usize) -> String {
    let mut text = String::new();
    for i in 0..blocks {
        match i % 4 {
            0 => text.push_str(&format!(
                "- block {i} links [[Target]] and [[Target]] again\n"
            )),
            1 => text.push_str(&format!("- block {i} tagged #Target and #[[Target]]\n")),
            2 => text.push_str(&format!(
                "- block {i} lower [[target]] with [x]([[Target]])\n"
            )),
            _ => text.push_str(&format!("- block {i} plain words, no reference\n")),
        }
    }
    text
}

/// Bytes kept and peak while building, per byte of input, in thousandths.
const RETAINED_PER_INPUT_BYTE_MILLI: usize = 33_300;
const BUILD_PEAK_PER_INPUT_BYTE_MILLI: usize = 67_000;

#[test]
fn a_prepared_referrers_retained_bytes_stay_within_budget() {
    let text = page(5_000);
    let map = HashMap::from([("target".to_owned(), "Renamed".to_owned())]);
    let (proposal, retained, peak) = crate::og_p10_journal_date_cost::live_bytes(|| {
        tine_core::refs::rename_proposal(&text, false, &map, FileNameFormat::TripleLowbar).unwrap()
    });
    assert!(proposal.text.contains("[[Renamed]]"));
    let (retained_milli, peak_milli) = (retained * 1000 / text.len(), peak * 1000 / text.len());
    println!("RETAINED {retained} PEAK {peak} INPUT {} -> {retained_milli} / {peak_milli} milli-bytes per input byte", text.len());
    assert!(
        retained_milli <= RETAINED_PER_INPUT_BYTE_MILLI,
        "PC-9: a prepared referrer keeps {retained_milli} milli-bytes per input byte (budget {RETAINED_PER_INPUT_BYTE_MILLI}); parts: observed reading, trace pieces/output/substitution/starts, input copy, new text"
    );
    assert!(
        peak_milli <= BUILD_PEAK_PER_INPUT_BYTE_MILLI,
        "PC-9: building a proposal peaks at {peak_milli} milli-bytes per input byte (budget {BUILD_PEAK_PER_INPUT_BYTE_MILLI})"
    );
}
