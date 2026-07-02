import { beforeAll, describe, expect, it } from "vitest";
import { initParser, parseBlock } from "./parse";
import type { Block, Inline, ListItem, Url } from "./ast";

const DRIFT = "ADR 0015: lsdoc wire contract drift — update src/render/ast.ts + this fixture together";

type BlockKind = Block["kind"];
type InlineKind = Inline["k"];
type UrlType = Url["type"];
type Seen = {
  blockKinds: Set<BlockKind>;
  inlineKinds: Set<InlineKind>;
  urlTypes: Set<UrlType>;
};

const BLOCK_REQUIRED_KEYS = {
  paragraph: ["kind", "inline"],
  heading: ["kind", "level", "size", "inline"],
  bullet: ["kind", "level", "inline"],
  list: ["kind", "items"],
  src: ["kind", "lang", "code"],
  quote: ["kind", "children"],
  custom: ["kind", "name", "children"],
  raw_html: ["kind", "text"],
  displayed_math: ["kind", "text"],
  drawer: ["kind", "name"],
  directive: ["kind", "name", "value"],
  comment: ["kind", "text"],
  example: ["kind", "code"],
  latex_env: ["kind", "name", "content"],
  properties: ["kind", "props"],
  hr: ["kind"],
  table: ["kind", "header", "rows"],
  footnote_def: ["kind", "name", "inline"],
  hiccup: ["kind", "v"],
} as const satisfies Record<BlockKind, readonly string[]>;

const INLINE_REQUIRED_KEYS = {
  plain: ["k", "text"],
  code: ["k", "text"],
  verbatim: ["k", "text"],
  break: ["k"],
  hardbreak: ["k"],
  emphasis: ["k", "emph", "children"],
  subscript: ["k", "children"],
  superscript: ["k", "children"],
  link: ["k", "url", "full"],
  nested_link: ["k", "content"],
  target: ["k", "text"],
  tag: ["k", "children"],
  macro: ["k", "name", "args"],
  latex: ["k", "mode", "body"],
  timestamp: ["k", "ts", "date"],
  fnref: ["k", "name"],
  inline_html: ["k", "text"],
  email: ["k", "text"],
  entity: ["k", "name", "latex", "latex_mathp", "html", "ascii", "unicode"],
  hiccup: ["k", "v"],
} as const satisfies Record<InlineKind, readonly string[]>;

const URL_REQUIRED_KEYS = {
  page_ref: ["type", "v"],
  block_ref: ["type", "v"],
  search: ["type", "v"],
  file: ["type", "v"],
  complex: ["type"],
} as const satisfies Record<UrlType, readonly string[]>;

const LIST_ITEM_REQUIRED_KEYS = ["ordered", "indent", "content", "items"] as const satisfies readonly (keyof ListItem)[];

const DECLARED_BLOCK_KINDS = Object.keys(BLOCK_REQUIRED_KEYS).sort();
const DECLARED_INLINE_KINDS = Object.keys(INLINE_REQUIRED_KEYS).sort();
const DECLARED_URL_TYPES = Object.keys(URL_REQUIRED_KEYS).sort();

type Fixture = {
  raw: string;
  format?: "md" | "org";
  assert(blocks: Block[], name: string): void;
};

