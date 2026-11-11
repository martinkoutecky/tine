// "Your name" (D-20261017-LD1): comments and replies the user writes carry
// `tine.author:: <name>` when a name is set; unset keeps today's behaviour (no author
// written, rendered "you"); an author equal to the name renders as "you".
import { openCommentThread, showThread } from "./tests/commentEntry";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { For, type JSX } from "solid-js";
import { readFileSync } from "node:fs";
import { render } from "solid-js/web";
import { Block } from "./components/Block";
import { CommentNameSetting } from "./components/CommentNameSetting";
import { backend } from "./backend";
import { commentRaw, replyRaw } from "./comments";
import { createComment, submitReply } from "./commentActions";
import { AUTHOR_TINTS, authorTint, commentAuthorName, isOwnAuthor, normalizeAuthorName, resetCommentAuthorNameForTests, initCommentIdentity } from "./commentIdentity";
import { blockRegions, initParser, editBlock } from "./render/parse";
import { loadSingle } from "./document/workingSet";
import { doc } from "./document/model";
import { pageByName, resetStore, setBlockProperty, undo } from "./document";
import { setGraphMeta } from "./graphSession";
import { endEdit } from "./editorController";
import type { BlockDto } from "./types";
import { bindTestHost } from "./document/host/wiring.test.support";
import { setAuthorMarksHidden } from "./authorMarksPref";

const PAGE = "CommentIdentity";
let dispose: (() => void) | undefined;

function block(id: string, raw: string, children: BlockDto[] = []): BlockDto {
  const properties = blockRegions(raw, "md").properties.filter((p) => p.primary).map((p): [string, string] => [p.key, p.value]);
  return { id, raw, collapsed: false, children, ...(properties.length ? { properties } : {}) };
}

function mount(blocks: BlockDto[], extra?: () => JSX.Element, format: "md" | "org" = "md"): HTMLElement {
  loadSingle({ name: PAGE, kind: "page", title: PAGE, format, pre_block: null, blocks });
  setGraphMeta({ root: "/fixture" } as any);
  const root = document.createElement("div");
  document.body.append(root);
  dispose = render((): JSX.Element => (
    <>
      <For each={pageByName(PAGE)?.roots ?? []}>{(id) => <Block id={id} />}</For>
      {extra?.()}
    </>
  ), root);
  openCommentThread(root);
  return document.body;
}

beforeAll(() => initParser());
beforeEach(async () => {
  await bindTestHost();
  vi.spyOn(backend(), "resolveBlocks").mockImplementation(async (ids) => ids.map(() => null));
  resetCommentAuthorNameForTests("");
  setAuthorMarksHidden(false);
});
afterEach(() => {
  dispose?.(); dispose = undefined;
  endEdit("page-navigation");
  resetStore(); setGraphMeta(null); vi.restoreAllMocks();
  resetCommentAuthorNameForTests("");
  document.body.innerHTML = "";
});

describe("the name", () => {
  it("is one trimmed line of at most 80 characters", () => {
    expect(normalizeAuthorName("  Ada \n  Lovelace  ")).toBe("Ada Lovelace");
    expect(normalizeAuthorName("x".repeat(200))).toHaveLength(80);
    expect(normalizeAuthorName("   ")).toBe("");
  });

  it("matches an author ignoring case, and nothing while unset", () => {
    expect(isOwnAuthor("Ada")).toBe(false);
    resetCommentAuthorNameForTests("Ada");
    expect(isOwnAuthor("ada")).toBe(true);
    expect(isOwnAuthor(" ADA ")).toBe(true);
    expect(isOwnAuthor("claude")).toBe(false);
  });

  it("loads from the device settings at startup", async () => {
    vi.spyOn(backend(), "getAppString").mockResolvedValue("Ada");
    initCommentIdentity();
    await vi.waitFor(() => expect(commentAuthorName()).toBe("Ada"));
  });
});

