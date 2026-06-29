//! Diff-oracle helper: emit `tine_core::render::parse_block` output for every
//! block in a `block-raws.json` ({raw, format} array), as a JSON array of ASTs.
//!
//! Paired with scripts/wasm-ipc-diff.mjs, which produces the SAME array via the
//! vendored wasm `parse_block_json` and asserts byte-equality — proving the
//! frontend wasm parser and the Rust index parser stay identical (guards the
//! duplicated bullet re-prepend, plan §7C/§3).
//!
//! Usage: cargo run -q -p tine-core --example parse-corpus -- <block-raws.json>

use serde_json::Value;
use std::{env, fs};

fn main() {
    let path = env::args().nth(1).expect("usage: parse-corpus <block-raws.json>");
    let text = fs::read_to_string(&path).expect("read block-raws.json");
    let records: Vec<Value> = serde_json::from_str(&text).expect("parse block-raws.json");

    let asts: Vec<Vec<tine_core::lsdoc::ast::Block>> = records
        .iter()
        .map(|r| {
            let raw = r.get("raw").and_then(Value::as_str).unwrap_or("");
            let is_org = r.get("format").and_then(Value::as_str) == Some("org");
            tine_core::render::parse_block(raw, is_org)
        })
        .collect();

    println!("{}", serde_json::to_string(&asts).expect("serialize asts"));
}
