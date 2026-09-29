// The unsaved-changes recovery panel (GH #540, master dc3f2104b). Opened when the
// user declines to discard at close, or from a failed-save toast. It lists every
// page whose edits are not on disk and offers the ways out that never lose text:
// retry the ordinary save, open the page (its conflict bar resolves it), or copy
// the draft. Copying is read-only and never acknowledges a save.
import { For, Show, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { flushAll, unsavedDrafts } from "../document";
import { mainPaneRouter } from "../router";
import { writeClipboardText } from "../clipboard";
import { closeUnsavedRecovery, unsavedRecoveryOpen } from "../unsavedRecovery";
import { dismissEarlierDraft, earlierDrafts } from "../draftStore";
import { registerTransientLayer } from "../transientLayers";
import { DEFAULT_EXPORT_OPTIONS, exportOutline, type ExportNode } from "../editor/exportText";
import type { PageDto } from "../types";
import "../styles/unsavedRecovery.css";

/** The page as source text: markup and properties kept, nothing expanded. */
export function recoveryDraftText(page: PageDto): string {
  const nodes = (blocks: PageDto["blocks"]): ExportNode[] => blocks.map((block) => ({
    raw: block.raw, format: page.format ?? "md", children: nodes(block.children),
  }));
  const outline = exportOutline(nodes(page.blocks), { ...DEFAULT_EXPORT_OPTIONS, content: "source" });
  return (page.pre_block === null ? "" : `${page.pre_block}\n`) + outline;
}

function RecoveryDraft(props: { page: PageDto }): JSX.Element {
  const [message, setMessage] = createSignal("");
  const copy = async (complete: boolean) => {
    try {
      await writeClipboardText(complete ? JSON.stringify(props.page, null, 2) : recoveryDraftText(props.page));
      setMessage("Copied. Paste it into a separate file to keep a recovery copy.");
    } catch {
      setMessage("Could not copy. Select the draft below and copy it manually. Your draft remains here.");
    }
  };
  return <div class="recovery-draft">
    <pre tabIndex={0}>{recoveryDraftText(props.page)}</pre>
    <button onClick={() => void copy(false)}>Copy draft</button>
    <button onClick={() => void copy(true)}>Copy complete recovery data</button>
    <p role="status">{message()}</p>
  </div>;
}

export function UnsavedRecovery(): JSX.Element {
  return <Show when={unsavedRecoveryOpen()}><RecoveryPanel /></Show>;
}

function RecoveryPanel(): JSX.Element {
  const [pages, setPages] = createSignal(unsavedDrafts());
  const [busy, setBusy] = createSignal(false);
  const [message, setMessage] = createSignal("");
  let root: HTMLDivElement | undefined;
  // The engine's dirty/saving sets are not signals; poll while the panel is open.
  const refresh = () => {
    const next = unsavedDrafts();
    if (JSON.stringify(next) !== JSON.stringify(pages())) setPages(next);
  };
  onMount(() => {
    const timer = window.setInterval(refresh, 500);
    const unregister = registerTransientLayer({ id: "unsaved-recovery", root: () => root ?? null,
      dismiss: () => { closeUnsavedRecovery(); return true; } });
    root?.focus();
    onCleanup(() => { clearInterval(timer); unregister(); });
  });
  const retry = async () => {
    if (busy()) return;
    setBusy(true);
    try {
      const saved = await flushAll();
      setMessage(saved ? "All pending changes saved. You can close the window now." : "Some changes still need attention. Review the pages below.");
    } catch {
      setMessage("Saving failed. Your drafts remain available below.");
    } finally {
      setBusy(false);
      refresh();
    }
  };
  return <div class="modal-overlay">
    <div ref={root} tabIndex={-1} class="unsaved-recovery-panel" role="dialog" aria-modal="true" aria-label="Unsaved changes">
      <h2>Unsaved changes</h2>
      <p>{pages().length} affected {pages().length === 1 ? "page" : "pages"}. Review or copy your drafts before closing. Copying does not save the graph.</p>
      <button disabled={busy()} onClick={() => void retry()}>{busy() ? "Saving…" : "Retry saving"}</button>
      <button onClick={closeUnsavedRecovery}>Keep working</button>
      <p role="status">{message()}</p>
      <Show when={pages().length === 0 && earlierDrafts().length === 0}><p>No pending page drafts. If closing still fails, check pending attachments and storage status.</p></Show>
      <For each={pages()}>{(entry) => <section class="unsaved-recovery-entry">
        <h3>{entry.name} — {entry.state}</h3>
        <button onClick={() => {
          const kind = entry.page?.kind ?? "page";
          closeUnsavedRecovery();
          if (entry.path) mainPaneRouter.openFile(entry.path, entry.name, kind, { inPlace: true });
          else mainPaneRouter.openPage(entry.name, kind, { inPlace: true });
        }}>{entry.state === "Conflict" ? "Open page to resolve the conflict" : "Open page"}</button>
        <Show when={entry.page} fallback={<p>No page draft is available in this window.</p>}>
          {(page) => <RecoveryDraft page={page()} />}
        </Show>
      </section>}</For>
      <For each={earlierDrafts()}>{(record) => <section class="unsaved-recovery-entry">
        <h3>{record.page_name} — kept from an earlier session ({new Date(record.saved_at).toLocaleString()})</h3>
        <p>This draft was never saved to the page's file. Copy what you need into the page, then dismiss it.</p>
        <RecoveryDraft page={record.page} />
        <button onClick={() => void dismissEarlierDraft(record.id)}>Dismiss this draft</button>
      </section>}</For>
    </div>
  </div>;
}
