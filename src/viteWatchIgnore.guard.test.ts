// RULE: the Vite file watcher must ignore generated output (`target/`,
// `artifacts/`) and the out-of-tree-style toolchain dirs. Each worktree carries a `target/` with
// tens of thousands of directories; watching them dies with inotify ENOSPC in
// `npm run dev` / vite-node / `createServer`. Exemplar: vite.config.ts server.watch.
import { describe, expect, it, vi } from "vitest";
import { createServer, resolveConfig } from "vite";
import { mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

describe("vite watcher", () => {
  it("ignores target/ and toolchain build dirs", async () => {
    const config = await resolveConfig({ configFile: new URL("../vite.config.ts", import.meta.url).pathname }, "serve");
    const ignored = (config.server.watch?.ignored ?? []) as string[];
    for (const pattern of ["**/target/**", "**/.toolchain/**"]) {
      expect(ignored, `vite server.watch.ignored must contain ${pattern} (inotify ENOSPC on target/)`).toContain(pattern);
    }
  });

  it("does not acquire watches for journey artifacts, but still watches sources", async () => {
    const root = fileURLToPath(new URL("..", import.meta.url));
    const artifacts = path.join(root, "artifacts");
    mkdirSync(artifacts, { recursive: true });
    const fixture = mkdtempSync(path.join(artifacts, "vite-watch-"));
    mkdirSync(path.join(fixture, "graph", "pages"), { recursive: true });
    const server = await createServer({ root, server: { middlewareMode: true }, logLevel: "silent" });
    try {
      await vi.waitFor(() => expect(Object.keys(server.watcher.getWatched())).toContain(path.join(root, "src")));
      const watched = Object.keys(server.watcher.getWatched());
      expect(watched.filter(dir => dir === artifacts || dir.startsWith(`${artifacts}${path.sep}`))).toEqual([]);
    } finally {
      await server.close();
      rmSync(fixture, { recursive: true });
    }
  });
});
