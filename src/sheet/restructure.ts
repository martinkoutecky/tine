import {
  deleteBlock,
  doc,
  formatForBlock,
  insertEmptyChildBlock,
  replaceChildOrders,
  setRaw,
  withUndoUnit,
  blockPageReadOnly,
} from "../store";
import { visibleBody } from "../render/block";
import { MARKERS } from "../markers";
import { fieldIdsForBlocks, groupKeyForBlock, isFieldId, readField, writeField, type FieldId } from "./fields";
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

  return withUndoUnit("sheet:hierarchify", [parent.page], () => {
    const groupIds: string[] = [];
    const nextOrders: Record<string, readonly string[]> = {};
    for (const bucket of buckets) {
      const groupId = insertEmptyChildBlock(parentId, doc.byId[parentId]?.children.length ?? 0);
      if (!groupId) throw new Error("failed to create group block");
      setRaw(groupId, bucket.label, { timetracking: false });
      groupIds.push(groupId);
      nextOrders[groupId] = bucket.rows;
    }
    nextOrders[parentId] = groupIds;
    if (!replaceChildOrders(nextOrders)) throw new Error("failed to reparent grouped rows");
    return true;
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

  return withUndoUnit("sheet:flatten", [parent.page], () => {
    const field = inferFlattenField(parentId, groups, childless);
    if (field) {
      for (const group of groups) {
        const value = parseLabel(field, group.label);
        if (value == null) continue;
        for (const row of group.rows) {
          if (!readField(row, field)) writeField(row, field, value);
        }
      }
    }
    const orders: Record<string, readonly string[]> = { [parentId]: nextParentOrder };
    for (const group of groups) orders[group.id] = [];
    if (!replaceChildOrders(orders)) throw new Error("failed to flatten rows");
    for (const group of groups) deleteBlock(group.id);
    return true;
  });
}
