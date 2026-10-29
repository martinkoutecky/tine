//! The one graph-file exception of autocommit (ADR 0078 §2): the
//! `.gitignore` lines it adds once per device and graph, `.syncthing.*`
//! (D-20261026-GI) and `logseq/.tine-*` (D-20261029-GT), through Store's
//! guarded non-page transaction.

use std::path::Path;
use std::sync::Mutex;

use super::{open_checked, registry};

/// The `.gitignore` lines autocommit adds once per device and graph: the
/// device-local notice key that remembers the attempt, the line, and a
/// graph-relative name git's rules are asked about (D-20261026-GI,
/// D-20261029-GT).
const IGNORE_LINES: [(&str, &str, &str); 2] = [
    (
        "git-syncthing-ignore-checked",
        ".syncthing.*",
        ".syncthing.tine-ignore-check.tmp",
    ),
    (
        "git-tine-own-ignore-checked",
        "logseq/.tine-*",
        "logseq/.tine-ignore-check",
    ),
];
static IGNORE_SETUP_LOCK: Mutex<()> = Mutex::new(());

/// The ignore lines [`prepare_git_ignore`] added, or with the error those it
/// was attempting.
pub(crate) type IgnoreSetup = Result<Vec<&'static str>, (Vec<&'static str>, String)>;

/// One durable attempt per line and device/graph, before any graph write;
/// every line still due goes into one write. Marking first favours respecting
/// a later removal over retrying after a crash or I/O error. A broader/parent
/// git rule counts; staging remains the unconditional backstop.
pub(crate) fn prepare_git_ignore(
    store: &tine_store::Store,
    root: &Path,
    app_data: Option<&Path>,
) -> IgnoreSetup {
    let Some(app_data) = app_data else {
        let lines = IGNORE_LINES.iter().map(|(_, line, _)| *line).collect();
        return Err((
            lines,
            "no app-data directory for remembering the attempt".into(),
        ));
    };
    let _setup = IGNORE_SETUP_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let due: Vec<_> = IGNORE_LINES
        .iter()
        .filter(|(key, _, _)| !crate::settings::notice_dismissed(app_data, root, key))
        .collect();
    let lines: Vec<&'static str> = due.iter().map(|(_, line, _)| *line).collect();
    if due.is_empty() {
        return Ok(Vec::new());
    }
    let fail = |error: String| (lines.clone(), error);
    let repo = open_checked(root).map_err(|e| fail(e.message()))?;
    let mut missing = Vec::new();
    for (_, line, probe) in &due {
        let ignored = repo
            .repo
            .status_should_ignore(&repo.prefix.join(probe))
            .map_err(|e| fail(e.to_string()))?;
        if !ignored {
            missing.push(*line);
        }
    }
    for (key, _, _) in &due {
        crate::settings::set_notice_at(app_data, root, key, true).map_err(fail)?;
    }
    if !missing.is_empty() {
        append_git_ignore(store, &missing).map_err(|e| (missing.clone(), e))?;
    }
    Ok(missing)
}

/// Fixed non-page target; read exact bytes once, then create without replacing
/// an occupied path or replace against that byte revision under Store's lock.
fn append_git_ignore(store: &tine_store::Store, lines: &[&str]) -> Result<(), String> {
    use tine_store::{Content, FileId, StoreError, TxOutcome};
    let id = FileId::from(".gitignore".to_owned());
    let (mut bytes, base) = match store.read(&id, Some(tine_store::PARSE_INPUT_MAX_BYTES)) {
        Ok((bytes, rev)) => (bytes, Some(rev)),
        Err(StoreError::NotFound) => (Vec::new(), None),
        Err(error) => return Err(format!("{error:?}")),
    };
    // Use the first existing newline's style, preserving every original byte.
    let newline: &[u8] = match bytes.iter().position(|b| *b == b'\n') {
        Some(at) if at > 0 && bytes[at - 1] == b'\r' => b"\r\n",
        _ => b"\n",
    };
    if !bytes.is_empty() && !bytes.ends_with(b"\n") {
        bytes.extend_from_slice(newline);
    }
    for line in lines {
        bytes.extend_from_slice(line.as_bytes());
        bytes.extend_from_slice(newline);
    }
    let mut tx = store.transaction(None);
    if let Some(base) = base {
        tx.replace(&id, base, bytes);
    } else {
        tx.create(&id, Content::Bytes(bytes));
    }
    match tx.commit() {
        TxOutcome::Committed { .. } => Ok(()),
        other => {
            crate::debug::diag_private("git-ignore-write-failed", format!("{other:?}"));
            Err("the file changed or could not be safely written".into())
        }
    }
}

/// Keep this setup result on the existing Settings git status surface. A later
/// no-op check must not erase the notice before the user can read it.
pub(crate) fn report_ignore_setup(root: &Path, result: IgnoreSetup) {
    if let Some(worker) = registry::get(root) {
        let mut state = worker.state.lock().unwrap();
        match result {
            Ok(lines) if lines.is_empty() => {}
            Ok(lines) => {
                state.last.gitignore_added = Some(root.join(".gitignore").display().to_string());
                state.last.gitignore_lines = lines.into_iter().map(str::to_owned).collect();
                state.last.gitignore_failed = false;
            }
            Err((lines, error)) => {
                crate::debug::diag_private("git-ignore-setup-failed", error);
                state.last.gitignore_lines = lines.into_iter().map(str::to_owned).collect();
                state.last.gitignore_failed = true;
            }
        }
    }
}
