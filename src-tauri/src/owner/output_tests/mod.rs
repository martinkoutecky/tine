//! The command line's output contract, pinned (`docs/contracts/cli-output.md`;
//! CLI REVIEW-CODE-1 findings 6, 7, 8, 10).
//!
//! Three families:
//! - **doc ⇄ payload**: every verb's real payload (the verbs run over a real
//!   Store and page host) has exactly the fields the contract documents, with
//!   the presence it documents; the exit-code table and the diagnostic-id table
//!   equal the code's;
//! - **payload ⇄ text**: every field the contract marks as a *disclosure* field,
//!   when set, shows up in the human text (by the value it carries, not by
//!   wording), and the text is derived from the same payload the JSON prints;
//! - the specific outcomes of the findings: errors, query cues, attribution and
//!   truncation.
//!
//! This is a directory module (`output_tests/mod.rs`) because
//! `owner_tests::the_endpoint_has_no_webview_or_plugin_door` counts the `.rs`
//! files directly under `owner/`.
#![cfg(unix)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{json, Value};
use tine_store::{PageHost, Store};

use super::client::dial;
use super::endpoint::Gate;
use super::lease::{hello, Claimed};
use super::lock::OwnerLock;
use super::namespace::GraphPaths;
use super::serve::ServeOwner;
use super::terminal::{error_json, error_text, exit_code, render, serve_exit, EXIT_CODES};
use super::verbs::{answer_verb, internal, Diag, PREVIEW_NODES};
use super::wire::{Drafts, ErrorBody, Request};
use crate::state::{GraphSlot, PageHostSlot};

const UUID: &str = "6512bd43-d9ca-4e3b-9f7a-1a2b3c4d5e6f";
const BIG_UUID: &str = "c20ad4d7-6fe9-4b9a-8c64-5a0a1b2c3d4e";
const BIG_CHILDREN: usize = 2100;
const DOC: &str = include_str!("../../../../docs/contracts/cli-output.md");

// ---------------------------------------------------------------------------
// A real graph, store and page host; the verbs answered in process.

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    owners: PathBuf,
    slot: Arc<GraphSlot>,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let graph = dir.path().join("graph");
        for sub in ["pages", "journals", "logseq"] {
            std::fs::create_dir_all(graph.join(sub)).unwrap();
        }
        let write = |name: &str, text: &str| std::fs::write(graph.join(name), text).unwrap();
        write("logseq/config.edn", "{:file/name-format :triple-lowbar}\n");
        // The contract's outline example is this page, byte for byte.
        write(
            "pages/a.md",
            &format!("- one\n  id:: {UUID}\n  author:: claude\n\t- under one\n- TODO two\n"),
        );
        write(
            "pages/b.md",
            &format!("- see [[a]]\n  author:: erin\n- and (({UUID}))\n- TODO see [[a]] again\n"),
        );
        // Literal `author::` text inside a code fence is content, not attribution.
        write(
            "pages/fence.md",
            "- Example\n  ```\n  author:: Bob\n  ```\n- Real\n  author:: Alice\n  note:: x\n  ```\n  author:: Bob\n  ```\n",
        );
        write(
            "pages/org1.org",
            "* Heading\n:PROPERTIES:\n:author: carol\n:END:\nbody\n",
        );
        write(
            "pages/lookalike.md",
            "- text\n  Author::   Dave  \n- authors:: x\n- plain\n  authors:: y\n",
        );
        write("pages/props.md", "foo:: bar\n\n- x\n");
        write("pages/ns___child.md", "- in a namespace\n");
        write("pages/x1.md", "alias:: dup\n\n- x1\n");
        write("pages/x2.md", "alias:: dup\n\n- x2\n");
        write("journals/2026_10_12.md", "- today thing\n");
        let mut big = format!("- big\n  id:: {BIG_UUID}\n");
        for n in 0..BIG_CHILDREN {
            big.push_str(&format!("\t- child {n}\n"));
        }
        write("pages/big.md", &big);

        let app_data = dir.path().join("app");
        let owners = dir.path().join("owners");
        std::fs::create_dir_all(&app_data).unwrap();
        std::fs::create_dir_all(&owners).unwrap();
        let store = Arc::new(Store::open(&graph, Default::default()).unwrap().0);
        store.whole_graph().unwrap();
        let root = Store::canonical_root(&graph).unwrap();
        let mut slot = GraphSlot::new(store.clone(), root.clone());
        let host = PageHost::start_for_tests(&store, &app_data).unwrap();
        *slot.host.get_mut().unwrap() = PageHostSlot::Running(host);
        Self {
            _dir: dir,
            root,
            owners,
            slot: Arc::new(slot),
        }
    }

    fn ask(&self, verb: &str, args: Value) -> Result<Value, ErrorBody> {
        answer_verb(
            &self.slot,
            &Request {
                id: 1,
                verb: verb.into(),
                args,
            },
        )
    }

    fn ok(&self, verb: &str, args: Value) -> Value {
        self.ask(verb, args)
            .unwrap_or_else(|e| panic!("{verb} {e:?}"))
    }

    /// `owner.status` from the app's lease and from `tine serve`'s owner, over
    /// their real endpoints.
    fn statuses(&self) -> Vec<Value> {
        let paths = |name: &str| GraphPaths {
            lock: self.owners.join(format!("{name}.lock")),
            socket: self.owners.join(format!("{name}.sock")),
            start_log: self.owners.join(format!("{name}.start.log")),
        };
        let claim = |paths: &GraphPaths, kind: &str| {
            Claimed::from_lock(
                OwnerLock::try_take(&paths.lock, kind).unwrap().unwrap(),
                GraphPaths {
                    lock: paths.lock.clone(),
                    socket: paths.socket.clone(),
                    start_log: paths.start_log.clone(),
                },
            )
        };
        let status = |paths: &GraphPaths| {
            dial(&paths.socket, &self.root)
                .unwrap()
                .expect("an endpoint")
                .call("owner.status", Value::Null)
                .unwrap()
        };
        let app = paths("app");
        let _app_lease = claim(&app, "app").serve_app(&self.slot);
        let serve = paths("serve");
        let gate = Gate::new();
        let owner = Arc::new(ServeOwner {
            slot: self.slot.clone(),
            gate: gate.clone(),
            root: self.root.clone(),
            drafts: Drafts {
                app_data: "a".into(),
                graph_id: "g".into(),
            },
            stops: std::sync::mpsc::channel().0,
        });
        let _serve_lease = claim(&serve, "serve").serve(hello(&self.root, "serve"), gate, owner);
        vec![status(&app), status(&serve)]
    }

    /// Every verb's real payloads, by contract section.
    fn payloads(&self) -> Vec<(&'static str, Value)> {
        let page = |name: &str| self.ok("page.show", json!({ "name": name }));
        let query = |text: &str| self.ok("query", json!({ "text": text }));
        let mut all = vec![
            ("page.show", page("a")),
            ("page.show", page("props")),
            (
                "page.show",
                self.ok("journal", json!({ "date": "2026-10-12" })),
            ),
            ("block.show", self.ok("block.show", json!({ "uuid": UUID }))),
            (
                "block.show",
                self.ok("block.show", json!({ "uuid": BIG_UUID })),
            ),
            ("refs", self.ok("refs", json!({ "target": "a" }))),
            (
                "refs",
                self.ok("refs", json!({ "target": UUID, "block": true })),
            ),
            ("search", self.ok("search", json!({ "text": "two" }))),
            ("search", self.ok("search", json!({ "text": "props" }))),
            ("query", query("(task)")),
            ("query", query("[[a]]")),
            ("query", query("(page-property foo bar)")),
            ("query", query(ADVANCED_RUNS)),
            ("query", query(ADVANCED_REFUSED)),
            ("pages", self.ok("pages", Value::Null)),
            ("pages", self.ok("pages", json!({ "namespace": "NS" }))),
        ];
        all.extend(self.statuses().into_iter().map(|s| ("owner.status", s)));
        all
    }
}

