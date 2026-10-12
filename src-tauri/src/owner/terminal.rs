//! The command line's read commands (`tine page show`, `tine query`, …):
//! reach the graph's owner, send one request, print the answer. The answer
//! is the typed `ok` payload (schema `tine.cli/1`); `--json` prints it
//! as is, the default renders it as a compact outline (spec A3). One schema,
//! no second result type: the human text is derived from the same payload and
//! says whatever the payload discloses (truncation, ignored clauses, hints,
//! refusal reasons). `docs/contracts/cli-output.md` is the contract and
//! `output_tests` pins it to this code.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::client::{self, Failure};
use super::verbs::PREVIEW_NODES;
use super::wire::ErrorBody;

/// How long a read keeps asking through a handover or a restart.
const RETRY_FOR: Duration = Duration::from_secs(30);

/// The one exit mapping (PROTOCOL §2.3): every code a command line can
/// report, from an owner's reply or from reaching/becoming an owner, and the
/// process exit code it maps to. `0` is success; `2` also covers a usage
/// error before any request; an unlisted code (a newer owner's) exits
/// [`EXIT_UNLISTED`], as `internal` does. The contract's table is pinned to
/// this one.
pub(super) const EXIT_CODES: &[(&str, i32)] = &[
    ("not-found", 3),
    ("ambiguous", 3),
    ("busy", 4),
    ("unknown-verb", 2),
    ("bad-args", 2),
    ("frame-too-large", 2),
    ("malformed", 2),
    ("too-large", 2),
    ("protocol", 5),
    ("owner-mismatch", 5),
    ("owner-start", 5),
    ("owner-unsafe", 5),
    ("owner-lock", 5),
    ("unsupported", 5),
    ("stale", 6),
    ("refused", 6),
    ("draft-failed", 7),
    ("unwritten", 8),
    ("superseded", 8),
    ("uncertain", 8),
    ("internal", 1),
];

/// The exit code of a code the table does not list.
pub(super) const EXIT_UNLISTED: i32 = 1;
/// The exit code of a usage error found before any request (no graph).
const EXIT_USAGE: i32 = 2;

/// Stable exit code per error code (PROTOCOL §2.3).
pub(super) fn exit_code(code: &str) -> i32 {
    EXIT_CODES
        .iter()
        .find(|(known, _)| *known == code)
        .map_or(EXIT_UNLISTED, |(_, exit)| *exit)
}

/// Which graph a command reads, and how it prints.
pub(crate) struct Target {
    pub(crate) graph: Option<PathBuf>,
    pub(crate) json: bool,
}

/// Run one read verb; the process exit code (PROTOCOL §2.3).
pub(crate) fn read_verb(target: Target, verb: &str, args: Value) -> i32 {
    let root = match graph_root(target.graph) {
        Ok(root) => root,
        Err(message) => {
            eprintln!("tine: {message}");
            return EXIT_USAGE;
        }
    };
    let deadline = Instant::now() + RETRY_FOR;
    loop {
        let result = client::connect(&root)
            .map_err(Failure::Reach)
            .and_then(|mut conn| conn.call(verb, args.clone()));
        let retry = match &result {
            // A read is safe to ask again: busy is never admitted, and a
            // lost read changed nothing.
            Err(Failure::Owner(body)) => body.code == "busy",
            Err(Failure::Lost(_)) => true,
            _ => false,
        };
        if retry && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(200));
            continue;
        }
        return match result {
            Ok(ok) => {
                print(&ok, target.json);
                0
            }
            Err(failure) => report_failure(failure, target.json, "owner-start"),
        };
    }
}

/// Print a failed call the one way and return its exit code.
/// `lost_as` names a lost connection: `owner-start` for a read (it was
/// asked again until the deadline), `uncertain` for a request that may have
/// been admitted.
fn report_failure(failure: Failure, json: bool, lost_as: &str) -> i32 {
    let body = match failure {
        Failure::Owner(body) => body,
        Failure::Reach(error) => ErrorBody {
            code: error.code().into(),
            message: error.to_string(),
            detail: None,
        },
        Failure::Lost(message) => ErrorBody {
            code: lost_as.into(),
            message: format!("the owner stopped answering ({message})"),
            detail: None,
        },
    };
    report(&body, json);
    exit_code(&body.code)
}

