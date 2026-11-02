import { describe, expect, it } from "vitest";
import {
  captureDestination, destinationName, shouldRedirect, templateOutline, QUICK_CAPTURE_PAGE,
} from "./quickCaptureConfig";

const ctx = { time: "14:37", date: "Nov 2nd, 2026", format: "md" as const };

describe("captureDestination (OG quick-capture page cond)", () => {
  it("keeps today's behaviour with no options", () => {
    expect(captureDestination("", {})).toEqual({ kind: "journal" });
    expect(captureDestination(undefined, {})).toEqual({ kind: "journal" });
    expect(captureDestination("  ", {})).toEqual({ kind: "journal" });
  });
  it("a typed title always wins", () => {
    const options = { defaultPage: "Inbox", insertToday: true };
    expect(captureDestination(" Idea ", options)).toEqual({ kind: "page", name: "Idea" });
  });
  it("uses the default page when set", () => {
    expect(captureDestination("", { defaultPage: "Inbox" })).toEqual({ kind: "page", name: "Inbox" });
  });
  it("insert-today? goes to the journal ahead of the default page", () => {
    expect(captureDestination("", { defaultPage: "Inbox", insertToday: true })).toEqual({ kind: "journal" });
  });
  it("a blank default page is no default page", () => {
    expect(captureDestination("", { defaultPage: "   " })).toEqual({ kind: "journal" });
    expect(captureDestination("", { defaultPage: null })).toEqual({ kind: "journal" });
  });
  it("with journals off, an untitled capture goes to OG's quick capture page", () => {
    expect(captureDestination("", { enableJournals: false })).toEqual({ kind: "page", name: QUICK_CAPTURE_PAGE });
    expect(captureDestination("", { enableJournals: false, insertToday: true })).toEqual({ kind: "page", name: QUICK_CAPTURE_PAGE });
    expect(captureDestination("", { enableJournals: false, defaultPage: "Inbox" })).toEqual({ kind: "page", name: "Inbox" });
  });
  it("names the destination page", () => {
    expect(destinationName({ kind: "journal" }, "Nov 2nd, 2026")).toBe("Nov 2nd, 2026");
    expect(destinationName({ kind: "page", name: "Inbox" }, "Nov 2nd, 2026")).toBe("Inbox");
  });
});

describe("shouldRedirect", () => {
  it("only when asked and the page is not already open", () => {
    expect(shouldRedirect({}, "Inbox", null)).toBe(false);
    expect(shouldRedirect({ redirectPage: true }, "Inbox", null)).toBe(true);
    expect(shouldRedirect({ redirectPage: true }, "Inbox", "other")).toBe(true);
    expect(shouldRedirect({ redirectPage: true }, "Inbox", "inbox")).toBe(false);
  });
});

describe("templateOutline (OG :quick-capture-templates :text)", () => {
  const one = [{ raw: "Follow up with client", children: [] }];
  it("leaves the capture alone without a usable template", () => {
    for (const textTemplate of [undefined, null, ""]) expect(templateOutline(one, { textTemplate }, ctx)).toBe(one);
    expect(templateOutline(one, { textTemplate: 5 as unknown as string }, ctx)).toBe(one);
    expect(templateOutline([], { textTemplate: "{time} {text}" }, ctx)).toEqual([]);
  });
  it("gives a timestamp through the template (GH #833)", () => {
    expect(templateOutline(one, { textTemplate: "[{time}] {text}" }, ctx)).toEqual([{ raw: "[14:37] Follow up with client", children: [] }]);
    expect(templateOutline(one, { textTemplate: "{date} {time} - {text}" }, ctx)[0].raw).toBe("Nov 2nd, 2026 14:37 - Follow up with client");
  });
  it("expands {url} to nothing and trims the tail", () => {
    expect(templateOutline(one, { textTemplate: "**{time}** {text} {url}" }, ctx)[0].raw).toBe("**14:37** Follow up with client");
  });
  it("does not re-expand placeholders inside the typed text", () => {
    const typed = [{ raw: "literal {time}", children: [] }];
    expect(templateOutline(typed, { textTemplate: "{time}: {text}" }, ctx)[0].raw).toBe("14:37: literal {time}");
  });
  it("shapes the first top-level block only; children and siblings follow", () => {
    const child = { raw: "child", children: [] };
    const nodes = [{ raw: "a", children: [child] }, { raw: "b", children: [] }];
    expect(templateOutline(nodes, { textTemplate: "{time} {text}" }, ctx)).toEqual([
      { raw: "14:37 a", children: [child] }, { raw: "b", children: [] },
    ]);
  });
  it("a template that comes out blank keeps the capture", () => {
    expect(templateOutline(one, { textTemplate: "{url}" }, ctx)).toBe(one);
  });
});