/// An advanced (datalog) form Tine can run, and one it refuses whole.
const ADVANCED_RUNS: &str =
    "[:find (pull ?b [*]) :where [?b :block/marker ?m] [(contains? #{\"TODO\" \"NOW\"} ?m)]]";
const ADVANCED_REFUSED: &str = "[:find ?x :where [?x :no/such ?y] [(foo ?y)]]";

// ---------------------------------------------------------------------------
// The contract document, read as data.

#[derive(Debug, Clone)]
struct Row {
    field: String,
    presence: String,
    disclosure: bool,
}

fn backticked(cell: &str) -> String {
    cell.trim().trim_matches('`').to_string()
}

fn cells(line: &str) -> Vec<String> {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(|c| c.trim().to_string())
        .collect()
}

/// The lines of the section whose heading line contains `needle`, up to the
/// next heading of the same or higher level.
fn section(heading_needle: &str) -> Vec<&'static str> {
    let mut lines = DOC.lines();
    let level = loop {
        let line = lines
            .next()
            .unwrap_or_else(|| panic!("no section {heading_needle}"));
        if line.starts_with('#') && line.contains(heading_needle) {
            break line.chars().take_while(|c| *c == '#').count();
        }
    };
    let mut body = Vec::new();
    let mut fenced = false;
    for line in lines {
        if line.starts_with("```") {
            fenced = !fenced;
        }
        if !fenced
            && line.starts_with('#')
            && line.chars().take_while(|c| *c == '#').count() <= level
        {
            break;
        }
        body.push(line);
    }
    body
}

/// The five-column field tables of a verb's section.
fn field_rows(heading_needle: &str) -> Vec<Row> {
    section(heading_needle)
        .into_iter()
        .filter(|l| l.starts_with('|') && !l.starts_with("|---"))
        .map(cells)
        .filter(|c| c.len() == 5 && c[0] != "field")
        .map(|c| Row {
            field: backticked(&c[0]),
            presence: c[2].clone(),
            disclosure: c[3] == "yes",
        })
        .collect()
}

/// The paths named after "Read by the text output:" in a verb's section.
fn read_by(heading_needle: &str) -> Vec<String> {
    let body = section(heading_needle).join("\n");
    let at = body
        .find("Read by the text output")
        .unwrap_or_else(|| panic!("{heading_needle}: no 'Read by the text output'"));
    let tail = &body[at..];
    let end = tail.find("\n\n").unwrap_or(tail.len());
    tail[..end]
        .split('`')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect()
}

