// Clipboard writes and the private, per-webview block payload slot. The payload
// never reaches the OS clipboard: text/plain + text/html remain the only flavors.

import { backend } from "./backend";
import type { Format, PageKind } from "./types";
import { graphMeta } from "./graphSession";

export const CLIPBOARD_PAYLOAD_MAX_BLOCKS = 10_000;
export const CLIPBOARD_PAYLOAD_MAX_RAW_BYTES = 4 * 1024 * 1024;

export interface ClipboardBlock {
  /** Live store key, retained only for the first successful paste of a cut. */
  key?: string;
  raw: string;
  children: ClipboardBlock[];
  sourceFormat: Format;
}

export interface ClipboardSourcePage {
  name: string;
  kind: PageKind;
  path?: string;
  generation: number;
}

export interface ClipboardPayloadData {
  blocks: ClipboardBlock[];
  sourcePages: ClipboardSourcePage[];
}

export interface ClipboardPayloadSlot extends ClipboardPayloadData {
  op: "copy" | "cut";
  generation: number;
  graph: string;
  text: string;
}

export interface ConsumedCutGrant {
  generation: number;
  sourcePages: ClipboardSourcePage[];
}

let slot: ClipboardPayloadSlot | null = null;
let nextGeneration = 0;
let writeRevision = 0;

/** O(1) per-webview ownership token for every clipboard replacement, even without a private payload. */
export function clipboardWriteRevision(): number {
  return writeRevision;
}

/** Read the live private slot. Callers must treat the returned payload as immutable. */
export function peekClipboardPayload(): ClipboardPayloadSlot | null {
  return slot;
}

/** Clear any private payload synchronously before replacing the OS clipboard. */
export function clearClipboardPayload(): void {
  slot = null;
  writeRevision++;
}

/**
 * Spend a cut grant before any await. The expected generation prevents a stale
 * paste continuation from consuming a newer copy. The slot stays usable as an
 * ordinary copy payload, so a second paste is structural-only.
 */
export function consumeClipboardCutGrant(expectedGeneration: number): ConsumedCutGrant | null {
  if (!slot || slot.generation !== expectedGeneration || slot.op !== "cut") return null;
  const grant = { generation: slot.generation, sourcePages: slot.sourcePages };
  slot = { ...slot, op: "copy" };
  return grant;
}

/** Downgrade only a matching in-memory Cut grant to Copy. The caller decides
 * whether the source still qualifies. This leaves OS clipboard text and source
 * blocks alone; stale/missing grants are O(1) no-ops. */
export function cancelClipboardCutGrant(expectedGeneration: number): void {
  consumeClipboardCutGrant(expectedGeneration);
}

/** Native round trips observed by stage B normalize line endings and one final LF. */
export function normalizeClipboardText(text: string): string {
  const lf = text.replace(/\r\n/g, "\n");
  return lf.endsWith("\n") ? lf.slice(0, -1) : lf;
}

// Concise stage-B read API named by the CB1 contract. Keep the descriptive
// names above for call-site readability and backwards-compatible focused tests.
export const peekClipboardSlot = peekClipboardPayload;
export const consumeCutGrant = consumeClipboardCutGrant;
export const clearClipboardSlot = clearClipboardPayload;
export const normalize = normalizeClipboardText;

const esc = (s: string): string =>
  s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

/** Convert a tab-indented Logseq outline into nested list HTML. */
export function outlineToHtml(md: string): string {
  const items: { depth: number; html: string }[] = [];
  for (const raw of md.split("\n")) {
    const m = /^(\t*)- ?(.*)$/.exec(raw);
    if (m) items.push({ depth: m[1].length, html: esc(m[2]) });
    else if (items.length && raw.trim()) items[items.length - 1].html += "<br>" + esc(raw.replace(/^[\t ]+/, ""));
  }
  if (!items.length) return "";
  let out = "";
  const stack: number[] = [];
  for (const it of items) {
    while (stack.length && stack[stack.length - 1] > it.depth) {
      out += "</li></ul>";
      stack.pop();
    }
    if (stack.length && stack[stack.length - 1] === it.depth) out += "</li>";
    else {
      out += "<ul>";
      stack.push(it.depth);
    }
    out += "<li>" + it.html;
  }
  while (stack.length) {
    out += "</li></ul>";
    stack.pop();
  }
  return out;
}

/** Ordinary text write: replacing the public clipboard invalidates private data. */
export function writeClipboardText(text: string): Promise<void> {
  clearClipboardPayload();
  return backend().writeText(text);
}

/**
 * Strict text write for UI that reports clipboard rejection to the user.
 * This preserves ImproveTab's former navigator transport semantics while still
 * synchronously invalidating private block data at the shared facade boundary.
 */
export function writeClipboardTextStrict(text: string): Promise<void> {
  clearClipboardPayload();
  return navigator.clipboard.writeText(text);
}

/** Ordinary rich write: replacing the public clipboard invalidates private data. */
export function writeClipboardRich(text: string, html: string): Promise<void> {
  clearClipboardPayload();
  return backend().writeRich(text, html);
}

/** Ordinary image write: replacing the public clipboard invalidates private data. */
export function writeClipboardImage(bytes: Uint8Array): Promise<void> {
  clearClipboardPayload();
  return backend().copyImageToClipboard(bytes);
}

/** Put an ordinary outline on the clipboard without recording a block payload. */
export function copyRich(text: string, html: string): Promise<void> {
  return writeClipboardRich(text, html);
}

export function copyOutline(md: string): Promise<void> {
  return copyRich(md, outlineToHtml(md));
}

/**
 * Dedicated block copy/cut ordering boundary: clear old private state, start the
 * external write, then publish the fresh generation before returning. The write
 * uses the transport directly so it cannot clear the slot it just created.
 * A rejected transport write clears that generation's private slot and
 * rejects to the caller, which must keep the source block and report failure.
 */
export function copyBlockOutline(
  op: "copy" | "cut",
  text: string,
  payload: ClipboardPayloadData | null,
): Promise<void> {
  clearClipboardPayload();
  const write = backend().writeRich(text, outlineToHtml(text));
  if (payload) {
    slot = {
      op,
      generation: ++nextGeneration,
      graph: graphMeta()?.root ?? "",
      text,
      blocks: payload.blocks,
      sourcePages: op === "cut" ? payload.sourcePages : [],
    };
  }
  const pendingGeneration = slot?.generation;
  return write.catch((error) => {
    // A rejected external write must not leave a private cut grant behind.
    // A newer clipboard action owns its own generation and must survive.
    if (pendingGeneration !== undefined && slot?.generation === pendingGeneration) clearClipboardPayload();
    throw error;
  });
}

// Browser-native copy/cut paths (notably selected textarea text) have no JS
// writer to route through the facade. Block-selection shortcuts prevent their
// native event, so this only clears clipboard content that really replaced it.
if (typeof document !== "undefined") {
  document.addEventListener("copy", clearClipboardPayload, true);
  document.addEventListener("cut", clearClipboardPayload, true);
}
