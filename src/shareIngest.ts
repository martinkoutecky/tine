/** S1 of the native-integrations batch (ADR 0073): shared items wait in the
 * app-owned share inbox (src-tauri/src/share_inbox.rs) and reach the graph
 * only here, through the single quick-capture writer `appendToTodayJournal`
 * inside `writeOwned`, as a new block at the bottom of today's journal (OG's
 * behaviour, approved by Martin). An item is removed from the inbox only
 * after its write reached disk; a failed write keeps it and says so.
 *
 * Idempotency across a crash between the journal write and the removal: the
 * item's `prepared.json` records the shaped Markdown, the target day and how
 * many root blocks of that day already had the same body (`baseline`), and is
 * durable before the append starts. A later pass that finds more such blocks
 * than the baseline knows the append landed and only removes the item.
 *
 * Runs one pass at a time, at launch (after the graph loaded), when the app
 * returns to the foreground, and when a native producer signals an arrival.
 * Cost per pass: one inbox listing plus, per item, one journal page read and
 * one append. */
import { createRoot, createEffect, on } from "solid-js";
import { backend, isTauri } from "./backend";
import { bindingOwner, writeOwned } from "./owned";
import { graphMeta } from "./graphSession";
import { pushToast } from "./toasts";
import { journalTitle, appNow } from "./journal";
import { appendToTodayJournal, countRootBlocks, flushPage, formatForPage, pageByName, trackAssetWrite } from "./document";
import { parseOutline } from "./editor/outline";
import { assetFileName, assetMarkdown } from "./media";
import { asOutlineBlock, captureTime, shapeShare } from "./shareShape";
import type { NativeShareInbox, ShareInboxItem, SharePrepared } from "./nativeTineLinks";

/** The toast the in-app quick capture shows (App.tsx `installQuickCaptureReceiver`). */
export const CAPTURED_TOAST = "Captured to today's journal";

type Outcome = "done" | "kept" | "stale";

function basename(path: string): string {
  return path.split(/[\\/]/).pop() ?? "";
}

function firstBody(markdown: string): string {
  return parseOutline(markdown)[0]?.raw ?? "";
}

/** Import the item's files and shape its block (OG transcription in shareShape.ts). */
async function shape(item: ShareInboxItem, day: string, live: () => boolean): Promise<string | "stale" | null> {
  const owner = bindingOwner(live);
  const format = formatForPage(day);
  const pagePath = pageByName(day)?.id;
  const assets: string[] = [];
  for (const resource of item.resources) {
    const label = resource.name || basename(resource.path) || undefined;
    const imported = await writeOwned(owner, trackAssetWrite(
      backend().importAsset(resource.path, assetFileName(label), backend().graphBindingGeneration())));
    if (imported.kind === "stale") return "stale";
    assets.push(assetMarkdown(imported.value, { label, pagePath, format }));
  }
  const meta = graphMeta();
  const content = shapeShare({ text: item.text, title: item.title, url: item.url, assets }, {
    time: captureTime(item.created ? new Date(item.created) : appNow()),
    date: day,
    format,
    textTemplate: meta?.quick_capture_template_text,
    mediaTemplate: meta?.quick_capture_template_media,
  });
  return content === null ? null : asOutlineBlock(content);
}

async function ingestOne(inbox: NativeShareInbox, item: ShareInboxItem, owner: () => boolean): Promise<Outcome> {
  let prepared: SharePrepared | null = item.prepared ?? null;
  // An attempt was recorded: did its append land before the crash?
  if (prepared && prepared.baseline !== null) {
    const count = await countRootBlocks(prepared.day, firstBody(prepared.markdown));
    if (!owner()) return "stale";
    if (count === null) return "kept";
    if (count > prepared.baseline) {
      if (!(await flushPage(prepared.day))) return owner() ? "kept" : "stale";
      if (!owner()) return "stale";
      await inbox.commit(item.id);
      return "done";
    }
  }
  const day = journalTitle(appNow());
  if (!prepared) {
    const markdown = await shape(item, day, owner);
    if (markdown === "stale") return "stale";
    // Nothing to write (a template that drops every field): keep the item.
    if (markdown === null) return "kept";
    prepared = { markdown, day, baseline: null };
  }
  // Count just before the append, on the day it targets.
  const baseline = await countRootBlocks(day, firstBody(prepared.markdown));
  if (!owner()) return "stale";
  if (baseline === null) return "kept";
  prepared = { ...prepared, day, baseline };
  await inbox.prepare(item.id, prepared);
  if (!owner()) return "stale";
  const written = await writeOwned(owner, appendToTodayJournal(prepared.markdown));
  if (written.kind === "stale") return "stale";
  if (!written.value) return "kept";
  await inbox.commit(item.id);
  return "done";
}

/** One pass over the inbox. Resolves when it finished; never rejects. */
export async function ingestShares(): Promise<void> {
  const inbox = backend().tineLinks?.inbox;
  if (!inbox || !graphMeta()) return;
  const owner = bindingOwner();
  try {
    const listing = await inbox.list();
    if (listing.rejected > 0 && owner()) pushToast(
      `${listing.rejected === 1 ? "A shared item" : `${listing.rejected} shared items`} couldn't be read and ${listing.rejected === 1 ? "was" : "were"} kept aside in Tine's share inbox.`, "error");
    for (const item of listing.items) {
      let outcome: Outcome;
      try {
        outcome = await ingestOne(inbox, item, owner);
      } catch (error) {
        if (!owner()) return;
        pushToast(`A shared item couldn't be added to today's journal: ${String(error)}. It is kept and will be retried.`, "error");
        continue;
      }
      if (outcome === "stale") return;
      if (outcome === "kept") {
        pushToast("A shared item couldn't be added to today's journal. It is kept and will be retried.", "error");
        continue;
      }
      pushToast(CAPTURED_TOAST, "info");
    }
  } catch (error) {
    if (owner()) pushToast(`Couldn't read Tine's share inbox: ${String(error)}`, "error");
  }
}

let running: Promise<void> | null = null;
let again = false;

/** Run a pass now, or once more after the running one. */
export function requestShareIngest(): Promise<void> {
  if (running) { again = true; return running; }
  running = (async () => {
    try {
      do { again = false; await ingestShares(); } while (again);
    } finally { running = null; }
  })();
  return running;
}

/** Ingest when a graph is bound, on foreground return and on native arrival.
 * Returns an idempotent disposer. */
export async function installShareIngest(alive: () => boolean): Promise<() => void> {
  const inbox = backend().tineLinks?.inbox;
  if (!isTauri() || !inbox) return () => {};
  let disposed = false;
  const run = () => { if (!disposed && alive()) void requestShareIngest(); };
  const disposeRoot = createRoot((dispose) => {
    createEffect(on(() => graphMeta()?.root, (root) => { if (root) run(); }));
    return dispose;
  });
  const visible = () => { if (document.visibilityState === "visible") run(); };
  document.addEventListener("visibilitychange", visible);
  let unlisten: () => void = () => {};
  try { unlisten = await inbox.subscribe(run); }
  catch { /* desktop has no producer plugin; launch and resume still ingest */ }
  if (disposed || !alive()) unlisten();
  return () => {
    if (disposed) return;
    disposed = true;
    disposeRoot();
    document.removeEventListener("visibilitychange", visible);
    unlisten();
  };
}
