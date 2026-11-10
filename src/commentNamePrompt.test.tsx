// tine#841 (D-20261101-CN): the first margin comment or reply of a person with
// no name, in a graph under git, asks for "Your name" once. Saving keeps the
// name (all graphs); skipping is remembered (all graphs); the comment is
// written either way; nothing is asked without git, with a name, after a
// decline, or before the facts are known.
import { openCommentThread } from "./tests/commentEntry";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { Block } from "./components/Block";
import { CommentNamePrompt } from "./components/CommentNamePrompt";
import { CommentComposer } from "./components/CommentParts";
import { backend } from "./backend";
import { commentOnRenderedSelection } from "./commentActions";
import { commentAuthorName, resetCommentAuthorNameForTests } from "./commentIdentity";
import { probeGraphGit, resetCommentNamePromptForTests, shouldAskCommentName, initCommentNamePrompt } from "./commentNamePrompt";
import { blockRegions, initParser } from "./render/parse";
import { loadSingle } from "./document/workingSet";
import { doc } from "./document/model";
import { pageByName, resetStore, setRaw } from "./document";
import { setGraphMeta } from "./graphSession";
import { invalidateBinding } from "./binding";
import { endEdit } from "./editorController";
import type { BlockDto } from "./types";
import { bindTestHost } from "./document/host/wiring.test.support";
import { loadLocale, showLocale } from "./i18n";

const PAGE = "CommentNamePrompt";
const TEXT = "An agent wrote a phrase worth disputing here.";
const HELD = { blockId: "p", text: TEXT, start: 15, end: 39 };
let dispose: (() => void) | undefined;
const spyOnSetAppString = () => vi.spyOn(backend(), "setAppString").mockResolvedValue(undefined);
let setApp: ReturnType<typeof spyOnSetAppString>;

function block(id: string, raw: string, children: BlockDto[] = []): BlockDto {
  const properties = blockRegions(raw, "md").properties.filter((p) => p.primary).map((p): [string, string] => [p.key, p.value]);
  return { id, raw, collapsed: false, children, ...(properties.length ? { properties } : {}) };
}

function mount(blocks: BlockDto[], extra?: () => JSX.Element): HTMLElement {
  loadSingle({ name: PAGE, kind: "page", title: PAGE, format: "md", pre_block: null, blocks });
  setGraphMeta({ root: "/fixture" } as any);
  const root = document.createElement("div");
  document.body.append(root);
  dispose = render((): JSX.Element => (
    <>
      <For each={pageByName(PAGE)?.roots ?? []}>{(id) => <Block id={id} />}</For>
      <CommentNamePrompt />
      {extra?.()}
    </>
  ), root);
  openCommentThread(root);
  return document.body;
}

const dialog = () => document.querySelector<HTMLElement>('[role="dialog"].comment-name-prompt');
const buttons = (label: string) => [...document.querySelectorAll<HTMLButtonElement>(".comment-name-prompt button")].find((b) => b.textContent === label)!;
const type = (value: string) => {
  const input = document.querySelector<HTMLInputElement>(".comment-name-prompt input")!;
  input.value = value;
  input.dispatchEvent(new Event("input", { bubbles: true }));
};
const submit = () => document.querySelector<HTMLFormElement>(".comment-name-prompt")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
const children = () => doc.byId["p"].children;
const gitGraph = () => resetCommentNamePromptForTests({ repository: true });

beforeAll(() => initParser());
beforeEach(async () => {
  showLocale("en");
  await bindTestHost();
  vi.spyOn(backend(), "resolveBlocks").mockImplementation(async (ids) => ids.map(() => null));
  setApp = spyOnSetAppString();
  resetCommentAuthorNameForTests("");
  resetCommentNamePromptForTests();
});
afterEach(() => {
  showLocale("en");
  dispose?.(); dispose = undefined;
  endEdit("page-navigation");
  resetStore(); setGraphMeta(null); vi.restoreAllMocks();
  resetCommentAuthorNameForTests("");
  resetCommentNamePromptForTests();
  document.body.innerHTML = "";
});

