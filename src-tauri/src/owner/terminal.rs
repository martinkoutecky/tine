//! The command line's read commands (`tine page show`, `tine query`, …):
//! reach the graph's owner, send one request, print the answer. The answer
//! is the typed `ok` payload (schema `tine.cli/1`); `--json` prints it
//! as is, the default renders it as a compact outline (spec A3). One schema,
//! no second result type.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::client::{self, Failure};
use super::wire::ErrorBody;

/// How long a read keeps asking through a handover or a restart.
const RETRY_FOR: Duration = Duration::from_secs(30);

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
            return 2;
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
            Err(Failure::Owner(body)) => {
                report(&body, target.json);
                exit_code(&body.code)
            }
            Err(Failure::Reach(error)) => {
                eprintln!("tine: {}: {error}", error.code());
                5
            }
            Err(Failure::Lost(message)) => {
                eprintln!("tine: owner-start: the owner stopped answering ({message})");
                5
            }
        };
    }
}

/// `tine serve GRAPH` runs the owner; `--stop` asks it to finish and exit.
pub(crate) fn serve(graph: &Path, stop: bool) -> i32 {
    if !stop {
        return match super::serve::serve_graph(graph) {
            Ok(()) => 0,
            Err(message) => {
                eprintln!("tine serve: {message}");
                1
            }
        };
    }
    let root = match graph_root(Some(graph.to_owned())) {
        Ok(root) => root,
        Err(message) => {
            eprintln!("tine: {message}");
            return 2;
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
            Err(Failure::Owner(body)) => {
                report(&body, false);
                exit_code(&body.code)
            }
            Err(Failure::Reach(error)) => {
                eprintln!("tine: {}: {error}", error.code());
                5
            }
            Err(Failure::Lost(message)) => {
                eprintln!("tine: uncertain: the owner stopped answering ({message})");
                8
            }
        },
        Err(error) => {
            eprintln!("tine: {}: {error}", error.code());
            5
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

/// Stable exit codes per error code (PROTOCOL §2.3).
fn exit_code(code: &str) -> i32 {
    match code {
        "not-found" | "ambiguous" => 3,
        "busy" => 4,
        "unknown-verb" | "bad-args" | "frame-too-large" | "malformed" | "too-large" => 2,
        "protocol" | "owner-mismatch" => 5,
        "stale" | "refused" => 6,
        "draft-failed" => 7,
        "unwritten" | "superseded" | "uncertain" => 8,
        _ => 1,
    }
}

/// One line on stderr with the stable code; `--json` adds the body.
fn report(body: &ErrorBody, json: bool) {
    if json {
        eprintln!(
            "{}",
            serde_json::to_string(
                &serde_json::json!({ "schema": super::wire::SCHEMA, "error": body })
            )
            .unwrap_or_default()
        );
    } else {
        eprintln!("tine: {}: {}", body.code, body.message);
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

/// The compact outline (spec A3): each block line starts with its address,
/// `1.2` = the second child of the first top-level block; continuation and
/// property lines are indented under it.
fn render(ok: &Value, out: &mut String) {
    use std::fmt::Write;
    match text(ok, "kind") {
        "page" | "journal" => {
            let page = &ok["page"];
            let _ = writeln!(out, "# {}", text(page, "name"));
            let _ = writeln!(out, "id {} rev {}", text(ok, "id"), text(ok, "rev"));
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
        }
        "refs" => groups(list(ok, "groups"), out),
        "search" => {
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
            if more["pages"].as_bool() == Some(true) || more["blocks"].as_bool() == Some(true) {
                let _ = writeln!(out, "(more results; raise --pages or --blocks)");
            }
        }
        "query" => {
            if ok.get("groups").is_some() {
                groups(list(ok, "groups"), out);
            } else {
                for page in list(ok, "pages") {
                    let _ = writeln!(out, "{}", text(page, "name"));
                }
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

fn outline(blocks: &[Value], prefix: &str, depth: usize, out: &mut String) {
    use std::fmt::Write;
    for (index, block) in blocks.iter().enumerate() {
        let address = format!("{prefix}{}", index + 1);
        let indent = "  ".repeat(depth);
        let mut lines = text(block, "raw").lines();
        let _ = writeln!(out, "{indent}{address} {}", lines.next().unwrap_or(""));
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
