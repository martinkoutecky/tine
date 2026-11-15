import assert from "node:assert/strict";
import test from "node:test";
import fs from "node:fs";
import { JSDOM } from "jsdom";
import { editorState, editorText, selectEditor, withEditor, pasteEditorText } from "./e2e-block-editor.mjs";
import { defaultClassicEditorProfile } from "./e2e-classic-profile.mjs";

test("migrated native journeys have one editor owner and no classic profile seed", () => {
  const migrated = fs.readdirSync(new URL("../", import.meta.url)).filter(file => /^e2e-.*\.mjs$/.test(file))
    .map(file => [file, fs.readFileSync(new URL(`../${file}`, import.meta.url), "utf8")])
    .filter(([, source]) => defaultClassicEditorProfile(source) === "0");
  assert.ok(migrated.length > 0);
  assert.deepEqual(migrated.filter(([, source]) => /textarea\.block-editor/.test(source)).map(([file]) => file), []);
});

test("native inspection reads the full CM document beyond rendered lines and scopes duplicate blocks", async () => {
  const dom = new JSDOM('<main><div class="ls-block" data-block-id="same"><div data-block-editor="cm6"><div class="cm-content" contenteditable="true" tabindex="0">visible line only</div></div></div></main><aside><textarea class="block-editor">sidebar</textarea></aside>', { runScripts: "outside-only" });
  try {
    const content = dom.window.document.querySelector(".cm-content");
    const text = "first\n😀 offscreen\nlast";
    const state = { doc: { length: text.length, toString: () => text }, selection: { main: { from: 6, to: 8 } } };
    const transactions = [];
    content.cmTile = { root: { view: { state, dispatch: tr => transactions.push(tr) } } };
    content.focus();
    let requests = 0;
    const browser = { execute: (fn, ...args) => { requests++; return dom.window.eval(`(${fn.toString()})`)(...args); } };
    assert.deepEqual({ ...await editorState(browser, "main") }, {
      value: text, start: 6, end: 8, focused: true, blockId: "same", kind: "cm6",
    });
    assert.equal(requests, 1, "one scoped observation is one WebDriver request");
    assert.equal(await editorText(browser, "aside"), "sidebar");
    await selectEditor(browser, 0, 5, "main");
    assert.equal(transactions.length, 1);
    assert.equal(JSON.stringify(transactions[0]), JSON.stringify({ selection: { anchor: 0, head: 5 }, userEvent: "select" }));
    assert.equal(content.textContent, "visible line only", "selection does not replace the DOM or buffer");
    assert.equal(await withEditor(browser, "main", editor => editor.element.closest("main").tagName), "MAIN");
    assert.equal(await withEditor(browser, "main", editor => editor.length), text.length);
  } finally { dom.window.close(); }
});

test("replacement does not send the outline-selecting Mod+A to an empty editor", async () => {
  for (const value of ["", "existing text"]) {
    const dom = new JSDOM('<textarea class="block-editor"></textarea>', { runScripts: "outside-only" });
    try {
      const element = dom.window.document.querySelector("textarea");
      element.value = value; element.focus();
      const keys = [];
      const browser = {
        execute: (fn, ...args) => dom.window.eval(`(${fn.toString()})`)(...args),
        executeAsync: (_fn, text) => { assert.equal(text, "replacement"); return { ok: true }; },
        keys: chord => keys.push(chord),
      };
      await pasteEditorText(browser, "replacement");
      assert.deepEqual(keys, value ? [["Control", "a"], ["Control", "v"]] : [["Control", "v"]]);
    } finally { dom.window.close(); }
  }
});

test("classic selection keeps UTF-16 offsets; absent or broken editor surfaces fail explicitly", async () => {
  const dom = new JSDOM('<textarea class="block-editor">a😀b</textarea>', { runScripts: "outside-only" });
  try {
    const browser = { execute: (fn, ...args) => dom.window.eval(`(${fn.toString()})`)(...args) };
    await selectEditor(browser, 1, 3);
    const selected = await editorState(browser);
    assert.equal(selected.value.slice(selected.start, selected.end), "😀");
    assert.equal(selected.focused, true);
    assert.equal(await editorState(browser, "aside"), null);
    await assert.rejects(editorText(browser, "aside"), /No mounted block editor/);
    dom.window.document.body.innerHTML = '<div data-block-editor="cm6"><div class="cm-content" contenteditable="true"></div></div>';
    await assert.rejects(editorText(browser), /no EditorView/);
  } finally { dom.window.close(); }
});