describe("the first comment in a git graph, no name", () => {
  it("asks about signing without git or GitHub in Simplified Chinese", async () => {
    await loadLocale("zh-CN"); showLocale("zh-CN");
    mount([block("p", TEXT)]); gitGraph(); commentOnRenderedSelection(HELD);
    expect(dialog()!.getAttribute("aria-label")).toBe("你的批注应该如何署名？");
    expect(dialog()!.textContent).not.toMatch(/git|github/i);
    expect(children()).toHaveLength(0);
  });
  it("asks first, says the name is a device setting for all graphs, and writes nothing until answered", () => {
    mount([block("p", TEXT)]);
    gitGraph();
    commentOnRenderedSelection(HELD);
    expect(dialog()).not.toBeNull();
    expect(dialog()!.getAttribute("aria-label")).toBe("How should your comments be signed?");
    expect(dialog()!.textContent).not.toMatch(/git|github/i);
    expect(dialog()!.textContent).toMatch(/Tine setting on this device, used in all your graphs, not a setting of this graph/);
    expect(children()).toHaveLength(0);
  });

  it("a given name signs this comment, is saved to the device, and is not asked again", async () => {
    mount([block("p", TEXT)]);
    gitGraph();
    commentOnRenderedSelection(HELD);
    type("  Ada  ");
    submit();
    expect(dialog()).toBeNull();
    expect(commentAuthorName()).toBe("Ada");
    expect(doc.byId[children()[0]].raw).toContain("tine.author:: Ada");
    await vi.waitFor(() => expect(setApp).toHaveBeenCalledWith("comment_author_name", "Ada"));
    expect(shouldAskCommentName()).toBe(false);
  });

  it("Skip writes the comment without tine.author::, remembers the refusal on the device, and never asks again", async () => {
    mount([block("p", TEXT)]);
    gitGraph();
    commentOnRenderedSelection(HELD);
    buttons("Skip").click();
    expect(dialog()).toBeNull();
    expect(children()).toHaveLength(1);
    expect(doc.byId[children()[0]].raw).not.toContain("tine.author::");
    await vi.waitFor(() => expect(setApp).toHaveBeenCalledWith("comment_name_prompt_declined", "1"));
    commentOnRenderedSelection({ ...HELD, start: 0, end: 2 });
    expect(dialog()).toBeNull();
    expect(children()).toHaveLength(2);
  });

  it("Escape and a click outside the dialog decline, too, and the comment is kept", async () => {
    mount([block("p", TEXT)]);
    gitGraph();
    commentOnRenderedSelection(HELD);
    document.querySelector<HTMLElement>(".modal-overlay")!.click();
    expect(dialog()).toBeNull();
    expect(children()).toHaveLength(1);
    await vi.waitFor(() => expect(setApp).toHaveBeenCalledWith("comment_name_prompt_declined", "1"));
    expect(shouldAskCommentName()).toBe(false);
  });

  it("a blank name is a decline", () => {
    mount([block("p", TEXT)]);
    gitGraph();
    commentOnRenderedSelection(HELD);
    type("   ");
    submit();
    expect(children()).toHaveLength(1);
    expect(commentAuthorName()).toBe("");
    expect(shouldAskCommentName()).toBe(false);
  });

  it("the second comment after a saved name is not asked", () => {
    mount([block("p", TEXT)]);
    gitGraph();
    commentOnRenderedSelection(HELD);
    type("Ada");
    submit();
    commentOnRenderedSelection({ ...HELD, start: 0, end: 2 });
    expect(dialog()).toBeNull();
    expect(children()).toHaveLength(2);
    expect(doc.byId[children()[1]].raw).toContain("tine.author:: Ada");
  });

  it("a block that changed while the dialog was open is refused, not mis-quoted", () => {
    mount([block("p", TEXT)]);
    gitGraph();
    commentOnRenderedSelection(HELD);
    setRaw("p", "Something else entirely.");
    buttons("Skip").click();
    expect(children()).toHaveLength(0);
  });
});

