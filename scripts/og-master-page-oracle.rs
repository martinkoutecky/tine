// Scratch-only oracle runner at master ddf408c55. The output is checked in
// under scripts/fixtures/og-master-pages-golden.json in the og worktree.
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct Input { path: String, format: String, source: String }

#[derive(Serialize)]
struct Output { path: String, bytes: String }

fn main() {
    let file = std::env::args().nth(1).expect("input JSON path");
    let inputs: Vec<Input> = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
    let output: Vec<Output> = inputs.into_iter().map(|input| {
        let bytes = if input.format == "org" {
            tine_core::org::serialize_org_detect(
                &tine_core::org::parse_org(&input.source), Some(&input.source))
        } else {
            tine_core::doc::serialize_with(
                &tine_core::doc::parse(&input.source),
                &tine_core::doc::SerializeOpts::detect(Some(&input.source)))
        };
        Output { path: input.path, bytes }
    }).collect();
    println!("{}", serde_json::to_string_pretty(&output).unwrap());
}
