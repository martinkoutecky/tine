import { doc, formatForBlock, setRaw, setBlockProperty, setSchedule, blockPageReadOnly, withUndoUnit } from "../store";
import { facetsFromDto, facetsOf, inlineText, parseBody, tagIdentityKey, type Facets } from "../render/facets";
import { isRenderHiddenProp } from "../render/block";
import { leadingMarker, nextMarker, setMarker } from "../editor/marker";
import { cycleMarkerSmart } from "../editor/repeat";
import { MARKERS } from "../markers";
import { MARKER_RE } from "../markers";
import { workflow, timetrackingEnabled, logbookWithSecondSupport } from "../ui";
import type { Inline } from "../render/ast";
import { rebulletedSourceByteToRawByte, utf8ByteLength, utf8ByteToUtf16Offset } from "../render/spans";
import { tagRef } from "../tags";
import { parseIsoDateLike } from "./typed";
import { evaluateFormulaForRow, formulaValueText } from "./formulaEval";
import type { BlockDto } from "../types";

export type FieldId =
  | "state"
  | "priority"
  | "scheduled"
  | "deadline"
  | "tags"
  | "page"
  | `prop:${string}`
  | `formula:${string}`;

export interface FieldValue {
  text: string;
  raw?: string;
}

export function isFieldId(value: string): value is FieldId {
  return (
    value === "state" ||
    value === "priority" ||
    value === "scheduled" ||
    value === "deadline" ||
    value === "tags" ||
    value === "page" ||
    value.startsWith("prop:") ||
    value.startsWith("formula:")
  );
}

export function isFormulaField(field: FieldId): field is `formula:${string}` {
  return field.startsWith("formula:");
}

function facetsForBlock(id: string): Facets | null {
  const n = doc.byId[id];
  return n ? facetsOf(n.raw, formatForBlock(id)) : null;
}

type GroupKeyInput = string | { id: string; page?: string; dto?: BlockDto };
interface GroupKeysOptions {
  formulas?: ReadonlyMap<string, string>;
  now?: Date;
}

function facetsForInput(input: GroupKeyInput): Facets | null {
  const id = typeof input === "string" ? input : input.id;
  const n = doc.byId[id];
  if (n) return facetsOf(n.raw, formatForBlock(id));
  return typeof input === "string" || !input.dto ? null : facetsFromDto(input.dto);
}

function tagSetHas(f: Facets, tag: string): boolean {
  const key = tagIdentityKey(tag);
  return f.tags.some((t) => tagIdentityKey(t) === key);
}

function visitTagInlines(inlines: readonly Inline[], fn: (tag: Extract<Inline, { k: "tag" }>) => boolean): boolean {
  for (const i of inlines) {
    if (i.k === "tag") {
      if (fn(i)) return true;
      if (visitTagInlines(i.children, fn)) return true;
    } else if (i.k === "emphasis" || i.k === "subscript" || i.k === "superscript") {
      if (visitTagInlines(i.children, fn)) return true;
    } else if (i.k === "link" && i.label) {
      if (visitTagInlines(i.label, fn)) return true;
    }
  }
  return false;
}

function firstLineByteLength(raw: string): number {
  const nl = raw.indexOf("\n");
  return utf8ByteLength(nl === -1 ? raw : raw.slice(0, nl));
}

function firstLineTagRange(raw: string, tag: string): [number, number] | null {
  const key = tagIdentityKey(tag);
  const firstLineEnd = firstLineByteLength(raw);
  let found: [number, number] | null = null;
  for (const b of parseBody(raw, "md")) {
    if (!("inline" in b) || !Array.isArray(b.inline)) continue;
    if (visitTagInlines(b.inline, (i) => {
      if (!i.span || tagIdentityKey(inlineText(i.children)) !== key) return false;
      const start = rebulletedSourceByteToRawByte(raw, i.span[0]);
      const end = rebulletedSourceByteToRawByte(raw, i.span[1]);
      if (start < end && end <= firstLineEnd) {
        found = [start, end];
        return true;
      }
      return false;
    })) break;
  }
  return found;
}

function normalizeFirstLineCut(raw: string, cutAt: number): string {
  const nl = raw.indexOf("\n");
  const lineEnd = nl === -1 ? raw.length : nl;
  let line = raw.slice(0, lineEnd);
  const rest = raw.slice(lineEnd);
  const pos = Math.min(cutAt, line.length);
  const left = line.slice(0, pos);
  const right = line.slice(pos);
  const leftWs = /[ \t]*$/.exec(left)?.[0] ?? "";
  const rightWs = /^[ \t]*/.exec(right)?.[0] ?? "";
  if (leftWs || rightWs) {
    const before = left.slice(0, left.length - leftWs.length);
    const after = right.slice(rightWs.length);
    if (before && after) line = `${before} ${after}`;
    else if (!before && left.length > 0 && after) line = `${left}${after}`;
    else line = `${before}${after}`;
  }
  line = line.replace(/[ \t]+$/, "");
  return `${line}${rest}`;
}

