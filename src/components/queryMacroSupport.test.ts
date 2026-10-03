// GH #619 item 9: a "Pages and blocks" family that the engine counted past what it returned (a `sample`)
// must say so. `resultTruncated` is the one place that decides it.
import { describe, expect, it } from "vitest";
import { resultTruncated } from "./queryMacroSupport";
import type { QueryResult } from "../editor/queryIr";

const pages = (n: number, matched?: number): QueryResult => ({
  anchor: "page",
  pages: Array.from({ length: n }, (_, i) => ({ path: `pages/P${i}.md`, name: `P${i}`, kind: "page" as const, properties: [] })),
  diagnostics: [], report: { ran: [], ignored: [], supported: true }, total: n, matched_total: matched, exceeded: false,
});
const blocks = (n: number, matched?: number): QueryResult => ({
  anchor: "block",
  groups: [{ page: "A", kind: "page", blocks: Array.from({ length: n }, (_, i) => ({ id: `b${i}`, raw: `b${i}`, collapsed: false, children: [] })) }],
  diagnostics: [], report: { ran: [], ignored: [], supported: true }, total: n, matched_total: matched, exceeded: false,
} as unknown as QueryResult);

describe("resultTruncated", () => {
  it("is true when the engine counted more pages than it returned", () => {
    expect(resultTruncated(pages(3, 10))).toBe(true);
  });
  it("is true when the engine counted more blocks than it returned", () => {
    expect(resultTruncated(blocks(2, 5))).toBe(true);
  });
  it("is false for a complete answer", () => {
    expect(resultTruncated(pages(3, 3))).toBe(false);
    expect(resultTruncated(blocks(2, 2))).toBe(false);
  });
  it("is false when the engine reports no count", () => {
    expect(resultTruncated(pages(3))).toBe(false);
  });
});
