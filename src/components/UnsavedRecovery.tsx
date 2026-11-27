import { t } from "../i18n";
import { DialogActions } from "./DialogActions";
// The unsaved-changes recovery panel (GH #540, master dc3f2104b). Opened when the
// user declines to discard at close, or from a failed-save toast. It lists every
// page whose edits are not on disk and offers the ways out that never lose text:
// retry the ordinary save, open the page (its conflict bar resolves it), or copy
// the draft. Copying is read-only and never acknowledges a save.
import { For, Show, createEffect, createSignal, on, onCleanup, onMount, type JSX } from "solid-js";
import { flushAll, hostScope, hostSession, unsavedDrafts, type KeptOriginal, type KeptText } from "../document";
import { backend } from "../backend";
import { bindingCurrent, captureBinding, type Binding } from "../binding";
import { latestOwner, ownedWhen, readOwned } from "../owned";
import { mainPaneRouter } from "../router";
import { writeClipboardText } from "../clipboard";
import { closeUnsavedRecovery, unsavedRecoveryOpen } from "../unsavedRecovery";
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
    <button class="settings-btn" type="button" onClick={() => void copy(false)}>{t("unsaved-recovery-copy-draft")}</button>
    <button class="settings-btn" type="button" onClick={() => void copy(true)}>{t("unsaved-recovery-copy-complete-recovery-data")}</button>
    <p role="status">{message()}</p>
  </div>;
}

/** The list one host session gave, which its rows carry. */
interface KeptList { session: number; rows: KeptOriginal[] }

/** The originals a cancelled rename kept, each with Copy original text (GH #540).
 * Only reads: nothing here writes to the graph. The panel lists them when it
 * opens and whenever a (re)bind moves the host scope (a window reload's new
 * session), never in its poll. A reply applies only to this panel, in the scope
 * and graph binding it was asked in; a list reply only if it is the newest. Every
 * command carries the session that listed its row, which the host checks. */
function KeptOriginals(props: { binding: Binding }): JSX.Element {
  const [list, setList] = createSignal<KeptList | null>(null);
  const [notice, setNotice] = createSignal("");
  /** This panel's own request sequence (`latestOwner`). */
  const requests = {};
  let mounted = true;
  onCleanup(() => { mounted = false; });
  /** The owner of work asked now: this panel, its graph and its host scope. */
  const owned = () => {
    const scope = hostScope();
    return () => mounted && bindingCurrent(props.binding) && scope === hostScope();
  };
  const unavailable = () => setNotice(t("unsaved-recovery-kept-unavailable"));
  const relist = async () => {
    const session = hostSession();
    const owner = latestOwner(requests, "kept-originals", owned());
    if (session === null) return unavailable();
    let rows: KeptOriginal[] | null;
    try {
      const read = await readOwned(owner, backend().pageKeptOriginals(session));
      if (read.kind === "stale") return;
      rows = read.value;
    } catch {
      if (!owner()) return;
      rows = null;
    }
    // Not answered: the last list stays.
    if (rows === null) return unavailable();
    setList({ session, rows });
  };
  createEffect(on(hostScope, () => void relist()));
  const copy = async (row: KeptOriginal, session: number) => {
    const owner = ownedWhen(owned(), () => hostSession() === session);
    if (!owner()) return;
    let answer: KeptText;
    try {
      const read = await readOwned(owner, backend().pageKeptText(session, row.key, row.origin));
      if (read.kind === "stale") return;
      answer = read.value;
    } catch {
      if (!owner()) return;
      answer = { kind: "unavailable" };
    }
    switch (answer.kind) {
      case "text":
        try {
          // The owner is checked again before each clipboard transport.
          await writeClipboardText(answer.text, owner);
          if (owner()) setNotice(t("unsaved-recovery-original-copied"));
        } catch {
          if (owner()) setNotice(t("unsaved-recovery-original-copy-failed"));
        }
        return;
      case "notText": return setNotice(t("unsaved-recovery-original-not-text"));
      case "missing":
        setNotice(t("unsaved-recovery-original-not-kept"));
        return void relist();
      case "unavailable": return unavailable();
    }
  };
  return <Show when={list()?.rows.length || notice()}>
    <section class="unsaved-recovery-entry">
      <h3>{t("unsaved-recovery-kept-heading")}</h3>
      <p>{t("unsaved-recovery-kept-detail")}</p>
      <For each={list()?.rows ?? []}>{(row) => <div class="recovery-kept-row">
        <span>{row.path}</span>
        <Show when={row.text} fallback={<span>{t("unsaved-recovery-original-not-text")}</span>}>
          <button class="settings-btn" type="button" onClick={() => void copy(row, list()!.session)}>{t("unsaved-recovery-copy-original")}</button>
        </Show>
      </div>}</For>
      <p role="status">{notice()}</p>
    </section>
  </Show>;
}

export function UnsavedRecovery(): JSX.Element {
  return <Show when={unsavedRecoveryOpen()}><RecoveryPanel /></Show>;
}

function RecoveryPanel(): JSX.Element {
  const binding = captureBinding();
  const [pages, setPages] = createSignal(unsavedDrafts());
  const [busy, setBusy] = createSignal(false);
  const [message, setMessage] = createSignal("");
  let root: HTMLDivElement | undefined;
  // The page client's state is not one signal; poll while the panel is open.
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
    <div ref={root} tabIndex={-1} class="unsaved-recovery-panel" role="dialog" aria-modal="true" aria-label={t("unsaved-recovery-unsaved-changes")}>
      <h2>{t("unsaved-recovery-unsaved-changes")}</h2>
      <p>{t("unsaved-recovery-summary", { count: pages().length })}</p>
      <DialogActions
        cancel={<button class="settings-btn" type="button" onClick={closeUnsavedRecovery}>{t("unsaved-recovery-keep-working")}</button>}
        primary={<button class="settings-btn" type="button" disabled={busy()} onClick={() => void retry()}>{busy() ? t("unsaved-recovery-saving") : t("unsaved-recovery-retry-saving")}</button>} />
      <p role="status">{message()}</p>
      <KeptOriginals binding={binding} />
      <Show when={pages().length === 0}><p>{t("unsaved-recovery-no-pending-page-drafts-if-closing-still-fails")}</p></Show>
      <For each={pages()}>{(entry) => <section class="unsaved-recovery-entry">
        <h3>{entry.name} — {entry.state}</h3>
        <button class="settings-btn" type="button" onClick={() => {
          const kind = entry.page?.kind ?? "page";
          closeUnsavedRecovery();
          if (entry.path) mainPaneRouter.openFile(entry.path, entry.name, kind, { inPlace: true });
          else mainPaneRouter.openPage(entry.name, kind, { inPlace: true });
        }}>{entry.state === "Conflict" ? t("unsaved-recovery-open-page-to-resolve-the-conflict") : t("unsaved-recovery-open-page")}</button>
        <Show when={entry.page} fallback={<p>{t("unsaved-recovery-no-page-draft-is-available-in-this-window")}</p>}>
          {(page) => <RecoveryDraft page={page()} />}
        </Show>
      </section>}</For>
    </div>
  </div>;
}
