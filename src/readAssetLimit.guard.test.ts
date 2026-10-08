import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import type { Backend } from "./backend";

// Compiled by tsc, never called: L-1 must reject an omitted cap at the TS door.
function readAssetTypeContract(api: Backend) {
  // @ts-expect-error L-1: every asset read requires the caller's explicit cap.
  void api.readAsset("image.png");
}
void readAssetTypeContract;

const source = (path: string) => readFileSync(new URL(`../${path}`, import.meta.url), "utf8");
const RULE = "L-1: read_asset must require a caller cap and never reach Store::read(None). " +
  "Internal unbounded store reads are not an IPC asset-read policy.";

describe("read_asset size boundary (L-1)", () => {
  it("requires the IPC cap and forwards it to the sole feature asset read", () => {
    const commands = source("src-tauri/src/commands.rs");
    const command = commands.match(/pub\(crate\) fn read_asset\([\s\S]*?\n\}/)?.[0];
    expect(command, RULE).toMatch(/max_bytes:\s*u64\b/);
    expect(command, RULE).toContain("tine_graph_features::assets::read_asset(&slot.store, &name, max_bytes)");
    expect(command, RULE).not.toMatch(/\.read\(/);
    expect(commands.match(/tine_graph_features::assets::read_asset\(/g), RULE).toHaveLength(1);
  });

  it("wraps the required feature cap in Some at the optional store door", () => {
    const assets = source("crates/tine-graph-features/src/assets.rs");
    const read = assets.match(/pub fn read_asset\([\s\S]*?\n\}/)?.[0];
    expect(read, RULE).toMatch(/max_bytes:\s*u64\b/);
    expect(read, RULE).toContain(".read(&id, Some(max_bytes))");
    expect(read?.match(/\.read\(/g), RULE).toHaveLength(1);
  });

  it("requires the cap in the Backend interface and every implementation", () => {
    for (const [path, count] of [["src/backend.ts", 2], ["src/mock.ts", 1], ["src/publishedBackend.ts", 1]] as const) {
      const signatures = source(path).match(/readAsset\(name: string, maxBytes: number\)/g);
      expect(signatures, RULE).toHaveLength(count);
    }
  });
});