describe("what is written", () => {
  it("writes tine.author:: after the quote lines in both formats, and none when unset", () => {
    const sel = { quote: "a passage", prefix: null, suffix: null };
    expect(commentRaw(sel, "md")).toBe("\ntine.quote:: a passage");
    expect(commentRaw(sel, "md", "Ada")).toBe("\ntine.quote:: a passage\ntine.author:: Ada");
    expect(commentRaw(sel, "org", "Ada")).toBe("\n:PROPERTIES:\n:tine.quote: a passage\n:tine.author: Ada\n:END:");
    expect(replyRaw("md", "Ada")).toBe("\ntine.author:: Ada");
  });

  it("a new comment carries the name as one undo unit; a reply too", () => {
    mount([block("p", "An agent wrote a phrase worth disputing here.")]);
    resetCommentAuthorNameForTests("Ada");
    const comment = createComment("p", "An agent wrote a phrase worth disputing here.", 15, 39)!;
    expect(editBlock(doc.byId[comment].raw, "md", { kind: "property", key: "tine.date", value: null })).toBe("\ntine.quote:: a phrase worth disputing\ntine.quote-prefix:: An agent wrote\ntine.quote-suffix:: here.\ntine.author:: Ada");
    const reply = submitReply(comment, "Response")!;
    expect(editBlock(doc.byId[reply].raw, "md", { kind: "property", key: "tine.date", value: null })).toBe("Response\ntine.author:: Ada");
    undo();
    expect(doc.byId[comment].children).toHaveLength(0);
    undo();
    expect(doc.byId["p"].children).toHaveLength(0);
  });

  it("without a name neither the comment nor the reply carries tine.author::", () => {
    mount([block("p", "An agent wrote a phrase worth disputing here.")]);
    const comment = createComment("p", "An agent wrote a phrase worth disputing here.", 15, 39)!;
    expect(editBlock(doc.byId[comment].raw, "md", { kind: "property", key: "tine.date", value: null })).toBe("\ntine.quote:: a phrase worth disputing\ntine.quote-prefix:: An agent wrote\ntine.quote-suffix:: here.");
    const reply = submitReply(comment, "Response")!;
    expect(editBlock(doc.byId[reply].raw, "md", { kind: "property", key: "tine.date", value: null })).toBe("Response");
  });
});