function removeTagFromRaw(raw: string, tag: string): string | null {
  const range = firstLineTagRange(raw, tag);
  if (!range) return null;
  const [startByte, endByte] = range;
  const start = utf8ByteToUtf16Offset(raw, startByte);
  const end = utf8ByteToUtf16Offset(raw, endByte);
  return normalizeFirstLineCut(raw.slice(0, start) + raw.slice(end), start);
}

function addTagToRaw(raw: string, tag: string): string {
  const nl = raw.indexOf("\n");
  const lineEnd = nl === -1 ? raw.length : nl;
  const first = raw.slice(0, lineEnd).replace(/[ \t]+$/, "");
  const rest = raw.slice(lineEnd);
  return `${first}${first ? " " : ""}${tagRef(tag)}${rest}`;
}

export function fieldIdsForBlocks(ids: readonly string[], opts: { includePage?: boolean } = {}): FieldId[] {
  const out: FieldId[] = [];
  const props: FieldId[] = [];
  const propSeen = new Set<string>();
  let hasState = false;
  let hasPriority = false;
  let hasScheduled = false;
  let hasDeadline = false;
  let hasTags = false;

  for (const id of ids) {
    const f = facetsForBlock(id);
    if (!f) continue;
    hasState ||= !!f.marker;
    hasPriority ||= !!f.priority;
    hasScheduled ||= !!f.scheduled;
    hasDeadline ||= !!f.deadline;
    hasTags ||= f.tags.length > 0;
    for (const [key] of f.properties) {
      if (isRenderHiddenProp(key)) continue;
      const field: FieldId = `prop:${key}`;
      if (!propSeen.has(field)) {
        propSeen.add(field);
        props.push(field);
      }
    }
  }

  if (hasState) out.push("state");
  if (hasPriority) out.push("priority");
  if (hasScheduled) out.push("scheduled");
  if (hasDeadline) out.push("deadline");
  if (hasTags) out.push("tags");
  out.push(...props);
  if (opts.includePage) out.push("page");
  return out;
}

export function boardGroupByOptions(ownerId: string): FieldId[] {
  const out: FieldId[] = ["state", "priority", "tags"];
  const seen = new Set<FieldId>(out);
  for (const field of fieldIdsForBlocks(doc.byId[ownerId]?.children ?? [])) {
    if (!field.startsWith("prop:") || seen.has(field)) continue;
    seen.add(field);
    out.push(field);
  }
  return out;
}

export function fieldLabel(field: FieldId): string {
  if (field.startsWith("prop:")) return field.slice(5);
  if (isFormulaField(field)) return field.slice("formula:".length);
  if (field === "state") return "State";
  if (field === "priority") return "Priority";
  if (field === "scheduled") return "Scheduled";
  if (field === "deadline") return "Deadline";
  if (field === "tags") return "Tags";
  return "Page";
}

export function readField(id: string, field: FieldId): FieldValue | null {
  if (isFormulaField(field)) return null;
  const n = doc.byId[id];
  const f = facetsForBlock(id);
  if (!n || !f) return null;
  switch (field) {
    case "state":
      return f.marker ? { text: f.marker, raw: f.marker } : null;
    case "priority":
      return f.priority ? { text: `[#${f.priority}]`, raw: f.priority } : null;
    case "scheduled":
      return f.scheduled ? { text: f.scheduled, raw: f.scheduled } : null;
    case "deadline":
      return f.deadline ? { text: f.deadline, raw: f.deadline } : null;
    case "tags":
      return f.tags.length ? { text: f.tags.map((t) => `#${t}`).join(" "), raw: f.tags.join(" ") } : null;
    case "page":
      return { text: n.page, raw: n.page };
    default: {
      const key = field.slice(5);
      const found = f.properties.find(([k]) => k === key);
      return found ? { text: found[1], raw: found[1] } : null;
    }
  }
}


export function writeField(id: string, field: FieldId, value: string): boolean {
  if (isFormulaField(field)) return false;
  const n = doc.byId[id];
  if (!n) return false;
  if (blockPageReadOnly(id)) return false; // org round-trip gate (review finding)
  const trimmed = value.trim();

  if (field === "state") {
    const target = trimmed && MARKERS.includes(trimmed as (typeof MARKERS)[number]) ? trimmed : null;
    const cur = leadingMarker(n.raw);
    let raw: string;
    if (target && target === nextMarker(cur, workflow())) {
      raw = cycleMarkerSmart(n.raw, workflow(), {
        format: formatForBlock(id),
        enabled: timetrackingEnabled(),
        withSeconds: logbookWithSecondSupport(),
      }).raw;
      // cycleMarkerSmart already baked the LOGBOOK/timetracking transition in --
      // letting setRaw apply it AGAIN wrote a duplicate (never-closed) CLOCK
      // entry on every kanban card move (review finding, validated).
      if (raw !== n.raw) setRaw(id, raw, { timetracking: false });
    } else {
      // Direct set (non-adjacent state): setRaw's own applyMarkerTransition is
      // the ONE place the clock transition applies.
      raw = setMarker(n.raw, target);
      if (raw !== n.raw) setRaw(id, raw);
    }
    return true;
  }

  if (field === "priority") {
    setRaw(id, setPriorityRaw(n.raw, trimmed === "A" || trimmed === "B" || trimmed === "C" ? trimmed : null), {
      timetracking: false,
    });
    return true;
  }

  if (field === "scheduled" || field === "deadline") {
    const date = trimmed ? parseIsoDateLike(trimmed) : null;
    if (trimmed && !date) return false;
    setSchedule(id, field, date);
    return true;
  }

  if (field.startsWith("prop:")) {
    setBlockProperty(id, field.slice(5), trimmed ? trimmed : null);
    return true;
  }

  return false;
}

