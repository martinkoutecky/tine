import { createSignal } from "solid-js";
import { backend } from "./backend";
import { bindingOwner, ownedWhen, readOwned, writeOwned, type WriteOwner } from "./owned";
import { applyCustomCss, applyGlobalCss, customCssVersion } from "./customCss";
import { closeSettings } from "./ui";
import { pushToast } from "./toasts";
import { t } from "./i18n";
import { clearOnBindingInvalidated } from "./binding";
import { isPublishedExport } from "./publishedBackend";
import { graphMeta } from "./graphSession";
import { injectCustomCss } from "./graph";
import { reportUiFailure } from "./uiFailure";

type CssSession = { global: boolean; revision?: string; saved: string; owner: WriteOwner; draftKey: string };
type CssDraft = { text: string; saved: string; revision?: string };
let draftWrites: Promise<void> = Promise.resolve();
function keepDraft(current: CssSession, text: string): Promise<void> {
  const value = text === current.saved ? "" : JSON.stringify({ text, saved: current.saved, revision: current.revision });
  // Serialize capsules and their removal, so an older write cannot resurrect a
  // saved draft. Existing app-settings persistence owns the durable file write.
  const write = draftWrites.then(() => backend().setAppString(current.draftKey, value));
  draftWrites = write.catch(error => { reportUiFailure("css-draft", error); });
  return write;
}
const [session, setSession] = createSignal<CssSession | null>(null);
export const cssEditorSession = session;
export const [cssEditorText, setCssEditorText] = createSignal("");
export const [cssEditorProblem, setCssEditorProblem] = createSignal("");
export const [cssEditorBusy, setCssEditorBusy] = createSignal(false);
let saving: Promise<boolean> | null = null;
let opening = 0;

/** A graph rebind retires only its graph editor, after the switching gate has
 * saved it. Device CSS remains editable across graph bindings. */
export function retireGraphCssEditor(): void {
  if (session() && !session()!.global) { ++opening; setSession(null); setCssEditorBusy(false); }
}
clearOnBindingInvalidated(retireGraphCssEditor);

/** One editor for the two fixed files. Opening another file first saves the
 * current buffer; a conflict retains the current editor and its user's text. */
export async function openCssEditor(global: boolean): Promise<void> {
  if (cssEditorBusy()) return;
  if (!(await flushCssEditor())) return;
  if (cssEditorBusy()) return;
  const generation = ++opening;
  const draftKey = global ? "css_draft_global_v1" : `css_draft_graph_v1:${graphMeta()?.root ?? ""}`;
  const baseOwner = global ? ownedWhen(() => true) : bindingOwner();
  const requestOwner = ownedWhen(baseOwner, () => generation === opening);
  // A failed read must not retire the editor it was going to replace. The
  // session owns its visible identity, independently of pending open requests.
  const owner: WriteOwner = ownedWhen(baseOwner, () => session()?.owner === owner);
  setCssEditorBusy(true);
  try {
    const result = await readOwned(requestOwner, backend().readCssFile(global));
    if (result.kind !== "current") return;
    await draftWrites;
    const recovery = await readOwned(requestOwner, backend().getAppString(draftKey, ""));
    if (recovery.kind !== "current") return;
    let draft: CssDraft | undefined;
    if (recovery.value) {
      const value: unknown = JSON.parse(recovery.value);
      if (value && typeof value === "object" && "text" in value && typeof value.text === "string"
        && "saved" in value && typeof value.saved === "string"
        && (!("revision" in value) || typeof value.revision === "string")) draft = value as CssDraft;
    }
    // If the guarded save completed before a reap, the capsule is already on disk.
    if (draft?.text === result.value.text) draft = undefined;
    setCssEditorText(draft?.text ?? result.value.text);
    setCssEditorProblem(draft ? t("css-draft-recovered") : "");
    setSession({ global, draftKey, revision: draft ? draft.revision : result.value.revision ?? undefined, saved: draft?.saved ?? result.value.text, owner });
    closeSettings();
  } catch (error) {
    if (requestOwner()) pushToast(t("css-read-failed", { error: String(error) }), "error");
  } finally { if (generation === opening) setCssEditorBusy(false); }
}

