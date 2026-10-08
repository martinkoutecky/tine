import { readdirSync, readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

/** Every non-test frontend source under `src/`, so an ownership rule cannot be
 *  escaped by putting the offending copy in a directory the scan forgot. */
function frontendSources(): string[] {
  const out: string[] = [];
  const pending = ["src"];
  while (pending.length > 0) {
    const dir = pending.pop()!;
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const path = `${dir}/${entry.name}`;
      if (entry.isDirectory()) pending.push(path);
      else if (/\.tsx?$/.test(entry.name) && !/\.(test|spec)\.tsx?$/.test(entry.name)) out.push(path);
    }
  }
  return out;
}

describe("outside-press ownership (GH #472, master 8198daf34)", () => {
  it("keeps 'the user pressed outside' on its one owner", () => {
    // Popovers had hand-rolled this capture listener, drifting between
    // mousedown/pointerdown and document/window, and a popover that forgot it
    // stayed open while the user clicked away. The producer is
    // `dismissOnOutsidePointer` in src/transientLayers.ts; anything that
    // registers a transient layer calls it instead of reaching for the document.
    // (A capture listener with no layer — blockDrag's drag start — is not this
    // question and is not scanned.)
    const owner = "src/transientLayers.ts";
    expect(readFileSync(owner, "utf8")).toContain("export function dismissOnOutsidePointer");

    const files = frontendSources().filter((path) => path !== owner);
    for (const file of files) {
      const source = readFileSync(file, "utf8");
      if (!source.includes("registerTransientLayer")) continue;
      expect(source, `I-12: ${file} defines its own outside-press producer; import it from ${owner}`)
        .not.toMatch(/function dismissOnOutsidePointer|const dismissOnOutsidePointer/);
      for (const type of ["pointerdown", "mousedown"]) {
        expect(
          source,
          `I-12: ${file} registers a transient layer and its own "${type}" listener. ` +
            `Outside-press dismissal has one owner — call dismissOnOutsidePointer from ` +
            `${owner}; src/components/TopbarOverflowMenu.tsx is the one-line exemplar.`,
        ).not.toContain(`addEventListener("${type}"`);
      }
    }
    // Non-vacuity: the scan must actually reach the components this rule is about.
    expect(files.filter((path) => readFileSync(path, "utf8").includes("registerTransientLayer")).length)
      .toBeGreaterThanOrEqual(20);
  });
});
