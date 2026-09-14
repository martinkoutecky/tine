import assert from "node:assert/strict";
import { openPageByName } from "./lib/e2e-navigation.mjs";

// Restore the requested route after Ctrl+K, before search results settle. The
// helper must release its modal before a subsequent native toolbar click.
let title = "";
let switcher = false;
const keys = [];
globalThis.document = {
  querySelector(selector) {
    if (selector === "h1.page-title") return { textContent: title };
    if (selector === ".switcher-input") return switcher ? {} : null;
    throw new Error(`unexpected selector: ${selector}`);
  },
};
const browser = {
  execute: async (fn, ...args) => fn(...args),
  keys: async (value) => {
    keys.push(value);
    if (value[0] === "Control") switcher = true;
    if (value[0] === "Escape") switcher = false;
  },
  $: async () => ({
    waitForExist: async () => assert(switcher),
    setValue: async (name) => { title = name; },
  }),
  waitUntil: async (predicate) => assert(await predicate(), "readiness did not settle"),
};
await openPageByName(browser, "Restored page");
assert.equal(title, "Restored page");
assert.equal(switcher, false, "navigation returned while its switcher still covered the restored page");
assert.deepEqual(keys, [["Control", "k"], ["Escape"]]);

// A route already open on entry grants no ownership over an unrelated modal.
keys.length = 0;
switcher = true;
await openPageByName(browser, "Restored page");
assert.equal(switcher, true);
assert.deepEqual(keys, []);
delete globalThis.document;
console.log("PASS: startup restore dismisses only the helper-owned switcher");
