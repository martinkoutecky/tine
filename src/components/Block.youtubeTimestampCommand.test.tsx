import { editorIn, type EditorDriver } from "../tests/editorDriver";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { startEditing } from "../editorController";
import { initParser } from "../render/parse";
import { pageByName, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import type { BlockDto, PageDto } from "../types";
import { VideoMacro } from "./Macro";
import { Block } from "./Block";
import { installKeybindings } from "../keybindings";

beforeAll(async () => {
  await initParser();
});

function installYoutube(currentTime: number) {
  const player = {
    seekTo: vi.fn(),
    getCurrentTime: vi.fn(() => currentTime),
    destroy: vi.fn(),
  };
  const Player = vi.fn(function Player() {
    return player;
  });
  Object.assign(window as Window & { YT?: { Player: typeof Player } }, { YT: { Player } });
  return Player;
}

function mount(node: () => JSX.Element): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.appendChild(root);
  return { root, dispose: render(node, root) };
}

function commandPage(): PageDto {
  const block: BlockDto = { id: "youtube-command", raw: "/youtube", collapsed: false, children: [] };
  return {
    name: "YouTube command",
    title: "YouTube command",
    kind: "page",
    pre_block: null,
    blocks: [block],
  };
}

function inputAt(textarea: EditorDriver, value: string) {
  textarea.focus();
  textarea.replaceText(value);
  textarea.select(value.length, value.length);
  textarea.reportInput({
    bubbles: true,
    inputType: "insertText",
    data: value.at(-1) ?? null,
  });
}

function choose(label: string) {
  const item = [...document.body.querySelectorAll<HTMLElement>(".autocomplete .ac-item")]
    .find((candidate) => candidate.querySelector(".ac-label")?.textContent === label);
  expect(item).toBeDefined();
  item!.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
}

afterEach(() => {
  delete (window as Window & { YT?: unknown }).YT;
  resetStore();
  document.body.innerHTML = "";
});

describe("Embed Youtube timestamp slash command", () => {
  it("leaves text and selection intact when the shortcut has no ready player", async () => {
    const disposeKeys = installKeybindings();
    loadSingle(commandPage());
    startEditing("youtube-command", 0);
    const { root, dispose } = mount(() => <Block id="youtube-command" />);
    try {
      const textarea = editorIn(root);
      inputAt(textarea, "keep this text");
      textarea.select(5, 9);
      const key = textarea.keydown({ key: "Y", ctrlKey: true, shiftKey: true, bubbles: true, cancelable: true });
      expect(key.defaultPrevented).toBe(true);
      expect(doc.byId["youtube-command"].raw).toBe("keep this text");
      expect([textarea.selection()[0], textarea.selection()[1]]).toEqual([5, 9]);
    } finally { disposeKeys(); dispose(); }
  });
  it("inserts at the selection with Mod+Shift+Y", async () => {
    const disposeKeys = installKeybindings();
    const Player = installYoutube(42.9);
    loadSingle(commandPage());
    startEditing("youtube-command", 0);
    const { root, dispose } = mount(() => <><VideoMacro body="youtube dQw4w9WgXcQ" /><Block id="youtube-command" /></>);
    try {
      await vi.waitFor(() => expect(Player).toHaveBeenCalledOnce());
      const textarea = editorIn(root);
      inputAt(textarea, "before selected after");
      textarea.select(7, 15);
      textarea.keydown({ key: "Y", ctrlKey: true, shiftKey: true, bubbles: true, cancelable: true });
      await vi.waitFor(() => expect(doc.byId["youtube-command"].raw).toBe("before {{youtube-timestamp 42}}  after"));
      expect(textarea.selection()[0]).toBe("before {{youtube-timestamp 42}} ".length);
    } finally { disposeKeys(); dispose(); }
  });

  it("floors the selected YouTube player's current time and inserts its macro", async () => {
    const Player = installYoutube(125.9);
    loadSingle(commandPage());
    startEditing("youtube-command", "/youtube".length);
    const { root, dispose } = mount(() => (
      <>
        <VideoMacro body="youtube dQw4w9WgXcQ" />
        <For each={pageByName("YouTube command")?.roots ?? []}>{(id) => <Block id={id} />}</For>
      </>
    ));

    try {
      await expect.poll(() => Player.mock.calls.length).toBe(1);
      const textarea = editorIn(root);
      inputAt(textarea, "/youtube");
      await vi.waitFor(() => expect([...document.body.querySelectorAll(".autocomplete .ac-label")]
        .some((element) => element.textContent === "Embed Youtube timestamp")).toBe(true));
      choose("Embed Youtube timestamp");
      await vi.waitFor(() => expect(doc.byId["youtube-command"].raw).toBe("{{youtube-timestamp 125}}"));
    } finally {
      dispose();
    }
  });

  it("inserts nothing when no player is registered, like OG (slash text still consumed)", async () => {
    // No installYoutube: OG's generator yields nothing without a ready player
    // (youtube.cljs:113-122) — the command consumes the trigger, inserts no macro.
    loadSingle(commandPage());
    startEditing("youtube-command", "/youtube".length);
    const { root, dispose } = mount(() => (
      <For each={pageByName("YouTube command")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));

    try {
      const textarea = editorIn(root);
      inputAt(textarea, "/youtube");
      await vi.waitFor(() => expect([...document.body.querySelectorAll(".autocomplete .ac-label")]
        .some((element) => element.textContent === "Embed Youtube timestamp")).toBe(true));
      choose("Embed Youtube timestamp");
      await vi.waitFor(() => expect(doc.byId["youtube-command"].raw).toBe(""));
    } finally {
      dispose();
    }
  });
});