/// Every value `path` reaches: segments split on `.`, `name[]` fans out an
/// array. A missing key reaches nothing; a present `null` reaches the null.
fn values_at<'a>(value: &'a Value, path: &str) -> Vec<&'a Value> {
    let mut now = vec![value];
    for segment in path.split('.') {
        let (key, fan) = match segment.strip_suffix("[]") {
            Some(key) => (key, true),
            None => (segment, false),
        };
        let mut next = Vec::new();
        for v in now {
            if let Some(child) = v.get(key) {
                if fan {
                    next.extend(child.as_array().map(|a| a.iter()).into_iter().flatten());
                } else {
                    next.push(child);
                }
            }
        }
        now = next;
    }
    now
}

// ---------------------------------------------------------------------------
// doc ⇄ payload

#[test]
fn every_payload_has_exactly_the_documented_fields() {
    let fixture = Fixture::new();
    let payloads = fixture.payloads();
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    for (section_name, payload) in &payloads {
        *seen.entry(section_name).or_default() += 1;
        let rows = field_rows(&format!("`{section_name}`"));
        assert!(
            !rows.is_empty(),
            "{section_name}: no field table in the contract"
        );
        // Top level: no undocumented key; every always/nullable key present.
        let top: BTreeMap<&str, &Row> = rows
            .iter()
            .filter(|r| !r.field.contains('.'))
            .map(|r| (r.field.as_str(), r))
            .collect();
        for key in payload.as_object().unwrap().keys() {
            assert!(
                top.contains_key(key.as_str()),
                "{section_name}: payload field `{key}` has no row in docs/contracts/cli-output.md ({payload})"
            );
        }
        for (field, row) in &top {
            if row.presence != "optional" {
                assert!(
                    payload.get(*field).is_some(),
                    "{section_name}: documented `{field}` ({}) is missing from a payload: {payload}",
                    row.presence
                );
            }
        }
        // Nested objects the contract enumerates: no undocumented key, every
        // always/nullable child present.
        let prefixes: BTreeSet<&str> = rows
            .iter()
            .filter_map(|r| r.field.rsplit_once('.').map(|(prefix, _)| prefix))
            .collect();
        for prefix in prefixes {
            let children: Vec<&Row> = rows
                .iter()
                .filter(|r| {
                    r.field
                        .strip_prefix(prefix)
                        .and_then(|rest| rest.strip_prefix('.'))
                        .is_some_and(|rest| !rest.contains('.'))
                })
                .collect();
            for object in values_at(payload, prefix) {
                let Some(map) = object.as_object() else {
                    continue; // null, e.g. `revision` of an unheld page
                };
                for key in map.keys() {
                    assert!(
                        children
                            .iter()
                            .any(|r| r.field.ends_with(&format!(".{key}"))),
                        "{section_name}: `{prefix}.{key}` has no row in the contract"
                    );
                }
                for row in &children {
                    if row.presence != "optional" {
                        let key = row.field.rsplit('.').next().unwrap();
                        assert!(
                            map.contains_key(key),
                            "{section_name}: documented `{}` missing from {object}",
                            row.field
                        );
                    }
                }
            }
        }
    }
    for needed in [
        "owner.status",
        "page.show",
        "block.show",
        "refs",
        "search",
        "query",
        "pages",
    ] {
        assert!(seen.contains_key(needed), "no fixture payload for {needed}");
    }
    // The optional anchors are exclusive and `og_hint` rides only the bare form.
    for (name, payload) in &payloads {
        if *name == "query" {
            let anchor = payload["anchor"].as_str().unwrap();
            assert_eq!(payload.get("groups").is_some(), anchor == "block");
            assert_eq!(payload.get("pages").is_some(), anchor == "page");
        }
    }
}

#[test]
fn the_documented_dto_fields_the_text_reads_exist_in_real_payloads() {
    let fixture = Fixture::new();
    let payloads = fixture.payloads();
    for section_name in [
        "page.show",
        "block.show",
        "refs",
        "search",
        "query",
        "pages",
    ] {
        for path in read_by(&format!("`{section_name}`")) {
            let reached = payloads
                .iter()
                .filter(|(name, _)| *name == section_name)
                .any(|(_, p)| !values_at(p, &path).is_empty());
            assert!(
                reached,
                "{section_name}: the text output reads `{path}`, but no real payload carries it"
            );
        }
    }
}

