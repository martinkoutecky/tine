// Ratchet (D-20261102-PNS, Discussion #825): every property key Tine writes on
// its own is `tine.<name>`, or an OG-owned key Tine writes with OG's meaning.
// A Tine key without the prefix collides with users' own properties (`quote::`
// on book notes became a margin comment; `author::` became an author chip).
//
// The scan reads the write sites of the window (TypeScript) and the native side
// (Rust production code, test modules cut): the property writers
// (`setBlockProperty`, `setPageProperty`, an `editBlock` property op,
// `Edit::Property`, `pdf.rs` `property_line`, `appendQueryBlock`'s properties),
// `${KEY}:: ` / `#+${KEY}: ` marker templates, and `key:: ` / `:key: ` lines in
// string literals. A key spelled as a literal or as a named constant is checked.
// A key the site receives from elsewhere (the user's own property name, a table
// of `tine.*` keys) is a pass-through: each such site is reviewed and counted
// below, so a new one fails until someone looks at it.
import { describe, expect, it } from "vitest";
import fs from "node:fs";
import path from "node:path";

const ROOT = path.resolve(__dirname, "..");

/** OG-owned keys Tine writes with OG's meaning, each with its OG owner. */
const OG_OWNED: Record<string, string> = {
  id: "OG block identity, graph-parser block.cljs (`id::` uuid)",
  collapsed: "OG fold state, handler/editor.cljs set-blocks-collapsed!",
  title: "OG page title, graph-parser property.cljs (page rename rewrites it)",
  alias: "OG page aliases, graph-parser extract.cljc (:alias properties)",
  aliases: "OG built-in property, graph-parser property.cljs; accepted alias variant in Tine (D-20261018-M2, K-Q1)",
  public: "OG publishing flag, publishing/html.cljs (`public:: false`)",
  filters: "OG linked-reference filters, page property `filters`",
  template: "OG templates, handler/editor.cljs (`template::` names a template)",
  "template-including-parent": "OG templates, handler/editor.cljs insert-template!",
  "background-color": "OG block background, built-in property (ui/colors)",
  "query-properties": "OG query table columns, built-in `query-properties`",
  "logseq.order-list-type": "OG numbered lists, handler/editor.cljs",
  "hl-page": "OG PDF highlight, extensions/pdf (pdf/assets.cljs)",
  "hl-color": "OG PDF highlight, extensions/pdf (pdf/assets.cljs)",
  "hl-type": "OG PDF area highlight, extensions/pdf (pdf/assets.cljs)",
  "hl-stamp": "OG PDF area highlight image stamp, extensions/pdf",
  "ls-type": "OG PDF annotation block marker (`ls-type:: annotation`)",
  file: "OG PDF highlights page (`hls__…`) pre-block, pdf/assets.cljs ensure-ref-page!",
  "file-path": "OG PDF highlights page (`hls__…`) pre-block, pdf/assets.cljs ensure-ref-page!",
};

/** Reviewed pass-through sites: `file` -> how many, and why each key is not Tine's choice. */
const PASS_THROUGH: Record<string, [number, string]> = {
  "crates/lsdoc-wasm/src/lib.rs": [1, "the wasm bridge of the one property writer: the window's key"],
  "src-tauri/src/owner/properties.rs": [1, "`tine prop set BLOCK key value`: the key the CLI user names"],
  "crates/tine-outline/src/page_property_edit.rs": [1, "the shared page-property form/owner writer: the user's key"],
  "src/components/ContextMenu.tsx": [2, "page icon: clears/sets `tine.icon` or OG `icon` (PAGE_ICON_PROPERTIES, D-20261018-IC1)"],
  "src/components/FormulaEditor.tsx": [2, "`tine.formula.<name>` for the user's formula name"],
  "src/components/Macro.tsx": [3, "query view keys from its `tine.*` writes table and displayPropertyPatch"],
  "src/components/PageProps.tsx": [2, "the properties form writes the key the user typed"],
  "src/components/QuerySheet.tsx": [1, "the query builder's property test names the user's key"],
  "src/components/QueryVocabularyPicker.tsx": [2, "the query builder's property test names the user's key"],
  "src/components/SheetTable.tsx": [2, "removes a `tine.formula.*` / `tine.fields` home key"],
  "src/document/convert.ts": [1, "queryBlockRaw: keys from appendQueryBlock callers (checked there)"],
  "src/document/edits/identity.ts": [1, "Org identity writer: `id`/`custom-id` from its caller"],
  "src/document/edits/paste.ts": [2, "paste keeps the pasted block's own properties"],
  "src/document/edits/properties.ts": [5, "the generic writers themselves (setBlockProperty, setPageProperty, toggle)"],
  "src/document/edits/propertyDeclaration.ts": [1, "property declaration page: the user's key"],
  "src/editor/editorCompletions.ts": [1, "both editors' property autocomplete inserts the key the user picked"],
  "src/editor/exportProjection.ts": [1, "source export restores only parser-owned comment presentation keys when Remove properties is selected"],
  "src/render/renderedText.ts": [1, "copy-as-text prints the block's own properties"],
  "src/sheet/conversions.ts": [1, "clears TINE_SHEET_PROPS, all `tine.*`"],
  "src/sheet/fields.ts": [1, "a sheet cell edit writes the column's user property"],
};