const FIXTURES: Record<string, Fixture> = {
  "markdown task marker, priority, and tag facets": {
    raw: "TODO [#A] Task title #ship",
    assert(blocks, name) {
      expectKinds(blocks, ["bullet"], name);
      const bullet = blockAt(blocks, 0, "bullet", name);
      expect(bullet.marker, msg(name, "marker text")).toBe("TODO");
      expect(bullet.priority, msg(name, "priority text")).toBe("A");
      expect(bullet.span, msg(name, "bullet source span")).toEqual([0, 28]);
      const tag = inlineAt(bullet.inline, 1, "tag", name);
      expect(inlineAt(tag.children, 0, "plain", name).text, msg(name, "tag child text")).toBe("ship");
    },
  },
  "markdown heading bullet facet": {
    raw: "### Heading text",
    assert(blocks, name) {
      expectKinds(blocks, ["bullet"], name);
      const bullet = blockAt(blocks, 0, "bullet", name);
      expect(bullet.size, msg(name, "bullet heading size")).toBe(3);
      expect(inlineAt(bullet.inline, 0, "plain", name).text, msg(name, "heading text")).toBe("Heading text");
      expect(bullet.span, msg(name, "heading bullet span")).toEqual([0, 18]);
    },
  },
  "markdown continuation heading block": {
    raw: "Intro\n# Heading",
    assert(blocks, name) {
      expectKinds(blocks, ["bullet", "heading"], name);
      const heading = blockAt(blocks, 1, "heading", name);
      expect(heading.level, msg(name, "heading level")).toBe(1);
      expect(heading.size, msg(name, "heading size")).toBe(1);
      expect(inlineAt(heading.inline, 0, "plain", name).text, msg(name, "heading inline text")).toBe("Heading");
      expect(heading.span, msg(name, "heading span")).toEqual([8, 17]);
    },
  },
  "org task facets and planning timestamps": {
    raw: "TODO [#A] Heading :alpha:beta:\nSCHEDULED: <2026-07-06 Mon 14:30>\nDEADLINE: <2026-07-10 Fri>",
    format: "org",
    assert(blocks, name) {
      expectKinds(blocks, ["bullet", "paragraph"], name);
      const bullet = blockAt(blocks, 0, "bullet", name);
      expect(bullet.marker, msg(name, "marker text")).toBe("TODO");
      expect(bullet.priority, msg(name, "priority text")).toBe("A");
      expect(bullet.htags, msg(name, "headline tags")).toEqual(["alpha", "beta"]);
      expect(bullet.span, msg(name, "org header span")).toEqual([0, 33]);
      const paragraph = blockAt(blocks, 1, "paragraph", name);
      const scheduled = inlineAt(paragraph.inline, 0, "timestamp", name);
      const deadline = inlineAt(paragraph.inline, 2, "timestamp", name);
      expect(inlineAt(paragraph.inline, 1, "break", name).k, msg(name, "timestamp separator")).toBe("break");
      expect(scheduled.ts, msg(name, "scheduled timestamp tag")).toBe("Scheduled");
      expect(scheduled.date, msg(name, "scheduled timestamp fields")).toMatchObject({
        active: true,
        date: { year: 2026, month: 7, day: 6 },
        time: { hour: 14, min: 30 },
        wday: "Mon",
      });
      expect(deadline.ts, msg(name, "deadline timestamp tag")).toBe("Deadline");
      expect(deadline.date, msg(name, "deadline timestamp fields")).toMatchObject({
        active: true,
        date: { year: 2026, month: 7, day: 10 },
        wday: "Fri",
      });
      expect(paragraph.span, msg(name, "planning paragraph span")).toEqual([33, 93]);
    },
  },
  "markdown properties block": {
    raw: "Task\npriority:: high\nowner:: Ada",
    assert(blocks, name) {
      expectKinds(blocks, ["bullet", "properties"], name);
      const properties = blockAt(blocks, 1, "properties", name);
      expect(properties.props, msg(name, "property tuples")).toEqual([
        ["priority", "high"],
        ["owner", "Ada"],
      ]);
      expect(properties.span, msg(name, "properties span")).toEqual([7, 34]);
    },
  },
  "markdown nested emphasis and code": {
    raw: "plain **bold** and *italic* and ~~strike~~ and ==mark== and `code` and ***nested***",
    assert(blocks, name) {
      expectKinds(blocks, ["bullet"], name);
      const bullet = blockAt(blocks, 0, "bullet", name);
      const emph = inlinesOfKind(bullet.inline, "emphasis");
      expect(emph.map((e) => e.emph), msg(name, "emphasis vocabulary")).toEqual([
        "Bold",
        "Italic",
        "Strike_through",
        "Highlight",
        "Italic",
      ]);
      expect(inlinesOfKind(bullet.inline, "code")[0]?.text, msg(name, "inline code text")).toBe("code");
      const nestedOuter = emph[4];
      expect(nestedOuter?.children[0]?.k, msg(name, "nested emphasis child kind")).toBe("emphasis");
      const nestedInner = nestedOuter?.children[0] as Extract<Inline, { k: "emphasis" }> | undefined;
      expect(nestedInner?.emph, msg(name, "nested emphasis inner kind")).toBe("Bold");
      expect(inlineAt(nestedInner?.children ?? [], 0, "plain", name).text, msg(name, "nested emphasis text")).toBe("nested");
      expect(bullet.span, msg(name, "nested emphasis span")).toEqual([0, 85]);
    },
  },
  "org inline target, verbatim, underline, scripts, and hiccup": {
    raw: 'see <<target>> =verbatim= _underline_ /italic/ +strike+ ~code~ H_2O x^2 [:span "inline"]',
    format: "org",
    assert(blocks, name) {
      expectKinds(blocks, ["bullet"], name);
      const inline = blockAt(blocks, 0, "bullet", name).inline;
      expect(inlineAt(inline, 1, "target", name).text, msg(name, "target text")).toBe("target");
      expect(inlineAt(inline, 3, "verbatim", name).text, msg(name, "verbatim text")).toBe("verbatim");
      expect(inlinesOfKind(inline, "emphasis").map((e) => e.emph), msg(name, "org emphasis vocabulary")).toEqual([
        "Underline",
        "Italic",
        "Strike_through",
      ]);
      expect(inlineAt(inlineAt(inline, 13, "subscript", name).children, 0, "plain", name).text, msg(name, "subscript text")).toBe("2O");
      expect(inlineAt(inlineAt(inline, 15, "superscript", name).children, 0, "plain", name).text, msg(name, "superscript text")).toBe("2");
      expect(inlineAt(inline, 17, "hiccup", name).v, msg(name, "inline hiccup text")).toBe('[:span "inline"]');
    },
  },
  "markdown links, refs, macro embed, email, raw inline html, and nested link": {
    raw:
      '[[Page]] ((block-123)) [site](https://example.com/a "Title") ![Alt](../asset.png){:width 10} #tag #[[two words]] {{embed [[Page]]}} <a@b.com> <https://autolink.example/x> raw <span class="x">html</span> [[a [[b]] c]]',
    assert(blocks, name) {
      expectKinds(blocks, ["bullet"], name);
      const bullet = blockAt(blocks, 0, "bullet", name);
      const links = inlinesOfKind(bullet.inline, "link");
      expect(links[0]?.url, msg(name, "page ref url")).toEqual({ type: "page_ref", v: "Page" });
      expect(links[1]?.url, msg(name, "block ref url")).toEqual({ type: "block_ref", v: "block-123" });
      expect(links[2]?.url, msg(name, "complex link url")).toEqual({
        type: "complex",
        protocol: "https",
        link: "example.com/a",
      });
      expect(links[2]?.title, msg(name, "complex link title")).toBe("Title");
      expect(links[2]?.label, msg(name, "complex link label")).toEqual([{ k: "plain", text: "site" }]);
      expect(links[3]?.url, msg(name, "search/image url")).toEqual({ type: "search", v: "../asset.png" });
      expect(links[3]?.image, msg(name, "image flag")).toBe(true);
      expect(links[3]?.metadata, msg(name, "image metadata")).toBe("{:width 10}");
      expect(links[4]?.url, msg(name, "autolink url")).toEqual({
        type: "complex",
        protocol: "https",
        link: "autolink.example/x",
      });
      expect(inlinesOfKind(bullet.inline, "macro")[0], msg(name, "embed macro")).toMatchObject({
        name: "embed",
        args: ["[[Page]]"],
      });
      expect(inlinesOfKind(bullet.inline, "email")[0]?.text, msg(name, "email record")).toEqual({
        local_part: "a",
        domain: "b.com",
      });
      expect(inlinesOfKind(bullet.inline, "inline_html")[0]?.text, msg(name, "inline span html")).toBe('<span class="x">html</span>');
      expect(inlinesOfKind(bullet.inline, "nested_link")[0]?.content, msg(name, "nested page ref content")).toBe("[[a [[b]] c]]");
      expect(bullet.span, msg(name, "links span")).toEqual([0, 218]);
    },
  },
  "org file URL link": {
    raw: "[[file:../notes/x.org][File Label]]",
    format: "org",
    assert(blocks, name) {
      expectKinds(blocks, ["bullet"], name);
      const link = inlineAt(blockAt(blocks, 0, "bullet", name).inline, 0, "link", name);
      expect(link.url, msg(name, "file URL shape")).toEqual({ type: "file", v: "file:../notes/x.org" });
      expect(link.label, msg(name, "file link label")).toEqual([{ k: "plain", text: "File Label" }]);
    },
  },
  "markdown active date, range, latex, and entity": {
    raw: "Meet <2026-06-30 Tue 14:00> range <2026-01-01 Thu>--<2026-01-02 Fri> $x^2$ $$y$$ \\Delta{}",
    assert(blocks, name) {
      expectKinds(blocks, ["bullet"], name);
      const inline = blockAt(blocks, 0, "bullet", name).inline;
      const timestamps = inlinesOfKind(inline, "timestamp");
      expect(timestamps[0]?.ts, msg(name, "active date tag")).toBe("Date");
      expect(timestamps[0]?.date, msg(name, "active date fields")).toMatchObject({
        active: true,
        date: { year: 2026, month: 6, day: 30 },
        time: { hour: 14, min: 0 },
        wday: "Tue",
      });
      expect(timestamps[1]?.ts, msg(name, "range timestamp tag")).toBe("Range");
      expect(timestamps[1]?.date, msg(name, "range timestamp fields")).toMatchObject({
        start: { active: true, date: { year: 2026, month: 1, day: 1 }, wday: "Thu" },
        stop: { active: true, date: { year: 2026, month: 1, day: 2 }, wday: "Fri" },
      });
      expect(inlinesOfKind(inline, "latex").map((l) => [l.mode, l.body]), msg(name, "latex modes")).toEqual([
        ["Inline", "x^2"],
        ["Displayed", "y"],
      ]);
      expect(inlinesOfKind(inline, "entity")[0], msg(name, "entity fields")).toMatchObject({
        name: "Delta",
        latex: "\\Delta",
        latex_mathp: true,
        html: "&Delta;",
        ascii: "Delta",
        unicode: "Δ",
      });
    },
  },
  "org inactive timestamp": {
    raw: "Meet [2026-06-30 Tue]",
    format: "org",
    assert(blocks, name) {
      expectKinds(blocks, ["bullet"], name);
      const timestamp = inlineAt(blockAt(blocks, 0, "bullet", name).inline, 1, "timestamp", name);
      expect(timestamp.ts, msg(name, "inactive timestamp tag")).toBe("Date");
      expect(timestamp.date, msg(name, "inactive timestamp active flag")).toMatchObject({
        active: false,
        date: { year: 2026, month: 6, day: 30 },
        wday: "Tue",
      });
    },
  },
  "markdown closed timestamp": {
    raw: "Task\nCLOSED: <2026-07-01 Wed>",
    assert(blocks, name) {
      expectKinds(blocks, ["bullet", "paragraph"], name);
      const timestamp = inlineAt(blockAt(blocks, 1, "paragraph", name).inline, 0, "timestamp", name);
      expect(timestamp.ts, msg(name, "closed timestamp tag")).toBe("Closed");
      expect(timestamp.date, msg(name, "closed timestamp fields")).toMatchObject({
        active: true,
        date: { year: 2026, month: 7, day: 1 },
        wday: "Wed",
      });
    },
  },
  "markdown hardbreak paragraph": {
    raw: "Intro\nfirst  \nsecond",
    assert(blocks, name) {
      expectKinds(blocks, ["bullet", "paragraph"], name);
      const paragraph = blockAt(blocks, 1, "paragraph", name);
      expect(paragraph.inline.map((i) => i.k), msg(name, "hardbreak inline sequence")).toEqual(["plain", "hardbreak", "plain"]);
      expect(inlineAt(paragraph.inline, 0, "plain", name).text, msg(name, "hardbreak leading text")).toBe("first");
      expect(inlineAt(paragraph.inline, 2, "plain", name).text, msg(name, "hardbreak trailing text")).toBe("second");
      expect(paragraph.span, msg(name, "hardbreak paragraph span")).toEqual([8, 22]);
    },
  },
  "markdown checkbox and definition lists": {
    raw: "List\n+ [x] done\n+ [ ] todo\nTea\n: another",
    assert(blocks, name) {
      expectKinds(blocks, ["bullet", "list", "list"], name);
      const checklist = blockAt(blocks, 1, "list", name);
      expect(checklist.items[0]?.checkbox, msg(name, "checked checkbox")).toBe(true);
      expect(checklist.items[1]?.checkbox, msg(name, "unchecked checkbox")).toBe(false);
      expect(blockAt(checklist.items[0]?.content ?? [], 0, "paragraph", name).inline, msg(name, "checkbox item body")).toEqual([
        { k: "plain", text: "done" },
      ]);
      const defList = blockAt(blocks, 2, "list", name);
      expect(defList.items[0]?.name, msg(name, "definition-list term")).toEqual([{ k: "plain", text: "Tea" }]);
      expect(blockAt(defList.items[0]?.content ?? [], 0, "paragraph", name).inline, msg(name, "definition-list body")).toEqual([
        { k: "plain", text: "another" },
      ]);
    },
  },
  "markdown fenced source block": {
    raw: "```ts\nconst x = 1;\n```",
    assert(blocks, name) {
      expectKinds(blocks, ["bullet", "src"], name);
      const src = blockAt(blocks, 1, "src", name);
      expect(src.lang, msg(name, "source language")).toBe("ts");
      expect(src.code, msg(name, "source code body")).toBe("const x = 1;\n");
      expect(src.span, msg(name, "source span")).toEqual([2, 24]);
    },
  },
  "markdown quote block": {
    raw: "> quoted **bold**\n> second",
    assert(blocks, name) {
      expectKinds(blocks, ["bullet", "quote"], name);
      const quote = blockAt(blocks, 1, "quote", name);
      expectKinds(quote.children, ["paragraph"], name);
      const paragraph = blockAt(quote.children, 0, "paragraph", name);
      expect(paragraph.inline.map((i) => i.k), msg(name, "quote child inline sequence")).toEqual([
        "plain",
        "emphasis",
        "break",
        "plain",
        "break",
      ]);
      expect(inlineAt(paragraph.inline, 1, "emphasis", name).emph, msg(name, "quote emphasis")).toBe("Bold");
      expect(quote.span, msg(name, "quote span")).toEqual([2, 28]);
    },
  },
  "markdown custom block": {
    raw: "#+BEGIN_NOTE\ncustom body\n#+END_NOTE",
    assert(blocks, name) {
      expectKinds(blocks, ["bullet", "custom"], name);
      const custom = blockAt(blocks, 1, "custom", name);
      expect(custom.name, msg(name, "custom block name")).toBe("note");
      expectKinds(custom.children, ["paragraph"], name);
      expect(inlineAt(blockAt(custom.children, 0, "paragraph", name).inline, 0, "plain", name).text, msg(name, "custom body text")).toBe(
        "custom body",
      );
      expect(custom.span, msg(name, "custom span")).toEqual([2, 37]);
    },
  },
  "markdown raw HTML block": {
    raw: '<div class="note">hi</div>',
    assert(blocks, name) {
      expectKinds(blocks, ["bullet", "raw_html"], name);
      const rawHtml = blockAt(blocks, 1, "raw_html", name);
      expect(rawHtml.text, msg(name, "raw html text")).toBe('<div class="note">hi</div>');
      expect(rawHtml.span, msg(name, "raw html span")).toEqual([2, 28]);
    },
  },
  "markdown displayed math block": {
    raw: "$$E=mc^2$$",
    assert(blocks, name) {
      expectKinds(blocks, ["bullet", "displayed_math"], name);
      const math = blockAt(blocks, 1, "displayed_math", name);
      expect(math.text, msg(name, "displayed math text")).toBe("E=mc^2");
      expect(math.span, msg(name, "displayed math span")).toEqual([2, 12]);
    },
  },
  "org drawer directive and comment blocks": {
    raw: "Task\n:LOGBOOK:\nCLOCK: [2026-07-06 Mon 10:00]--[2026-07-06 Mon 11:00]\n:END:\n#+CAPTION: A caption\n# a comment",
    format: "org",
    assert(blocks, name) {
      expectKinds(blocks, ["bullet", "drawer", "directive", "comment"], name);
      expect(blockAt(blocks, 1, "drawer", name), msg(name, "drawer fields")).toMatchObject({ name: "logbook", span: [7, 77] });
      expect(blockAt(blocks, 2, "directive", name), msg(name, "directive fields")).toMatchObject({
        name: "CAPTION",
        value: "A caption",
        span: [77, 98],
      });
      expect(blockAt(blocks, 3, "comment", name), msg(name, "comment fields")).toMatchObject({
        text: "a comment",
        span: [98, 109],
      });
    },
  },
  "markdown example and latex environment blocks": {
    raw: "#+BEGIN_EXAMPLE\nexample body\n#+END_EXAMPLE\n\\begin{align}\na&=b\\\\\n\\end{align}",
    assert(blocks, name) {
      expectKinds(blocks, ["bullet", "example", "latex_env"], name);
      const example = blockAt(blocks, 1, "example", name);
      const latexEnv = blockAt(blocks, 2, "latex_env", name);
      expect(example.code, msg(name, "example code")).toBe("example body\n");
      expect(example.span, msg(name, "example span")).toEqual([2, 45]);
      expect(latexEnv.name, msg(name, "latex environment name")).toBe("align");
      expect(latexEnv.content, msg(name, "latex environment content")).toBe("a&=b\\\\\n");
      expect(latexEnv.span, msg(name, "latex environment span")).toEqual([45, 77]);
    },
  },
  "markdown horizontal rule and table": {
    raw: "Intro\n---\n| A | B |\n| --- | --- |\n| 1 | 2 |",
    assert(blocks, name) {
      expectKinds(blocks, ["bullet", "hr", "table"], name);
      expect(blockAt(blocks, 1, "hr", name).span, msg(name, "hr span")).toEqual([8, 12]);
      const table = blockAt(blocks, 2, "table", name);
      expect(table.header, msg(name, "table header cells")).toEqual([[{ k: "plain", text: "A" }], [{ k: "plain", text: "B" }]]);
      expect(table.rows, msg(name, "table row cells")).toEqual([[[{ k: "plain", text: "1" }], [{ k: "plain", text: "2" }]]]);
      expect(table.span, msg(name, "table span")).toEqual([12, 45]);
    },
  },
  "markdown footnote reference and definition": {
    raw: "Footnote ref[^n]\n[^n]: note **body**",
    assert(blocks, name) {
      expectKinds(blocks, ["bullet", "footnote_def"], name);
      expect(inlineAt(blockAt(blocks, 0, "bullet", name).inline, 1, "fnref", name).name, msg(name, "footnote ref name")).toBe("n");
      const def = blockAt(blocks, 1, "footnote_def", name);
      expect(def.name, msg(name, "footnote definition name")).toBe("n");
      expect(def.inline, msg(name, "footnote definition inline")).toMatchObject([
        { k: "plain", text: "note " },
        { k: "emphasis", emph: "Bold", children: [{ k: "plain", text: "body" }] },
      ]);
      expect(def.span, msg(name, "footnote definition span")).toEqual([19, 38]);
    },
  },
  "markdown block hiccup": {
    raw: 'Intro\n[:div.note "hi"]',
    assert(blocks, name) {
      expectKinds(blocks, ["bullet", "hiccup"], name);
      const hiccup = blockAt(blocks, 1, "hiccup", name);
      expect(hiccup.v, msg(name, "hiccup block raw value")).toBe('[:div.note "hi"]');
      expect(hiccup.span, msg(name, "hiccup block span")).toEqual([8, 24]);
    },
  },
};