/// `tine serve GRAPH` runs the owner; `--stop` asks it to finish and exit.
pub(crate) fn serve(graph: &Path, stop: bool) -> i32 {
    if !stop {
        return serve_exit(super::serve::serve_graph(graph));
    }
    let root = match graph_root(Some(graph.to_owned())) {
        Ok(root) => root,
        Err(message) => {
            eprintln!("tine: {message}");
            return EXIT_USAGE;
        }
    };
    match client::connect_existing(&root) {
        Ok(None) => {
            println!("No command-line owner is running for {}", root.display());
            0
        }
        Ok(Some(mut conn)) => match conn.call("owner.yield", Value::Null) {
            Ok(_) => {
                println!("The command-line owner of {} has stopped", root.display());
                0
            }
            Err(failure) => report_failure(failure, false, "uncertain"),
        },
        Err(error) => report_failure(Failure::Reach(error), false, "owner-start"),
    }
}

/// The exit code of a finished `tine serve` (0 when it stopped cleanly).
pub(super) fn serve_exit(result: Result<(), super::serve::ServeFailure>) -> i32 {
    use super::serve::ServeFailure;
    match result {
        Ok(()) => 0,
        Err(failure) => {
            eprintln!("tine serve: {failure}");
            exit_code(match &failure {
                ServeFailure::Claim(error) => error.code(),
                ServeFailure::Other(_) => "internal",
            })
        }
    }
}

/// `--graph`, else `$TINE_GRAPH`, else the graph around the working
/// directory (the nearest ancestor with a `logseq/` folder).
fn graph_root(explicit: Option<PathBuf>) -> Result<PathBuf, String> {
    let chosen = explicit
        .or_else(|| {
            std::env::var_os("TINE_GRAPH")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        })
        .or_else(|| {
            let cwd = std::env::current_dir().ok()?;
            cwd.ancestors()
                .find(|dir| dir.join("logseq").is_dir())
                .map(Path::to_path_buf)
        })
        .ok_or("no graph: pass --graph, set TINE_GRAPH, or run inside a graph folder")?;
    crate::state::canonical_graph_root(&chosen.display().to_string())
}

/// The error on stderr: one line with the stable discriminator
/// (`tine: <code>: <message>`, spec A3), and for a refusal the reason joins
/// the code (`tine: refused:<reason>: <message>`). What the JSON error
/// carries in `detail` that a human needs follows as indented lines:
/// ambiguity candidates and query diagnostics.
pub(super) fn error_text(body: &ErrorBody) -> String {
    use std::fmt::Write;
    let detail = body.detail.as_ref();
    let reason = (body.code == "refused")
        .then(|| detail.and_then(|d| d.get("reason")).and_then(Value::as_str))
        .flatten();
    let mut out = match reason {
        Some(reason) => format!("tine: refused:{reason}: {}", body.message),
        None => format!("tine: {}: {}", body.code, body.message),
    };
    if let Some(detail) = detail {
        let candidates = detail.get("candidates").and_then(Value::as_array);
        if let Some(candidates) = candidates.filter(|c| !c.is_empty()) {
            let names: Vec<&str> = candidates.iter().filter_map(Value::as_str).collect();
            let _ = write!(out, "\n  candidates: {}", names.join(", "));
        }
        for diagnostic in detail
            .get("diagnostics")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice)
        {
            let _ = write!(out, "\n  diagnostic: {}", text(diagnostic, "message"));
        }
    }
    out
}

/// The `--json` error: one line, the envelope
/// `{"schema":"tine.cli/1","error":{code,message,detail?}}`.
pub(super) fn error_json(body: &ErrorBody) -> String {
    serde_json::to_string(&serde_json::json!({ "schema": super::wire::SCHEMA, "error": body }))
        .unwrap_or_default()
}

/// The error on stderr: [`error_text`], or with `--json` [`error_json`].
fn report(body: &ErrorBody, json: bool) {
    if json {
        eprintln!("{}", error_json(body));
    } else {
        eprintln!("{}", error_text(body));
    }
}

