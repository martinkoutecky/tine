import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

/** A modal is an app layer whose backdrop covers the whole window and blocks the
 * pointer from what is behind it. These are the classes of those backdrops. While
 * one is open, a key whose target is not a field leaves every selection behind it
 * alone, which works only if the component that renders it registers through
 * `useModal` (src/transientLayers.ts). Pointer-only click-away scrims, popovers,
 * menus and the compact drawers are not modals and do not use these classes. */
const BACKDROPS = ["modal-overlay", "welcome-overlay", "lightbox-overlay", "audio-overlay", "switcher-overlay"];

function sources(dir: string, out: string[] = []): string[] {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) sources(p, out);
    else if (/\.tsx$/.test(name) && !/\.test\.tsx$/.test(name)) out.push(p);
  }
  return out;
}

describe("every full-window backdrop registers its modal", () => {
  const attribute = new RegExp(`class="[^"]*\\b(${BACKDROPS.join("|")})\\b[^"]*"`, "g");
  const renderers = sources("src").flatMap((path) => {
    const text = readFileSync(path, "utf8");
    return [...text.matchAll(attribute)].map((match) => {
      // The component is the nearest top-level function declared above the backdrop.
      const head = [...text.slice(0, match.index).matchAll(/^(?:export )?function (\w+)/gm)].at(-1);
      return { path, component: head?.[1] ?? "?", body: text.slice(head?.index ?? 0, match.index) };
    });
  });

  it("finds the backdrops it guards", () => {
    expect(renderers.length).toBeGreaterThanOrEqual(21);
  });

  it("each component that renders one calls useModal before it", () => {
    const missing = renderers.filter((r) => !/\buseModal\(/.test(r.body)).map((r) => `${r.path} ${r.component}`);
    expect(missing).toEqual([]);
  });
});
