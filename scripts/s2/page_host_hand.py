#!/usr/bin/env python3
"""Mapped model and physical-phase mutations; only scratch copies are changed."""
import json
import os
from pathlib import Path
import subprocess
import page_host_mutations as harness

ROOT = harness.ROOT
# name, file, exact original, replacement, observable boundary
MUTATIONS = [
    ("MST", "mod.rs", "if self.version != base_version {", "if false {", "stale submit"),
    ("MLO", "mod.rs", "} else if self.clean() {", "} else if !self.risk {", "dirty watcher read"),
    ("MQ", "mod.rs", "} else if self.buf == bytes {\n            self.base", "} else if self.buf == bytes {\n            self.risk = false; self.typed = false;\n            self.base", "equal-byte observation"),
    ("MIS", "mod.rs", "self.set_page(key, Some(page));\n                self.answer(request, key, true);", "self.answer(request, key, true);", "submit answer"),
    ("MDO", "mod.rs", "self.set_page(key, Some(page));\n                self.answer(request, key, true);", "self.set_page(key, Some(page));\n                self.pages.retain(|p, _| p == key);\n                self.answer(request, key, true);", "unrelated held page"),
    ("MCL", "mod.rs", "self.pages.get(key).is_some_and(Page::clean)", "self.pages.get(key).is_some_and(|p| !p.risk)", "dirty close"),
    ("MDE", "mod.rs", "let hold = page.risk", "let hold = false && page.risk", "discard and power"),
    ("MSM", "mod.rs", "if a.version != *source_version || b.version != *receiver_version {", "if false {", "stale move"),
    ("MOD", "mod.rs", "let present = worker.task.stage == Stage::Present;", "let present = true;", "failed move installation"),
    ("MC-MC1", "mod.rs", "Ok(bytes) if job.base == Base::Known(bytes.clone()) =>", "Ok(bytes) if true =>", "save guard read under both profiles"),
    ("MLS-MRE", "mod.rs", "if outcome == Outcome::Published {", "if outcome != Outcome::Failed {", "uncertain result"),
    ("MDT", "mod.rs", "let movement = self.fs.trash_move(&key, &job.trash_name);", "let movement = io::MoveResult { removed: None, result: Ok(()) };", "trash move"),
    ("MTS", "mod.rs", "job.phase = SavePhase::TrashSync;", "job.phase = SavePhase::DirectorySync;", "trash durability barrier"),
    ("MBD", "mod.rs", "base: page.base.clone(),\n            bytes: page.buf.clone(),", "base: if page.base == Base::Unknown { Base::Known(page.obs.clone().flatten()) } else { page.base.clone() },\n            bytes: page.buf.clone(),", "unknown-base draft"),
    ("MDQ", "mod.rs", "self.outbox.clear();\n        self.subscriptions.clear();\n        self.switch_abort();", "self.queue.clear();\n        self.outbox.clear();\n        self.subscriptions.clear();\n        self.switch_abort();", "window crash custody"),
    ("MRB", "mod.rs", "base: record.base.clone(),", "base: if record.base == Base::Unknown { Base::Known(None) } else { record.base.clone() },", "unknown-base launch"),
    ("MX", "mod.rs", "Some(p) => drafts.get(key).is_some_and(|r| {", "Some(p) => true || drafts.get(key).is_some_and(|r| {", "switch draft barrier"),
    ("MRN1", "operations.rs", "records.push(host.record(key, page));", "if key != target { records.push(host.record(key, page)); }", "rename destination capture"),
    ("MRN4", "operations.rs", "records.push(host.record(key, page));", "if !refs.contains(key) { records.push(host.record(key, page)); }", "rename referrer capture"),
    ("MRN5", "operations.rs", "records.push(host.record(key, page));", "if key != source { records.push(host.record(key, page)); }", "rename source deletion"),
    ("MRN2", "operations.rs", ["if refs\n            .iter()\n            .any(|key| self.pages.get(key).is_some_and(|p| !p.clean()))", "if !page.clean() {"], ["if false", "if !page.clean() && !refs.contains(key) {"], "unclean referrer (both guard sites)"),
    ("MRN2-preflight", "operations.rs", "if refs\n            .iter()\n            .any(|key| self.pages.get(key).is_some_and(|p| !p.clean()))", "if false", "unclean referrer preflight"),
    ("MRN3", "operations.rs", "if full && pages[target].buf.is_some() {", "if false {", "occupied target"),
    ("MDD", "operations.rs", "vec![record],", "vec![],", "deletion draft capture"),
    ("MUL", "operations.rs", "if !load && !host.pages.contains_key(key) {", "if false {", "unheld raw deletion refusal"),
    ("H-wseq-selection", "drafts.rs", "if record.wseq > current.wseq {", "if record.version > current.version {", "same-version refresh launch"),
    ("H-retirement-order", "drafts.rs", "files.sort();", "files.sort(); files.reverse();", "oldest-first unlink"),
    ("H-unlink-sync-retry", "drafts.rs", "Stage::UnlinkSync if result.is_ok() => Stage::Absent,", "Stage::UnlinkSync if result.is_ok() => Stage::Absent,\n            Stage::UnlinkSync => Stage::Unlink,", "post-unlink sync retry"),
    ("H-cleanup-sync-retry", "drafts.rs", "Stage::CleanupSync if result.is_ok() => Stage::Absent,", "Stage::CleanupSync if result.is_ok() => Stage::Absent,\n            Stage::CleanupSync => Stage::CleanupUnlink,", "cleanup sync retry"),
    ("H-durable-before-apply", "drafts.rs", "Stage::Rename if result.is_ok() => Stage::Sync,", "Stage::Rename if result.is_ok() => Stage::Present,", "install directory witness"),
    ("H-live-readable", "mod.rs", "self.fs.draft_files(self.alive)", "self.fs.draft_files(true)", "stopped readable recovery"),
    ("H-switch-ready-lifecycle", "mod.rs", "fn switch_ready(&mut self, consumed_last_id: u64) -> Disposition {\n        if !self.alive {", "fn switch_ready(&mut self, consumed_last_id: u64) -> Disposition {\n        if false {", "late confirmation after stop"),
    ("H-draft-backoff", "progress.rs", "let draft_ready = self.draft_retry.is_none_or(|due| now >= due);", "let draft_ready = true;", "early cleanup/copy/retirement polls"),
    ("H-save-fairness", "progress.rs", "due.sort();", "due.sort_by(|a, b| a.1.cmp(&b.1));", "recurring first-page input while another is overdue"),
    ("H-replaced-copy-failures", "mod.rs", "worker.failures = worker\n                .failures\n                .checked_add(worker.task.failures - failures)\n                .expect(\"draft failure count exhausted\");", "worker.failures = worker.task.failures;", "repeated failed fresh explosion vehicles"),
    ("H-error-obligation", "progress.rs", "self.draft_errors.remove(effect);", "self.draft_errors.clear();", "failed move cannot recover an earlier refresh"),
    ("H-error-abandonment", "progress.rs", "self.host.pages.get(key).is_some_and(|page| page.risk)", "true", "Discard after terminal failed refresh"),
    ("H-backoff-independent-saves", "progress.rs", "if let Some(w) = self.host.worker.as_ref().filter(|_| draft_ready) {", "if self.host.worker.is_some() && !draft_ready { return Disposition::Disabled; }\n        if let Some(w) = self.host.worker.as_ref().filter(|_| draft_ready) {", "unrelated overdue/running save during capped draft backoff"),
    ("H-retryable-sync-notice", "mod.rs", "worker.task.stage == Stage::Absent && worker.refresh.is_some()", "worker.task.failures > 0 && worker.refresh.is_some()", "first retryable sync failure must stay silent"),
    ("H-adapter-trash-witnesses", "mod.rs", "witness == Witness::Durable && job.trash_durable && job.bytes.is_none()", "witness == Witness::Durable && job.bytes.is_none()", "weak trash / strong source witness"),
    ("H-adapter-private-sync", "production.rs", "durability::sync_private_directory(&self.drafts).map_err(sync_failure)?;", "durability::sync_directory_entry(&self.drafts).map_err(sync_failure)?;", "EINVAL cannot complete an app-data vehicle"),
    ("H-adapter-trash-payload", "production.rs", "crate::atomic_file::sync_file_bytes(&path)", "Ok::<(), std::io::Error>(())", "unflushed R1 payload and failed file sync"),
    ("H-adapter-directory-custody", "production.rs", "fn graph_directory(&mut self, dir: &Path) -> IoResult<()> {", "fn graph_directory(&mut self, dir: &Path) -> IoResult<()> {\n        self.directories.clear();", "retry must retain the created chain's missing directory sync"),
    ("H-trash-capture", "mod.rs", "pending_trash: custody.pending,", "pending_trash: vec![],", "deletion install must persist the payload obligation before moving"),
    ("H-trash-launch", "mod.rs", "host.apply_custody(record);", "let _ = record;", "restart must recover recorded pending payloads"),
    ("H-trash-recovered-collision", "mod.rs", "if self.fresh_trash.remove(&old) {", "if true {", "a recovered colliding candidate may already own an unsynced payload"),
    ("H-trash-collision-record", "mod.rs", "record.trash = Some(name);", "record.trash = Some(old);", "fresh collision target must agree with the durable record"),
    ("H-trash-recreation-phase", "mod.rs", "job.phase = if self.trash.get(&key).is_some_and(|c| !c.pending.is_empty()) {", "job.phase = if false {", "recreated and undone deletes still owe the recorded payload sync"),
    ("H-trash-clear-after-witness", "mod.rs", "if job.trash_durable {", "if false {", "new at-risk snapshots clear custody only after a strong witness"),
    ("H-trash-failed-capture-custody", "mod.rs", "for record in &worker.records {", "for record in std::iter::empty::<&Record>() {", "repeated refused installs release only their unowned names"),
    ("H-trash-synced-name-custody", "mod.rs", "for name in custody.pending {", "for name in std::iter::empty::<[u8; 16]>() {", "undo before a move releases the missing candidate after its successful witness"),
    ("H-trash-before-removal", "mod.rs", "sync_trash: host.trash.get(key).is_some_and(|c| !c.pending.is_empty()),", "sync_trash: false,", "Discard cannot retire the last unsynced payload identity"),
    ("H-trash-removal-error", "mod.rs", "Err(_) => worker.task.failures += 1,", "Err(_) => {},", "pending removal surfaces the third payload-witness failure"),
]