describe("no prompt", () => {
  it("in a graph without git: the comment is written at once", () => {
    mount([block("p", TEXT)]);
    resetCommentNamePromptForTests({ repository: false });
    commentOnRenderedSelection(HELD);
    expect(dialog()).toBeNull();
    expect(children()).toHaveLength(1);
  });

  it("while the git answer has not arrived: the comment is written at once and the question is sent", async () => {
    const status = vi.spyOn(backend(), "gitAutocommitStatus").mockResolvedValue({ available: true, repository: true });
    mount([block("p", TEXT)]);
    resetCommentNamePromptForTests({ repository: undefined });
    commentOnRenderedSelection(HELD);
    expect(dialog()).toBeNull();
    expect(children()).toHaveLength(1);
    expect(status).toHaveBeenCalled();
    await vi.waitFor(() => expect(shouldAskCommentName()).toBe(true));
  });

  it("when the git question fails or the platform has no git", async () => {
    vi.spyOn(backend(), "gitAutocommitStatus").mockResolvedValueOnce({ available: false }).mockRejectedValueOnce(new Error("x"));
    probeGraphGit();
    await new Promise((r) => setTimeout(r, 0));
    expect(shouldAskCommentName()).toBe(false);
    probeGraphGit();
    await new Promise((r) => setTimeout(r, 0));
    expect(shouldAskCommentName()).toBe(false);
  });

  it("with a name set", () => {
    mount([block("p", TEXT)]);
    gitGraph();
    resetCommentAuthorNameForTests("Ada");
    commentOnRenderedSelection(HELD);
    expect(dialog()).toBeNull();
    expect(doc.byId[children()[0]].raw).toContain("tine.author:: Ada");
  });

  it("after a decline", () => {
    mount([block("p", TEXT)]);
    resetCommentNamePromptForTests({ repository: true, declined: true });
    commentOnRenderedSelection(HELD);
    expect(dialog()).toBeNull();
    expect(children()).toHaveLength(1);
  });

  it("before the name or the declined flag has loaded from the device settings", () => {
    mount([block("p", TEXT)]);
    resetCommentNamePromptForTests({ repository: true, loaded: false });
    commentOnRenderedSelection(HELD);
    expect(dialog()).toBeNull();
    resetCommentAuthorNameForTests("", false);
    resetCommentNamePromptForTests({ repository: true });
    commentOnRenderedSelection({ ...HELD, start: 0, end: 2 });
    expect(dialog()).toBeNull();
    expect(children()).toHaveLength(2);
  });
});

describe("the device preference, across graphs", () => {
  it("loads the declined flag from the device settings", async () => {
    vi.spyOn(backend(), "getAppString").mockImplementation(async (key: string) => key === "comment_name_prompt_declined" ? "1" : "");
    resetCommentNamePromptForTests({ loaded: false, repository: true });
    resetCommentAuthorNameForTests("", true);
    initCommentNamePrompt();
    await vi.waitFor(() => expect(shouldAskCommentName()).toBe(false));
  });

  it("a decline and a saved name survive a graph switch; only the git answer is forgotten", () => {
    mount([block("p", TEXT)]);
    gitGraph();
    commentOnRenderedSelection(HELD);
    buttons("Skip").click();
    invalidateBinding();
    expect(shouldAskCommentName()).toBe(false);
    resetCommentNamePromptForTests({ declined: false, repository: true });
    resetCommentAuthorNameForTests("Ada");
    invalidateBinding();
    expect(commentAuthorName()).toBe("Ada");
    expect(shouldAskCommentName()).toBe(false);
  });

  it("a graph switch while the dialog is open closes it and drops the comment with a notice", () => {
    mount([block("p", TEXT)]);
    gitGraph();
    commentOnRenderedSelection(HELD);
    expect(dialog()).not.toBeNull();
    invalidateBinding();
    expect(dialog()).toBeNull();
    expect(setApp).not.toHaveBeenCalled();
  });
});

describe("replies", () => {
  it("the composer keeps its draft while asking, then posts the reply with the name", () => {
    const root = mount([block("p", TEXT, [block("c", "\ntine.quote:: a phrase worth disputing")])], () => <CommentComposer id="c" surface="test" />);
    gitGraph();
    const draft = root.querySelector<HTMLTextAreaElement>("textarea.comment-reply-draft")!;
    draft.dispatchEvent(new FocusEvent("focus"));
    draft.value = "I doubt this";
    draft.dispatchEvent(new Event("input", { bubbles: true }));
    root.querySelector<HTMLButtonElement>("button.comment-reply")!.click();
    expect(dialog()).not.toBeNull();
    expect(root.querySelector<HTMLTextAreaElement>("textarea.comment-reply-draft")!.value).toBe("I doubt this");
    type("Ada");
    submit();
    const replies = doc.byId["c"].children;
    expect(replies).toHaveLength(1);
    expect(doc.byId[replies[0]].raw).toContain("tine.author:: Ada");
  });

  it("an empty reply draft asks nothing", () => {
    const root = mount([block("p", TEXT, [block("c", "\ntine.quote:: x")])], () => <CommentComposer id="c" surface="test" />);
    gitGraph();
    draftSubmitEmpty(root);
    expect(dialog()).toBeNull();
  });
});

function draftSubmitEmpty(root: HTMLElement) {
  const draft = root.querySelector<HTMLTextAreaElement>("textarea.comment-reply-draft")!;
  draft.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", ctrlKey: true, bubbles: true, cancelable: true }));
}
