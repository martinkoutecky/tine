/** S1 of the native-integrations batch (ADR 0073): shared items wait in the
 * app-owned share inbox (src-tauri/src/share_inbox.rs) and reach the graph
 * only here, through the single quick-capture writer (`appendToJournalDay`,
 * the day-parameterised `appendToTodayJournal`) inside `writeOwned`, as a new
 * block at the bottom of a journal (OG's behaviour, approved by Martin). An
 * item is removed from the inbox only after its write reached disk; a failed
 * write keeps it and says so.
 *
 * Loss-free first, then no duplicates (manager decision, review round 1).
 * Every step that touches the graph is preceded by a durable `prepared.json`:
 * 1. bind: the graph (its root) and the journal day, frozen once; `{date}`
 *    is that day;
 * 2. shape: the item's files are imported into that graph only, once, and
 *    the shaped Markdown and asset names are recorded before any append;
 * 3. arm: inside the admitted read the append uses, the day file's revision
 *    and the number of equal blocks on the page are recorded, then the block
 *    is inserted and flushed with no further await.
 * Recovery (a later pass) acts only in the recorded graph (another graph
 * keeps the item silently). An item that was never armed is appended (its
 * append never started). An armed item whose day now holds more equal
 * blocks than recorded has landed: flush, then remove it. Otherwise it is
 * appended again: the user may have edited it, and a possible duplicate
 * beats a lost share. Remaining duplicate/loss windows are listed in ADR 0073.
 *
 * Runs one pass at a time, at launch (after the graph loaded), when the app
 * returns to the foreground, and when a native producer signals an arrival.
 * Cost per pass: one inbox listing plus, per item, one journal page read, two
 * or three `prepared.json` writes and one append. */
import { createRoot, createEffect, on } from "solid-js";
import { backend, isTauri } from "./backend";
import { bindingOwner, writeOwned } from "./owned";
import { graphMeta } from "./graphSession";
import { pushToast } from "./toasts";
import { journalTitle, appNow } from "./journal";
import { appendToJournalDay, formatForPage, pageByName, trackAssetWrite } from "./document";
import { assetFileName, assetMarkdown } from "./media";
import { asOutlineBlock, captureTime, shapeShare } from "./shareShape";
import type { NativeShareInbox, ShareInboxItem, SharePrepared } from "./nativeTineLinks";

/** The toast the in-app quick capture shows (App.tsx `installQuickCaptureReceiver`). */
export const CAPTURED_TOAST = "Captured to today's journal";

/** done: written and removed; kept: retried later, the user is told;
 * elsewhere: bound to another graph, kept silently; stale: the binding moved. */
type Outcome = "done" | "kept" | "elsewhere" | "stale";

function basename(path: string): string {
  return path.split(/[\\/]/).pop() ?? "";
}

/** Import the item's files into the bound graph and shape its block (OG
 * transcription in shareShape.ts). */
async function shape(item: ShareInboxItem, day: string, live: () => boolean): Promise<{ markdown: string | null; assets: string[] } | "stale"> {
  const owner = bindingOwner(live);
  const format = formatForPage(day);
  const pagePath = pageByName(day)?.id;
  const names: string[] = [];
  const links: string[] = [];
  for (const resource of item.resources) {
    const label = resource.name || basename(resource.path) || undefined;
    const imported = await writeOwned(owner, trackAssetWrite(
      backend().importAsset(resource.path, assetFileName(label), backend().graphBindingGeneration())));
    if (imported.kind === "stale") return "stale";
    names.push(imported.value);
    links.push(assetMarkdown(imported.value, { label, pagePath, format }));
  }
  const meta = graphMeta();
  const content = shapeShare({ source: item.source, text: item.text, title: item.title, url: item.url, assets: links }, {
    time: captureTime(item.created ? new Date(item.created) : appNow()),
    date: day,
    format,
    textTemplate: meta?.quick_capture_template_text,
    mediaTemplate: meta?.quick_capture_template_media,
  });
  return { markdown: content === null ? null : asOutlineBlock(content), assets: names };
}

async function ingestOne(inbox: NativeShareInbox, item: ShareInboxItem, owner: () => boolean): Promise<Outcome> {
  const graph = graphMeta()?.root;
  if (!graph) return "stale";
  let prepared: SharePrepared | null = item.prepared ?? null;
  if (prepared && prepared.graph !== graph) return "elsewhere";
  if (!prepared) {
    // 1. Bind the item to this graph and freeze its day before any graph write.
    prepared = { graph, day: journalTitle(appNow()), markdown: null, assets: [], armed: null };
    await inbox.prepare(item.id, prepared);
    if (!owner()) return "stale";
  }
  if (prepared.markdown === null) {
    // 2. Import and shape once; recorded before any append.
    const shaped = await shape(item, prepared.day, owner);
    if (shaped === "stale") return "stale";
    // Nothing to write (a template that drops every field): keep the item.
    if (shaped.markdown === null) return "kept";
    prepared = { ...prepared, markdown: shaped.markdown, assets: shaped.assets };
    await inbox.prepare(item.id, prepared);
    if (!owner()) return "stale";
  }
  const record = prepared;
  const markdown = prepared.markdown;
  if (markdown === null) return "kept";
  // 3. Arm inside the admitted read, then append into exactly the recorded day.
  const written = await writeOwned(owner, appendToJournalDay(record.day, markdown, async (state) => {
    if (record.armed && state.matches > record.armed.matches) return "landed";
    await inbox.prepare(item.id, { ...record, armed: { before: state.before, matches: state.matches } });
    return "append";
  }));
  if (written.kind === "stale") return "stale";
  if (written.value === "failed") return "kept";
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
      if (outcome === "elsewhere") continue;
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