def main():
    import argparse
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--only")
    ap.add_argument("--adapter-only", action="store_true")
    ap.add_argument("--output", type=Path, help="explicit hand-run artifact directory")
    args = ap.parse_args()
    harness.WORK = ROOT / "scratch/page-host/hand-harness"
    work = harness.prepare()
    output = args.output or ROOT / "scratch/page-host" / ("hand-rerun-" + args.only if args.only else "hand-census")
    output.mkdir(parents=True, exist_ok=True)
    env = {**os.environ, "TINE_HOST_REPO_ROOT": str(ROOT),
           "CARGO_TARGET_DIR": str(work / "target")}
    command = ["rtk", "proxy", "cargo", "test", "--manifest-path", str(work / "Cargo.toml"),
               "--lib", "page_host", "--", "--skip", "scheduler_random_walks",
               "--skip", "committed_witnesses_through_host"]
    outcomes = []
    with (output / "baseline.log").open("w") as log:
        baseline = subprocess.run(command, env=env, stdout=log, stderr=log)
    assert baseline.returncode == 0, "baseline must pass before mutation"
    for name, filename, before, after, barrier in MUTATIONS:
        if args.only and name != args.only:
            continue
        if args.adapter_only and not name.startswith("H-adapter-"):
            continue
        path = work / "src/page_host" / filename
        original = path.read_text()
        replacements = list(zip(before, after)) if isinstance(before, list) else [(before, after)]
        changed = original
        for old, new in replacements:
            count = changed.count(old)
            assert count == 1, (name, count)
            changed = changed.replace(old, new)
        try:
            path.write_text(changed)
            with (output / f"{name}.log").open("w") as log:
                result = subprocess.run(command, env=env, stdout=log, stderr=log, timeout=180)
            log_text = (output / f"{name}.log").read_text()
            status = "killed" if result.returncode != 0 and "test result: FAILED" in log_text else "survived" if result.returncode == 0 else "unviable"
        except subprocess.TimeoutExpired:
            status = "timeout"
        finally:
            path.write_text(original)
        outcomes.append(dict(name=name, file=filename, barrier=barrier, status=status))
        (output / "outcomes.json").write_text(json.dumps(outcomes, indent=2) + "\n")
        print(name, status, flush=True)
    assert all(o["status"] == "killed" for o in outcomes), outcomes


if __name__ == "__main__":
    main()
