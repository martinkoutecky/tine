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
    ("MDT", "mod.rs", "let movement = self.fs.trash_move(&key, &payload);", "let movement = io::MoveResult { removed: None, result: Ok(()) };", "trash move"),
    ("MTS", "mod.rs", "SavePhase::TrashSync\n                        } else {", "SavePhase::DirectorySync\n                        } else {", "trash durability barrier"),
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
    # A4 trash custody (STEP2-DESIGN §3): one mutant per rule and review finding.
    ("H-custody-before-save", "mod.rs", "phase: if self.custody.contains_key(key) {", "phase: if false {", "REVIEW-2b F9: recreation renames before custody"),
    ("H-custody-launch", "mod.rs", "if self.fs.trash_sync(key, &payload).is_ok() {", "if true {", "REVIEW-A2 B2: a marker retires before (a)+(b)"),
    ("H-custody-launch-order", "mod.rs", "        self.list_custody();\n        for key in self.custody.keys()", "        self.fs.graph_launch(&self.keys);\n        self.list_custody();\n        for key in self.custody.keys()", "REVIEW-A3 B2: source syncs before destination custody"),
    ("H-custody-marker", "mod.rs", "match self\n                    .fs\n                    .custody_write(&name, &drafts::encode_marker(&marker))\n                {", "match Ok::<(), io::IoFailure>(()) {", "rule 1: no durable marker before the move"),
    ("H-custody-collision-retire", "mod.rs", "self.retire_marker(&key, &marker, payload);\n                        job.marker = None;", "job.marker = None;", "REVIEW-A3: a collision leaves an unused marker"),
    ("H-custody-escape", "mod.rs", "if debt.failures >= 3 {", "if false {", "REVIEW-2b F5: no three-failure escape"),
    ("H-custody-escape-early", "mod.rs", "if debt.failures >= 3 {", "if true {", "custody failure must fail the save first"),
    ("H-custody-retire-after", "mod.rs", "if result.is_some() && job.phase == SavePhase::DirectorySync {", "if false {", "REVIEW-2b F4: markers accumulate"),
    ("H-adapter-trash-ancestors", "production.rs", "        chain(&self.graph, &self.trash).try_fold(", "        chain(&self.trash, &self.trash).try_fold(", "REVIEW-2b F2: trash ancestors"),
    ("H-adapter-launch-ancestors", "production.rs", "chain(&self.graph, path.parent().unwrap())", "chain(path.parent().unwrap(), path.parent().unwrap())", "nested page parent chain at launch"),
    ("H-adapter-custody-sync", "production.rs", ".and_then(|()| durability::sync_private_directory(&dir))", "", "marker entry durable before the move"),
    ("H-adapter-trash-witnesses", "mod.rs", "witness == Witness::Durable && job.trash_durable && job.bytes.is_none()", "witness == Witness::Durable && job.bytes.is_none()", "weak trash / strong source witness"),
    ("H-adapter-private-sync", "production.rs", "durability::sync_private_directory(&self.drafts).map_err(sync_failure)?;", "durability::sync_directory_entry(&self.drafts).map_err(sync_failure)?;", "EINVAL cannot complete an app-data vehicle"),
    ("H-adapter-trash-payload", "production.rs", "crate::atomic_file::sync_file_bytes(&self.trash.join(payload))", "Ok::<(), std::io::Error>(())", "unflushed R1 payload and failed file sync"),
    ("H-adapter-directory-custody", "production.rs", "fn graph_directory(&mut self, dir: &Path) -> IoResult<()> {", "fn graph_directory(&mut self, dir: &Path) -> IoResult<()> {\n        self.directories.clear();", "retry must retain the created chain's missing directory sync"),
    # REVIEW-2b-r2 V1-V3 and continuation 3b (the R1 trash-chain hole).
    ("H-custody-unknown-as-empty", "mod.rs", "self.custody_unknown = Some(error);\n                return;", "let _ = error;\n                return;", "V1: a failed listing silently reads as no debt"),
    ("H-custody-unknown-no-retry", "progress.rs", "self.host.recover_custody();", "", "V1: unknown custody is never relisted"),
    ("H-custody-file-not-quarantined", "production.rs", "if ((entry.file_name() == \"unreadable\" || entry.file_name() == CUSTODY)\n                && entry.file_type()?.is_dir())", "if (entry.file_name() == \"unreadable\" && entry.file_type()?.is_dir()) || entry.file_name() == CUSTODY", "V1: a regular-file trash-custody is not quarantined"),
    ("H-retire-debt-dropped", "mod.rs", "        if self.fs.custody_retire(marker).is_ok() {", "        if self.fs.custody_retire(marker).is_ok() || true {", "V2: a failed retirement is forgotten"),
    ("H-retire-third-not-sticky", "mod.rs", "if failures >= 3 {\n                    self.custody_errors.insert(marker);", "if false {\n                    self.custody_errors.insert(marker);", "V2/V3: the third failed retirement is not sticky"),
    ("H-marker-temps-kept", "production.rs", "temps |= remove_present(&entry.path()).is_ok();", "temps |= false;", "V2: unpublished marker temps accumulate"),
    ("H-custody-notice-unset", "mod.rs", "self.custody_errors.insert(marker.clone());", "", "V3: the escape sets no sticky notice"),
    ("H-custody-notice-never-cleared", "mod.rs", "            self.custody_errors.remove(marker);\n        } else {", "        } else {", "V3: retirement does not clear the notice"),
    ("H-custody-notice-cleared-by-save", "mod.rs", "            if outcome == Outcome::Published {", "            if outcome == Outcome::Published {\n                self.custody_errors.clear();", "V3: a later save clears the notice"),
    ("H-adapter-launch-trash-chain", "production.rs", "        directories.extend(\n            chain(&self.graph, &self.trash)", "        directories.extend(\n            chain(&self.graph, &self.trash).take(0)", "3b: launch skips the trash chain"),
    ("H-model-launch-trash-chain", "model_fs.rs", "            for dir in CHAIN {\n                self.sync_entry(dir);", "            for dir in CHAIN.iter().take(0) {\n                self.sync_entry(dir);", "3b: the sweep's launch skips the trash chain"),
    ("H-model-owed-forgotten", "model_fs.rs", "            // DirectoryCreation: mkdir each missing chain entry, then sync", "            fs.owed.clear();\n            // DirectoryCreation: mkdir each missing chain entry, then sync", "3b no-crash: a failed chain creation's owed syncs are dropped"),
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