#[test]
fn the_exit_table_in_the_contract_is_the_codes_table() {
    let documented: BTreeMap<String, i32> = section("Error codes and exit codes")
        .into_iter()
        .filter(|l| l.starts_with('|') && !l.starts_with("|---"))
        .map(cells)
        .filter(|c| c.len() == 3 && c[0] != "code")
        .map(|c| (backticked(&c[0]), c[1].parse().unwrap()))
        .collect();
    let coded: BTreeMap<String, i32> = EXIT_CODES
        .iter()
        .map(|(code, exit)| (code.to_string(), *exit))
        .collect();
    assert_eq!(
        documented, coded,
        "docs/contracts/cli-output.md vs terminal.rs EXIT_CODES"
    );
    assert_eq!(
        coded.len(),
        EXIT_CODES.len(),
        "a code appears twice in EXIT_CODES"
    );
    // PROTOCOL §2.3's table, the part slice 1 can produce.
    for (code, exit) in [
        ("not-found", 3),
        ("ambiguous", 3),
        ("busy", 4),
        ("unknown-verb", 2),
        ("bad-args", 2),
        ("too-large", 2),
        ("protocol", 5),
        ("owner-mismatch", 5),
        ("internal", 1),
    ] {
        assert_eq!(exit_code(code), exit, "{code}");
    }
    assert_eq!(exit_code("a-code-from-a-newer-owner"), 1);
    // Every code the owner side can name in slice 1 is in the table.
    let fixture = Fixture::new();
    for (verb, args) in [
        ("page.show", json!({ "name": "nope" })),
        ("page.show", json!({ "name": "dup" })),
        ("journal", json!({ "date": "soon" })),
        ("block.show", json!({ "uuid": "3" })),
        ("page.show", json!({ "page": "a" })),
        ("page.edit", Value::Null),
        ("query", json!({ "text": "(nosuch x)" })),
    ] {
        let body = fixture.ask(verb, args).expect_err(verb);
        assert!(
            EXIT_CODES.iter().any(|(code, _)| *code == body.code),
            "{verb}: `{}` is not in the exit table",
            body.code
        );
    }
    for error in [
        super::OwnerError::Unsafe(String::new()),
        super::OwnerError::Lock(String::new()),
        super::OwnerError::Start(String::new()),
        super::OwnerError::Protocol(String::new()),
        super::OwnerError::Mismatch(String::new()),
        super::OwnerError::Unsupported,
    ] {
        assert_eq!(exit_code(error.code()), 5, "{}", error.code());
    }
}

#[test]
fn the_diagnostic_ids_in_the_contract_are_the_codes() {
    let documented: BTreeSet<String> = section("Diagnostic ids")
        .into_iter()
        .filter(|l| l.starts_with('|') && !l.starts_with("|---"))
        .map(cells)
        .filter(|c| c.len() == 2 && c[0] != "id")
        .map(|c| backticked(&c[0]))
        .collect();
    let coded: BTreeSet<String> = Diag::ALL.iter().map(|d| d.id().to_string()).collect();
    assert_eq!(
        documented, coded,
        "docs/contracts/cli-output.md vs verbs.rs Diag::ALL"
    );
    assert_eq!(
        coded.len(),
        Diag::ALL.len(),
        "two Diag variants share an id"
    );
    for diag in Diag::ALL {
        let body = internal(diag, "the detail");
        assert_eq!(body.code, "internal");
        assert!(
            body.message.starts_with(&format!("{}: ", diag.id())),
            "an internal error's message names its diag id first: {}",
            body.message
        );
        let detail = body.detail.as_ref().unwrap();
        assert_eq!(detail["diag"], diag.id());
        assert_eq!(detail["admitted"], true);
        assert!(error_text(&body).contains(diag.id()));
    }
}

#[test]
fn the_error_envelope_fields_are_documented() {
    let documented: BTreeSet<String> = section("## Errors")
        .into_iter()
        .filter(|l| l.starts_with('|') && !l.starts_with("|---"))
        .map(cells)
        .filter(|c| c.len() == 4 && c[0] != "field")
        .map(|c| backticked(&c[0]))
        .collect();
    let mut refused = ErrorBody::new(
        "refused",
        "the opening app keeps its drafts elsewhere",
        false,
    );
    refused.detail = Some(json!({ "admitted": false, "reason": "draft-namespace" }));
    let fixture = Fixture::new();
    let samples = vec![
        refused,
        fixture
            .ask("page.show", json!({ "name": "dup" }))
            .unwrap_err(),
        fixture
            .ask("query", json!({ "text": "(nosuch x)" }))
            .unwrap_err(),
        internal(Diag::Encode, "x"),
        ErrorBody {
            code: "owner-start".into(),
            message: "no answer".into(),
            detail: None,
        },
    ];
    for body in samples {
        let line = error_json(&body);
        assert!(!line.contains('\n'), "the envelope is one line");
        let envelope: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(envelope["schema"], "tine.cli/1");
        let mut paths = vec!["schema".to_string()];
        for (key, value) in envelope["error"].as_object().unwrap() {
            paths.push(format!("error.{key}"));
            if let (true, Some(map)) = (key == "detail", value.as_object()) {
                paths.extend(map.keys().map(|k| format!("error.detail.{k}")));
            }
        }
        for path in paths {
            assert!(
                documented.contains(&path),
                "the error envelope carries `{path}`, which the contract does not document ({line})"
            );
        }
    }
}

#[test]
fn the_contract_outline_example_is_the_real_output() {
    let fixture = Fixture::new();
    let page = fixture.ok("page.show", json!({ "name": "a" }));
    let mut text = String::new();
    render(&page, &mut text);
    let example = section("The text outline")
        .into_iter()
        .skip_while(|l| !l.starts_with("```"))
        .skip(1)
        .take_while(|l| !l.starts_with("```"))
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        text.trim_end(),
        example,
        "the outline example in cli-output.md"
    );
}

// ---------------------------------------------------------------------------
// payload ⇄ text: every disclosure field shows up

