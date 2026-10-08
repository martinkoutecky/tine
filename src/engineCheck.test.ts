import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
// GH #572: on a web engine older than Safari 15.4 (macOS before 12.3, even with the
// Safari app updated; iOS < 15.4) the lsdoc wasm fails to compile and ES2022 built-ins are
// missing, so Tine loaded half-broken. index.html's classic script must detect
// that and say so before the application module can run.
const root = path.resolve(import.meta.dirname, "..");
const index = fs.readFileSync(path.join(root, "index.html"), "utf8");
const script = /<script id="tine-engine-check">([\s\S]*?)<\/script>/.exec(index)?.[1] ?? "";
const macosMinimum = (
  JSON.parse(fs.readFileSync(path.join(root, "src-tauri/tauri.macos.conf.json"), "utf8")) as {
    bundle?: { macOS?: { minimumSystemVersion?: string } };
  }
).bundle?.macOS?.minimumSystemVersion;
function runCheck(engine: { at?: boolean; hasOwn?: boolean; structuredClone?: boolean; referenceTypes?: boolean; ua?: string }) {
  const rootEl = { innerHTML: "Opening Tine…" };
  const window: Record<string, unknown> = {};
  const ArrayFake = { prototype: engine.at === false ? {} : { at() {} } };
  const ObjectFake = engine.hasOwn === false ? {} : { hasOwn() {} };
  const wasm = {
    validate: (bytes: Uint8Array<ArrayBuffer>) =>
      engine.referenceTypes === false ? false : WebAssembly.validate(bytes),
  };
  new Function("window", "document", "navigator", "WebAssembly", "structuredClone", "Object", "Array", script)(
    window,
    { getElementById: () => rootEl },
    { userAgent: engine.ua ?? "Mozilla/5.0 (Macintosh; Intel Mac OS X 11_7_8) AppleWebKit/605.1.15" },
    wasm,
    engine.structuredClone === false ? undefined : () => {},
    ObjectFake,
    ArrayFake,
  );
  return { unsupported: window.__tineUnsupportedEngine === true, html: rootEl.innerHTML };
}
describe("unsupported web engine check (GH #572)", () => {
  it("leaves a current engine alone, using a real wasm reference-types probe", () => {
    expect(runCheck({})).toEqual({ unsupported: false, html: "Opening Tine…" });
  });
  it.each([
    ["the lsdoc wasm cannot compile (no reference types, Safari < 15)", { referenceTypes: false }],
    ["Array.prototype.at is missing (Safari < 15.4)", { at: false }],
    ["Object.hasOwn is missing", { hasOwn: false }],
    ["structuredClone is missing", { structuredClone: false }],
  ])("stops startup and tells a Mac user which macOS Tine needs when %s", (_why, engine) => {
    const result = runCheck(engine);
    expect(result.unsupported).toBe(true);
    expect(result.html).toContain("too old for Tine");
    // Updating the Safari app does not update the engine other apps embed on
    // older macOS (GH #572: Safari 16.5.2 on Big Sur, Tine's web view still
    // lacked Array.prototype.at), so the remedy names the macOS version.
    expect(result.html).toContain(`macOS ${macosMinimum}`);
    expect(result.html).not.toMatch(/update Safari/i);
  });
  it("names the same macOS floor the app bundle declares", () => {
    // Safari 15.4's engine shipped with macOS 12.3; the bundle refuses to
    // launch below it, and the message and the bundle must agree.
    expect(macosMinimum).toBe("12.3");
  });
  it("gives a non-Apple system a generic instruction", () => {
    const result = runCheck({ at: false, ua: "Mozilla/5.0 (Windows NT 10.0) Edg/90" });
    expect(result.html).toContain("Update your system's web view");
    expect(result.html).not.toContain("Safari (System");
  });
  it("stays ES5 so it parses on the engines it rejects, and runs before the app module", () => {
    const code = script
      .replace(/^\s*\/\/.*$/gm, "")
      .replace(/"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'/g, '""');
    expect(code).not.toMatch(/\b(const|let|class)\b|=>|`|\?\.|\?\?/);
    expect(index.indexOf('id="tine-engine-check"')).toBeLessThan(index.indexOf('type="module"'));
    const main = fs.readFileSync(path.join(root, "src/main.tsx"), "utf8");
    const stop = main.indexOf("__tineUnsupportedEngine");
    expect(stop).toBeGreaterThan(0);
    expect(stop).toBeLessThan(main.indexOf("installPlatformAttribute();\n"));
  });
  it("keeps Tine's own frontend source inside the Safari 15.4 floor", () => {
    // The floor above is only true while nothing newer is used. esbuild lowers
    // syntax but never polyfills a missing built-in, so a newer API here would
    // reintroduce GH #572 for every 15.4-17.3 engine. (pdf.js, a dependency, needs
    // Safari 17.4; only the PDF viewer is affected by that.)
    const newer = /Promise\.withResolvers|AbortSignal\.any|\.findLast(Index)?\(|\.to(Sorted|Reversed|Spliced)\(|(Object|Map)\.groupBy|Array\.fromAsync|\.isWellFormed\(|\(\?<[=!]/;
    const offenders: string[] = [];
    const walk = (dir: string) => {
      for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
        const file = path.join(dir, entry.name);
        if (entry.isDirectory()) walk(file);
        else if (/\.tsx?$/.test(entry.name) && !/\.test\.tsx?$/.test(entry.name)) {
          fs.readFileSync(file, "utf8").split("\n").forEach((line, i) => {
            if (newer.test(line)) offenders.push(`${path.relative(root, file)}:${i + 1}`);
          });
        }
      }
    };
    walk(path.join(root, "src"));
    expect(offenders, "an API newer than Safari 15.4 (GH #572); use an older equivalent").toEqual([]);
  });
});
