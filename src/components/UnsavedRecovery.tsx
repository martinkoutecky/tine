import { For, Show, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { flushAll } from "../persistence";
import { mainPaneRouter } from "../router";
import { writeClipboardTextResilient } from "../clipboard";
import { closeUnsavedRecovery, recoveryDraftText, unsavedRecoveryOpen, unsavedRecoveryPages } from "../unsavedRecovery";
import type { PageDto } from "../types";
import { registerTransientLayer } from "../transientLayers";

export function RecoveryDraft(props: { page: PageDto; label?: string }): JSX.Element {
  const [message, setMessage] = createSignal("");
  const copy = async (complete = false) => {
    try {
      await writeClipboardTextResilient(complete ? JSON.stringify(props.page, null, 2) : recoveryDraftText(props.page));
      setMessage("Copied. Paste into a separate file to keep a recovery copy.");
    } catch {
      setMessage("Could not copy. Select the draft below and copy it manually. Your draft remains here.");
    }
  };
  return <section class="recovery-draft">
    <h3>{props.label ?? "Retained draft"}</h3>
    <pre tabIndex={0}>{recoveryDraftText(props.page)}</pre>
    <button onClick={() => void copy()}>Copy draft</button>
    <button onClick={() => void copy(true)}>Copy complete recovery data</button>
    <p role="status">{message()}</p>
  </section>;
}

export function UnsavedRecovery(): JSX.Element {
  return <Show when={unsavedRecoveryOpen()}><RecoveryPanel /></Show>;
}

function RecoveryPanel(): JSX.Element {
  const [pages, setPages] = createSignal(unsavedRecoveryPages());
  const [busy, setBusy] = createSignal(false);
  const [message, setMessage] = createSignal("");
  let root: HTMLDivElement | undefined;
  const refresh = () => {
    const next = unsavedRecoveryPages();
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
    } catch { setMessage("Saving failed. Your drafts remain available below."); }
    finally { setBusy(false); refresh(); }
  };
  return <div class="unsaved-recovery-overlay">
    <div ref={root} tabIndex={-1} class="unsaved-recovery-panel" role="dialog" aria-modal="true" aria-label="Unsaved changes">
      <h2>Unsaved changes</h2>
      <p>{pages().length} affected pages. Review or copy your drafts before closing. Copying does not save the graph.</p>
      <button disabled={busy()} onClick={() => void retry()}>{busy() ? "Saving…" : "Retry saving"}</button>
      <button onClick={closeUnsavedRecovery}>Keep working</button>
      <p role="status">{message()}</p>
      <Show when={pages().length === 0}><p>No pending page drafts. If closing still fails, check pending attachments and storage status.</p></Show>
      <For each={pages()}>{(entry) => <section>
        <h3>{entry.name} — {entry.state}</h3>
        <button onClick={() => {
          const page = entry.page ?? entry.retained;
          if (!page) return;
          closeUnsavedRecovery();
          if (page.path) mainPaneRouter.openFile(page.path, page.name, page.kind, { inPlace: true });
          else mainPaneRouter.openPage(page.name, page.kind);
        }}>Open page / resolve conflict</button>
        <Show when={entry.page}>{(page) => <RecoveryDraft page={page()} label="Current draft" />}</Show>
        <Show when={entry.retained && JSON.stringify(entry.retained) !== JSON.stringify(entry.page)}>
          <RecoveryDraft page={entry.retained!} label="Earlier retained conflict draft" />
        </Show>
        <Show when={!entry.page && !entry.retained}><p>No page draft is available in this window. Check the conflict and storage status before closing.</p></Show>
      </section>}</For>
    </div>
  </div>;
}