function walk(dir: string, keep: (file: string) => boolean, out: string[] = []): string[] {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const file = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      if (["target", "node_modules", "tests", "wasm", "devtools", "fixtures", "gen", "benches", "examples"].includes(entry.name)
        || entry.name.endsWith("_tests")) continue;
      walk(file, keep, out);
    } else if (keep(file)) out.push(file);
  }
  return out;
}

/** Drop comments, keeping line structure. */
function stripComments(text: string): string {
  return text.replace(/\/\*[\s\S]*?\*\//g, (m) => m.replace(/[^\n]/g, " ")).replace(/^\s*\/\/.*$/gm, "");
}

/** Remove `#[cfg(test)]` items (modules and functions): brace-matched, skipping
 *  string and `'{'` / `'}'` char literals. */
function stripRustTests(text: string): string {
  let out = text;
  for (let at = out.indexOf("#[cfg(test)]"); at !== -1; at = out.indexOf("#[cfg(test)]", at)) {
    let i = at + "#[cfg(test)]".length;
    while (i < out.length && out[i] !== "{" && out[i] !== ";") i++;
    if (out[i] === "{") {
      let depth = 0;
      for (; i < out.length; i++) {
        const c = out[i];
        if (c === '"') { for (i++; i < out.length && out[i] !== '"'; i++) if (out[i] === "\\") i++; continue; }
        if (c === "'" && (out.slice(i, i + 3) === "'{'" || out.slice(i, i + 3) === "'}'")) { i += 2; continue; }
        if (c === "{") depth++;
        if (c === "}" && --depth === 0) break;
      }
    }
    out = out.slice(0, at) + out.slice(i + 1);
  }
  return out;
}

type Site = { file: string; kind: string; expr: string };

const KEY = String.raw`[A-Za-z][\w.\-/]*`;

function tsSites(): { sites: Site[]; consts: Map<string, string> } {
  const files = walk(path.join(ROOT, "src"), (f) => /\.(ts|tsx)$/.test(f) && !/\.test\.|\.d\.ts$|messages\.gen|[/\\]mock[^/\\]*$/.test(f));
  const sites: Site[] = [];
  const consts = new Map<string, string>();
  const patterns: [RegExp, string][] = [
    [/setBlockProperty\(\s*[^,()]+(?:\([^()]*\))?,\s*([^,]+?)\s*,/g, "setBlockProperty"],
    [/setPageProperty\(\s*[^,()]+(?:\([^()]*\))?,\s*([^,]+?)\s*,/g, "setPageProperty"],
    [/kind:\s*"property",\s*key(?:\s*:\s*([^,}]+))?/g, "property edit"],
    [/appendQueryBlock\([^;]*?\[\[\s*([^,\]]+)\s*,/g, "appendQueryBlock"],
    [/\$\{([^}]+)\}:: /g, "md template"],
    [/[#:]\+?\$\{([^}]+)\}: /g, "org template"],
    [new RegExp(String.raw`(?:["'\x60]|\\n)(${KEY}):: `, "g"), "md literal"],
    [new RegExp(String.raw`(?:["'\x60]|\\n)(?:#\+|:)(${KEY}): `, "g"), "org literal"],
  ];
  for (const file of files) {
    const text = stripComments(fs.readFileSync(file, "utf8"));
    const rel = path.relative(ROOT, file).split(path.sep).join("/");
    for (const m of text.matchAll(/(?:export\s+)?const\s+([A-Z][A-Z0-9_]*)\s*(?::\s*string)?\s*=\s*("[^"]*"|[A-Z][A-Z0-9_]*)\s*;/g)) consts.set(m[1], m[2]);
    for (const [re, kind] of patterns) for (const m of text.matchAll(re)) {
      const literal = kind.endsWith("literal");
      sites.push({ file: rel, kind, expr: literal ? JSON.stringify(m[1]) : (m[1] ?? "key").trim() });
    }
  }
  return { sites, consts };
}

function rustSites(): { sites: Site[]; consts: Map<string, string> } {
  const dirs = [
    ...fs.readdirSync(path.join(ROOT, "crates")).map((c) => path.join(ROOT, "crates", c, "src")),
    path.join(ROOT, "src-tauri", "src"),
  ].filter((d) => fs.existsSync(d));
  const files = dirs.flatMap((d) => walk(d, (f) => f.endsWith(".rs") && !/_tests?\.rs$|[/\\]tests\.rs$/.test(f)));
  const sites: Site[] = [];
  const consts = new Map<string, string>();
  const patterns: [RegExp, string][] = [
    [/Edit::Property\s*\{\s*key:\s*([^,]+?)(?:\.into\(\)|\.to_string\(\)|\.to_owned\(\))?\s*,/g, "Edit::Property"],
    [/pub fn set\([^)]*\b(key):\s*&str/g, "page property writer"],
    [/\bproperty_line\(\s*("[^"]*")\s*,/g, "pdf property_line"],
    [new RegExp(String.raw`(?:"|\\n)(${KEY}):: `, "g"), "md literal"],
    [new RegExp(String.raw`(?:"|\\n)(?:#\+|:)(${KEY}): `, "g"), "org literal"],
  ];
  for (const file of files) {
    const text = stripRustTests(stripComments(fs.readFileSync(file, "utf8")));
    const rel = path.relative(ROOT, file).split(path.sep).join("/");
    for (const m of text.matchAll(/pub(?:\([a-z]+\))?\s+const\s+([A-Z][A-Z0-9_]*)\s*:\s*&str\s*=\s*("[^"]*")\s*;/g)) consts.set(m[1], m[2]);
    for (const [re, kind] of patterns) for (const m of text.matchAll(re)) {
      const literal = kind.endsWith("literal");
      sites.push({ file: rel, kind, expr: literal ? JSON.stringify(m[1]) : m[1].trim() });
    }
  }
  return { sites, consts };
}

/** The key a site writes, or null when it is a pass-through. */
function resolve(expr: string, consts: Map<string, string>): string | null {
  let current = expr.split("::").pop()!.trim();
  for (let hops = 0; hops < 5; hops++) {
    if (/^"[^"]*"$/.test(current)) return JSON.parse(current);
    if (!/^[A-Z][A-Z0-9_]*$/.test(current)) return null;
    const next = consts.get(current);
    if (next === undefined) throw new Error(`cannot resolve the constant ${current}`);
    current = next;
  }
  throw new Error(`constant chain too long at ${expr}`);
}

function violations(): { bad: string[]; passThrough: Map<string, number> } {
  const bad: string[] = [];
  const passThrough = new Map<string, number>();
  for (const { sites, consts } of [tsSites(), rustSites()]) {
    for (const site of sites) {
      const key = resolve(site.expr, consts);
      if (key === null) {
        passThrough.set(site.file, (passThrough.get(site.file) ?? 0) + 1);
        continue;
      }
      const norm = key.trim().toLowerCase();
      if (norm.startsWith("tine.") || norm.startsWith("tine.formula.") || OG_OWNED[norm]) continue;
      bad.push(`${site.file} (${site.kind}) writes \`${key}::\`: use \`tine.${key}\`, or add an OG-owned key with its OG owner`);
    }
  }
  return { bad, passThrough };
}

describe("Tine writes only tine.* keys or OG-owned keys (D-20261102-PNS)", () => {
  it("every literal or named key Tine writes is tine.* or OG-owned", () => {
    expect(violations().bad).toEqual([]);
  });

  it("every pass-through write site is reviewed", () => {
    const { passThrough } = violations();
    const seen = Object.fromEntries([...passThrough].sort());
    const reviewed = Object.fromEntries(Object.entries(PASS_THROUGH).map(([file, [count]]) => [file, count]).sort());
    expect(seen).toEqual(reviewed);
  });

  it("the scan sees the writers it guards (a stray write would fail)", () => {
    const { sites: ts, consts: tsConsts } = tsSites();
    const { sites: rs, consts: rsConsts } = rustSites();
    const keys = new Set([
      ...ts.map((s) => resolve(s.expr, tsConsts)),
      ...rs.map((s) => resolve(s.expr, rsConsts)),
    ]);
    for (const key of ["tine.quote", "tine.quote-prefix", "tine.quote-suffix", "tine.author", "tine.export",
      "tine.exports", "tine.comments", "tine.favorites", "tine.journal-sections", "tine.share", "tine.view", "public", "collapsed", "id", "hl-page"]) {
      expect(keys, key).toContain(key);
    }
    for (const old of ["quote", "author", "export", "tine/exports", "tine/favorites", "tine/journal-sections"]) {
      expect(keys, old).not.toContain(old);
    }
  });
});
