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

beforeAll(initParser);
afterEach(() => {
  endEdit("page-navigation"); resetStore(); closeSettings();
  changeCodeWrapping(false); document.body.innerHTML = ""; vi.restoreAllMocks();
});
const tick = () => new Promise(resolve => setTimeout(resolve, 0));

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
  const ta = root.querySelector("textarea.code-edit")!;
  expect(ta.getAttribute("wrap")).toBe("off");
  const toggle = root.querySelector<HTMLButtonElement>('[data-setting-label="Wrap code lines"] [role="switch"]')!;
  toggle.click(); await tick();
  expect(toggle.getAttribute("aria-checked")).toBe("true");
  expect(ta.getAttribute("wrap")).toBe("soft");
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
    expect.soft(root.querySelector("textarea")?.getAttribute("wrap")).toBe("soft");
    expect.soft(root.querySelector("pre.code-block")?.classList.contains("code-wrapping")).toBe(true);
  } finally { dispose(); }
});
