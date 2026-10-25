import { maybeEditorIn } from "../tests/editorDriver";
import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { codeWrapping, changeCodeWrapping, initCodeDisplay } from "../codeDisplay";
import { closeSettings, openSettings } from "../ui";
import { initParser } from "../render/parse";

import { loadSingle } from "../document/workingSet";
import { resetStore } from "../document";
import { startEditing, endEdit } from "../editorController";
import { Block } from "./Block";
import { Settings } from "./Settings";
import { commandDefaults, paletteCommands, runGlobalCommand } from "../keybindings";
import { readFileSync } from "node:fs";

beforeAll(initParser);
afterEach(() => {
  endEdit("page-navigation"); resetStore(); closeSettings();
  changeCodeWrapping(false); document.body.innerHTML = ""; vi.restoreAllMocks();
});
const tick = () => new Promise(resolve => setTimeout(resolve, 0));

it("GH453: bindable palette command preserves each reading viewport in both directions", async () => {
  const saved = vi.spyOn(backend(), "setAppString").mockResolvedValue();
  expect(commandDefaults().find(c => c.id === "ui/toggle-code-block-wrap")).toMatchObject({ binding: "", label: "Toggle code block wrap" });
  expect(paletteCommands().some(c => c.id === "ui/toggle-code-block-wrap")).toBe(true);
  loadSingle({ name: "Code", title: "Code", kind: "page", pre_block: null, blocks: [
    { id: "above", raw: "```js\n" + "long ".repeat(80) + "\n```", collapsed: false, children: [] },
    { id: "anchor", raw: "Reading here", collapsed: false, children: [] },
    { id: "below", raw: "```js\n" + "long ".repeat(80) + "\n```", collapsed: false, children: [] },
  ] });
  const disposers: (() => void)[] = [];
  const scrollers = ["main-content", "main-content", "right-sidebar"].map(className => {
    const scroller = document.createElement("div"); scroller.className = className; document.body.append(scroller);
    const content = document.createElement("div"); scroller.append(content);
    disposers.push(render(() => <><Block id="above" /><Block id="anchor" /><Block id="below" /></>, content));
    scroller.scrollTop = 200;
    vi.spyOn(scroller, "getBoundingClientRect").mockReturnValue({ top: 0, bottom: 500 } as DOMRect);
    const blocks = content.querySelectorAll<HTMLElement>("[data-block-id]");
    vi.spyOn(blocks[0], "getBoundingClientRect").mockReturnValue({ top: -200, bottom: -10 } as DOMRect);
    vi.spyOn(blocks[1], "getBoundingClientRect").mockImplementation(() => ({ top: 20 + (codeWrapping() ? 160 : 0) - (scroller.scrollTop - 200), bottom: 60 } as DOMRect));
    return scroller;
  });
  scrollers[0].tabIndex = -1;
  scrollers[0].focus();
  try {
    for (const value of [true, false]) {
      expect(runGlobalCommand("ui/toggle-code-block-wrap")).toBe(true);
      await tick();
      expect(codeWrapping()).toBe(value);
      for (const scroller of scrollers) {
        expect(scroller.querySelector('[data-block-id="anchor"]')!.getBoundingClientRect().top).toBe(20);
        expect(scroller.querySelector("pre.code-block")?.classList.contains("code-wrapping")).toBe(value);
      }
      expect(saved).toHaveBeenLastCalledWith("code_line_wrapping", value ? "1" : "0");
    }
  } finally { disposers.forEach(dispose => dispose()); }
});

it("GH453: Guide explains the command and shortcut assignment", () => {
  const guide = readFileSync("crates/tine-core/src/templates/tips.md", "utf8");
  expect(guide).toContain("Toggle code block wrap");
  expect(guide).toContain("keeps your reading position");
  expect(guide).toContain("Keyboard shortcuts");
});

it("Settings globally wraps reading and editing, and remembers the device preference", async () => {
  const saved = vi.spyOn(backend(), "setAppString").mockResolvedValue();
  loadSingle({ name: "Code", title: "Code", kind: "page", pre_block: null, blocks: [
    { id: "code2", raw: "```js\na\nb\n```", collapsed: false, children: [] },
    { id: "code", raw: "```python\nx = 1\ny = 2\n```", collapsed: false, children: [] },
  ] });
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <><Settings /><Block id="code" /><Block id="code2" /></>, root);
  openSettings("appearance"); await tick();
  expect(codeWrapping()).toBe(false);
  expect(root.querySelector(".code-language")?.textContent).toBe("python");
  expect([...root.querySelectorAll("pre.code-block .calc-lineno")].map(el => el.textContent)).toEqual(["1", "2", "1", "2"]);
  startEditing("code", 0); await tick();
  const ta = maybeEditorIn(root)!;
  expect(ta.element.getAttribute("wrap")).toBe("off");
  const toggle = root.querySelector<HTMLButtonElement>('[data-setting-label="Wrap code lines"] [role="switch"]')!;
  runGlobalCommand("ui/toggle-code-block-wrap"); await tick();
  expect(toggle.getAttribute("aria-checked")).toBe("true");
  expect(ta.element.getAttribute("wrap")).toBe("soft");
  expect(root.querySelector("pre.code-block")?.classList.contains("code-wrapping")).toBe(true);
  expect(saved).toHaveBeenCalledWith("code_line_wrapping", "1");
  endEdit("page-navigation"); await tick();
  expect([...root.querySelectorAll("pre.code-block")].every(card => card.classList.contains("code-wrapping"))).toBe(true);
  toggle.click(); await tick();
  expect(saved).toHaveBeenLastCalledWith("code_line_wrapping", "0");
  dispose();
});

it("loads the preference without letting a delayed read undo a later Settings change", async () => {
  vi.spyOn(backend(), "getAppString").mockResolvedValue("1");
  await initCodeDisplay(); expect(codeWrapping()).toBe(true);
  let resolve!: (value: string) => void;
  vi.spyOn(backend(), "getAppString").mockImplementation(() => new Promise(r => { resolve = r; }));
  vi.spyOn(backend(), "setAppString").mockResolvedValue();
  const pending = initCodeDisplay(); changeCodeWrapping(false); resolve("1");
  await pending; expect(codeWrapping()).toBe(false);
});

it("open code cards react immediately to the global wrap preference", async () => {
  vi.spyOn(backend(), "setAppString").mockResolvedValue();
  loadSingle({ name: "Code", title: "Code", kind: "page", pre_block: null, blocks: [
    { id: "edit", raw: "```js\na\n```", collapsed: false, children: [] },
    { id: "view", raw: "```js\nb\n```", collapsed: false, children: [] },
  ] });
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <><Block id="edit" /><Block id="view" /></>, root);
  try {
    startEditing("edit", 0); await tick();
    changeCodeWrapping(true); await tick();
    expect.soft(maybeEditorIn(root)?.element.getAttribute("wrap")).toBe("soft");
    expect.soft(root.querySelector("pre.code-block")?.classList.contains("code-wrapping")).toBe(true);
  } finally { dispose(); }
});
