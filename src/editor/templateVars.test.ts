import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { applyTemplateVars, prepareTemplateVars } from "./templateVars";
import { journalTitle } from "../journal";

describe("applyTemplateVars", () => {
  // Use a local Date so these assertions exercise the same local-clock semantics
  // as journal naming and Logseq's chrono-node path.
  const NOW = new Date(2026, 11, 31, 13, 45, 0);

  beforeAll(() => prepareTemplateVars());

  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(NOW);
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("expands today / yesterday / tomorrow as journal links", () => {
    const today = new Date(NOW);
    expect(applyTemplateVars("see <% today %>")).toBe(`see [[${journalTitle(today)}]]`);
    const y = new Date(NOW);
    y.setDate(y.getDate() - 1);
    expect(applyTemplateVars("<% yesterday %>")).toBe(`[[${journalTitle(y)}]]`);
    const t = new Date(NOW);
    t.setDate(t.getDate() + 1);
    expect(applyTemplateVars("<% tomorrow %>")).toBe(`[[${journalTitle(t)}]]`);
  });

  it("expands current page only when a page is supplied", () => {
    expect(applyTemplateVars("on <% current page %>", "My Page")).toBe("on [[My Page]]");
    // No context → left verbatim rather than producing a broken [[]].
    expect(applyTemplateVars("on <% current page %>")).toBe("on <% current page %>");
  });

  it("expands a date: token via the shared date resolver", () => {
    const t = new Date(NOW);
    t.setDate(t.getDate() + 3);
    expect(applyTemplateVars("<% date: +3d %>")).toBe(`[[${journalTitle(t)}]]`);
    expect(applyTemplateVars("<% date: 2026-07-01 %>")).toBe(`[[${journalTitle(new Date(2026, 6, 1))}]]`);
    // An impossible date is left verbatim, not silently rolled to a wrong day.
    expect(applyTemplateVars("<% date: 2026-02-31 %>")).toBe("<% date: 2026-02-31 %>");
  });

  it("expands time as HH:MM and leaves unknown/empty vars verbatim", () => {
    expect(applyTemplateVars("<% time %>")).toBe("13:45");
    expect(applyTemplateVars("<% wat %>")).toBe("<% wat %>");
    expect(applyTemplateVars("<% date: %>")).toBe("<% date: %>");
  });

  it("expands unmatched expressions through Logseq's natural-language date parser", () => {
    // Thursday, Dec 31st crosses both the month and year boundary for the
    // reported lower-case weekday expression.
    expect(applyTemplateVars("<% next monday %>")).toBe(`[[${journalTitle(new Date(2027, 0, 4))}]]`);
    expect(applyTemplateVars("<% in 5 days %>")).toBe(`[[${journalTitle(new Date(2027, 0, 5))}]]`);
    expect(applyTemplateVars("<% 3 days ago %>")).toBe(`[[${journalTitle(new Date(2026, 11, 28))}]]`);
    expect(applyTemplateVars("<% next tuesday %>")).toBe(`[[${journalTitle(new Date(2027, 0, 5))}]]`);
    expect(applyTemplateVars("<% last friday %>")).toBe(`[[${journalTitle(new Date(2026, 11, 25))}]]`);
  });
});