/// Lines of `after` that `before` does not have.
fn new_lines(before: &str, after: &str) -> String {
    let had: BTreeSet<&str> = before.lines().collect();
    after
        .lines()
        .filter(|l| !had.contains(l))
        .collect::<Vec<_>>()
        .join("\n")
}

fn text_of(payload: &Value) -> String {
    let mut out = String::new();
    render(payload, &mut out);
    out
}

fn set(payload: &mut Value, path: &str, value: Value) {
    let mut at = payload;
    let segments: Vec<&str> = path.split('.').collect();
    for segment in &segments[..segments.len() - 1] {
        at = &mut at[*segment];
    }
    at[segments[segments.len() - 1]] = value;
}

/// One disclosure field: set in a clean payload, the text gains a line that
/// carries the value (or, for a flag, one of `words`).
struct Disclosure {
    section: &'static str,
    field: &'static str,
    /// The clean payload.
    base: fn(&Fixture) -> Value,
    /// Set the field.
    on: fn(&mut Value),
    /// The new text must contain one of these (case-insensitive).
    words: &'static [&'static str],
}

fn page_a(f: &Fixture) -> Value {
    f.ok("page.show", json!({ "name": "a" }))
}
fn block_a(f: &Fixture) -> Value {
    f.ok("block.show", json!({ "uuid": UUID }))
}
fn search_two(f: &Fixture) -> Value {
    f.ok("search", json!({ "text": "two" }))
}
fn query_ab(f: &Fixture) -> Value {
    f.ok("query", json!({ "text": "[[a]]" }))
}

const DISCLOSURES: &[Disclosure] = &[
    Disclosure {
        section: "page.show",
        field: "read_only",
        base: page_a,
        on: |p| set(p, "read_only", json!("reason-zq41")),
        words: &["reason-zq41"],
    },
    Disclosure {
        section: "block.show",
        field: "block.truncated",
        base: block_a,
        on: |p| set(p, "block.truncated", json!(1337)),
        words: &["1337"],
    },
    Disclosure {
        section: "search",
        field: "diagnostics",
        base: search_two,
        on: |p| {
            set(
                p,
                "diagnostics",
                json!([{ "code": "code-k9", "message": "message-k9" }]),
            )
        },
        words: &["message-k9"],
    },
    Disclosure {
        section: "search",
        field: "has_more.pages",
        base: search_two,
        on: |p| set(p, "has_more.pages", json!(true)),
        words: &["page"],
    },
    Disclosure {
        section: "search",
        field: "has_more.blocks",
        base: search_two,
        on: |p| set(p, "has_more.blocks", json!(true)),
        words: &["block"],
    },
    Disclosure {
        section: "search",
        field: "cancelled",
        base: search_two,
        on: |p| set(p, "cancelled", json!(true)),
        words: &["cancel"],
    },
    Disclosure {
        section: "query",
        field: "diagnostics",
        base: query_ab,
        on: |p| {
            set(
                p,
                "diagnostics",
                json!([{ "message": "message-d3", "kind": "syntax", "disabled": false }]),
            )
        },
        words: &["message-d3"],
    },
    Disclosure {
        section: "query",
        field: "report.ignored",
        base: query_ab,
        on: |p| set(p, "report.ignored", json!(["clause-w8"])),
        words: &["clause-w8"],
    },
    Disclosure {
        section: "query",
        field: "report.supported",
        base: query_ab,
        on: |p| set(p, "report.supported", json!(false)),
        words: &["support"],
    },
    Disclosure {
        section: "query",
        field: "exceeded",
        base: query_ab,
        on: |p| set(p, "exceeded", json!(true)),
        words: &["truncat", "incomplete", "limit"],
    },
];

#[test]
fn every_disclosure_field_the_contract_names_is_tested_and_shows_in_the_text() {
    // The contract's `yes` rows are exactly the cases below.
    let documented: BTreeSet<(String, String)> = [
        "owner.status",
        "page.show",
        "block.show",
        "refs",
        "search",
        "query",
        "pages",
    ]
    .iter()
    .flat_map(|section_name| {
        field_rows(&format!("`{section_name}`"))
            .into_iter()
            .filter(|r| r.disclosure)
            .map(move |r| (section_name.to_string(), r.field))
    })
    .collect();
    let covered: BTreeSet<(String, String)> = DISCLOSURES
        .iter()
        .map(|d| (d.section.to_string(), d.field.to_string()))
        .chain([("query".to_string(), "og_hint".to_string())])
        .collect();
    assert_eq!(
        documented, covered,
        "the contract's disclosure rows and this test's cases must be the same set"
    );

    let fixture = Fixture::new();
    for case in DISCLOSURES {
        let clean = (case.base)(&fixture);
        let base = text_of(&clean);
        let mut set_payload = clean.clone();
        (case.on)(&mut set_payload);
        assert_ne!(clean, set_payload, "{}: the case sets nothing", case.field);
        let gained = new_lines(&base, &text_of(&set_payload)).to_lowercase();
        assert!(
            case.words.iter().any(|w| gained.contains(w)),
            "{} {}: set in the JSON ({set_payload}) but the text says nothing about it; new text: {gained:?}",
            case.section,
            case.field
        );
    }

    // `og_hint`: the bare form's cue names the bare head and prints every
    // explicit form the payload carries, verbatim.
    let hinted = fixture.ok("query", json!({ "text": "(task)" }));
    let mut bare = hinted.clone();
    bare.as_object_mut().unwrap().remove("og_hint");
    let gained = new_lines(&text_of(&bare), &text_of(&hinted));
    assert!(gained.contains("task"), "{gained}");
    let rewrites = hinted["og_hint"]["rewrites"].as_array().unwrap();
    assert!(!rewrites.is_empty());
    for rewrite in rewrites {
        let text = rewrite["text"].as_str().expect("an OG spelling");
        assert!(
            gained.contains(text),
            "the rewrite {text:?} is not in {gained:?}"
        );
    }
    assert_eq!(hinted["og_hint"]["no_conditions"], true);
    assert!(
        gained.contains("no condition"),
        "no_conditions: the text says the query shows nothing: {gained}"
    );
}

