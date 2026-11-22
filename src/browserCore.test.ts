import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { initParser } from "./render/parse";
import { BrowserGraph } from "./render/wasm/lsdoc_wasm";

// The browser core (D-20261122-WEX) answers as the desktop app's store: the
// same graph gives the rows src-tauri's discovery test pins from this fixture.
it("the browser core's page inventory is the desktop app's", async () => {
  await initParser();
  const fixture = JSON.parse(readFileSync("tests/fixtures/browser-core-inventory.json", "utf8"));
  const encode = (text: string) => new TextEncoder().encode(text);
  const graph = new BrowserGraph(
    fixture.files.map(([path]: [string, string]) => path),
    fixture.files.map(([, text]: [string, string]) => encode(text)),
  );
  const wire = JSON.parse(graph.page_inventory_json());
  graph.free();
  expect(wire.entries).toEqual(fixture.rows);
  expect(wire.unreadable).toEqual(fixture.unreadable);
});