beforeAll(async () => {
  await initParser();
});

describe("lsdoc AST wire contract (ADR 0015)", () => {
  it("accepts only declared discriminants and required fields", () => {
    validateFixtures();
  });

  it("coverage fixtures produce every declared AST variant", () => {
    const seen = validateFixtures();
    expect([...seen.blockKinds].sort(), msg("all fixtures", "block kind coverage")).toEqual(DECLARED_BLOCK_KINDS);
    expect([...seen.inlineKinds].sort(), msg("all fixtures", "inline kind coverage")).toEqual(DECLARED_INLINE_KINDS);
    expect([...seen.urlTypes].sort(), msg("all fixtures", "URL type coverage")).toEqual(DECLARED_URL_TYPES);
  });

  for (const [name, fixture] of Object.entries(FIXTURES)) {
    it(name, () => {
      fixture.assert(parseFixture(fixture), name);
    });
  }
});

function parseFixture(fixture: Fixture): Block[] {
  return parseBlock(fixture.raw, fixture.format === "org");
}

function validateFixtures(): Seen {
  const seen = {
    blockKinds: new Set<BlockKind>(),
    inlineKinds: new Set<InlineKind>(),
    urlTypes: new Set<UrlType>(),
  };
  for (const [name, fixture] of Object.entries(FIXTURES)) {
    visitBlocks(parseFixture(fixture), `fixture "${name}"`, seen);
  }
  return seen;
}

