import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

// GH #610: the public `--tine-*` tokens are a promise to users' custom.css.
// docs/contracts/theme-tokens.md is the single source of truth. This guard
// keeps three things honest, each with a message that states the rule:
//   1. a documented public token is declared AND consumed by the shipped CSS;
//   2. a `--tine-*` name appearing in the code is documented (public or
//      internal), so no token can ship unannounced;
//   3. the Guide page's token table and control names say what the contract
//      and the Settings source say.
// Exemplar for the shape: src/editableEmojiFont.test.ts.

const CONTRACT = "docs/contracts/theme-tokens.md";
const GUIDE = "crates/tine-core/src/templates/customize-look.md";
const RULE = `Public theming tokens are a stability promise (${CONTRACT}): document a token there in the same commit that adds, renames or removes it, and add the CHANGELOG entry.`;

function walk(dir: string, accept: (file: string) => boolean, out: string[] = []): string[] {
  for (const name of readdirSync(dir)) {
    const full = path.join(dir, name);
    if (statSync(full).isDirectory()) walk(full, accept, out);
    else if (accept(full)) out.push(full);
  }
  return out;
}
const stripCssComments = (css: string) => css.replace(/\/\*[\s\S]*?\*\//g, "");
const stripTsComments = (ts: string) => ts.replace(/\/\*[\s\S]*?\*\//g, "").replace(/(^|[^:"'`])\/\/[^\n]*/g, "$1");
const TOKEN = /--tine-[a-z0-9]+(?:-[a-z0-9]+)*/g;

/** Token names in the first column of the markdown table under `heading`. */
function tableTokens(markdown: string, heading: RegExp): Set<string> {
  const start = markdown.search(heading);
  if (start < 0) throw new Error(`missing section ${heading}`);
  const rest = markdown.slice(start).split("\n");
  const body: string[] = [];
  for (const line of rest.slice(1)) {
    if (/^##\s/.test(line)) break;
    body.push(line);
  }
  const tokens = new Set<string>();
  for (const line of body) {
    const cell = line.match(/^\s*(?:-\s+)?\|\s*`(--tine-[a-z0-9-]+)`/);
    if (cell) tokens.add(cell[1]);
  }
  return tokens;
}

const contract = readFileSync(CONTRACT, "utf8");
const publicTokens = tableTokens(contract, /^## 2\. Public tokens/m);
const internalTokens = tableTokens(contract, /^## 3\. Internal/m);

const cssFiles = walk("src/styles", (f) => f.endsWith(".css"));
const cssText = cssFiles.map((f) => stripCssComments(readFileSync(f, "utf8"))).join("\n");
const declaredInCss = new Set([...cssText.matchAll(/(--tine-[a-z0-9-]+)\s*:/g)].map((m) => m[1]));
const usedInCss = new Set([...cssText.matchAll(/var\(\s*(--tine-[a-z0-9-]+)/g)].map((m) => m[1]));
const codeFiles = walk("src", (f) => /\.(ts|tsx)$/.test(f) && !/\.test\.tsx?$/.test(f) && !f.includes(`${path.sep}test${path.sep}`));
const inCode = new Set(codeFiles.flatMap((f) => [...stripTsComments(readFileSync(f, "utf8")).matchAll(TOKEN)].map((m) => m[0])));

describe("public theme tokens (GH #610)", () => {
  it("the contract documents a meaningful set", () => {
    expect(publicTokens.size).toBeGreaterThanOrEqual(10);
    expect(publicTokens.size).toBeLessThanOrEqual(20);
    for (const required of ["--tine-embed-bg", "--tine-embed-accent", "--tine-bullet-color", "--tine-content-width", "--tine-content-font", "--tine-font-size"]) {
      expect(publicTokens.has(required), `${required} is part of the brief`).toBe(true);
    }
    for (const token of publicTokens) expect(internalTokens.has(token), `${token} cannot be both public and internal`).toBe(false);
  });

  it("every documented public token is declared by, and consumed by, the shipped CSS", () => {
    const undeclared = [...publicTokens].filter((t) => !declaredInCss.has(t));
    const unused = [...publicTokens].filter((t) => !usedInCss.has(t));
    expect(undeclared, `${RULE} Declare each documented token (unset: \`initial\`) in src/styles/theme.css; exemplar --tine-embed-bg.`).toEqual([]);
    expect(unused, `${RULE} A documented token must change something: read it with var(--tine-x, <previous value>) in the stylesheet that draws it.`).toEqual([]);
  });

  it("every --tine-* name in the code is documented as public or internal", () => {
    const documented = new Set([...publicTokens, ...internalTokens]);
    const names = new Set([...declaredInCss, ...usedInCss, ...inCode]);
    const undocumented = [...names].filter((t) => !documented.has(t)).sort();
    expect(undocumented, `${RULE} Undocumented: ${undocumented.join(", ")}.`).toEqual([]);
    const stale = [...internalTokens].filter((t) => !names.has(t));
    expect(stale, `${CONTRACT} lists internal variables that no longer exist: delete the rows.`).toEqual([]);
  });

  it("the Guide page lists exactly the public tokens, and names the controls Settings shows", () => {
    const guide = readFileSync(GUIDE, "utf8");
    expect([...tableTokens(guide, /^- ## 3\. The supported names/m)].sort()).toEqual([...publicTokens].sort());
    const settings = readFileSync("src/components/CustomCssSettings.tsx", "utf8");
    for (const control of ["Edit custom.css", "Disable custom CSS", "Developer tools"]) {
      expect(settings, `Settings must show the control the Guide names: ${control}`).toContain(`label="${control}"`);
      expect(guide, `the Guide must name the Settings control ${control}`).toContain(`**${control}**`);
    }
  });

  it("the contract's test list names files (and Rust functions) that exist", () => {
    const section = contract.split(/^## Tests$/m)[1] ?? "";
    const entries = [...section.matchAll(/^- (\S+?)(?:::(\w+))?(?: \(.*\))?$/gm)];
    expect(entries.length).toBeGreaterThanOrEqual(6);
    for (const [, file, fn] of entries) {
      expect(existsSync(file), `${CONTRACT} lists ${file}, which does not exist`).toBe(true);
      if (fn) expect(readFileSync(file, "utf8"), `${file} has no ${fn}`).toMatch(new RegExp(`fn ${fn}\\b`));
    }
  });
});
