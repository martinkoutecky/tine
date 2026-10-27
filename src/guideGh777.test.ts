import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

it("the Guide explains the Linux NVIDIA fallback and user overrides (GH #777)", () => {
  const guide = readFileSync("crates/tine-core/src/templates/troubleshooting-recovery.md", "utf8");
  for (const detail of ["NVIDIA driver older than version 610", "other drivers keep", "TINE_GPU=0", "WEBKIT_DISABLE_DMABUF_RENDERER", "TINE_GPU=1"])
    expect(guide).toContain(detail);
});