function visitBlocks(value: unknown, path: string, seen: Seen): void {
  requireArray(value, path).forEach((block, i) => visitBlock(block, `${path}[${i}]`, seen));
}

function visitBlock(value: unknown, path: string, seen: Seen): void {
  const block = requireRecord(value, path);
  const kind = block.kind;
  if (typeof kind !== "string" || !isBlockKind(kind)) throw new Error(`${DRIFT}: ${path}.kind unknown block kind ${String(kind)}`);
  seen.blockKinds.add(kind);
  requireKeys(block, BLOCK_REQUIRED_KEYS[kind], path);

  switch (kind) {
    case "paragraph":
    case "heading":
    case "bullet":
    case "footnote_def":
      visitInlines(block.inline, `${path}.inline`, seen);
      break;
    case "list":
      requireArray(block.items, `${path}.items`).forEach((item, i) => visitListItem(item, `${path}.items[${i}]`, seen));
      break;
    case "quote":
    case "custom":
      visitBlocks(block.children, `${path}.children`, seen);
      break;
    case "properties":
      visitProperties(block.props, `${path}.props`);
      break;
    case "table":
      visitTable(block, path, seen);
      break;
    case "src":
    case "raw_html":
    case "displayed_math":
    case "drawer":
    case "directive":
    case "comment":
    case "example":
    case "latex_env":
    case "hr":
    case "hiccup":
      break;
  }
}