fn print(ok: &Value, json: bool) {
    let mut out = String::new();
    if json {
        out = serde_json::to_string(ok).unwrap_or_default();
        out.push('\n');
    } else {
        render(ok, &mut out);
    }
    let _ = std::io::stdout().lock().write_all(out.as_bytes());
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

fn list<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

/// The count under `key`, 0 when absent.
fn count(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or(0)
}

/// `1 node` / `3 nodes`.
fn nodes(n: u64) -> String {
    format!("{n} node{}", if n == 1 { "" } else { "s" })
}

/// The compact outline (spec A3): each block line starts with its address
/// (and its `[author:: NAME]` when it has one), `1.2` = the second child of
/// the first top-level block; continuation and property lines are indented
/// under it. Everything the payload discloses is said here too (`cli-output.md`
/// "Disclosure fields"): the human text never reads more complete than the
/// JSON.
pub(super) fn render(ok: &Value, out: &mut String) {
    use std::fmt::Write;
    match text(ok, "kind") {
        "page" | "journal" => {
            let page = &ok["page"];
            let _ = writeln!(out, "# {}", text(page, "name"));
            let _ = writeln!(out, "id {} rev {}", text(ok, "id"), text(ok, "rev"));
            if let Some(reason) = ok["read_only"].as_str() {
                let _ = writeln!(out, "(read-only: {reason})");
            }
            if let Some(pre) = page.get("pre_block").and_then(Value::as_str) {
                for line in pre.lines() {
                    let _ = writeln!(out, "{line}");
                }
            }
            outline(list(page, "blocks"), "", 0, out);
        }
        "block" => {
            let group = &ok["block"]["group"];
            let _ = writeln!(out, "# {}", text(group, "page"));
            outline(list(group, "blocks"), "", 0, out);
            let omitted = count(&ok["block"], "truncated");
            if omitted > 0 {
                let _ = writeln!(
                    out,
                    "... {} omitted (the preview is bounded at {PREVIEW_NODES} nodes and a byte cap)",
                    nodes(omitted)
                );
            }
        }
        "refs" => groups(list(ok, "groups"), out),
        "search" => {
            for diagnostic in list(ok, "diagnostics") {
                let _ = writeln!(
                    out,
                    "diagnostic {}: {}",
                    text(diagnostic, "code"),
                    text(diagnostic, "message")
                );
            }
            if ok["cancelled"].as_bool() == Some(true) {
                let _ = writeln!(out, "(the search was cancelled; no results)");
            }
            for hit in list(ok, "hits") {
                match text(hit, "entity") {
                    "page" => {
                        let _ = writeln!(out, "page  {}", text(hit, "display_text"));
                    }
                    _ => {
                        let _ = writeln!(
                            out,
                            "block {} | {}",
                            text(hit, "page"),
                            first_line(text(hit, "display_text"))
                        );
                    }
                }
            }
            let more = &ok["has_more"];
            if more["pages"].as_bool() == Some(true) {
                let _ = writeln!(out, "(more page results; raise --pages)");
            }
            if more["blocks"].as_bool() == Some(true) {
                let _ = writeln!(out, "(more block results; raise --blocks)");
            }
        }
        "query" => {
            og_hint(&ok["og_hint"], out);
            if ok["report"]["supported"].as_bool() == Some(false) {
                let _ = writeln!(
                    out,
                    "warning: this advanced query is not fully supported by Tine; its results may differ from Logseq's"
                );
            }
            for ignored in list(&ok["report"], "ignored") {
                let _ = writeln!(out, "warning: ignored: {}", ignored.as_str().unwrap_or(""));
            }
            for diagnostic in list(ok, "diagnostics") {
                let disabled = if diagnostic["disabled"].as_bool() == Some(true) {
                    " (disabled clause)"
                } else {
                    ""
                };
                let _ = writeln!(out, "diagnostic{disabled}: {}", text(diagnostic, "message"));
            }
            if ok.get("groups").is_some() {
                groups(list(ok, "groups"), out);
            } else {
                for page in list(ok, "pages") {
                    let _ = writeln!(out, "{}", text(page, "name"));
                }
            }
            if ok["exceeded"].as_bool() == Some(true) {
                let _ = writeln!(
                    out,
                    "(truncated: the query hit its result limit; {} matches counted, the list is incomplete)",
                    count(ok, "total")
                );
            }
        }
        "pages" => {
            for page in list(ok, "pages") {
                let _ = writeln!(out, "{}", text(page, "name"));
            }
        }
        "status" => {
            let _ = writeln!(
                out,
                "{} owns {} (pid {}, {}, Tine {})",
                text(ok, "owner"),
                text(ok, "graph"),
                ok["pid"],
                text(ok, "state"),
                text(ok, "version")
            );
        }
        _ => {
            let _ = writeln!(
                out,
                "{}",
                serde_json::to_string_pretty(ok).unwrap_or_default()
            );
        }
    }
}

/// D-18's on-query cue (`query.og_hint`): which bare heads add no condition,
/// whether nothing is left, and the explicit forms to write instead.
fn og_hint(hint: &Value, out: &mut String) {
    use std::fmt::Write;
    if !hint.is_object() {
        return;
    }
    let bare: Vec<String> = list(hint, "bare")
        .iter()
        .filter_map(Value::as_str)
        .map(|head| format!("({head})"))
        .collect();
    if !bare.is_empty() {
        let _ = writeln!(
            out,
            "hint: bare {} adds no condition in Logseq; Tine evaluates it the same way",
            bare.join(" and ")
        );
    }
    if hint["no_conditions"].as_bool() == Some(true) {
        let _ = writeln!(
            out,
            "hint: no condition is left, so the query shows nothing"
        );
    }
    for rewrite in list(hint, "rewrites") {
        match rewrite.get("text").and_then(Value::as_str) {
            Some(query) => {
                let _ = writeln!(
                    out,
                    "hint: explicit form ({}): {query}",
                    text(rewrite, "rewrite")
                );
            }
            None => {
                let _ = writeln!(
                    out,
                    "hint: an explicit form ({}) exists but has no Logseq spelling",
                    text(rewrite, "rewrite")
                );
            }
        }
    }
}

fn groups(groups: &[Value], out: &mut String) {
    use std::fmt::Write;
    for group in groups {
        let _ = writeln!(out, "## {}", text(group, "page"));
        outline(list(group, "blocks"), "", 0, out);
    }
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or("")
}

/// The block's `(key, value)` properties as the payload carries them
/// (`BlockDto.properties`, the DocBlock projection's answer).
fn block_properties(block: &Value) -> Vec<(String, String)> {
    list(block, "properties")
        .iter()
        .filter_map(|pair| {
            Some((
                pair.get(0)?.as_str()?.to_string(),
                pair.get(1)?.as_str()?.to_string(),
            ))
        })
        .collect()
}

fn is_author_key(key: &str) -> bool {
    tine_core::doc::property_key_norm(key) == "author"
}

/// What the DocBlock projection says a block body's properties are, for
/// Markdown or Org syntax.
fn projected_properties(raw: &str, org: bool) -> Vec<(String, String)> {
    let mut block = tine_core::doc::DocBlock::new(raw);
    block.set_org(org);
    block.properties()
}

/// A block's `author::` and its other lines. Whether the block has an author
/// is the projection's answer (`properties` in the payload, which treats a
/// `key::` line inside a code fence as content), never a scan of the lines.
/// The line shown beside the address is not repeated below it: it is the
/// candidate author line whose removal leaves exactly the projection's other
/// properties, so a literal `author::` line (in a fence, or a look-alike with
/// the same value) stays.
fn split_author<'a>(
    raw: &'a str,
    properties: &[(String, String)],
) -> (Option<String>, Vec<&'a str>) {
    let lines: Vec<&str> = raw.lines().collect();
    let Some(first) = properties.iter().position(|(key, _)| is_author_key(key)) else {
        return (None, lines);
    };
    let author = properties[first].1.trim().to_string();
    // The payload was projected with the page's own syntax; find which.
    let Some(org) = [false, true]
        .into_iter()
        .find(|org| projected_properties(raw, *org) == properties)
    else {
        return (None, lines);
    };
    let mut others = properties.to_vec();
    others.remove(first);
    let without = |at: usize| -> Vec<&'a str> {
        lines
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != at)
            .map(|(_, line)| *line)
            .collect()
    };
    for at in 1..lines.len() {
        if author_value(lines[at]).map(str::trim) != Some(author.as_str()) {
            continue;
        }
        let rest = without(at);
        if projected_properties(&rest.join("\n"), org) == others {
            return (Some(author), rest);
        }
    }
    (Some(author), lines)
}

