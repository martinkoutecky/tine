import { afterEach, beforeAll, beforeEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { Block } from "./Block";
import { initParser, blockRegions } from "../render/parse";
import { loadSingle } from "../document/workingSet";
import { node, resetStore, setRaw, undo, flushPage } from "../document";
import { createComment, submitReply, setThreadResolved, commentAndEdit } from "../commentActions";
import { endEdit, startEditing } from "../editorController";
import { bindTestHost } from "../document/host/wiring.test.support";
import { answerOpensFromDocument } from "../document/host/documentHost.test.support";
import { resetCommentAuthorNameForTests } from "../commentIdentity";
import { persistCommentMarginHidden } from "../commentMarginPref";
import { backend } from "../backend";
import type { BlockDto, Format } from "../types";
import { CommentSurface } from "./CommentSurface";
import { quotedSourceRanges } from "../render/quoteHighlight";
import { descendantOpenThreads } from "../commentGroups";
import { editorIn } from "../tests/editorDriver";
import { setActiveLocale } from "../i18n";
import { pageByName, setDoc } from "../document/model";
import { utf8ByteLength } from "../render/spans";

let dispose: (() => void) | undefined;
beforeAll(async () => {
  await initParser();
  if (!Range.prototype.getClientRects) Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
});
beforeEach(async () => {
  const host = await bindTestHost(); answerOpensFromDocument(host);
  vi.spyOn(backend(), "resolveBlocks").mockImplementation(async ids => ids.map(() => null));
  resetCommentAuthorNameForTests(""); persistCommentMarginHidden(false);
});
afterEach(async () => { endEdit("page-navigation"); dispose?.(); resetStore(); document.body.replaceChildren(); vi.restoreAllMocks(); await setActiveLocale("en"); });
function b(id: string, raw: string, children: BlockDto[] = [], format: Format = "md"): BlockDto {
  return { id, raw, children, collapsed: false, properties: blockRegions(raw, format).properties.filter(p => p.primary).map(p => [p.key, p.value]) };
}
function mount(format: Format = "md") {
  const raw = format === "md" ? "Comment\ntine.quote:: words" : "Comment\n:PROPERTIES:\n:tine.quote: words\n:END:";
  loadSingle({ name: "Dates", title: "Dates", id: `/graph/pages/Dates.${format}`, rev: "fixture", kind: "page", format, pre_block: null,
    blocks: [b("p", "some words", [b("c", raw, [b("r", "Legacy reply")], format)], format)] });
  const el = document.createElement("div"); document.body.append(el);
  vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockImplementation(function (this: HTMLElement) { return this.classList.contains("comment-surface") ? 1400 : 0; });
  dispose = render(() => <CommentSurface page={pageByName("Dates")}><Block id="p" /></CommentSurface>, el);
  return el;
}
it("offers a root-only Resolve action for the whole thread", () => {
  const el = mount();
  const resolve = el.querySelector<HTMLButtonElement>('[data-block-id="c"] > .comment-header .comment-resolve');
  expect(resolve).not.toBeNull();
  resolve!.click();
  expect(node("c")!.raw).toContain("tine.resolved:: true");
  expect(node("r")!.raw).toBe("Legacy reply");
  expect(el.querySelector('[data-block-id="c"]')).toBeNull();
});
it.each(["md", "org"] as const)("%s comments and unnamed replies get UTC minute creation dates", format => {
  mount(format);
  const created = createComment("p", "some words", 0, 4)!;
  const reply = submitReply(created, "New reply")!;
  for (const id of [created, reply]) {
    const date = blockRegions(node(id)!.raw, format).properties.find(p => p.key.toLowerCase() === "tine.date")?.value;
    expect(date).toMatch(/^\d{4}-\d\d-\d\dT\d\d:\d\dZ$/);
  }
  // I-25: an unnamed Org reply also needs its first property drawer (19 B).
  expect(utf8ByteLength(node(reply)!.raw) - utf8ByteLength("New reply")).toBe(format === "md" ? 30 : 49);
  expect(node("p")!.raw).toBe("some words");
});

it.each(["md", "org"] as const)("%s reveals greyed threads, reopens by reply, and undoes the complete reply and resolution", format => {
  const el = mount(format);
  const original = node("c")!.raw;
  expect(quotedSourceRanges("p", format)).toHaveLength(1);
  expect(el.querySelector('[data-block-id="r"] .comment-resolve')).toBeNull();
  setThreadResolved("r", true);
  expect(node("c")!.raw).toContain(format === "md" ? "tine.resolved:: true" : ":tine.resolved: true");
  expect(node("r")!.raw).toBe("Legacy reply");
  expect(quotedSourceRanges("p", format)).toEqual([]);
  expect(el.querySelector(".comment-quote-anchor")).toBeNull();
  const reveal = el.querySelector<HTMLButtonElement>(".comment-resolved-toggle")!;
  expect(reveal.textContent).toBe("1 resolved"); reveal.click();
  expect(el.querySelector('[data-block-id="c"]')?.classList.contains("comment-resolved")).toBe(true);
  expect(el.querySelector('[data-block-id="c"] .comment-resolve')?.textContent).toBe("Reopen");
  expect(el.querySelector('[data-block-id="c"] .block-properties')?.textContent ?? "").not.toContain("tine.resolved");
  expect(quotedSourceRanges("p", format)).toEqual([]);
  const reply = submitReply("r", "Reopening reply")!;
  expect(node("c")!.raw).not.toMatch(/tine.resolved/);
  expect(node(reply)!.parent).toBe("c");
  expect(el.querySelector(".comment-resolved-toggle")).toBeNull();
  expect(quotedSourceRanges("p", format)).toHaveLength(1);
  expect(undo()).toBe(true);
  expect(node(reply)).toBeUndefined();
  expect(node("c")!.raw).toContain("tine.resolved");
  expect(el.querySelector(".comment-resolved-toggle")?.textContent).toBe("1 resolved");
  expect(undo()).toBe(true);
  expect(node("c")!.raw).toBe(original);
});

// Review round 3 (AR-15, UI-4, TE-4): the scope's thread roots follow raw
// property deltas, not only structure. Each transition below is a raw edit of a
// block the scope did not already list as a root.
it("the resolved control follows blocks gaining and losing a quote and roots becoming replies", () => {
  loadSingle({ name: "Moves", title: "Moves", id: "/graph/pages/Moves.md", rev: "fixture", kind: "page", format: "md", pre_block: null,
    blocks: [b("p", "some words here", [
      b("c", "Comment\ntine.quote:: words"),
      b("o", "Ordinary child"),
      b("a", "Aside", [b("n", "Note\ntine.quote:: Aside\ntine.resolved:: true")]),
    ])] });
  const el = document.createElement("div"); document.body.append(el);
  vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockImplementation(function (this: HTMLElement) { return this.classList.contains("comment-surface") ? 1400 : 0; });
  dispose = render(() => <CommentSurface page={pageByName("Moves")}><Block id="p" /></CommentSurface>, el);
  const toggle = () => el.querySelector<HTMLButtonElement>(".comment-resolved-toggle")?.textContent ?? null;
  expect(toggle()).toBe("1 resolved");
  // A root becomes a reply: its own flag no longer closes anything.
  setRaw("a", "Aside\ntine.quote:: here", { timetracking: false });
  expect(toggle()).toBeNull();
  setRaw("a", "Aside", { timetracking: false });
  expect(toggle()).toBe("1 resolved");
  // A resolved root loses its quote: it is an ordinary block again.
  setRaw("n", "Note\ntine.resolved:: true", { timetracking: false });
  expect(toggle()).toBeNull();
  // The reviewer's case: an existing ordinary child becomes a comment, then Resolve.
  setRaw("o", "Ordinary child\ntine.quote:: some", { timetracking: false });
  expect(el.querySelector('.margin-column [data-block-id="o"]')?.classList.contains("comment-card")).toBe(true);
  setThreadResolved("o", true);
  expect(el.querySelector('[data-block-id="o"]')).toBeNull();
  expect(toggle(), "the resolved thread keeps its way back").toBe("1 resolved");
  el.querySelector<HTMLButtonElement>(".comment-resolved-toggle")!.click();
  expect(el.querySelector('[data-block-id="o"]')?.classList.contains("comment-resolved")).toBe(true);
});

it("resolved descendants are excluded from below counts", () => {
  // A body ancestor's below count includes this thread once, never its replies.
  loadSingle({ name: "Ancestor", title: "Ancestor", id: "/graph/pages/Ancestor.md", rev: "fixture", kind: "page", format: "md", pre_block: null,
    blocks: [b("ancestor", "Parent", [b("body", "Body", [b("below", "Hidden\ntine.quote:: Body", [b("deep", "Deep reply")])])])] });
  expect(descendantOpenThreads("ancestor", "md")).toBe(1);
  setThreadResolved("below", true);
  expect(descendantOpenThreads("ancestor", "md")).toBe(0);
});

it("commenting again on a closed passage reveals its composer without creating or reopening a thread", async () => {
  const el = mount(); setThreadResolved("c", true);
  expect(commentAndEdit("p", "some words", 5, 10)).toBe("c");
  await Promise.resolve();
  expect(node("p")!.children).toEqual(["c"]);
  expect(node("c")!.raw).toContain("tine.resolved:: true");
  expect(document.activeElement).toBe(el.querySelector(".comment-reply-draft"));
});

it.each(["md", "org"] as const)("%s date stays hidden and byte-exact through editing a dated comment and reply", async format => {
  const el = mount(format);
  const root = createComment("p", "some words", 0, 4)!;
  const reply = submitReply(root, "Reply")!;
  for (const id of [root, reply]) {
    const dateBefore = blockRegions(node(id)!.raw, format).properties.find(p => p.key === "tine.date")!.value;
    startEditing(id, 0); await Promise.resolve();
    const editor = editorIn(el);
    expect(editor.value()).not.toContain("tine."); editor.input("Revised words"); endEdit("page-navigation");
    expect(blockRegions(node(id)!.raw, format).properties.find(p => p.key === "tine.date")!.value).toBe(dateBefore);
    const row = el.querySelector(`[data-block-id="${id}"]`)!;
    expect(row.querySelector(".comment-date")?.getAttribute("datetime")).toBe(dateBefore);
    expect(row.querySelector(".block-properties")?.textContent ?? "").not.toContain("tine.date");
  }
});

it("localized dates never sort messages; invalid and absent dates remain quiet", async () => {
  const el = mount();
  expect(el.querySelectorAll(".comment-date")).toHaveLength(0);
  setRaw("c", node("c")!.raw + "\ntine.date:: 2026-11-07T13:37Z");
  setRaw("r", "Legacy reply\ntine.date:: 2020-01-02T03:04Z");
  const dates = () => [...el.querySelectorAll<HTMLTimeElement>(".comment-date")];
  expect(dates().map(date => date.dateTime)).toEqual(["2026-11-07T13:37Z", "2020-01-02T03:04Z"]);
  const english = dates()[0].textContent;
  await setActiveLocale("zh-CN");
  expect(dates()[0].textContent).not.toBe(english);
  expect(dates()[0].textContent).toBe(new Intl.DateTimeFormat("zh-CN", { dateStyle: "short", timeStyle: "short" }).format(new Date("2026-11-07T13:37Z")));
  setRaw("r", "Legacy reply\ntine.date:: 2026-02-30T00:00Z");
  expect(dates()).toHaveLength(1);
});

it("explicit Reopen removes just the root flag and Undo restores it", () => {
  const el = mount(); setThreadResolved("c", true);
  const closed = node("c")!.raw;
  el.querySelector<HTMLButtonElement>(".comment-resolved-toggle")!.click();
  el.querySelector<HTMLButtonElement>(".comment-resolve")!.click();
  expect(node("c")!.raw).toBe("Comment\ntine.quote:: words");
  expect(node("c")!.children).toEqual(["r"]);
  expect(undo()).toBe(true); expect(node("c")!.raw).toBe(closed);
});

it.each(["md", "org"] as const)("%s quoted-reply creation reopens only the outer root in its undo unit", format => {
  mount(format); setThreadResolved("c", true);
  const closed = node("c")!.raw;
  const reply = createComment("r", "Legacy reply", 0, 6)!;
  expect(node(reply)!.parent).toBe("c");
  expect(node(reply)!.raw).toContain("tine.date");
  expect(node("c")!.raw).not.toContain("tine.resolved");
  expect(node("r")!.children).toEqual([]);
  expect(undo()).toBe(true); expect(node(reply)).toBeUndefined(); expect(node("c")!.raw).toBe(closed);
});

it("Resolve and closing revealed cards preserve unsent drafts", () => {
  const el = mount();
  const draft = el.querySelector<HTMLTextAreaElement>(".comment-reply-draft")!;
  draft.value = "Unsent words"; draft.dispatchEvent(new Event("input", { bubbles: true }));
  el.querySelector<HTMLButtonElement>(".comment-resolve")!.click();
  expect(node("c")!.raw).not.toContain("tine.resolved");
  expect(el.querySelector(".comment-reply-draft")).toBe(draft);
  draft.value = ""; draft.dispatchEvent(new Event("input", { bubbles: true }));
  setThreadResolved("c", true);
  const reveal = el.querySelector<HTMLButtonElement>(".comment-resolved-toggle")!; reveal.click();
  const revealedDraft = el.querySelector<HTMLTextAreaElement>(".comment-reply-draft")!;
  revealedDraft.value = "Keep this too"; revealedDraft.dispatchEvent(new Event("input", { bubbles: true }));
  reveal.click();
  expect(reveal.getAttribute("aria-pressed")).toBe("true"); expect(revealedDraft.isConnected).toBe(true);
});

it("read-only pages expose no Resolve action and refuse mutation, reply and creation", () => {
  const el = mount();
  setDoc("pages", 0, "readOnly", true);
  expect(el.querySelector(".comment-resolve")).toBeNull();
  const original = node("c")!.raw;
  setThreadResolved("c", true);
  expect(node("c")!.raw).toBe(original);
  expect(submitReply("c", "Refused")).toBeNull(); expect(createComment("p", "some words", 0, 4)).toBeNull();
});

it.each((["md", "org"] as const).flatMap(format => [1, 60].map(bodyBlocks => ({ format, bodyBlocks }))))("I-25 measures $format save requests on $bodyBlocks body blocks", async ({ format, bodyBlocks }) => {
  const quote = format === "md" ? "Comment\ntine.quote:: words" : "Comment\n:PROPERTIES:\n:tine.quote: words\n:END:";
  loadSingle({ name: "Dates", title: "Dates", id: `/graph/pages/Dates.${format}`, rev: "fixture", kind: "page", format, pre_block: null,
    blocks: [b("p", "some words", [b("c", quote, [], format)], format), ...Array.from({ length: bodyBlocks - 1 }, (_, i) => b(`f${i}`, `Filler ${i}`, [], format))] });
  const submit = vi.spyOn(backend(), "pageSubmit");
  const bytes = () => {
    const [session, id, key, dto, version, resolve, kinds] = submit.mock.calls.at(-1)!;
    return utf8ByteLength(JSON.stringify({ session, id, key, dto, version, resolve, kinds }));
  };
  setThreadResolved("c", true); expect(await flushPage("Dates")).toBe(true);
  expect(submit).toHaveBeenCalledOnce(); const resolveRequestBytes = bytes(); submit.mockClear();
  expect(submitReply("c", "Cost reply")).not.toBeNull(); expect(await flushPage("Dates")).toBe(true);
  expect(submit).toHaveBeenCalledOnce();
  console.log(JSON.stringify({ measurement: "MC010 local page_submit JSON payload, excluding Tauri framing", format, bodyBlocks,
    resolveRequests: 1, resolveRequestBytes, replyAndReopenRequests: 1, replyAndReopenRequestBytes: bytes(), featureSpecificTransportRecords: 0 }));
});
