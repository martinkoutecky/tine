import {
  applyPageMutationPlan,
  createPageMutationPlan,
  doc,
  formatForBlock,
  blockPageReadOnly,
  type PageMutationDraft,
} from "../store";
import { visibleBody } from "../render/block";
import { MARKERS } from "../markers";
import {
  fieldIdsForBlocks,
  groupKeyForBlock,
  isFieldId,
  readField,
  writeGroupingFieldToDraft,
  type FieldId,
} from "./fields";
import { sheetConfigFromRaw } from "./config";

const NONE_LABEL = "(none)";

type WritableGroupField = "state" | "priority" | `prop:${string}`;

interface GroupBucket {
  key: string | null;
  label: string;
  rows: string[];
}

interface FlattenGroup {
  id: string;
  label: string;
  rows: string[];
}

function groupLabel(field: FieldId, key: string | null, sampleId: string): string {
  if (key === null) return NONE_LABEL;
  if (field === "priority") return `[#${key}]`;
  return readField(sampleId, field)?.text ?? key;
}

function writable(field: FieldId): field is WritableGroupField {
  return field === "state" || field === "priority" || field.startsWith("prop:");
}

function firstVisibleLine(id: string): string {
  return (visibleBody(doc.byId[id]?.raw ?? "")[0] ?? "").trim();
}

function parseLabel(field: WritableGroupField, label: string): string | null | undefined {
  const text = label.trim();
  if (text === NONE_LABEL) return null;
  if (field === "state") return MARKERS.includes(text as (typeof MARKERS)[number]) ? text : undefined;
  if (field === "priority") {
    const m = /^\[#([ABC])\]$/.exec(text) ?? /^([ABC])$/.exec(text);
    return m ? m[1] : undefined;
  }
  return text;
}

function candidateFields(parentId: string, groups: readonly FlattenGroup[], childless: readonly string[]): WritableGroupField[] {
  const out: WritableGroupField[] = [];
  const add = (field: FieldId | null | undefined) => {
    if (field && writable(field) && !out.includes(field)) out.push(field);
  };
  const parent = doc.byId[parentId];
  if (parent) {
    const configured = sheetConfigFromRaw(parent.raw, formatForBlock(parentId)).groupBy;
    if (configured && isFieldId(configured)) add(configured);
  }
  for (const field of fieldIdsForBlocks([...childless, ...groups.flatMap((g) => g.rows)])) add(field);
  if (groups.some((g) => parseLabel("state", g.label) !== undefined)) add("state");
  if (groups.some((g) => parseLabel("priority", g.label) !== undefined)) add("priority");
  return out;
}

function inferFlattenField(parentId: string, groups: readonly FlattenGroup[], childless: readonly string[]): WritableGroupField | null {
  for (const field of candidateFields(parentId, groups, childless)) {
    let sawExisting = false;
    let sawValueLabel = false;
    let valid = true;
    for (const group of groups) {
      const parsed = parseLabel(field, group.label);
      if (parsed === undefined) {
        valid = false;
        break;
      }
      if (parsed !== null) sawValueLabel = true;
      for (const row of group.rows) {
        const existing = groupKeyForBlock(row, field);
        if (existing !== null) {
          sawExisting = true;
          if (existing !== parsed) {
            valid = false;
            break;
          }
        }
      }
      if (!valid) break;
    }
    const configured = doc.byId[parentId]
      ? sheetConfigFromRaw(doc.byId[parentId].raw, formatForBlock(parentId)).groupBy === field
      : false;
    if (valid && (sawExisting || configured || (field !== "state" && field !== "priority" ? false : sawValueLabel))) return field;
  }
  return null;
}

function runRestructurePlan(
  page: string,
  tag: string,
  build: (draft: PageMutationDraft) => boolean | null,
): boolean {
  const plan = createPageMutationPlan(page, tag, build);
  if (!plan) return false;
  return applyPageMutationPlan(plan).kind !== "refused";
}

export function canFlatten(parentId: string): boolean {
  return (doc.byId[parentId]?.children ?? []).some((id) => (doc.byId[id]?.children.length ?? 0) > 0);
}

export function hierarchify(parentId: string, field: FieldId): boolean {
  if (blockPageReadOnly(parentId)) return false; // org round-trip gate (review finding)
  const parent = doc.byId[parentId];
  if (!parent || !parent.children.length) return false;
  const buckets: GroupBucket[] = [];
  const byKey = new Map<string, GroupBucket>();
  for (const row of parent.children) {
    if (!doc.byId[row] || doc.byId[row].page !== parent.page) return false;
    const key = groupKeyForBlock(row, field);
    const mapKey = key ?? "\0";
    let bucket = byKey.get(mapKey);
    if (!bucket) {
      bucket = { key, label: groupLabel(field, key, row), rows: [] };
      byKey.set(mapKey, bucket);
      buckets.push(bucket);
    }
    bucket.rows.push(row);
  }
  if (!buckets.length) return false;

  return runRestructurePlan(parent.page, "sheet:hierarchify", (draft) => {
    const groupIds: string[] = [];
    for (const bucket of buckets) {
      const groupId = draft.createChild(parentId, draft.node(parentId)?.children.length ?? -1, bucket.label);
      if (!groupId) return null;
      groupIds.push(groupId);
      if (!draft.replaceChildren(groupId, bucket.rows)) return null;
    }
    return draft.replaceChildren(parentId, groupIds) ? true : null;
  });
}

export function flatten(parentId: string): boolean {
  if (blockPageReadOnly(parentId)) return false; // org round-trip gate (review finding)
  const parent = doc.byId[parentId];
  if (!parent || !parent.children.length) return false;

  const groups: FlattenGroup[] = [];
  const childless: string[] = [];
  const nextParentOrder: string[] = [];
  for (const childId of parent.children) {
    const child = doc.byId[childId];
    if (!child || child.page !== parent.page) return false;
    if (!child.children.length) {
      childless.push(childId);
      nextParentOrder.push(childId);
      continue;
    }
    const rows = [...child.children];
    groups.push({ id: childId, label: firstVisibleLine(childId), rows });
    nextParentOrder.push(...rows);
  }
  if (!groups.length) return false;

  const field = inferFlattenField(parentId, groups, childless);
  const missingValues = field
    ? groups.flatMap((group) => {
        const value = parseLabel(field, group.label);
        if (value == null) return [];
        return group.rows.filter((row) => !readField(row, field)).map((row) => [row, value] as const);
      })
    : [];

  return runRestructurePlan(parent.page, "sheet:flatten", (draft) => {
    if (field) {
      for (const [row, value] of missingValues) {
        if (!writeGroupingFieldToDraft(draft, row, field, value)) return null;
      }
    }
    // Empty each group while it is still attached, then delete the now-empty
    // group. This keeps every effect independently replayable; moved rows are
    // finally reparented and ordered by the parent replacement below.
    for (const group of groups) {
      if (!draft.replaceChildren(group.id, [])) return null;
      if (!draft.deleteSubtree(group.id)) return null;
    }
    return draft.replaceChildren(parentId, nextParentOrder) ? true : null;
  });
}