function visitListItem(value: unknown, path: string, seen: Seen): void {
  const item = requireRecord(value, path);
  requireKeys(item, LIST_ITEM_REQUIRED_KEYS, path);
  visitBlocks(item.content, `${path}.content`, seen);
  requireArray(item.items, `${path}.items`).forEach((child, i) => visitListItem(child, `${path}.items[${i}]`, seen));
  if (hasOwn(item, "name")) visitInlines(item.name, `${path}.name`, seen);
}

function visitInlines(value: unknown, path: string, seen: Seen): void {
  requireArray(value, path).forEach((inline, i) => visitInline(inline, `${path}[${i}]`, seen));
}

function visitInline(value: unknown, path: string, seen: Seen): void {
  const inline = requireRecord(value, path);
  const kind = inline.k;
  if (typeof kind !== "string" || !isInlineKind(kind)) throw new Error(`${DRIFT}: ${path}.k unknown inline kind ${String(kind)}`);
  seen.inlineKinds.add(kind);
  requireKeys(inline, INLINE_REQUIRED_KEYS[kind], path);

  switch (kind) {
    case "emphasis":
    case "subscript":
    case "superscript":
    case "tag":
      visitInlines(inline.children, `${path}.children`, seen);
      break;
    case "link":
      visitUrl(inline.url, `${path}.url`, seen);
      if (hasOwn(inline, "label")) visitInlines(inline.label, `${path}.label`, seen);
      break;
    case "plain":
    case "code":
    case "verbatim":
    case "break":
    case "hardbreak":
    case "nested_link":
    case "target":
    case "macro":
    case "latex":
    case "timestamp":
    case "fnref":
    case "inline_html":
    case "email":
    case "entity":
    case "hiccup":
      break;
  }
}

