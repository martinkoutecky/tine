import { expect, it } from "vitest";
import { loadBrowserCore } from "./browserCore";

// A memory graph holds each name once and a small node per file: a flat
// directory of 180,000 tiny assets opens in less linear memory than the
// path-keyed maps it replaced (48.1 MiB of growth at 12607c208, 38.3 after;
// review 0d6586f8c #1). Its own file, so the instance starts fresh.
it("a flat asset directory opens within its memory budget", async () => {
  const { BrowserGraph, initSync } = await loadBrowserCore();
  // Initialised already: initSync answers the live instance, ignoring its input.
  const memory = () =>
    initSync({ module: new Uint8Array(0) }).memory.buffer.byteLength / 1048576;
  const start = memory();
  const paths = ["pages/a.md"];
  const files = [new TextEncoder().encode("- a\n")];
  for (let i = 0; i < 180000; i++) {
    paths.push(`assets/f${i}.bin`);
    files.push(new Uint8Array(4));
  }
  const graph = new BrowserGraph(paths, files);
  const grown = memory() - start;
  graph.free();
  expect(grown).toBeLessThan(44);
});
