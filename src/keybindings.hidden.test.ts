// @vitest-environment jsdom
// #927 review R1-1: a command hidden after installation (Show Workspaces off)
// must not run from a saved shortcut, and its chord must reach other handlers.
import { afterEach, expect, it, vi } from "vitest";

const state = vi.hoisted(() => ({ shown: true, created: 0 }));
vi.mock("./navSettings", async (importOriginal) => ({ ...(await importOriginal<typeof import("./navSettings")>()), showWorkspaces: () => state.shown }));
vi.mock("./workspaceCreation", () => ({ openWorkspaceCreation: () => { state.created++; } }));

import { installKeybindings } from "./keybindings";

let dispose = () => {};
afterEach(() => { dispose(); state.shown = true; state.created = 0; });
const press = () => { const e = new KeyboardEvent("keydown", { key: "F9", bubbles: true, cancelable: true }); window.dispatchEvent(e); return e; };

it("a saved Workspaces shortcut stops firing when Workspaces are hidden after install, and fires again when shown", () => {
  dispose = installKeybindings({ "workspace/new": "f9" });
  expect(press().defaultPrevented).toBe(true);
  expect(state.created).toBe(1);
  state.shown = false;
  const hidden = press();
  expect(state.created).toBe(1);
  expect(hidden.defaultPrevented).toBe(false);
  state.shown = true;
  press();
  expect(state.created).toBe(2);
});