function visitUrl(value: unknown, path: string, seen: Seen): void {
  const url = requireRecord(value, path);
  const type = url.type;
  if (typeof type !== "string" || !isUrlType(type)) throw new Error(`${DRIFT}: ${path}.type unknown URL type ${String(type)}`);
  seen.urlTypes.add(type);
  requireKeys(url, URL_REQUIRED_KEYS[type], path);
}

function visitProperties(value: unknown, path: string): void {
  requireArray(value, path).forEach((pair, i) => {
    const tuple = requireArray(pair, `${path}[${i}]`);
    if (tuple.length !== 2 || typeof tuple[0] !== "string" || typeof tuple[1] !== "string") {
      throw new Error(`${DRIFT}: ${path}[${i}] must be a [string, string] property tuple`);
    }
  });
}

function visitTable(block: Record<string, unknown>, path: string, seen: Seen): void {
  if (block.header !== null) {
    requireArray(block.header, `${path}.header`).forEach((cell, i) => visitInlines(cell, `${path}.header[${i}]`, seen));
  }
  requireArray(block.rows, `${path}.rows`).forEach((row, rowIndex) => {
    requireArray(row, `${path}.rows[${rowIndex}]`).forEach((cell, cellIndex) =>
      visitInlines(cell, `${path}.rows[${rowIndex}][${cellIndex}]`, seen),
    );
  });
}

