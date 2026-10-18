// GH #417 (with #418's toolbar remainder): act on ONE occurrence of a block
// reference `((uuid))` or an embed `{{embed …}}` inside the block that contains
// it (its host). The occurrence is located by the parser's source span of the
// rendered token, so two identical references in one block stay distinct.
//
// "Edit link" opens the host's editor with the occurrence selected (OG's
// double-click on an embed edits the host block, block.cljs `edit-parent-block`);
// "Remove" deletes the occurrence's text from the host only (OG
// `delete-current-ref!`), never the referenced block or page, as one undo step
// through the ordinary block save path.
import { createContext, useContext } from "solid-js";
import { msg } from "./i18n";
import { blockWritable, setRaw, withUndoUnit, node as docNode } from "./document";
import { startEditing } from "./editorController";
import { occurrenceSelection } from "./components/ReferenceEvidence";
import { pushToast } from "./toasts";
import { rebulletedSourceByteToRawByte, utf8ByteToUtf16Offset } from "./render/spans";
import type { Span } from "./render/ast";

export interface RefOccurrence {
  /** The block whose text contains the occurrence. */
  host: string;
  /** The parser's source span of the rendered token (UTF-8 bytes, rebulleted source). */
  span: Span;
  /** "ref" for `((uuid))` / `[label](((uuid)))`, "embed" for `{{embed …}}`. */
  kind: "ref" | "embed";
  /** Text the occurrence must contain: the uuid of a reference, the target of an embed. */
  target: string;
}

/** An occurrence with the rendered host instance (see `useOccurrenceOwner`) that
 *  Edit link opens; the block-reference context menu carries it. */
export type OwnedOccurrence = RefOccurrence & { owner: string | null };

/** The rendered block instance an occurrence's actions belong to. Rendered.tsx
 *  provides it, so Edit link turns exactly that copy of the host into an editor. */
export const OccurrenceHostContext = createContext<{ id: string; owner: string } | null>(null);

/** The rendered instance owning `host`, for `editOccurrence`; null when the
 *  nearest rendered block is another one (the editor then follows the caret's surface). */
export function useOccurrenceOwner(host: () => string | undefined): () => string | null {
  const ctx = useContext(OccurrenceHostContext);
  return () => (ctx && ctx.id === host() ? ctx.owner : null);
}

/** The occurrence's UTF-16 range in the host's current raw, or null when that
 *  text is no longer the rendered token (the block changed after it was drawn,
 *  e.g. an external editor or a sync delivery replaced it). */
function rawRange(o: RefOccurrence): { start: number; end: number } | null {
  const raw = docNode(o.host)?.raw;
  if (raw === undefined) return null;
  const start = utf8ByteToUtf16Offset(raw, rebulletedSourceByteToRawByte(raw, o.span[0]));
  const end = utf8ByteToUtf16Offset(raw, rebulletedSourceByteToRawByte(raw, o.span[1]));
  const text = raw.slice(start, end);
  const shaped = o.kind === "embed" ? text.startsWith("{{") && text.endsWith("}}") : text.includes("((");
  return shaped && text.toLowerCase().includes(o.target.toLowerCase()) ? { start, end } : null;
}

/** Whether Edit link and Remove may act: the host exists and its page accepts edits. */
export function occurrenceEditable(o: RefOccurrence | undefined): o is RefOccurrence {
  return !!o && !!docNode(o.host) && blockWritable(o.host);
}

function reportStale(): false {
  pushToast(msg("ref-occurrence-changed"), "warn");
  return false;
}

/** Open the host's editor with the occurrence's text selected. `owner` is the
 *  rendered host instance, so only that copy of the block turns into an editor. */
export function editOccurrence(o: RefOccurrence, owner: string | null): boolean {
  const n = docNode(o.host);
  const range = rawRange(o);
  if (!n || !range) return reportStale();
  startEditing(o.host, occurrenceSelection(n.raw, range, n.page), owner);
  return true;
}

/** Remove the occurrence's text from its host, with one adjacent space or tab
 *  (OG removes one whitespace before the reference; a line break is kept so
 *  lines never merge). The block itself stays, even when it ends up empty. */
export function removeOccurrence(o: RefOccurrence): boolean {
  const n = docNode(o.host);
  const range = rawRange(o);
  if (!n || !range || !blockWritable(o.host)) return reportStale();
  const raw = n.raw;
  const blank = (ch: string | undefined) => ch === " " || ch === "\t";
  let { start, end } = range;
  if (blank(raw[start - 1])) start -= 1;
  else if (blank(raw[end])) end += 1;
  const next = raw.slice(0, start) + raw.slice(end);
  withUndoUnit(o.kind === "embed" ? "remove-embed" : "remove-reference", [n.page], () => {
    setRaw(o.host, next, { timetracking: false });
  });
  return true;
}
