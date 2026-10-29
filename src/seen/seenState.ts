// Graph-level state and actions of "changed since you last looked" (ADR 0079):
// whether the graph can answer at all (checked once per binding, from
// getPageChanges.available), the revision every open page re-reads on, and the
// mark / forget actions. Marking and forgetting are skipped, silently, while the
// graph cannot answer (not a git repository): no error toasts on any platform.
import { createSignal } from "solid-js";
import { backend } from "../backend";
import { bindingOwner, graphOwner, readOwned, writeOwned } from "../owned";
import { clearOnBindingInvalidated } from "../binding";
import { msg } from "../i18n";
import { pushToast } from "../toasts";
import { dbg } from "../debug";
import { loadedPage } from "../document";
import { pageIdentityKey } from "../pageIdentity";
import { clearDisplayed, registerDisplayed, takeAllDisplayed, takeDisplayed } from "./displayed";

// true = a read answered (git graph on a supported device); false = the first
// read said it cannot; undefined = not asked yet. Once true it stays true for
// the binding: a page-level "baseline no longer on this branch" is not the
// graph losing git.
const [availability, setAvailability] = createSignal<boolean | undefined>(undefined);
/** Bumped after a mark or forget so every open page re-reads. */
const [revision, setRevision] = createSignal(0);
let probing = false;
clearOnBindingInvalidated(() => {
  setAvailability(undefined);
  probing = false;
  clearDisplayed();
  setRevision((n) => n + 1);
});

export const seenAvailable = availability;
export const seenRevision = revision;

export function noteSeenAnswer(available: boolean): void {
  if (available) setAvailability(true);
  else if (availability() === undefined) setAvailability(false);
}

/** Ask once per binding whether this graph can answer (for Settings and the
 *  palette before any page was read). A path no page owns reaches the same
 *  repository check a real read starts with. */
export function ensureSeenAvailability(): void {
  if (availability() !== undefined || probing) return;
  probing = true;
  const owner = graphOwner();
  void readOwned(owner, backend().getPageChanges("", "pages/.tine-seen-probe.md", ""))
    .then((answer) => { if (answer.kind === "current") noteSeenAnswer(answer.value.available); })
    .catch((error) => { dbg(`seen: availability probe failed: ${String(error)}`); if (owner()) noteSeenAnswer(false); })
    .finally(() => { probing = false; });
}

/** The identity a loaded page is marked under, or null when it has no file. */
export function seenIdentityOf(name: string): { key: string; path: string } | null {
  const page = loadedPage(name);
  return page?.id && !page.guide ? { key: pageIdentityKey(name), path: page.id } : null;
}

const failed = (error: unknown) => String(error instanceof Error ? error.message : error);

/** Mark a page seen: its baseline becomes the current HEAD. */
export async function markPageSeenNow(name: string, announce = true): Promise<boolean> {
  const page = seenIdentityOf(name);
  if (!page || availability() !== true) return false;
  try {
    const done = await writeOwned(bindingOwner(), backend().markPageSeen(page.key, page.path));
    if (done.kind === "stale") return false;
    setRevision((n) => n + 1);
    if (announce) pushToast(msg("seen-page-marked", { name }), "success");
    return true;
  } catch (error) {
    pushToast(msg("seen-mark-failed", { error: failed(error) }), "error");
    return false;
  }
}

export async function markGraphSeenNow(): Promise<boolean> {
  if (availability() !== true) return false;
  try {
    const done = await writeOwned(bindingOwner(), backend().markGraphSeen());
    if (done.kind === "stale") return false;
    setRevision((n) => n + 1);
    pushToast(msg("seen-graph-marked"), "success");
    return true;
  } catch (error) {
    pushToast(msg("seen-mark-failed", { error: failed(error) }), "error");
    return false;
  }
}

export async function forgetSeenStateNow(): Promise<boolean> {
  if (availability() !== true) return false;
  try {
    const done = await writeOwned(bindingOwner(), backend().forgetSeenState());
    if (done.kind === "stale") return false;
    setRevision((n) => n + 1);
    pushToast(msg("seen-state-forgotten"), "success");
    return true;
  } catch (error) {
    pushToast(msg("seen-forget-failed", { error: failed(error) }), "error");
    return false;
  }
}

async function markLeft(entry: NonNullable<ReturnType<typeof takeDisplayed>>): Promise<void> {
  if (availability() !== true || !entry.owner()) return;
  try {
    await writeOwned(bindingOwner(), backend().markPageSeen(entry.key, entry.path));
  } catch (error) {
    dbg(`seen: marking ${entry.path} seen on leave failed: ${failed(error)}`); // disposable cache: never an error for the user
  }
}

/** The user left this page (route away, pane or tab closed). A page Tine is
 *  renaming, merging or deleting was dropped from the registry already. */
export function leaveSeenPage(name: string): Promise<void> {
  const entry = takeDisplayed(name);
  return entry ? markLeft(entry) : Promise.resolve();
}

/** The app is closing: every page still shown counts as left. */
export async function leaveAllSeenPages(): Promise<void> {
  await Promise.all(takeAllDisplayed().map(markLeft));
}

export { registerDisplayed };

/** Test hook: forget what this binding learned. */
export function resetSeenForTests(): void {
  setAvailability(undefined);
  probing = false;
  clearDisplayed();
}