function requireArray(value: unknown, path: string): unknown[] {
  if (!Array.isArray(value)) throw new Error(`${DRIFT}: ${path} must be an array`);
  return value;
}

function requireRecord(value: unknown, path: string): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error(`${DRIFT}: ${path} must be an object`);
  }
  return value as Record<string, unknown>;
}

function requireKeys(value: Record<string, unknown>, keys: readonly string[], path: string): void {
  for (const key of keys) {
    if (!hasOwn(value, key)) throw new Error(`${DRIFT}: ${path} missing required key "${key}"`);
  }
}

function hasOwn(value: Record<string, unknown>, key: string): boolean {
  return Object.prototype.hasOwnProperty.call(value, key);
}

function isBlockKind(kind: string): kind is BlockKind {
  return hasOwn(BLOCK_REQUIRED_KEYS, kind);
}

function isInlineKind(kind: string): kind is InlineKind {
  return hasOwn(INLINE_REQUIRED_KEYS, kind);
}

function isUrlType(type: string): type is UrlType {
  return hasOwn(URL_REQUIRED_KEYS, type);
}

function msg(fixture: string, detail: string): string {
  return `${DRIFT}: ${fixture}: ${detail}`;
}

function expectKinds(blocks: Block[], kinds: BlockKind[], fixture: string): void {
  expect(blocks.map((block) => block.kind), msg(fixture, "block kind sequence")).toEqual(kinds);
}

function blockAt<K extends BlockKind>(blocks: Block[], index: number, kind: K, fixture: string): Extract<Block, { kind: K }> {
  expect(blocks[index]?.kind, msg(fixture, `block ${index} kind`)).toBe(kind);
  return blocks[index] as Extract<Block, { kind: K }>;
}

function inlineAt<K extends InlineKind>(inlines: Inline[], index: number, kind: K, fixture: string): Extract<Inline, { k: K }> {
  expect(inlines[index]?.k, msg(fixture, `inline ${index} kind`)).toBe(kind);
  return inlines[index] as Extract<Inline, { k: K }>;
}

function inlinesOfKind<K extends InlineKind>(inlines: Inline[], kind: K): Extract<Inline, { k: K }>[] {
  return inlines.filter((inline): inline is Extract<Inline, { k: K }> => inline.k === kind);
}