#[test]
fn a_disclosure_field_left_clear_adds_nothing_to_the_text() {
    // Positive-property guard: the cases above would pass vacuously if the
    // clean payloads already printed their words.
    let fixture = Fixture::new();
    for (clean, forbidden) in [
        (page_a(&fixture), "read-only"),
        (block_a(&fixture), "omitted"),
        (search_two(&fixture), "cancel"),
        (search_two(&fixture), "more "),
        (query_ab(&fixture), "warning"),
        (query_ab(&fixture), "hint"),
        (query_ab(&fixture), "truncated"),
    ] {
        let text = text_of(&clean);
        assert!(
            !text.to_lowercase().contains(forbidden),
            "a clean payload prints {forbidden:?}: {text}"
        );
    }
}

// ---------------------------------------------------------------------------
// F6: errors

#[test]
fn a_refusal_keeps_its_reason_in_the_stable_discriminator() {
    let mut body = ErrorBody::new(
        "refused",
        "the opening app keeps its crash drafts elsewhere",
        false,
    );
    body.detail = Some(json!({ "admitted": false, "reason": "draft-namespace" }));
    assert_eq!(
        error_text(&body),
        "tine: refused:draft-namespace: the opening app keeps its crash drafts elsewhere"
    );
    // Other codes keep `tine: <code>: <message>`; a refusal without a reason too.
    assert_eq!(
        error_text(&ErrorBody::new("not-found", "no page", true)),
        "tine: not-found: no page"
    );
    assert_eq!(
        error_text(&ErrorBody::new("refused", "no reason given", false)),
        "tine: refused: no reason given"
    );
}

#[test]
fn an_ambiguous_name_and_a_query_that_does_not_parse_say_what_the_json_says() {
    let fixture = Fixture::new();
    let ambiguous = fixture
        .ask("page.show", json!({ "name": "dup" }))
        .unwrap_err();
    assert_eq!(ambiguous.code, "ambiguous");
    let text = error_text(&ambiguous);
    for candidate in ambiguous.detail.as_ref().unwrap()["candidates"]
        .as_array()
        .unwrap()
    {
        assert!(text.contains(candidate.as_str().unwrap()), "{text}");
    }
    let broken = fixture
        .ask("query", json!({ "text": "(nosuch x)" }))
        .unwrap_err();
    assert_eq!(broken.code, "bad-args");
    let diagnostics = broken.detail.as_ref().unwrap()["diagnostics"]
        .as_array()
        .expect("a query that does not parse carries its diagnostics");
    assert!(!diagnostics.is_empty());
    let text = error_text(&broken);
    for diagnostic in diagnostics {
        assert!(
            text.contains(diagnostic["message"].as_str().unwrap()),
            "{text}"
        );
    }
}

#[test]
fn tine_serve_exits_5_for_its_typed_startup_errors_and_0_for_a_clean_stop() {
    use super::serve::ServeFailure;
    assert_eq!(serve_exit(Ok(())), 0);
    for error in [
        super::OwnerError::Lock("the lock call failed".into()),
        super::OwnerError::Unsafe("the directory is not yours".into()),
        super::OwnerError::Unsupported,
    ] {
        let code = error.code();
        assert_eq!(serve_exit(Err(ServeFailure::Claim(error))), 5, "{code}");
    }
    // Anything else is an internal failure, whatever its text says.
    for message in [
        "cannot open the command endpoint",
        "owner-lock: looks like a code but is not one",
        "no app data directory for crash drafts",
    ] {
        assert_eq!(serve_exit(Err(message.into())), 1, "{message}");
    }
}

// ---------------------------------------------------------------------------
// F7: query cues

