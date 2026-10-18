import { createSignal } from "solid-js";
import { backend } from "./backend";
import { loadPreference, seedPreference, writePreference } from "./preferenceWrites";

// "Your name" (margin dialogue, D-20261017-LD1): the name a comment or reply you
// write carries as `author::`, so a graph shared through git shows who wrote
// what. A device preference in tine-settings.json (generic app_string backend),
// never in the graph. Unset: no `author::` is written and your own comments
// render as "you", as before. A comment whose `author::` equals the name is
// yours and renders as "you" too.

const KEY = "comment_author_name";
const MAX_LENGTH = 80;

const [nameSig, setNameSig] = createSignal("");

/** Reactive: the configured name, `""` when unset. */
export const commentAuthorName = nameSig;

/** A name as it is stored and written: one line, single spaces, trimmed, at most
 * 80 characters. Pure. */
export function normalizeAuthorName(value: string): string {
  return value.replace(/\s+/g, " ").trim().slice(0, MAX_LENGTH).trim();
}

/** True when `author` (a block's `author::` value) is the configured name, compared
 * ignoring case. False when no name is set. Reactive. */
export function isOwnAuthor(author: string): boolean {
  const own = nameSig();
  return own !== "" && normalizeAuthorName(author).toLowerCase() === own.toLowerCase();
}

/** Apply now and persist; a failed save rolls back and toasts (writePreference). */
export function setCommentAuthorName(value: string): void {
  writePreference(nameSig, setNameSig, normalizeAuthorName(value), (next) => backend().setAppString(KEY, next), "comment author name");
}

/** Load the name at startup; a read failure toasts and leaves it unset. */
export function initCommentIdentity(): void {
  loadPreference(nameSig, setNameSig, () => backend().getAppString(KEY, ""), (v) => normalizeAuthorName(v), "comment author name");
}

/** Test seam: set the name without persisting. */
export function resetCommentAuthorNameForTests(value = ""): void {
  setNameSig(value);
  seedPreference(nameSig);
}

