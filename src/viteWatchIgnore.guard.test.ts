// RULE: the Vite file watcher must ignore Rust build output (`target/`) and the
// out-of-tree-style toolchain dirs. Each Rust worktree carries a `target/` with
// tens of thousands of directories; watching them dies with inotify ENOSPC in
// `npm run dev` / vite-node / `createServer`. Exemplar: vite.config.ts server.watch.
import { describe, expect, it } from "vitest";
import { resolveConfig } from "vite";

describe("vite watcher", () => {
  it("ignores target/ and toolchain build dirs", async () => {
    const config = await resolveConfig({ configFile: new URL("../vite.config.ts", import.meta.url).pathname }, "serve");
    const ignored = (config.server.watch?.ignored ?? []) as string[];
    for (const pattern of ["**/target/**", "**/.toolchain/**"]) {
      expect(ignored, `vite server.watch.ignored must contain ${pattern} (inotify ENOSPC on target/)`).toContain(pattern);
    }
  });
});