#[test]
fn a_bare_query_carries_its_cue_and_the_text_prints_it() {
    let fixture = Fixture::new();
    let payload = fixture.ok("query", json!({ "text": "(task)" }));
    let hint = &payload["og_hint"];
    assert_eq!(hint["bare"], json!(["task"]));
    assert_eq!(hint["no_conditions"], true);
    let kinds: Vec<&str> = hint["rewrites"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["rewrite"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["open_tasks", "any_task"]);
    // The text of a rewrite is a Logseq query that parses back without a cue
    // and has no bare head (D-18: the explicit form means what it says).
    for rewrite in hint["rewrites"].as_array().unwrap() {
        let text = rewrite["text"].as_str().unwrap();
        assert!(text.starts_with("(task TODO"), "{text}");
        let again = fixture.ok("query", json!({ "text": text }));
        assert!(again.get("og_hint").is_none(), "{text} still needs a cue");
    }
    // An explicit form needs none.
    assert!(fixture
        .ok("query", json!({ "text": "(task TODO)" }))
        .get("og_hint")
        .is_none());
    let text = text_of(&payload);
    assert!(text.contains("(task)"), "{text}");
}

#[test]
fn an_advanced_query_runs_and_its_report_reaches_the_text() {
    let fixture = Fixture::new();
    let runs = fixture.ok("query", json!({ "text": ADVANCED_RUNS }));
    assert_eq!(runs["report"]["supported"], true);
    assert!(!runs["report"]["ran"].as_array().unwrap().is_empty());
    assert!(!text_of(&runs).to_lowercase().contains("warning"));

    let refused = fixture.ok("query", json!({ "text": ADVANCED_REFUSED }));
    assert_eq!(refused["report"]["supported"], false);
    let ignored = refused["report"]["ignored"].as_array().unwrap();
    assert!(!ignored.is_empty());
    let text = text_of(&refused);
    assert!(
        text.to_lowercase().contains("not fully supported"),
        "{text}"
    );
    for item in ignored {
        assert!(text.contains(item.as_str().unwrap()), "{text}");
    }
    // The engine's own diagnostic reaches the text too.
    for diagnostic in refused["diagnostics"].as_array().unwrap() {
        assert!(
            text.contains(diagnostic["message"].as_str().unwrap()),
            "{text}"
        );
    }
}

// ---------------------------------------------------------------------------
// F8: attribution and truncation

#[test]
fn author_sits_beside_the_address_not_on_a_later_line() {
    let fixture = Fixture::new();
    let text = text_of(&fixture.ok("page.show", json!({ "name": "a" })));
    let lines: Vec<&str> = text.lines().collect();
    assert!(
        lines.contains(&"1 [author:: claude] one"),
        "attribution beside the address: {text}"
    );
    assert!(
        !lines.iter().any(|l| l.trim_start().starts_with("author::")),
        "the author:: line is not repeated below the block: {text}"
    );
    // Other properties stay as lines, an unauthored block is unchanged, and
    // the JSON keeps the raw text.
    assert!(
        lines.contains(&"  id:: 6512bd43-d9ca-4e3b-9f7a-1a2b3c4d5e6f"),
        "{text}"
    );
    assert!(lines.contains(&"2 TODO two"), "{text}");
    let payload = fixture.ok("page.show", json!({ "name": "a" }));
    assert!(payload["page"]["blocks"][0]["raw"]
        .as_str()
        .unwrap()
        .contains("author:: claude"));
    // `block show` and `refs` use the same outline.
    let block = text_of(&fixture.ok("block.show", json!({ "uuid": UUID })));
    assert!(block.contains("1 [author:: claude] one"), "{block}");
}

#[test]
fn author_attribution_is_the_projections_not_a_scan_of_the_lines() {
    let fixture = Fixture::new();
    let outline = |name: &str| text_of(&fixture.ok("page.show", json!({ "name": name })));

    // Code fence content (CODE-2 F6): a block whose only `author::` is inside a
    // fence has no author; the literal line stays where it is.
    let fence = outline("fence");
    let lines: Vec<&str> = fence.lines().collect();
    let at = lines
        .iter()
        .position(|l| *l == "1 Example")
        .unwrap_or_else(|| panic!("{fence}"));
    assert_eq!(
        &lines[at..at + 4],
        ["1 Example", "  ```", "  author:: Bob", "  ```"],
        "{fence}"
    );
    // A block with a real property AND a fenced line: the property is the
    // attribution (and is not repeated); the fenced line stays.
    let at = lines
        .iter()
        .position(|l| *l == "2 [author:: Alice] Real")
        .unwrap_or_else(|| panic!("{fence}"));
    assert_eq!(
        &lines[at..],
        [
            "2 [author:: Alice] Real",
            "  note:: x",
            "  ```",
            "  author:: Bob",
            "  ```"
        ],
        "{fence}"
    );

    // Org drawer.
    let org = outline("org1");
    assert!(org.contains("1 [author:: carol] Heading"), "{org}");
    assert!(!org.contains(":author:"), "{org}");

    // Key spelling and look-alikes: the value is trimmed; `authors::` is another key.
    let look = outline("lookalike");
    assert!(look.contains("1 [author:: Dave] text"), "{look}");
    assert!(!look.contains("Dave  "), "{look}");
    assert!(
        look.contains("2 authors:: x"),
        "the first line is text: {look}"
    );
    assert!(
        look.contains("3 plain") && look.contains("authors:: y"),
        "{look}"
    );
    assert!(
        !look.contains("[author:: x]") && !look.contains("[author:: y]"),
        "{look}"
    );

    // The same block reached through another verb.
    let query = text_of(&fixture.ok("query", json!({ "text": "[[a]]" })));
    assert!(query.contains("[author:: claude] one"), "{query}");
}

#[test]
fn a_preview_cut_at_the_node_bound_says_how_many_nodes_it_left_out() {
    let fixture = Fixture::new();
    let payload = fixture.ok("block.show", json!({ "uuid": BIG_UUID }));
    let omitted = BIG_CHILDREN + 1 - PREVIEW_NODES;
    assert_eq!(
        payload["block"]["truncated"], omitted,
        "the preview holds PREVIEW_NODES nodes, the block counts as one"
    );
    let text = text_of(&payload);
    assert!(text.contains(&omitted.to_string()), "{text}");
    assert!(
        text.contains(&PREVIEW_NODES.to_string()),
        "the bound is named: {text}"
    );
    // A complete preview prints no such line.
    let whole = text_of(&fixture.ok("block.show", json!({ "uuid": UUID })));
    assert!(!whole.contains("omitted"), "{whole}");
    // `max_nodes` lowers the bound and the count follows it.
    let two = fixture.ok("block.show", json!({ "uuid": UUID, "max_nodes": 1 }));
    assert_eq!(two["block"]["truncated"], 1);
    assert!(
        text_of(&two).contains("1 node omitted"),
        "{}",
        text_of(&two)
    );
}

// ---------------------------------------------------------------------------
// Value semantics the producers back (CODE-2 F7)

#[test]
fn a_page_the_parser_refuses_has_no_payload_and_text_is_never_lossy() {
    let fixture = Fixture::new();
    // The contract: invalid UTF-8 is refused before any `text` exists.
    std::fs::write(fixture.root.join("journals/2026_10_13.md"), b"- \xff\n").unwrap();
    let refusal = fixture
        .ask("journal", json!({ "date": "2026-10-13" }))
        .expect_err("a journal holding invalid UTF-8 has no payload");
    assert_eq!(refusal.code, "internal");
    assert_eq!(refusal.message, "cli-store-failed: Undecodable");
    assert_eq!(refusal.detail.as_ref().unwrap()["diag"], "cli-store-failed");
    assert_eq!(exit_code(&refusal.code), 1);
    assert!(
        DOC.contains("cli-store-failed: Undecodable") && DOC.contains("\\xff"),
        "the contract states this refusal"
    );
    // So `text` is the file's text, byte for byte.
    let payload = fixture.ok("page.show", json!({ "name": "a" }));
    let file = std::fs::read_to_string(fixture.root.join("pages/a.md")).unwrap();
    assert_eq!(payload["text"], file);
}

#[test]
fn has_more_is_false_for_a_category_that_was_switched_off() {
    let fixture = Fixture::new();
    let search = |args: Value| fixture.ok("search", args);
    let hit_kinds = |payload: &Value| -> BTreeSet<String> {
        payload["hits"]
            .as_array()
            .unwrap()
            .iter()
            .map(|h| h["entity"].as_str().unwrap().to_string())
            .collect()
    };
    // `x` matches two pages by name and many blocks ("child N" holds no `x`,
    // so use text that both categories can match).
    let full = search(json!({ "text": "x" }));
    assert!(hit_kinds(&full).contains("page"), "{full}");
    assert_eq!(
        full["cancelled"], false,
        "tine search names no cancellation lane"
    );

    // A limit that cuts the list says so.
    let cut = search(json!({ "text": "x", "pages": 1 }));
    assert_eq!(cut["has_more"]["pages"], true, "{cut}");
    let cut = search(json!({ "text": "child", "blocks": 1 }));
    assert_eq!(cut["has_more"]["blocks"], true, "{cut}");

    // A category switched off with 0 is not searched: nothing of it is returned
    // and its flag is false, which means "not checked", not "complete".
    let off = search(json!({ "text": "x", "pages": 0, "blocks": 0 }));
    assert!(off["hits"].as_array().unwrap().is_empty(), "{off}");
    assert_eq!(off["has_more"], json!({ "pages": false, "blocks": false }));
    let no_pages = search(json!({ "text": "x", "pages": 0 }));
    assert!(!hit_kinds(&no_pages).contains("page"), "{no_pages}");
    assert_eq!(no_pages["has_more"]["pages"], false);
    let no_blocks = search(json!({ "text": "child", "blocks": 0 }));
    assert!(!hit_kinds(&no_blocks).contains("block"), "{no_blocks}");
    assert_eq!(no_blocks["has_more"]["blocks"], false);
    // The contract says both halves of this.
    assert!(
        DOC.contains("switched off") && DOC.contains("not checked"),
        "the contract states what false means for a switched-off category"
    );
}

#[test]
fn query_total_counts_the_returned_matches_unless_exceeded() {
    let fixture = Fixture::new();
    for text in ["[[a]]", "(task TODO)", ADVANCED_RUNS] {
        let payload = fixture.ok("query", json!({ "text": text }));
        assert_eq!(payload["exceeded"], false, "{text}");
        let returned: usize = values_at(&payload, "groups[].blocks")
            .iter()
            .map(|blocks| blocks.as_array().unwrap().len())
            .sum();
        assert!(returned > 0, "{text}: the fixture must match something");
        assert_eq!(payload["total"], returned, "{text}: {payload}");
    }
    let pages = fixture.ok("query", json!({ "text": "(page-property foo bar)" }));
    assert_eq!(pages["total"], pages["pages"].as_array().unwrap().len());
}
