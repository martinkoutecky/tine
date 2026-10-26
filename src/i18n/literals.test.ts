import { describe, expect, it } from "vitest";
// @ts-expect-error: plain ESM build script, no types.
import { compare, identity, readBaseline, scan, scanSource } from "../../scripts/i18n-literals.mjs";

type Site = { file: string; kind: string; owner: string; text: string; line: number };

const violationsOf = (text: string, rel = "src/x.ts") => (scanSource(rel, text) as { violations: string[] }).violations;
const sitesOf = (text: string, rel = "src/components/X.tsx") => (scanSource(rel, text) as { sites: Site[] }).sites;

// tine#107 G4 (DESIGN rev. 2 #4, #6): no NEW English reaches the UI outside the
// catalog, and nothing resolves a message at module load.
describe("the localization ratchet", () => {
  it("does not mark template substitutions as already reported literal text", () => {
    const sites = sitesOf(`export const Review = () => <>
      <button title={\`Action: \${flag ? "Collapse" : "Expand"}\`} />
    </>`);
    expect(sites.map(site => site.text)).toEqual(expect.arrayContaining(["Collapse", "Expand"]));
  });

  it.each([
    "heading() || ''", "heading() ?? ''", "flag ? heading() : ''",
    "flag ? 'Fallback heading' : heading()", "`Review: ${heading()}`", "'Review: ' + heading()",
  ])("follows the dynamic part of %s at every UI sink", (expression) => {
    for (const sink of [
      `<h1>{${expression}}</h1>`,
      `<h1 title={${expression}} />`,
      `({ label: ${expression} })`,
    ]) {
      const sites = sitesOf(`
        const heading = () => flag ? "Two versions arrived" : "Your edits on disk";
        export const Review = () => ${sink};
      `);
      expect(sites.map(site => site.text).join("\n"), sink).toContain("Two versions arrived");
      expect(sites.map(site => site.text).join("\n"), sink).toContain("Your edits on disk");
    }
  });

  it("follows helper results, branch bodies, and displayed helper arguments without treating syntax as prose", () => {
    const sites = sitesOf(`
      const heading = () => flag ? "Two versions arrived" : "Your edits on disk";
      const button = (decision, label) => <button data-decision={decision}>{label}</button>;
      export const Review = () => <h1>{heading()}{button("mine", "Keep this")}</h1>;
    `);
    expect(sites.map(site => site.text).sort()).toEqual(["Keep this", "Two versions arrived", "Your edits on disk"]);
    const helper = sitesOf(`export function querySentence(q) { return q.empty ? "All blocks" : "Blocks where"; }`, "src/editor/x.ts");
    expect(helper.map(site => site.text).sort()).toEqual(["All blocks", "Blocks where"]);
  });

  it("follows memo fields, constant vocabulary and render callbacks, and checks component text props", () => {
    const sites = sitesOf(`
      const WORDS = { empty: "No blocks", busy: "Reading blocks" };
      const heading = createMemo(() => ({ mine: WORDS[state], code: "internal-enum" }));
      const result = { notice: "No matching rows" };
      export const Review = () => <Field label="Review versions" hint="Choose a side">
        <h1 aria-valuetext={heading().mine}>{heading().mine}</h1>
        <Show when={date}>{value => \`Last scan: \${value()}\`}</Show>
      </Field>;
    `);
    expect(sites.map(site => site.text).sort()).toEqual(["Choose a side", "Last scan: {}", "No blocks", "No matching rows", "Reading blocks", "Review versions"]);
  });
  it("GH #107: traces query validation returns and alert setters without treating query syntax as prose", () => {
    const sites = sitesOf(`
      export function friendlySearchToDsl() { return { dsl: "(task TODO)", error: "Include a search term." }; }
    `, "src/editor/searchQuery.ts");
    expect(sites.map(site => site.text)).toEqual(["Include a search term."]);
    const alerts = sitesOf(`
      export function QueryBuilder() { setError("Edit options with Display."); setSaveNotice(\`Saved \${name}.\`); }
    `, "src/components/QueryBuilder.tsx");
    expect(alerts.map(site => site.text).sort()).toEqual(["Edit options with Display.", "Saved {}."]);
  });

  it("finds no literal UI text outside the baseline, no stale baseline entry, and no module-level t()", () => {
    const { sites, violations } = scan() as { sites: Site[]; violations: string[] };
    expect(violations).toEqual([]);
    const { added, gone } = compare(sites, readBaseline()) as { added: Site[]; gone: string[] };
    expect(added.map((site) => `${site.file}:${site.line} ${site.kind} ${site.text}`),
      "new literal UI text: put it in src/i18n/locales/en and use t()/msg()").toEqual([]);
    expect(gone, "converted sites: run node scripts/i18n-literals.mjs --update").toEqual([]);
    // A whole-tree TypeScript parse: seconds, more on a loaded machine.
  }, 60_000);

  it("counts JSX text, UI attributes, toasts, table labels and dialogs, by identity", () => {
    const { sites } = scanSource("src/components/Example.tsx", `
      import { msg, t } from "../i18n";
      import { pushToast } from "../toasts";
      const ROWS = [{ id: "a", label: "Open the thing" }, { id: "b", label: msg("command-go-search") }];
      export function Example() {
        pushToast(\`Could not save \${name}.\`, "error");
        pushToast(String(error), "error");
        pushToast(msg("toast-copied"), "success");
        if (!confirm("Delete it?")) return null;
        return <button title="Close the pane" aria-label={t("toast-dismiss")}>Close {name}</button>;
      }
    `) as { sites: Site[] };
    expect(sites.map((site) => `${site.kind} ${site.text}`)).toEqual([
      "prop:label Open the thing",
      "toast Could not save {}.",
      "toast {String(error)}",
      "dialog:confirm Delete it?",
      "attr:title Close the pane",
      "jsx-text Close",
    ]);
    // The same text twice in one file is two sites; one baseline entry covers one.
    const twice = [...sites, sites[0]];
    expect(compare(twice, sites.map(identity)).added).toHaveLength(1);
  });

  it("rejects resolving a message at module level, and allows it inside a function", () => {
    const { violations } = scanSource("src/x.ts", `
      import { t, render as show } from "./i18n";
      const LABEL = t("toast-copy");
      const OTHER = show(m);
      export const later = () => t("toast-copy");
      export class C { label = t("toast-copy"); }
    `) as { violations: string[] };
    expect(violations).toHaveLength(2);
    expect(violations[0]).toContain("src/x.ts:3");
  });

  // Finding 5 (REVIEW-CODE-1): the rule is "the nearest function is not invoked at load",
  // not "has a function ancestor".
  describe("load-time resolution", () => {
    const head = `import { t, render } from "./i18n";\n`;
    it.each([
      ["an eager map", `export const labels = ids.map(id => t(id));`],
      ["an eager map written with a function expression", `export const labels = ids.map(function (id) { return t(id); });`],
      ["forEach", `ids.forEach(id => { t(id); });`],
      ["filter", `export const kept = ids.filter(id => t(id) !== "");`],
      ["reduce", `export const joined = ids.reduce((acc, id) => acc + t(id), "");`],
      ["flatMap", `export const flat = ids.flatMap(id => [t(id)]);`],
      ["Object.fromEntries over a map", `export const table = Object.fromEntries(ids.map(id => [id, t(id)]));`],
      ["a nested eager callback", `export const nested = ids.map(id => others.map(o => t(o)));`],
      ["an IIFE", `export const label = (() => t("x"))();`],
      ["a new Promise executor", `export const p = new Promise(resolve => resolve(t("x")));`],
      ["a callback to an arbitrary module-level call", `register(() => t("x"));`],
      ["a declared function invoked at load", `function label() { return t("x"); }\nexport const l = label();`],
      ["a declared function passed to an eager map", `function label(id) { return t(id); }\nexport const ls = ids.map(label);`],
      ["a const arrow invoked at load", `const label = () => t("x");\nexport const l = label();`],
      ["a static field", `export class C { static label = t("x"); }`],
      ["a static block", `export class C { static { C.label = t("x"); } }`],
      ["an instance field of a class constructed at load", `class C { label = t("x"); }\nexport const c = new C();`],
      ["a plain module-level call", `export const label = t("x");`],
      ["a state initializer", `export const [s] = createSignal(t("x"));`],
      // Finding E (REVIEW-CODE-2): a static helper invoked while the module loads.
      ["a static helper called by a static field", `class Labels { static copy() { return t("toast-copy"); } static row = Labels.copy(); }`],
      ["a static helper called at module level", `class Labels { static copy() { return t("toast-copy"); } }\nexport const label = Labels.copy();`],
      ["this.copy() in a static block", `class Labels { static copy() { return t("x"); } static { Labels.row = this.copy(); } }`],
      ["this.copy() in a static field", `class Labels { static copy() { return t("x"); } static row = this.copy(); }`],
      ["this.copy() inside an arrow in a static field", `class Labels { static copy() { return t("x"); } static row = (() => this.copy())(); }`],
      ["a static helper called through another static helper", `class Labels { static copy() { return t("x"); } static all() { return [this.copy()]; } }\nexport const rows = Labels.all();`],
      ["a static helper passed to an eager map", `class Labels { static copy(id) { return t(id); } }\nexport const rows = ids.map(Labels.copy);`],
      ["a static helper named by a string index", `class Labels { static copy() { return t("x"); } }\nexport const label = Labels["copy"]();`],
      ["a static getter read at load", `class Labels { static get copy() { return t("x"); } }\nexport const label = Labels.copy;`],
      ["a static helper of a class expression", `const Labels = class { static copy() { return t("x"); } };\nexport const label = Labels.copy();`],
    ])("rejects %s", (_name, body) => {
      expect(violationsOf(head + body), body).toHaveLength(1);
    });

    it("rejects a namespace import, for t and for render", () => {
      const ns = `import * as i18n from "./i18n";\n`;
      expect(violationsOf(`${ns}export const row = { label: i18n.t("toast-copy") };`)).toHaveLength(1);
      expect(violationsOf(`${ns}export const row = { label: i18n.render(m) };`)).toHaveLength(1);
      expect(violationsOf(`${ns}export const labels = ids.map(id => i18n.t(id));`)).toHaveLength(1);
      expect(violationsOf(`${ns}export const row = { label: () => i18n.t("x") };`)).toEqual([]);
      expect(violationsOf(`${ns}export const row = { label: other.t("x") };`), "only the i18n namespace counts").toEqual([]);
    });

    it.each([
      ["a function stored unevaluated in a table", `export const ROWS = [{ label: () => t("x") }];`],
      ["a function declaration", `export function label() { return t("x"); }`],
      ["a declared function that is never invoked at load", `function label() { return t("x"); }\nexport const f = [label];`],
      ["a component body", `export function C() { const ids = [1]; return ids.map(id => t(id)); }`],
      ["an eager map inside a function", `export const f = () => ids.map(id => t(id));`],
      ["a method", `export class C { label() { return t("x"); } }`],
      ["a static helper only called from a function", `class Labels { static copy() { return t("x"); } }\nexport const f = () => Labels.copy();`],
      ["a static helper only called from a component", `class Labels { static copy() { return t("x"); } }\nexport function C() { return h("b", Labels.copy()); }`],
      ["a static helper only stored unevaluated", `class Labels { static copy() { return t("x"); } }\nexport const ROWS = [{ run: Labels.copy }];`],
      ["a static helper called from an instance method", `class Labels { static copy() { return t("x"); } label() { return Labels.copy(); } }\nexport const l = new Labels();`],
      ["a static helper called by a static method never invoked at load", `class Labels { static copy() { return t("x"); } static all() { return this.copy(); } }\nexport const f = () => Labels.all();`],
      ["a static getter only read from a function", `class Labels { static get copy() { return t("x"); } }\nexport const f = () => Labels.copy;`],
      ["an instance method of another class with the same name", `class A { static copy() { return t("x"); } }\nclass B { copy() { return 1; } }\nexport const l = new B().copy();`],
      ["an instance field of a class never constructed at load", `export class C { label = t("x"); }`],
      ["createMemo", `export const label = createMemo(() => t("x"));`],
      ["createEffect", `createEffect(() => { document.title = t("x"); });`],
      ["createRenderEffect", `createRenderEffect(() => { document.title = t("x"); });`],
      ["createComputed", `export const label = createComputed(() => t("x"));`],
      ["a handler argument of a JSX attribute", `export const C = () => <button onClick={() => t("x")} />;`],
      ["a resolver that is not ours", `const t = (s) => s;\nexport const label = t("x");`],
    ])("allows %s", (_name, body) => {
      const imports = body.includes("const t =") ? `` : head;
      expect(violationsOf(imports + body), body).toEqual([]);
    });
  });

  // Finding 6 (REVIEW-CODE-1): the ratchet's UI-site scope.
  describe("UI-site soundness", () => {
    it("counts lower-case words at UI sinks, and fragment expression children", () => {
      const sites = sitesOf(`
        export const A = () => <button title="save">save</button>;
        export const B = () => <>{"Save this page"}</>;
        export const C = () => <div>{"Save this page"}</div>;
        export const D = () => <input placeholder="search" aria-label={"filter"} alt={cond ? "on" : "off"} />;
        export const E = () => <p>{ready() && "ready"}{name() || "untitled"}</p>;
      `);
      expect(sites.map((site) => `${site.kind} ${site.text}`)).toEqual([
        "attr:title save",
        "jsx-text save",
        "jsx-text Save this page",
        "jsx-text Save this page",
        "attr:placeholder search",
        "attr:aria-label filter",
        "attr:alt on | off",
        "jsx-text ready",
        "jsx-text {} | untitled",
      ]);
    });

    it("still ignores text without words: symbols, glyphs, single letters, numbers", () => {
      expect(sitesOf(`export const A = () => <b title="×" aria-label={"1"}>x · … {1}</b>;`)).toEqual([]);
    });

    it("scans index.html text nodes and attribute sinks, not script or style bodies", () => {
      const { sites } = scanSource("index.html", `<!doctype html><html><head><title>Tine</title><style>.a { content: "css words" }</style></head>
        <body><div id="root"><span id="msg" title="Open it">Opening Tine…</span><img alt="logo mark" src="/a.png"><script>var s = "script words";</script></div></body></html>`) as { sites: Site[] };
      expect(sites.map((site) => `${site.kind} ${site.owner} ${site.text}`)).toEqual([
        "html-text <document> Tine",
        "attr:title #msg Open it",
        "html-text #msg Opening Tine…",
        "attr:alt #root logo mark",
      ]);
    });

    it("includes the real index.html in the scan", () => {
      const { sites } = scan() as { sites: Site[] };
      expect(sites.some((site) => site.file === "index.html" && site.text.startsWith("Opening"))).toBe(true);
    }, 60_000);

    it("identity: a converted site plus a new same-text site in another owner is one added site", () => {
      const before = sitesOf(`
        export const Toolbar = () => <button>Save</button>;
        export const Menu = () => <b>Other thing</b>;
      `);
      const baseline = before.map(identity);
      const after = sitesOf(`
        import { t } from "../i18n";
        export const Toolbar = () => <button>{t("save")}</button>;
        export const Menu = () => <b>Other thing</b>;
        export const Footer = () => <button>Save</button>;
      `);
      const { added, gone } = compare(after, baseline) as { added: Site[]; gone: string[] };
      expect(added.map((site) => `${site.owner} ${site.text}`)).toEqual(["Footer Save"]);
      expect(gone).toHaveLength(1);
    });

    it("identity: a move to another file and an extra duplicate are still new sites", () => {
      const source = `export const Toolbar = () => <button>Save</button>;`;
      const baseline = sitesOf(source).map(identity);
      expect(compare(sitesOf(source, "src/components/Y.tsx"), baseline).added).toHaveLength(1);
      expect(compare([...sitesOf(source), ...sitesOf(source)], baseline).added).toHaveLength(1);
      // The same site moved down the file is the same identity: no line numbers.
      expect(compare(sitesOf(`\n\n\n${source}`), baseline)).toEqual({ added: [], gone: [] });
    });

    it("identity: the sink kind separates a title from a text", () => {
      const baseline = sitesOf(`export const A = () => <button title="Save" />;`).map(identity);
      expect(compare(sitesOf(`export const A = () => <button>Save</button>;`), baseline).added).toHaveLength(1);
    });
  });
});