describe("what is shown", () => {
  it("exposes the exact stored facet value with author marks hidden", () => {
    setAuthorMarksHidden(true);
    const root = mount([{ id: "p", raw: "Words\nTiNe.Author:: Ada  ", collapsed: false, children: [], properties: [["TiNe.Author", "Ada  "]] }]);
    expect(root.querySelector('[data-block-id="p"]')?.getAttribute("data-author")).toBe("Ada  ");
  });
  it.each(["md", "org"] as const)("keeps data-author current through property edits in %s, including empty values", format => {
    const raw = format === "md" ? "Words\ntine.author:: Ada" : "Words\n:PROPERTIES:\n:tine.author: Ada\n:END:";
    const root = mount([{ id: "p", raw, collapsed: false, children: [], properties: [["tine.author", "Ada"]] }], undefined, format);
    const row = root.querySelector('[data-block-id="p"]')!;
    expect(row.getAttribute("data-author")).toBe("Ada");
    setBlockProperty("p", "tine.author", "Bo");
    expect(row.getAttribute("data-author")).toBe("Bo");
    setBlockProperty("p", "tine.author", "");
    expect(row.getAttribute("data-author")).toBe("");
    setBlockProperty("p", "tine.author", null);
    expect(row.hasAttribute("data-author")).toBe(false);
  });
  it("exposes the stored author to custom.css, including your own name, and omits absent authors", () => {
    resetCommentAuthorNameForTests("ada");
    const root = mount([block("p", "A passage\ntine.author:: Ada", [
      block("c", "Mine\ntine.quote:: passage\ntine.author:: Ada"),
      block("d", "Agent\ntine.quote:: A\ntine.author:: claude"),
    ]), block("plain", "Unsigned")]);
    expect(root.querySelector('[data-block-id="p"]')?.getAttribute("data-author")).toBe("Ada");
    expect(showThread("c")?.getAttribute("data-author")).toBe("Ada");
    expect(showThread("d")?.getAttribute("data-author")).toBe("claude");
    expect(root.querySelector('[data-block-id="plain"]')?.hasAttribute("data-author")).toBe(false);
  });

  it("renders every public comment class promised to custom.css", () => {
    const root = mount([block("p", "A passage", [block("c", "Discuss\ntine.quote:: passage\ntine.author:: claude\ntine.date:: 2026-11-07T13:37Z")])]);
    const section = readFileSync("docs/contracts/theme-tokens.md", "utf8").split("## 7. Comment styling")[1]?.split("## Tests")[0] ?? "";
    const selectors = [...section.matchAll(/^\| `([^`]+)`/gm)].map(match => match[1]);
    expect(selectors.length).toBeGreaterThanOrEqual(9);
    for (const selector of selectors) expect(root.querySelector(selector), `${selector} is a public styling hook`).not.toBeNull();
  });
  // Each passage has one thread; only the shown passage is mounted (review round 3).
  const chips = (_root: HTMLElement) => ["c", "d", "e"].map(id => showThread(id)!.querySelector<HTMLElement>(":scope > .comment-header .author-chip")!).map((c) => ({ text: c.textContent, agent: c.classList.contains("agent") }));
  const fixture = () => [block("p", "A passage to discuss.", [
    block("c", "Mine.\ntine.quote:: passage\ntine.author:: Ada"),
    block("d", "Theirs.\ntine.quote:: discuss\ntine.author:: claude"),
    block("e", "Unsigned.\ntine.quote:: A"),
  ])];

  it("shows an author equal to the name as you, and others as before", () => {
    resetCommentAuthorNameForTests("ada");
    const root = mount(fixture());
    expect(chips(root)).toEqual([
      { text: "you", agent: false },
      { text: "claude", agent: true },
      { text: "you", agent: false },
    ]);
  });

  it("with no name set an tine.author:: of your name shows as written", () => {
    const root = mount(fixture());
    expect(chips(root)[0]).toEqual({ text: "Ada", agent: true });
  });
});

describe("Settings: Your name", () => {
  it("saves on blur through the device settings and rolls nothing into the graph", async () => {
    const set = vi.spyOn(backend(), "setAppString").mockResolvedValue(undefined);
    const root = mount([block("p", "x")], () => <CommentNameSetting />);
    const input = root.querySelector<HTMLInputElement>("input.settings-input")!;
    input.value = "  Ada  ";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    expect(set).not.toHaveBeenCalled();
    input.dispatchEvent(new Event("change", { bubbles: true }));
    expect(commentAuthorName()).toBe("Ada");
    await vi.waitFor(() => expect(set).toHaveBeenCalledWith("comment_author_name", "Ada"));
    expect(doc.byId["p"].raw).toBe("x");
  });
});

describe("author tints (MARGIN-AUTHOR, Martin 2026-10-31)", () => {
  it("gives a name one stable tint, ignoring case and spacing", () => {
    for (const name of ["Claude", "Ellis Morrow", "x"]) {
      const tint = authorTint(name);
      expect(tint).toBeGreaterThanOrEqual(0);
      expect(tint).toBeLessThan(AUTHOR_TINTS);
      expect(authorTint(` ${name.toUpperCase()}  `)).toBe(tint);
    }
  });

  it("spreads different names over the tints", () => {
    const names = ["Claude", "Codex", "Ellis", "Martin", "Ana", "Bo", "Chen", "Dita", "Eva", "Filip"];
    expect(new Set(names.map(authorTint)).size).toBeGreaterThan(2);
  });
});
