import { For, Show, createEffect, createSignal, on, type JSX } from "solid-js";
import { backend } from "../backend";
import { graphOwner, readOwned, writeOwned } from "../owned";
import { pushToast } from "../toasts";
import { graphEpoch } from "../graphSession";
import { journalMigrationSkipMessage, refreshJournalConflicts } from "../ui";
import type { JournalFilenameMigration } from "../types";

/** Journal files named by title (left by a date-format change or another tool)
 *  can't be placed on their day, so the day looks empty. Renaming them is a real
 *  repair, but it changes files the user owns: opening a graph only proposes it
 *  (master e6f9b6e1ceae), and this button applies it after a snapshot. Apply
 *  sends exactly the listed (from, to) pairs the user confirmed; the backend
 *  renames only pairs still valid and reports the rest as skipped. Results and
 *  failures are shown only while the graph that listed them is still open. */
export function JournalFilenamePanel(): JSX.Element {
  const [pending, setPending] = createSignal<JournalFilenameMigration[]>([]);
  const [busy, setBusy] = createSignal(false);
  const load = async () => {
    try {
      const listed = await readOwned(graphOwner(), backend().listJournalFilenameMigrations());
      if (listed.kind !== "stale") setPending(listed.value);
    } catch (e) {
      setPending([]);
      pushToast(`Couldn’t list journal files to rename: ${String(e)}`, "error");
    }
  };
  // A graph switch or journal-format change moves the epoch and can change the proposals.
  createEffect(on(graphEpoch, () => void load()));
  const apply = async () => {
    const owner = graphOwner();
    // The confirmation names this exact list; the backend renames only it.
    const migrations = pending();
    const count = migrations.length;
    const confirmed = await readOwned(owner, backend().confirm(
      `Rename ${count} journal file${count === 1 ? "" : "s"} to their date names?\n\n` +
        "A snapshot is taken first, so the original names stay in Backups & recovery. " +
        "Nothing is overwritten: a file whose date name is taken is left alone."
    ));
    if (confirmed.kind === "stale" || !confirmed.value) return;
    setBusy(true);
    try {
      const result = await writeOwned(owner, backend().applyJournalFilenameMigrations(migrations));
      if (result.kind === "stale") return;
      const { migrated, skipped } = result.value;
      pushToast(`Renamed ${migrated} journal file${migrated === 1 ? "" : "s"}`, "success");
      const skippedMessage = journalMigrationSkipMessage({ migrated, skipped });
      if (skippedMessage) pushToast(skippedMessage, "info");
      await load();
      await refreshJournalConflicts(true);
    } catch (e) {
      // A durable failure from a graph the user already left is not this graph's.
      if (owner()) pushToast(`Couldn’t rename them: ${String(e)}`, "error");
    } finally {
      setBusy(false);
    }
  };
  return (
    <Show when={pending().length}>
      <div class="settings-section" style={{ "margin-top": "18px" }}>
        Journal files named by title
      </div>
      <div class="settings-hint settings-block">
        These journal files aren’t named after their date, so Tine can’t place them in the journal
        feed and their days look empty. Renaming them fixes that, but it changes files you own, so
        Tine never does it on its own. Version control and sync will see these as renames.
      </div>
      <For each={pending()}>
        {(m) => (
          <div class="settings-block">
            <span class="journal-conflict-preview mono">{m.from}</span>{" "}
            <span class="journal-conflict-preview mono">→ {m.to}</span>
          </div>
        )}
      </For>
      <button class="settings-btn" disabled={busy()} onClick={() => void apply()}>
        {busy() ? "Renaming…" : "Rename to date names"}
      </button>
    </Show>
  );
}
