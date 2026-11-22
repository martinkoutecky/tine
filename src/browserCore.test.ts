import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { initParser } from "./render/parse";
import { BrowserGraph, __tineReinstantiate } from "./render/wasm/lsdoc_wasm";

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

const encode = (text: string) => new TextEncoder().encode(text);
const open = (files: [string, string][]) =>
  new BrowserGraph(
    files.map(([path]) => path),
    files.map(([, text]) => encode(text)),
  );
const names = (graph: BrowserGraph) =>
  JSON.parse(graph.page_inventory_json()).entries.map((row: { name: string }) => row.name);

// A handle points into the instance that made it: after a fresh instance (a
// trap's recovery) it is refused, and freeing it leaves the new instance alone.
it("a handle from a replaced instance is refused", async () => {
  await initParser();
  const old = open([["pages/old.md", "- old\n"]]);
  __tineReinstantiate();
  const fresh = open([["pages/fresh.md", "- fresh\n"]]);
  expect(() => old.page_inventory_json()).toThrow(/replaced instance/);
  old.free();
  expect(names(fresh)).toEqual(["fresh"]);
  // A freed handle is refused too, and freeing it again is a no-op.
  const freed = open([["pages/freed.md", "- freed\n"]]);
  freed.free();
  expect(() => freed.page_inventory_json()).toThrow(/freed handle/);
  expect(() => freed.config_problem_json()).toThrow(/freed handle/);
  freed.free();
  freed[Symbol.dispose]();
  expect(names(fresh)).toEqual(["fresh"]);
  fresh.free();
});

// The constructor refuses what the desktop open refuses, and mismatched arrays.
it("the browser core refuses what the desktop open refuses", async () => {
  await initParser();
  expect(() => new BrowserGraph(["pages/a.md"], [])).toThrow(/1 paths, 0 files/);
  expect(() => open([["/abs.md", "- x\n"]])).toThrow(/graph-relative/);
  expect(() => open([["logseq/config.edn", '{:pages-directory "../out"}']])).toThrow(/pages directory/);
  const fine = open([["pages/a.md", "- a\n"]]);
  expect(JSON.parse(fine.config_problem_json())).toBeNull();
  fine.free();
  const unreadable = new BrowserGraph(["logseq/config.edn", "pages/a.md"], [new Uint8Array([0xff]), encode("- a\n")]);
  expect(JSON.parse(unreadable.config_problem_json())).toMatchObject({ kind: "config-read" });
  unreadable.free();
  expect(() => open([["pages/a.md", "- a\n"], ["pages//a.md", "- b\n"]])).toThrow(/given twice/);
  expect(() => open([["pages/a.md", "x"], ["x/pages/b.md", "x"], ["logseq/config.edn", '{:pages-directory "pages/a.md/p"}']])).toThrow(/under a file/);
  const icloud = open([["logseq/.config.edn.icloud", "x"], ["pages/a.md", "- a\n"]]);
  expect(JSON.parse(icloud.config_problem_json())).toMatchObject({ kind: "config-read" });
  icloud.free();
});
