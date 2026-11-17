import assert from "node:assert/strict";
import test from "node:test";
import { waitDrawer } from "./lib/e2e-drawer.mjs";

test("an active drawer must finish sliding before a native click can use its geometry", async (t) => {
  const originalDocument = globalThis.document;
  const originalStyle = globalThis.getComputedStyle;
  t.after(() => {
    globalThis.document = originalDocument;
    globalThis.getComputedStyle = originalStyle;
  });
  let moving = true;
  const panel = { getAnimations: () => moving ? [{}] : [] };
  globalThis.document = {
    querySelector: () => ({ getAttribute: () => "right" }),
    querySelectorAll: selector => selector === "[data-mobile-drawer-panel]" ? [panel] : [{}],
  };
  globalThis.getComputedStyle = () => ({ transform: "none" });
  await waitDrawer({
    execute: (callback, ...args) => callback(...args),
    waitUntil: async (ready) => {
      assert.equal(await ready(), false, "active state alone must not release a click during the opening animation");
      moving = false;
      assert.equal(await ready(), true, "the resting drawer must release the click");
    },
  }, "right");
});
