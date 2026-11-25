import { afterEach, expect, it, vi } from "vitest";

// The keymap reads the platform once, when it loads. An iPad that reports
// "iPad" is not the Mac keyboard convention (one that reports "MacIntel" is, and
// is covered by Block.macOptionKeys.test.tsx).
vi.hoisted(() => Object.defineProperty(navigator, "platform", { configurable: true, value: "iPad" }));

import { resetStore } from "../document";
import { installKeybindings } from "../keybindings";
import * as router from "../router";

let dispose: (() => void) | null = null;
afterEach(() => {
  dispose?.(); dispose = null;
  resetStore();
  document.body.innerHTML = "";
});

// GH #1040 changes the Mac keyboard convention only.
it("on an iPad reporting iPad, Option+Left in a text field goes Back, as before", () => {
  dispose = installKeybindings();
  router.openPage("Earlier page", "page");
  router.openPage("iPad keys", "page");
  const input = document.createElement("input");
  document.body.appendChild(input);
  input.focus();
  expect(navigator.platform).toBe("iPad");
  const event = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "ArrowLeft", code: "ArrowLeft", altKey: true });
  input.dispatchEvent(event);
  expect(event.defaultPrevented).toBe(true);
  expect((router.route() as { name?: string }).name).toBe("Earlier page");
});
