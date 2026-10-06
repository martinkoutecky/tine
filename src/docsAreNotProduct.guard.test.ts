// Two mechanisms already assume a `docs/` edit cannot change the shipped
// binary, and Rust has a way to make that false in one line.
//
//   - `scripts/build-e2e-inputs.mjs` excludes `docs/**` from the E2E build-input
//     digest, so a docs edit does not mark the tree dirty.
//   - `tine-coordination integrate --test-only` waives the release build for a
//     commit confined to non-product paths, and `docs/` is one of them.
//
// Both are true today only because every `include_str!`/`include_bytes!` that
// reaches into `docs/` sits behind `#[cfg(test)]` — five of them do, pinning
// contract documents to the code they describe, which is a house pattern worth
// keeping. A sixth, written outside a test module, would make a documentation
// edit change `cargo build --release` output while both mechanisms went on
// insisting it could not. Nothing else in the repository would notice.
//
// So the rule is not "don't embed docs". It is: embed them from test code only.
import { describe, expect, it } from "vitest";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), "..");
// The root READMEs, CHANGELOG.md, CONTRIBUTING.md, the agent pointers
// AGENTS.md / CLAUDE.md, and website/ are waived too:
// `--test-only` skips the build for them and `--docs-only` skips every gate.
const EXCLUDED_FROM_BUILD_INPUTS =
  /include_(?:str|bytes)!\s*\(\s*"([^"]*(?:docs\/|website\/|src-tauri\/gen\/schemas\/|(?:README[^"\/]*|CHANGELOG|CONTRIBUTING|AGENTS|CLAUDE)\.md(?="))[^"]*)"/g;

// The frontend can make the same mistake in one line: Vite bundles any file a
// module imports (`import notes from "../CHANGELOG.md?raw"`).
const FRONTEND_DOCS_IMPORT =
  /(?:\bfrom\s*|\bimport\s*\(?\s*)["']([^"']*(?:\/docs\/|\/website\/|(?:README[^"'\/]*|CHANGELOG|CONTRIBUTING|AGENTS|CLAUDE)\.md)[^"']*)["']/g;

export function frontendDocsImports(source: string): string[] {
  return [...source.matchAll(FRONTEND_DOCS_IMPORT)].map((match) => match[1]!);
}

/** Byte ranges of every `#[cfg(test)]`-attributed `mod … { … }` block. */
export function testModuleRanges(source: string): [number, number][] {
  const ranges: [number, number][] = [];
  for (const match of source.matchAll(/#\[cfg\(test\)\][\s\S]{0,200}?\bmod\s+\w+\s*\{/g)) {
    let depth = 0;
    let at = match.index! + match[0].length - 1;
    for (; at < source.length; at += 1) {
      if (source[at] === "{") depth += 1;
      else if (source[at] === "}" && (depth -= 1) === 0) break;
    }
    ranges.push([match.index!, at]);
  }
  return ranges;
}

/** Embeds of a build-input-excluded file that are NOT inside a test module. */
export function productionEmbeds(source: string): string[] {
  const ranges = testModuleRanges(source);
  const found: string[] = [];
  for (const match of source.matchAll(EXCLUDED_FROM_BUILD_INPUTS)) {
    const inTest = ranges.some(([start, end]) => match.index! > start && match.index! < end);
    if (!inTest) found.push(match[1]!);
  }
  return found;
}

/** Files that are themselves compiled only under `cfg(test)` via `#[path]`. */
function testOnlyModuleFiles(sources: Map<string, string>): Set<string> {
  const testOnly = new Set<string>();
  for (const [file, source] of sources) {
    for (const match of source.matchAll(/#\[cfg\(test\)\]\s*#\[path\s*=\s*"([^"]+)"\]/g)) {
      testOnly.add(path.resolve(path.dirname(file), match[1]!));
    }
  }
  // A module declared from a test-only file is test-only too, whatever its
  // own declaration says (model_tests.rs declares its submodules by #[path]).
  for (let grew = true; grew; ) {
    grew = false;
    for (const file of [...testOnly]) {
      const source = sources.get(file);
      if (source === undefined) continue;
      for (const match of source.matchAll(/#\[path\s*=\s*"([^"]+)"\]/g)) {
        const child = path.resolve(path.dirname(file), match[1]!);
        if (!testOnly.has(child)) {
          testOnly.add(child);
          grew = true;
        }
      }
    }
  }
  return testOnly;
}

function frontendSources(dir: string, into: Map<string, string>): Map<string, string> {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) frontendSources(full, into);
    else if (/\.(?:ts|tsx|js|mjs)$/.test(entry.name) && !/\.test\.|\.guard\./.test(entry.name)) {
      into.set(full, fs.readFileSync(full, "utf8"));
    }
  }
  return into;
}

function rustSources(dir: string, into: Map<string, string>): Map<string, string> {
  if (!fs.existsSync(dir)) return into;
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) rustSources(full, into);
    else if (entry.isFile() && entry.name.endsWith(".rs")) into.set(full, fs.readFileSync(full, "utf8"));
  }
  return into;
}

describe("docs are not a product input", () => {
  const sources = new Map<string, string>();
  rustSources(path.join(root, "crates"), sources);
  rustSources(path.join(root, "src-tauri", "src"), sources);
  const testOnlyFiles = testOnlyModuleFiles(sources);

  it("finds the Rust sources it is supposed to scan", () => {
    expect(sources.size).toBeGreaterThan(50);
    // If this empties, the guard passes while proving nothing.
    expect([...sources.values()].filter((source) => /include_str!/.test(source)).length).toBeGreaterThan(5);
  });

  it("detects the shape it exists to catch", () => {
    const regression = [
      'const CONTRACT: &str = include_str!("../../../docs/storage-sync-contract.md");',
      "#[cfg(test)]",
      "mod tests {",
      '    let ok = include_str!("../../../docs/contracts/direct-move-recovery.md");',
      "}",
    ].join("\n");
    expect(productionEmbeds(regression)).toEqual(["../../../docs/storage-sync-contract.md"]);
  });

  it("detects a production embed of the root README", () => {
    expect(productionEmbeds('const ABOUT: &str = include_str!("../../../README.md");')).toEqual(["../../../README.md"]);
  });

  it("detects production embeds of the other docs-only paths", () => {
    expect(productionEmbeds('const C: &str = include_str!("../../../CHANGELOG.md");')).toEqual(["../../../CHANGELOG.md"]);
    expect(productionEmbeds('const W: &str = include_str!("../../../website/index.html");')).toEqual([
      "../../../website/index.html",
    ]);
  });

  it("detects a frontend import of a docs-only path", () => {
    expect(frontendDocsImports('import notes from "../CHANGELOG.md?raw";')).toEqual(["../CHANGELOG.md?raw"]);
    expect(frontendDocsImports('const f = await import("../website/app.js");')).toEqual(["../website/app.js"]);
    expect(frontendDocsImports('import { x } from "./docsHelpers";')).toEqual([]);
  });

  it("no frontend module imports a docs-only path", () => {
    const frontend = frontendSources(path.join(root, "src"), new Map());
    expect(frontend.size).toBeGreaterThan(50);
    const offenders = [...frontend].flatMap(([file, source]) =>
      frontendDocsImports(source).map((spec) => `${path.relative(root, file)} -> ${spec}`),
    );
    expect(
      offenders,
      "docs/, website/, root README*.md, CHANGELOG.md, CONTRIBUTING.md, AGENTS.md and CLAUDE.md are integrated with " +
        "`tine-coordination integrate --docs-only` (no gates, no build); importing one into the " +
        "frontend would ship it unbuilt. Link to it instead (see src/components/AboutTab.tsx).",
    ).toEqual([]);
  });

  it("treats a module declared from a test-only file as test-only, and only that", () => {
    const synthetic = new Map([
      ["/r/src/model.rs", '#[cfg(test)]\n#[path = "model_tests.rs"]\nmod tests;\n#[path = "shipped.rs"]\nmod shipped;'],
      ["/r/src/model_tests.rs", '#[path = "model_contract_tests.rs"]\nmod contract;'],
      ["/r/src/model_contract_tests.rs", ""],
      ["/r/src/shipped.rs", ""],
    ]);
    expect([...testOnlyModuleFiles(synthetic)].sort()).toEqual([
      "/r/src/model_contract_tests.rs",
      "/r/src/model_tests.rs",
    ]);
  });

  it("accepts the house pattern: a contract pinned from a test module", () => {
    const fixed = [
      "#[cfg(test)]",
      "mod tests {",
      "    fn nested() { if true { } }",
      '    let contract = include_str!("../../../docs/storage-sync-contract.md");',
      "}",
    ].join("\n");
    expect(productionEmbeds(fixed)).toEqual([]);
  });

  for (const [file, source] of sources) {
    if (!EXCLUDED_FROM_BUILD_INPUTS.test(source)) continue;
    EXCLUDED_FROM_BUILD_INPUTS.lastIndex = 0;
    const relative = path.relative(root, file);
    it(`${relative}: embeds build-input-excluded files only from test code`, () => {
      const embeds = testOnlyFiles.has(file) ? [] : productionEmbeds(source);
      expect(
        embeds,
        `${relative} embeds ${embeds.join(", ")} outside #[cfg(test)]. Those paths are excluded from `
          + "the E2E build-input digest (scripts/build-e2e-inputs.mjs) and waived by "
          + "`tine-coordination integrate --test-only`, both of which assert that editing them cannot "
          + "change the shipped binary — which this makes false. Move the embed into a #[cfg(test)] "
          + "module (see crates/tine-core/src/concord_ledger.rs), or remove docs/ from both exclusions.",
      ).toEqual([]);
    });
  }
});
