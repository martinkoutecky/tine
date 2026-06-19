import { describe, it, expect } from "vitest";
import { applyTemplateVars } from "./templateVars";
import { journalTitle } from "../journal";

describe("applyTemplateVars", () => {
  it("expands today / yesterday / tomorrow as journal links", () => {
    const today = new Date();
    expect(applyTemplateVars("see <% today %>")).toBe(`see [[${journalTitle(today)}]]`);
    const y = new Date();
    y.setDate(y.getDate() - 1);
    expect(applyTemplateVars("<% yesterday %>")).toBe(`[[${journalTitle(y)}]]`);
    const t = new Date();
    t.setDate(t.getDate() + 1);
    expect(applyTemplateVars("<% tomorrow %>")).toBe(`[[${journalTitle(t)}]]`);
  });

  it("expands current page only when a page is supplied", () => {
    expect(applyTemplateVars("on <% current page %>", "My Page")).toBe("on [[My Page]]");
    // No context → left verbatim rather than producing a broken [[]].
    expect(applyTemplateVars("on <% current page %>")).toBe("on <% current page %>");
  });

  it("expands a date: token via the shared date resolver", () => {
    const t = new Date();
    t.setDate(t.getDate() + 3);
    expect(applyTemplateVars("<% date: +3d %>")).toBe(`[[${journalTitle(t)}]]`);
    expect(applyTemplateVars("<% date: 2026-07-01 %>")).toBe(`[[${journalTitle(new Date(2026, 6, 1))}]]`);
    // An impossible date is left verbatim, not silently rolled to a wrong day.
    expect(applyTemplateVars("<% date: 2026-02-31 %>")).toBe("<% date: 2026-02-31 %>");
  });

  it("expands time as HH:MM and leaves unknown/empty vars verbatim", () => {
    expect(applyTemplateVars("<% time %>")).toMatch(/^\d{2}:\d{2}$/);
    expect(applyTemplateVars("<% wat %>")).toBe("<% wat %>");
    expect(applyTemplateVars("<% date: %>")).toBe("<% date: %>");
  });
});