fn author_value(line: &str) -> Option<&str> {
    if let Some((key, value)) = tine_core::property_line::parse_property_line(line) {
        return is_author_key(key).then_some(value);
    }
    // Org drawer: `:author: value`.
    let rest = line.trim().strip_prefix(':')?;
    let (key, value) = rest.split_once(':')?;
    (is_author_key(key) && value.starts_with(' ')).then(|| value.trim())
}

fn outline(blocks: &[Value], prefix: &str, depth: usize, out: &mut String) {
    use std::fmt::Write;
    for (index, block) in blocks.iter().enumerate() {
        let address = format!("{prefix}{}", index + 1);
        let indent = "  ".repeat(depth);
        let (author, lines) = split_author(text(block, "raw"), &block_properties(block));
        let mut lines = lines.into_iter();
        let head = lines.next().unwrap_or("");
        match author {
            Some(author) => {
                let _ = writeln!(out, "{indent}{address} [author:: {author}] {head}");
            }
            None => {
                let _ = writeln!(out, "{indent}{address} {head}");
            }
        }
        let pad = " ".repeat(address.len() + 1);
        for line in lines {
            let _ = writeln!(out, "{indent}{pad}{line}");
        }
        outline(
            list(block, "children"),
            &format!("{address}."),
            depth + 1,
            out,
        );
    }
}