/** Participates in graph-switch and exit persistence. Uses the revision from
 * the read, never a newly fetched revision for a stale full-file buffer. */
export function flushCssEditor(): Promise<boolean> {
  if (saving) return saving;
  const current = session();
  if (!current || cssEditorText() === current.saved) return Promise.resolve(true);
  if (!current.owner()) return Promise.resolve(false);
  const text = cssEditorText();
  const owner = ownedWhen(current.owner);
  setCssEditorBusy(true);
  saving = (async () => {
    try {
      // Keep the exact buffer and original revision before attempting a save.
      // A conflict or a background WebView reap must not erase the user's work.
      await keepDraft(current, text);
      if (!owner()) return false;
      const result = await writeOwned(owner, backend().saveCssFile(current.global, text, current.revision ?? null));
      if (result.kind !== "current") return false;
      const saved = { ...current, revision: result.value, saved: text };
      setSession(saved);
      setCssEditorProblem("");
      (current.global ? applyGlobalCss : applyCustomCss)(text);
      // A watcher read started before this IPC reply may contain NEWER bytes.
      // Retire those reads and start a fresh one after the durable save.
      await (current.global ? loadGlobalCss() : injectCustomCss(true));
      await checkCssEditorDisk(current.global, true);
      if (!owner()) return false;
      await keepDraft(saved, cssEditorText());
      return true;
    } catch (error) {
      const message = t("css-save-failed", { error: String(error) });
      if (session()?.owner === current.owner) setCssEditorProblem(message);
      pushToast(t("css-save-failed", { error: String(error) }), "error");
      return false;
    } finally { setCssEditorBusy(false); saving = null; }
  })();
  return saving;
}

export async function closeCssEditor(): Promise<void> {
  if (cssEditorBusy()) return;
  if (!(await flushCssEditor())) return;
  ++opening;
  setSession(null);
}

export async function reloadCssEditor(): Promise<void> {
  const current = session();
  if (!current || cssEditorBusy()) return;
  const owner = ownedWhen(current.owner);
  if (cssEditorText() !== current.saved) {
    const answer = await readOwned(owner, backend().confirm(t("css-reload-confirm"), t("css-editor-title")));
    if (answer.kind !== "current" || !answer.value) return;
  }
  setCssEditorBusy(true);
  try {
    const result = await readOwned(owner, backend().readCssFile(current.global));
    if (result.kind !== "current") return;
    setCssEditorText(result.value.text);
    setSession({ ...current, revision: result.value.revision ?? undefined, saved: result.value.text });
    await keepDraft({ ...current, saved: result.value.text }, result.value.text);
    setCssEditorProblem("");
  } catch (error) { if (current.owner()) setCssEditorProblem(t("css-read-failed", { error: String(error) })); }
  finally { setCssEditorBusy(false); }
}

/** Watcher/focus notification: keep the editor's base and buffer on a change.
 * The subsequent Save still has the authoritative guarded revision check. */
export async function checkCssEditorDisk(global: boolean, afterSave = false): Promise<void> {
  const current = session();
  if (!current || current.global !== global || !current.owner() || (cssEditorBusy() && !afterSave)) return;
  const owner = ownedWhen(current.owner);
  try {
    const result = await readOwned(owner, backend().readCssFile(global));
    if (result.kind !== "current" || session() !== current) return;
    if ((result.value.revision ?? undefined) !== current.revision) setCssEditorProblem(t("css-external-conflict"));
  } catch (error) { if (current.owner()) setCssEditorProblem(t("css-read-failed", { error: String(error) })); }
}

let globalReadGeneration = 0;
export async function loadGlobalCss(): Promise<void> {
  // A published site has no access to the native app's per-user config.
  if (isPublishedExport()) return;
  const generation = ++globalReadGeneration;
  const version = customCssVersion(true);
  try {
    const result = await readOwned(ownedWhen(() => generation === globalReadGeneration && version === customCssVersion(true)), backend().readCssFile(true));
    if (result.kind === "current") applyGlobalCss(result.value.text);
  } catch (error) { pushToast(t("css-read-failed", { error: String(error) }), "error"); }
}