export function writeTagDelta(id: string, delta: { add?: string; remove?: string }): boolean {
  const n = doc.byId[id];
  if (!n) return false;
  if (blockPageReadOnly(id)) return false;
  if (formatForBlock(id) !== "md") return false;

  const remove = delta.remove?.trim() || undefined;
  const add = delta.add?.trim() || undefined;
  if (!remove && !add) return false;

  let raw = n.raw;
  if (remove) {
    // The first line may carry the same tag more than once — cut until gone
    // (a single cut would return true while the block still carries the tag;
    // Phase-6 review finding, validated).
    let next = removeTagFromRaw(raw, remove);
    if (next == null) return false;
    while (next != null) {
      raw = next;
      next = removeTagFromRaw(raw, remove);
    }
  }

  if (add && !tagSetHas(facetsOf(raw, "md"), add)) raw = addTagToRaw(raw, add);

  if (raw !== n.raw) {
    withUndoUnit("sheet:tag-move", [n.page], () => {
      setRaw(id, raw, { timetracking: false });
    });
  }
  return true;
}

export function cycleField(id: string, field: "state" | "priority"): boolean {
  if (blockPageReadOnly(id)) return false; // org round-trip gate
  const n = doc.byId[id];
  if (!n) return false;
  if (field === "state") {
    const raw = cycleMarkerSmart(n.raw, workflow(), {
      format: formatForBlock(id),
      enabled: timetrackingEnabled(),
      withSeconds: logbookWithSecondSupport(),
    }).raw;
    setRaw(id, raw, { timetracking: false });
    return true;
  }
  const cur = facetsForBlock(id)?.priority ?? null;
  const next = cur === "A" ? "B" : cur === "B" ? "C" : cur === "C" ? null : "A";
  return writeField(id, "priority", next ?? "");
}

export function groupKeyForBlock(id: string, field: FieldId): string | null {
  if (isFormulaField(field)) return null;
  const v = readField(id, field);
  if (!v) return null;
  if (field === "priority" || field === "state") return v.raw ?? v.text;
  if (field.startsWith("prop:")) return v.raw ?? v.text;
  return v.text || null;
}

export function groupKeysForBlock(input: GroupKeyInput, field: FieldId, opts: GroupKeysOptions = {}): (string | null)[] {
  if (isFormulaField(field)) {
    const formulas = opts.formulas;
    if (!formulas) return [null];
    const id = typeof input === "string" ? input : input.id;
    const page = typeof input === "string" ? doc.byId[id]?.page ?? "" : input.page ?? doc.byId[id]?.page ?? "";
    const value = evaluateFormulaForRow(
      { id, page, dto: typeof input === "string" ? undefined : input.dto },
      field.slice("formula:".length),
      formulas,
      opts.now ?? new Date()
    );
    if (value.kind === "error") return ["(error)"];
    if (value.kind === "null") return [null];
    return [formulaValueText(value) || null];
  }
  if (field === "tags") {
    const f = facetsForInput(input);
    return f && f.tags.length > 0 ? f.tags : [null];
  }

  const id = typeof input === "string" ? input : input.id;
  if (doc.byId[id]) return [groupKeyForBlock(id, field)];

  const f = facetsForInput(input);
  if (!f) return [null];
  if (field === "state") return [f.marker];
  if (field === "priority") return [f.priority];
  if (field === "scheduled") return [f.scheduled];
  if (field === "deadline") return [f.deadline];
  if (field === "page") return [typeof input === "string" ? null : input.page ?? null];
  const key = field.slice(5);
  return [f.properties.find(([k]) => k === key)?.[1] ?? null];
}

function setPriorityRaw(raw: string, level: "A" | "B" | "C" | null): string {
  const lines = raw.split("\n");
  const first = lines[0] ?? "";
  const m = MARKER_RE.exec(first);
  const markerEnd = m ? m[0].length : 0;
  const head = first.slice(0, markerEnd);
  let rest = first.slice(markerEnd).replace(/^\s+/, "");
  rest = rest.replace(/^\[#[ABC]\]\s*/, "");
  const prefix = head ? `${head} ` : "";
  lines[0] = level ? (rest ? `${prefix}[#${level}] ${rest}` : `${prefix}[#${level}]`) : `${prefix}${rest}`;
  return lines.join("\n");
}
