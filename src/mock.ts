// In-memory mock backend seeded with a fixture graph. Used only when running
// outside Tauri (browser dev / Playwright screenshots). Mirrors the real
// backend's shape so the UI behaves identically.

import type { Backend, GpuEnv, DebugInfo, InstalledPluginRecord } from "./backend";
import type { BacklinkFilterContext, BacklinkFilterTarget, BlockDto, BlockPreview, GuideCopyResult, GuidePage, Highlight, PageDto, PageEntry, PdfState, QueryExecution, QueryExportBatch, QueryExportSpec, RefGroup } from "./types";
import { SAMPLE_PDF_B64 } from "./sample-pdf";
import { hlsPageName } from "./pdf";
import { MARKER_RE } from "./markers";
import { fuzzyScore } from "./editor/autocomplete";
import { canonicalFold, matcherMatches, matchHighlights, parseSearchQuery, simpleTerm } from "./editor/searchQuery";
import { parseJournalWith } from "./journal";

/** Mock feed membership must use a Logseq journal-title parser, never the
 * host's permissive/non-portable Date string parser. Keep the same explicit
 * patterns the fixture can emit so pagination remains deterministic in every
 * browser/runtime. */
function mockJournalDayKey(name: string): number | null {
  for (const format of ["MMM do, yyyy", "EEEE, dd-MM-yyyy", "yyyy-MM-dd", "dd-MM-yyyy", "yyyy_MM_dd"]) {
    const parsed = parseJournalWith(name, format);
    if (parsed) return parsed.y * 10_000 + parsed.m * 100 + parsed.d;
  }
  return null;
}

function pageRefs(raw: string): string[] {
  const out: string[] = [];
  const re = /\[\[([^\]]+)\]\]|#\[\[([^\]]+)\]\]|#([\w/_.-]+)/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(raw))) out.push(m[1] ?? m[2] ?? m[3]);
  return out;
}
// Block uuids `raw` references, deduped. Mirrors refs::block_ref_ids: labeled
// `[label](((uuid)))` is matched (and removed) FIRST so the bare `((uuid))` scan
// doesn't mis-read the triple paren; the bare scan also covers `{{embed ((uuid))}}`.
function blockRefIds(raw: string): string[] {
  const out: string[] = [];
  const push = (id: string) => {
    id = id.trim();
    if (id && !out.includes(id)) out.push(id);
  };
  const labeled = /\[[^\]]*\]\(\(\(([^)]+)\)\)\)/g;
  let m: RegExpExecArray | null;
  while ((m = labeled.exec(raw))) push(m[1]);
  const bare = /\(\(([^)]+)\)\)/g;
  const rest = raw.replace(labeled, "");
  while ((m = bare.exec(rest))) push(m[1]);
  return out;
}
function leadingMarker(raw: string): string | null {
  const m = MARKER_RE.exec(raw);
  return m ? m[1] : null;
}
function priorityOf(raw: string): string | undefined {
  const m = /(?:^|\s)\[#([ABC])\]/.exec(raw.split("\n", 1)[0] ?? "");
  return m?.[1];
}
function planningOf(raw: string, tag: "SCHEDULED" | "DEADLINE"): string | undefined {
  const m = new RegExp(`^${tag}:\\s*<([^>]+)>`, "m").exec(raw);
  return m?.[1];
}
function tagsOf(raw: string): string[] {
  const out: string[] = [];
  // (?<!\[) keeps the [#A] priority token from leaking a fake #A tag.
  const re = /#\[\[([^\]]+)\]\]|(?<!\[)#([\w/_.-]+)/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(raw))) {
    const tag = (m[1] ?? m[2]).trim();
    if (tag && !out.some((t) => t.toLowerCase() === tag.toLowerCase())) out.push(tag);
  }
  return out;
}
function propertyLines(raw: string): [string, string][] {
  const out: [string, string][] = [];
  for (const line of raw.split("\n")) {
    const m = /^([A-Za-z0-9_./-]+):: ?(.*)$/.exec(line.trim());
    if (m) out.push([m[1], m[2].trim()]);
  }
  return out;
}

let _id = 0;
const nid = () => `mock-${_id++}`;
const mockPlugins: InstalledPluginRecord[] = [];
const mockPluginEntries = new Map<string, Uint8Array>();

function b(raw: string, children: BlockDto[] = [], collapsed = false, properties?: [string, string][]): BlockDto {
  // Mirror the real backend: a block carrying an `id::` property uses that uuid as
  // its store id (so block refs resolve to it and the count badge keys correctly).
  const m = raw.match(/^\s*id::\s*(.+)$/m);
  return {
    id: m ? m[1].trim() : nid(),
    raw,
    collapsed,
    children,
    marker: leadingMarker(raw) ?? undefined,
    priority: priorityOf(raw),
    scheduled: planningOf(raw, "SCHEDULED"),
    deadline: planningOf(raw, "DEADLINE"),
    tags: tagsOf(raw),
    properties: properties ?? propertyLines(raw),
  };
}

function mockPagePath(p: PageDto): string {
  if (p.path) return p.path;
  const dir = p.kind === "journal" ? "journals" : "pages";
  const ext = p.format === "org" ? "org" : "md";
  return `${dir}/${p.name.replace(/\//g, "___")}.${ext}`;
}

function mockPageEntry(p: PageDto): PageEntry {
  return { name: p.name, kind: p.kind, date_key: null, path: mockPagePath(p) };
}

const PAGES: PageDto[] = [
  {
    name: "Jun 14th, 2026",
    kind: "journal",
    title: "Jun 14th, 2026",
    pre_block: null,
    blocks: [
      b("## Today"),
      b("Started the [[Tine]] rewrite — aiming for a #fast native feel.", [
        b("The outliner is the core; everything hangs off **blocks**."),
        b("Reading the OG source for the *exact* file format and `mldoc` quirks."),
      ]),
      b("TODO [#A] Ship the M0 vertical slice\nSCHEDULED: <2026-06-15 Mon>"),
      b("DOING Wire up the [[block editor]] with caret preservation"),
      b("A code block:\n```rust\nfn main() {\n    println!(\"hello, tine\");\n}\n```"),
      b("A table:\n| Feature | Status |\n| --- | --- |\n| Outliner | done |\n| Queries | partial |"),
      b("DONE Validate round-trip on the real `shui-graph`"),
      b("Inline math works too: $E = mc^2$ and references like ((arch-1))."),
      b("```calc\n1 + 2\n2+4\n5 + 4\nx = 12 * 3\nx / 4\n```"),
      b("Open tasks across the graph:"),
      b("{{query (todo TODO DOING)}}"),
      b("All todos + Prio A {{query (and (task TODO) (priority A))}}\nid:: 1002fa7a-7164-456c-9e53-3032f783711c"),
    ],
  },
  {
    name: "Jun 13th, 2026",
    kind: "journal",
    title: "Jun 13th, 2026",
    pre_block: null,
    blocks: [
      b("Yesterday's notes about [[parameterized complexity]].", [
        b("n-fold IP shows up again — see [[n-fold IP]]."),
        b("Key idea:", [b("decompose the constraint matrix into blocks"), b("solve via dynamic programming over the bricks")]),
      ]),
      b("LATER Read the new #SODA submission"),
    ],
  },
  {
    name: "Jun 12th, 2026",
    kind: "journal",
    title: "Jun 12th, 2026",
    pre_block: null,
    blocks: [
      b("Set up the [[Tine]] repo and Rust core.", [
        b("Round-trip tests pass on the real graph."),
      ]),
      b("DONE Decide the stack: **Tauri** + *SolidJS*"),
    ],
  },
];

const NAMED: PageDto[] = [
  {
    name: "Tine",
    kind: "page",
    title: "Tine",
    pre_block: "title:: Tine\ntags:: project, tooling",
    blocks: [
      b("A fast clone of [[Logseq]] built with **Tauri** + *SolidJS*.", [
        b("Goal: #functional + #visual equivalent."),
        b("Reads the same markdown graph as OG Logseq."),
      ]),
      b("## Architecture"),
      b("Rust core owns parsing; the frontend owns the live editing tree.\nid:: arch-1"),
      b("A PDF asset: [sample.pdf](../assets/sample.pdf)"),
    ],
  },
  // Org-mode rendering parity: same idea as kitchen-sink, but format:"org" so the
  // org inline rules (*, /, _, +, ~, =, ^^, [[t][d]]) and src/quote blocks render.
  {
    name: "org-sink",
    kind: "page",
    title: "org-sink",
    pre_block: "#+TITLE: org-sink\n#+FILETAGS: :demo:org:\n#+ALIAS: org parity",
    format: "org",
    blocks: [
      b("Inline styles: *bold*, /italic/, _underline_, +strike+, ~code~, =verbatim=, ^^highlight^^"),
      b("Org links: [[Tine]], [[Tine][the project]], and [[https://orgmode.org][Org website]]"),
      b("Boundary-safe plain text: a/b/c paths, snake_case_var, and 2*3*4 stay literal"),
      b("TODO [#A] high-priority org task\nSCHEDULED: <2026-06-25 Thu>"),
      b("DOING in-progress task referencing [[n-fold IP]]"),
      b("Inline timestamps: met on <2026-06-26 Fri> (active), logged [2026-06-20 Sat] (inactive)"),
      b("a parent headline", [
        b("child block under it with /emphasis/"),
        b("DONE finished child task"),
      ]),
      b("Org table:\n| Feature | Status |\n|---------+--------|\n| Outliner | done |\n| Queries | partial |"),
      b("Org source block:\n#+BEGIN_SRC clojure\n(defn hello [] \"world\")\n#+END_SRC"),
      b("Org quote block:\n#+BEGIN_QUOTE\nto be or not to be\n#+END_QUOTE"),
      b("A property drawer stays as content:\n:PROPERTIES:\n:key: value\n:END:"),
      b("A plain list — org bullets are - and + (in md, - is the outline bullet):\n- milk\n- eggs\n+ also fine"),
    ],
  },
  // Rendering parity harness: one block per construct, so a screenshot makes any
  // unrendered/mis-rendered syntax obvious at a glance.
  {
    name: "kitchen-sink",
    kind: "page",
    title: "kitchen-sink",
    pre_block: null,
    blocks: [
      b("Blockquote:\n> a quoted line\n> a second quoted line"),
      b("Callout NOTE:\n> [!NOTE] Heads up\n> body of the note"),
      b("Callout WARNING:\n> [!WARNING] Be careful here"),
      b("Callout TIP:\n> [!TIP] a helpful tip"),
      b("Horizontal rule below:\n---"),
      b("Table with alignment:\n| Left | Center | Right |\n|:---|:---:|---:|\n| a | b | c |\n| 1 | 2 | 3 |"),
      b("Autolink (bare): visit https://logseq.com/docs for details"),
      b("Autolink (angle): <https://example.com>"),
      b("Inline math: $E = mc^2$ and chemistry: $\\ce{H2O + CO2}$"),
      b("Display math:\n$$\\int_0^1 x^2 \\, dx = \\tfrac13$$"),
      b("DONE finished task with a logbook drawer\n:LOGBOOK:\nCLOCK: [2026-06-16 Tue 09:00:00]--[2026-06-16 Tue 09:30:45] =>  00:30:45\nCLOCK: [2026-06-17 Wed 10:00:00]--[2026-06-17 Wed 10:20:00] =>  00:20:00\n:END:"),
      b("Task markers: TODO a, DOING b, NOW c, LATER d, WAIT e, DONE f"),
      b("in-block checklist (+ list inside one bullet — ticks in OG/mobile):\n+ [ ] pack toothbrush\n+ [x] pack charger\n+ [ ] pack passport"),
      b("in-block nested list:\n+ groceries\n  + milk\n  + eggs\n+ hardware"),
      b("Numbered list (logseq.order-list-type — the block itself is numbered):", [
        b("Bump the version\nlogseq.order-list-type:: number"),
        b("Tag and push\nlogseq.order-list-type:: number", [
          b("run the test suite\nlogseq.order-list-type:: number"),
          b("build the installers\nlogseq.order-list-type:: number"),
        ]),
        b("Announce on Discord\nlogseq.order-list-type:: number"),
      ]),
      b("TODO [#A] high-priority task"),
      b("Inline styles: **bold**, *italic*, ~~strike~~, ==highlight==, `code`"),
      b("Video asset (plays inline where the codec is supported; otherwise a click-to-open chip):\n![](../assets/demo_clip.mp4)"),
      b("Audio asset (⇔ Widen stretches the seek bar for precise scrubbing):\n![](../assets/voice_memo.wav)"),
      b("Footnote reference[^1] in a sentence.\n[^1]: the footnote definition."),
      b("Video embed: {{video https://www.youtube.com/watch?v=dQw4w9WgXcQ}}"),
      b("Tweet embed: {{tweet https://twitter.com/logseq/status/123}}"),
      b("More embeds: {{twitter https://twitter.com/logseq/status/9}} · {{vimeo 76979871}} · {{bilibili BV1xx411c7mD}}"),
      b("youtube-timestamp {{youtube-timestamp 125}} · cloze {{cloze the answer\\\\the cue}} · zotero {{zotero-imported-file abc, paper.pdf}}"),
      b("User macro (config.edn :macros): {{poem red, blue}} and {{hi Martin, kitchen-sink}}"),
      b("Block user macro:\n{{card Topic, Body text}}"),
      b("Block reference (bare): see ((64b9c0e2-0000-0000-0000-000000000000)) inline"),
      b("Labeled block reference: see [Related Work](((64b9c0e2-0000-0000-0000-000000000000))) inline"),
      b("Project notes", [
        b("Methods", [
          b("a nested reference: see ((64b9c0e2-0000-0000-0000-000000000000)) here"),
        ]),
      ]),
      b("Block-ref target: the **Related Work** section\nid:: 64b9c0e2-0000-0000-0000-000000000000"),
    ],
  },
  {
    name: "Sheets demo",
    kind: "page",
    title: "Sheets demo",
    pre_block: null,
    blocks: [
      b(
        "Readonly grid demo\ntine.view:: grid\ntine.header:: true\ntine.col-widths:: 0=140;1=180;2=220",
        [
          b("", [b("Project"), b("Status"), b("Notes")]),
          b("", [
            b("TODO Ship [[Tine]] sheet"),
            b(
              "Nested sub-grid\ntine.view:: grid",
              [
                b("", [b("Inner A"), b("Inner B")]),
                b("", [b("Inner C")]),
              ],
              false,
              [["tine.view", "grid"]]
            ),
            b("Uses tree geometry only"),
          ]),
          b("", [b("Ragged row"), b("missing note cell")]),
          b("", [b("Done"), b("Read-only"), b("Phase 2 adds interaction")]),
        ],
        false,
        [
          ["tine.view", "grid"],
          ["tine.header", "true"],
          ["tine.col-widths", "0=140;1=180;2=220"],
        ]
      ),
      b(
        "Field table demo\ntine.view:: table\ntine.col-aggregates:: prop:estimate=sum\ntine.fields:: state=state;owner=text;topic=enum:infra,ui,docs;points=number;shipped=checkbox;due=date;estimate=text\ntine.formula.effort:: points * 2\ntine.formula.due-soon:: if(isEmpty(due), false, due < today() + \"14d\")\ntine.formula.broken:: points +",
        [
          b("TODO [#A] Draft spec #sheets\nSCHEDULED: <2026-07-08 Wed>\nowner:: Martin\nestimate:: 2h\ntopic:: docs\npoints:: 3\nshipped:: false\ndue:: 2026-07-09"),
          b("DOING Build table renderer #sheets\nowner:: Codex\nestimate:: 5h\ntopic:: ui\npoints:: 8\nshipped:: false\nnote:: stray column"),
          b("DONE Verify screenshots\nDEADLINE: <2026-07-10 Fri>\nowner:: Codex\ntopic:: infra\npoints:: 1\nshipped:: true\ndue:: 2026-07-07"),
        ],
        false,
        [
          ["tine.view", "table"],
          ["tine.col-aggregates", "prop:estimate=sum"],
          ["tine.fields", "state=state;owner=text;topic=enum:infra,ui,docs;points=number;shipped=checkbox;due=date;estimate=text"],
          ["tine.formula.effort", "points * 2"],
          ["tine.formula.due-soon", 'if(isEmpty(due), false, due < today() + "14d")'],
          ["tine.formula.broken", "points +"],
        ]
      ),
      b("{{query (todo TODO DOING DONE)}}\ntine.view:: board\ntine.group-by:: state"),
      b(
        "Reading list by topic\ntine.view:: board\ntine.group-by:: tags",
        [
          b("Aaronson survey #reading"),
          b("n-fold draft #reading #writing"),
          b("ChoCo rebuttal #writing"),
        ],
        false,
        [
          ["tine.view", "board"],
          ["tine.group-by", "tags"],
        ]
      ),
    ],
  },
  // Namespace + page-icon demo: {{namespace}} renders the nested descendant tree,
  // each page showing its `icon::`.
  {
    name: "Formula1",
    kind: "page",
    title: "Formula1",
    pre_block: "icon:: 🏁\ncolor:: steelblue",
    blocks: [b("{{namespace Formula1}}"), b("2024 overview [[joplin]]")],
  },
  { name: "Formula1/2026", kind: "page", title: "Formula1/2026", pre_block: "icon:: 🏁", blocks: [b("Season 2026")] },
  {
    name: "Formula1/2026/08 Austrian Grand Prix",
    kind: "page",
    title: "Formula1/2026/08 Austrian Grand Prix",
    pre_block: "icon:: 🏁",
    blocks: [b("Race notes")],
  },
  {
    name: "Formula1/2026/09 Italian Grand Prix",
    kind: "page",
    title: "Formula1/2026/09 Italian Grand Prix",
    pre_block: "icon:: 🏁",
    blocks: [b("Race notes")],
  },
  // No "Formula1/2025" page of its own — only this leaf. The Hierarchy must still
  // synthesize a "Formula1 / 2025" level row (OG parity).
  {
    name: "Formula1/2025/12 Abu Dhabi Grand Prix",
    kind: "page",
    title: "Formula1/2025/12 Abu Dhabi Grand Prix",
    pre_block: "icon:: 🏁",
    blocks: [b("Race notes")],
  },
];

// A large synthetic page (~2000 root blocks) cycling through construct types, for
// the lazy-body virtualization harness: most blocks start as deferred raw-text
// placeholders and only parse/render on scroll. Gated behind `?big` so it never
// pollutes the normal mock screenshots (it would otherwise show up in All-Pages /
// quick-switch). Reach it with `…/?big` then quick-switch (Ctrl+K) to "Big".
function bigPageBlocks(n: number): BlockDto[] {
  const out: BlockDto[] = [];
  for (let i = 0; i < n; i++) {
    switch (i % 6) {
      case 0:
        out.push(b(`Paragraph **${i}** with *emphasis*, a [[ref ${i % 50}]] and a #tag${i % 20}.`));
        break;
      case 1:
        out.push(b(`## Heading ${i}`));
        break;
      case 2:
        out.push(b("```js\nfunction f" + i + "(x) {\n  return x * " + i + ";\n}\n```"));
        break;
      case 3:
        out.push(b(`| Col A | Col B |\n| --- | --- |\n| row ${i} | val ${i} |\n| row ${i + 1} | val ${i + 1} |`));
        break;
      case 4:
        out.push(b(`Display math: $$\\sum_{k=0}^{${i}} k = \\frac{${i}(${i}+1)}{2}$$`));
        break;
      default:
        out.push(
          b(`Block ${i}: a longer line of prose that wraps so the placeholder height is a realistic proxy for the rendered paragraph, with a [[link ${i % 30}]].`)
        );
        break;
    }
  }
  return out;
}
if (typeof location !== "undefined" && /[?&]big\b/.test(location.search)) {
  NAMED.push({ name: "Big", kind: "page", title: "Big", pre_block: "title:: Big", blocks: bigPageBlocks(2000) });
}
if (typeof location !== "undefined" && /[?&]regressions\b/.test(location.search)) {
  NAMED.push(
    {
      name: "Preamble regression",
      kind: "page",
      title: "Preamble regression",
      pre_block: "Intro before the first outline marker",
      blocks: [b("First marked block")],
    },
    {
      name: "First-block properties regression",
      kind: "page",
      title: "First-block properties regression",
      pre_block: null,
      blocks: [b("alias:: fbpr\ntags:: testing, properties"), b("Visible body")],
    },
    {
      name: "Block embed source regression",
      kind: "page",
      title: "Block embed source regression",
      pre_block: null,
      blocks: [
        b("Embedded root\nid:: ui-block-embed-root", [
          b("Embedded child", [b("Embedded grandchild")]),
        ]),
      ],
    },
    {
      name: "Block embed regression",
      kind: "page",
      title: "Block embed regression",
      pre_block: null,
      blocks: [b("{{embed ((ui-block-embed-root))}}")],
    },
  );
}

const mockHighlights: Record<string, { label: string; highlights: Highlight[]; page?: number; scale?: number }> = {};
// In-memory UI session for the browser mock (no backend file).
let mockSession: string | null = null;
let mockLinkFirstMatch = false;
let mockGuideAnnounced = false;
const mockAssets: Record<string, Uint8Array> = {};
const mockAppBools: Record<string, boolean> = {};
const mockAppStrings: Record<string, string> = {};

// A tiny valid silent WAV (0.2s, 8kHz/8-bit mono) so the mock audio asset actually
// renders the <audio> player — WAV is natively decodable in headless Chromium,
// unlike mp4/mp3 — letting the screenshot harness verify the audio controls + the
// widen toggle. (Real graphs hold mp3/mp4; those play in WebKitGTK, codec permitting.)
const SILENT_WAV_B64 =
  "UklGRmQGAABXQVZFZm10IBAAAAABAAEAQB8AAEAfAAABAAgAZGF0YUAGAACAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgIA==";

// Synthesize an hls__ page DTO from stored highlights (mirrors the Rust
// hls_page_document), so the Notes flow is demoable in the browser.
function hlsPageDto(name: string): PageDto | null {
  for (const [pdf, { label, highlights }] of Object.entries(mockHighlights)) {
    if (hlsPageName(pdf) !== name) continue;
    const blocks: BlockDto[] = highlights.map((h) => {
      const lines = [h.text ?? ""];
      lines.push(`hl-page:: ${h.page}`, `hl-color:: ${h.color}`);
      if (h.image != null) lines.push("hl-type:: area");
      lines.push("ls-type:: annotation", `id:: ${h.id}`);
      return { id: h.id, raw: lines.join("\n"), collapsed: false, children: [] };
    });
    return {
      name,
      kind: "page",
      title: label,
      pre_block: `file:: [${label}](../assets/${pdf})\nfile-path:: ../assets/${pdf}`,
      blocks,
    };
  }
  return null;
}

function decodeB64(b64: string): Uint8Array {
  const bin = atob(b64);
  const arr = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) arr[i] = bin.charCodeAt(i);
  return arr;
}

function cloneBlock(block: BlockDto): BlockDto {
  return { ...block, children: block.children.map(cloneBlock) };
}

function clonePage(page: PageDto): PageDto {
  return { ...page, blocks: page.blocks.map(cloneBlock) };
}

function mockGuidePage(title: string, blocks: BlockDto[]): GuidePage {
  return {
    title,
    markdown: `- # ${title}\n`,
    page: {
      name: `Tine-guide/${title}`,
      kind: "page",
      title,
      pre_block: null,
      blocks,
      format: "md",
      read_only: true,
      guide: true,
    },
  };
}

function mockGuidePages(): GuidePage[] {
  return [
    mockGuidePage("Tine Guide", [
      b("# Tine Guide", [
        b("[[Features/Sheets]] - create grids, tables, boards, queries, and formulas"),
        b("[[Features/Formulas]] - build read-only computed columns with the visual editor"),
        b("[[Features/Quick capture]] - capture into your graph from anywhere"),
        b("[[Features/PDF annotation]] - highlight PDFs beside your notes"),
        b("[[Features/Tips & shortcuts]] - learn the daily commands"),
      ]),
    ]),
    mockGuidePage("Features/Sheets", [
      b("# Sheets"),
      b(
        "## Positional grid\ntine.view:: grid\ntine.header:: true",
        [
          b("", [b("Area"), b("Owner"), b("Notes")]),
          b("", [b("Spec"), b("Martin"), b("Keep v1 narrow")]),
          b("", [b("Build"), b("Codex"), b("Live grid in the Guide")]),
        ],
        false,
        [["tine.view", "grid"], ["tine.header", "true"]]
      ),
      b(
        "## Formula table\ntine.view:: table\ntine.fields:: status=enum:todo,reading,done;rating=number;done=checkbox\ntine.formula.effort:: rating * 2",
        [
          b("Bases study\nstatus:: reading\nrating:: 5\ndone:: false"),
          b("CSV import notes\nstatus:: todo\nrating:: 3\ndone:: false"),
        ],
        false,
        [
          ["tine.view", "table"],
          ["tine.fields", "status=enum:todo,reading,done;rating=number;done=checkbox"],
          ["tine.formula.effort", "rating * 2"],
        ]
      ),
      b("## Create one yourself", [
        b("1. Type a heading block and add `tine.view:: grid` under it."),
        b("2. Add child rows, one bullet per row and one child bullet per cell."),
        b("3. What you should see: the outline renders as a live grid."),
      ]),
    ]),
    mockGuidePage("Features/Formulas", [
      b("# Formulas"),
      b(
        "## A formula in action\ntine.view:: table\ntine.fields:: task=text;hours=number;done=checkbox\ntine.formula.plan:: if(hours > 3, \"focus block\", \"quick task\")",
        [
          b("Sketch the outline\nhours:: 2\ndone:: true"),
          b("Write the first draft\nhours:: 5\ndone:: false"),
        ],
        false,
        [
          ["tine.view", "table"],
          ["tine.fields", "task=text;hours=number;done=checkbox"],
          ["tine.formula.plan", 'if(hours > 3, "focus block", "quick task")'],
        ]
      ),
      b("## Create one yourself", [
        b("1. Make a table with a numeric field, then right-click a column header and choose Add formula."),
        b("2. Build the value from the visual faces, or use the `</> raw` box to type it."),
        b("3. What you should see: a read-only computed column that evaluates live."),
      ]),
    ]),
    mockGuidePage("Features/Quick capture", [b("# Global quick-capture"), b("## Create one yourself", [b("1. Bind `tine --capture` to a desktop shortcut."), b("2. What you should see: a capture box opens over any app.")])]),
    mockGuidePage("Features/PDF annotation", [b("# PDF annotation"), b("## Create one yourself", [b("1. Drop a PDF into the graph and open it."), b("2. What you should see: highlights become linked note blocks.")])]),
    mockGuidePage("Features/Tips & shortcuts", [b("# Tips & shortcuts"), b("## Create one yourself", [b("1. Press Ctrl+K and run a command."), b("2. What you should see: the command runs without leaving the page.")])]),
    mockGuidePage("Feature showcase", [b("# Feature showcase"), b("## Create one yourself", [b("1. Create one block per construct you want to inspect."), b("2. What you should see: each construct renders live.")])]),
  ];
}

function mockGuideCopyName(title: string): string {
  return `tine-guide/${title}`;
}

function rewriteMockGuideRefs(raw: string, copied: Map<string, string>): string {
  return raw.replace(/\[\[([^\]]+)\]\]/g, (match, target: string) => {
    const to = copied.get(target.trim().toLowerCase());
    return to ? `[[${to}]]` : match;
  });
}

function cloneGuideBlockForCopy(block: BlockDto, copied: Map<string, string>): BlockDto {
  const raw = rewriteMockGuideRefs(block.raw, copied);
  return {
    ...block,
    id: nid(),
    raw,
    children: block.children.map((child) => cloneGuideBlockForCopy(child, copied)),
    properties: propertyLines(raw),
  };
}

export function mockBackend(): Backend {
  const all = [...PAGES, ...NAMED];
  const find = (name: string) =>
    all.find((p) => p.name.toLowerCase() === name.toLowerCase()) ?? null;

  // Parse a block's `key:: value` property lines (mirrors the real backend's
  // block_to_dto), so query results carry `properties` for the table columns and
  // the aggregation summary (sum/avg of a property).
  const parseProps = (raw: string): [string, string][] => {
    const out: [string, string][] = [];
    for (const line of raw.split("\n")) {
      const m = /^([A-Za-z][\w-]*):: ?(.*)$/.exec(line.trim());
      if (m && !["id", "collapsed"].includes(m[1])) out.push([m[1], m[2].trim()]);
    }
    return out;
  };

  // Collect (page, matching blocks) where keep() holds, grouped by page.
  const collect = (keep: (b: BlockDto) => boolean, exclude?: string): RefGroup[] => {
    const groups: RefGroup[] = [];
    for (const p of all) {
      if (exclude && p.name.toLowerCase() === exclude.toLowerCase()) continue;
      const matched: BlockDto[] = [];
      // Track the ancestor chain so a matched nested block carries a breadcrumb
      // (like the real backend's query::collect), exercising the block-ref panel.
      const walk = (bs: BlockDto[], anc: string[]) =>
        bs.forEach((b) => {
          if (keep(b)) matched.push({ ...b, breadcrumb: anc, properties: parseProps(b.raw) });
          walk(b.children, [...anc, b.raw.split("\n")[0] ?? ""]);
        });
      walk(p.blocks, []);
      if (matched.length) groups.push({ page: p.name, kind: p.kind, blocks: matched });
    }
    return groups;
  };

  return {
    async loadGraph() {
      return { kind: "loaded" as const, binding_generation: 1, meta: {
        root: "/mock/graph",
        journals_dir: "journals",
        pages_dir: "pages",
        preferred_workflow: "todo",
        shortcuts: {},
        start_of_week: 0,
        block_hidden_properties: [],
        default_journal_template: null,
        favorites: [],
        journal_page_title_format: "MMM do, yyyy",
        journal_file_name_format: "yyyy_MM_dd",
        preferred_format: "md",
        enable_timetracking: true,
        logbook_with_second_support: true,
        logbook_enabled_in_timestamped_blocks: true,
        logbook_enabled_in_all_blocks: false,
        guide_announced: mockGuideAnnounced,
        macros: {
          // Demo user macros so the kitchen-sink exercises inline and block expansions.
          poem: "Roses are $1, violets are $2.",
          hi: "Hello, **$1**! See [[$2]].",
          card: "## $1\n\n$2\n\n+ see [[$1]]",
        },
      }};
    },
    async listKnownGraphs() {
      return [{ path: "/mock/graph", name: "graph" }];
    },
    async inspectGraphAccess(path: string) {
      return { graph_root: path || "/mock/graph", external_assets_path: null, approved: true };
    },
    async approveExternalAssets() {},
    async openGraphWindow() {
      return { kind: "focused_existing" as const, window_label: "main" };
    },
    async startupGraphPath() {
      return "/mock/graph";
    },
    async captureTarget() {
      return "main";
    },
    async bindCaptureGraph() {},
    async forgetKnownGraph() {},
    async appPlatform(): Promise<"android" | "ios" | "desktop"> {
      const requested = new URLSearchParams(globalThis.location?.search ?? "").get("platform");
      if (requested === "android" || requested === "ios") return requested;
      return "desktop";
    },
    async listInstalledPlugins() {
      return mockPlugins.map((plugin) => ({ ...plugin }));
    },
    async installPlugin(manifestJson: string, wasm: Uint8Array) {
      const manifest = JSON.parse(manifestJson) as { id: string; version: string };
      const key = `${manifest.id}@${manifest.version}`;
      mockPluginEntries.set(key, wasm.slice());
      const record: InstalledPluginRecord = {
        id: manifest.id,
        version: manifest.version,
        manifest_json: manifestJson,
        sha256: "mock",
        selected: false,
        enabled: false,
      };
      mockPlugins.push(record);
      return { ...record };
    },
    async uninstallPlugin(id: string, version: string) {
      const index = mockPlugins.findIndex((record) => {
        const manifest = JSON.parse(record.manifest_json) as { id: string; version: string };
        return manifest.id === id && manifest.version === version;
      });
      if (index === -1) throw new Error("plugin version is not installed");
      mockPlugins.splice(index, 1);
      mockPluginEntries.delete(`${id}@${version}`);
      if (!mockPlugins.some((record) => (JSON.parse(record.manifest_json) as { id: string }).id === id)) {
        delete mockAppStrings[`plugin-settings:${id}`];
      }
    },
    async readPluginEntry(id: string, version: string) {
      const entry = mockPluginEntries.get(`${id}@${version}`);
      if (!entry) throw new Error("plugin version is not installed");
      return entry.slice();
    },
    async setPluginEnabled(id: string, version: string, enabled: boolean) {
      for (const record of mockPlugins) {
        const manifest = JSON.parse(record.manifest_json) as { id: string; version: string };
        if (manifest.id === id) {
          record.selected = manifest.version === version;
          record.enabled = record.selected && enabled;
        }
      }
    },
    async verifyPluginRegistry() {
      // Browser mock has no embedded native key. Registry tests mock this boundary.
    },
    async setSystemBarAppearance(): Promise<void> {},
    async quit(): Promise<void> {
      // No-op in the mock/screenshot harness — there's no process to exit.
    },
    async closeGraphWindow(): Promise<void> {
      // No-op in the mock/screenshot harness.
    },
    async openDevtools(): Promise<void> {
      // No-op in the mock/screenshot harness — no native WebView inspector.
    },
    async defaultGraphParent(): Promise<string> {
      return "/mock";
    },
    async listPages(): Promise<PageEntry[]> {
      return all.map(mockPageEntry);
    },
    async journalFeedPage(limit: number, beforeDay: number | null) {
      const now = new Date();
      const as_of_day = now.getFullYear() * 10_000 + (now.getMonth() + 1) * 100 + now.getDate();
      const candidates = PAGES
        .filter((p) => p.kind === "journal")
        .map((page) => ({ page, day: mockJournalDayKey(page.name) }))
        .filter((row): row is { page: PageDto; day: number } =>
          row.day !== null && row.day <= as_of_day && (beforeDay === null || row.day < beforeDay)
        )
        .sort((a, b) => b.day - a.day);
      const rows = candidates.slice(0, limit);
      const done = rows.length === candidates.length;
      return {
        pages: rows.map(({ page }) => page),
        next_before_day: done || !rows.length ? null : rows[rows.length - 1].day,
        done,
        as_of_day,
      };
    },
    async journalContentDays(): Promise<number[]> {
      return [];
    },
    async getPage(name: string): Promise<PageDto | null> {
      if (name.startsWith("hls__")) return hlsPageDto(name);
      return find(name);
    },
    async graphSourceFiles(includeJournals: boolean) {
      // Synthetic sources so the diff panel is exercisable against the mock
      // backend (the real comparison still needs mldoc + lsdoc-wasm at runtime).
      const files = [
        { rel: "pages/welcome.md", text: "# Welcome\n- a **bold** note with [[Link]]\n", format: "md" as const },
        { rel: "pages/tasks.md", text: "- TODO finish #project\n- DONE ship it\n", format: "md" as const },
      ];
      if (includeJournals) {
        files.push({ rel: "journals/2026_07_04.md", text: "- met with [[Alice]] re: $$x^2$$\n", format: "md" as const });
      }
      return files.map((f) => ({ ...f, bytes: new TextEncoder().encode(f.text).length }));
    },
    async savePage(_page: PageDto, _baseRev: string | null, _force?: boolean): Promise<string> {
      return "mock-rev"; // no-op in mock
    },
    async guidePages(): Promise<GuidePage[]> {
      return mockGuidePages().map((g) => ({ ...g, page: clonePage(g.page) }));
    },
    async copyGuideIntoGraph(title: string): Promise<GuideCopyResult> {
      const guides = mockGuidePages();
      const viewed = guides.find((g) => g.title.toLowerCase() === title.trim().toLowerCase());
      if (!viewed) throw new Error("unknown bundled guide page");
      const copied = new Map(guides.map((g) => [g.title.toLowerCase(), mockGuideCopyName(g.title)]));
      const createdPages: string[] = [];
      const skippedPages: string[] = [];
      for (const guide of guides) {
        const name = mockGuideCopyName(guide.title);
        if (find(name)) {
          skippedPages.push(name);
          continue;
        }
        const blocks = guide.page.blocks.map((block) => cloneGuideBlockForCopy(block, copied));
        all.push({ name, kind: "page", title: name, pre_block: null, blocks, format: "md", read_only: false, guide: false });
        createdPages.push(name);
      }
      if (!mockAssets["quick-capture.png"]) mockAssets["quick-capture.png"] = new Uint8Array([0]);
      return {
        name: mockGuideCopyName(viewed.title),
        created: createdPages.length > 0,
        created_pages: createdPages,
        skipped_pages: skippedPages,
        copied_assets: ["quick-capture.png"],
      };
    },
    async setGuideAnnounced(announced: boolean): Promise<void> {
      mockGuideAnnounced = announced;
    },
    async createGraph(_dir: string): Promise<string> {
      return "/mock/new-graph"; // no real scaffolding in the browser mock
    },
    async getBacklinks(name: string): Promise<RefGroup[]> {
      const n = name.toLowerCase();
      return collect((b) => pageRefs(b.raw).some((r) => r.toLowerCase() === n), name);
    },
    async getBacklinkFilterContext(name: string, targets: BacklinkFilterTarget[]): Promise<BacklinkFilterContext> {
      const excluded = name.trim().toLowerCase();
      const wanted = new Map(targets.map((item) => [
        `${item.kind}\0${item.page.toLowerCase()}\0${item.block_id}`,
        item,
      ]));
      const entries: BacklinkFilterContext["entries"] = [];
      const visit = (page: PageDto, block: BlockDto): void => {
        const key = `${page.kind}\0${page.name.toLowerCase()}\0${block.id}`;
        const target = wanted.get(key);
        if (target) {
          const text: string[] = [];
          const facets = new Map<string, string>();
          const subtree = (node: BlockDto) => {
            text.push(node.raw);
            for (const ref of pageRefs(node.raw)) {
              const normalized = ref.trim().toLowerCase();
              if (normalized && normalized !== excluded && !facets.has(normalized)) facets.set(normalized, ref);
            }
            if (node.marker) facets.set(node.marker.toLowerCase(), node.marker);
            node.children.forEach(subtree);
          };
          subtree(block);
          entries.push({ ...target, text: text.join("\n"), facets: [...facets.values()] });
        }
        block.children.forEach((child) => visit(page, child));
      };
      for (const page of all) page.blocks.forEach((block) => visit(page, block));
      return { entries, truncated: entries.length < wanted.size };
    },
    async getUnlinkedRefs(name: string): Promise<RefGroup[]> {
      const n = name.toLowerCase();
      const re = new RegExp(`\\b${n.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\b`);
      return collect(
        (b) => re.test(b.raw.toLowerCase()) && !pageRefs(b.raw).some((r) => r.toLowerCase() === n),
        name
      );
    },
    async warmDone(): Promise<boolean> {
      return true;
    },
    async getBlockRefCounts(): Promise<Record<string, number>> {
      const counts: Record<string, number> = {};
      const walk = (bs: BlockDto[]) =>
        bs.forEach((b) => {
          for (const id of blockRefIds(b.raw)) counts[id] = (counts[id] ?? 0) + 1;
          walk(b.children);
        });
      for (const p of all) walk(p.blocks);
      return counts;
    },
    async getBlockReferrers(uuid: string): Promise<RefGroup[]> {
      // No exclude → same-page referrers included (matches the backend).
      return collect((b) => blockRefIds(b.raw).includes(uuid));
    },
    async deletePage(): Promise<void> {
      // no-op in mock
    },
    async renamePage(): Promise<void> {
      // no-op in mock
    },
    async publishHtml(): Promise<[string, number]> {
      return ["/mock/graph/publish", all.length];
    },
    async pagePrintHtml(name: string, _opts): Promise<string> {
      // Dev-preview stub: a small self-contained doc so the print harness/flow can
      // render something without the real publish pipeline.
      return (
        `<!doctype html><html><head><meta charset="utf-8"><title>${name}</title>` +
        `<style>body{font-family:Inter,sans-serif;margin:0;padding:24px;line-height:1.6}` +
        `h1{font-size:1.9rem;margin:0 0 1rem}@page{margin:16mm}@media print{a{color:inherit}}</style>` +
        `</head><body class="print"><main><h1 class="page">${name}</h1>` +
        `<ul class="outline"><li>Mock print export of <strong>${name}</strong>.</li>` +
        `<li>The real document is rendered by tine-core.</li></ul></main></body></html>`
      );
    },
    async runAdvancedQuery(query: string) {
      // Dev-preview approximation (the real engine runs in the Tauri backend): map
      // the one head the harness needs — `(task ?b #{"TODO" …})` — so the "switch to
      // advanced" skeleton returns results in the screenshot harness. Anything else
      // is reported as unsupported, matching the real ran/ignored contract shape.
      const markers = query.match(/\b(TODO|DOING|DONE|NOW|LATER|WAITING|CANCELED)\b/g) ?? [];
      if (/\(\s*task\b/i.test(query) && markers.length) {
        const set = new Set(markers);
        const groups = collect((b) => {
          const m = leadingMarker(b.raw);
          return !!m && set.has(m);
        });
        return { groups, ran: ["task"], ignored: [], supported: true };
      }
      return { groups: [], ran: [], ignored: [], supported: false };
    },
    async runQuery(query: string): Promise<RefGroup[]> {
      // Simplified mock evaluator: task/todo filter or page-ref filter.
      if (/\b(todo|task)\b/i.test(query)) {
        // Uppercase markers only (the lowercase `todo`/`task` keyword is excluded
        // by the case-sensitive match, so no extra filtering is needed).
        const named = query.match(/\b(TODO|DOING|DONE|NOW|LATER|WAITING|CANCELED)\b/g) ?? [];
        const set = named.length ? named : ["TODO", "DOING", "NOW", "LATER"];
        return collect((b) => {
          const m = leadingMarker(b.raw);
          return !!m && set.includes(m);
        });
      }
      const tag = /\(\s*tag\s+(?:"((?:[^"\\]|\\.)*)"|([^) \t\r\n]+))\s*\)/i.exec(query);
      if (tag) {
        const n = (tag[1] ?? tag[2] ?? "").replace(/\\"/g, "\"").replace(/\\\\/g, "\\").toLowerCase();
        return collect((b) => pageRefs(b.raw).some((r) => r.toLowerCase() === n));
      }
      const ref = pageRefs(query)[0];
      if (ref) {
        const n = ref.toLowerCase();
        return collect((b) => pageRefs(b.raw).some((r) => r.toLowerCase() === n));
      }
      return [];
    },
    async exportQuerySubtrees(specs: QueryExportSpec[]): Promise<QueryExportBatch> {
      let remainingRoots = 50;
      let remainingNodes = 2_000;
      let remainingBytes = 8 * 1024 * 1024;
      const estimate = (block: BlockDto) => block.id.length + block.raw.length + 128;
      const countTree = (root: BlockDto) => {
        let count = 0;
        const stack = [root];
        while (stack.length) {
          const block = stack.pop()!;
          count++;
          for (const child of block.children) stack.push(child);
        }
        return count;
      };
      const copyTree = (block: BlockDto): BlockDto | null => {
        const bytes = estimate(block);
        if (remainingNodes === 0 || bytes > remainingBytes) return null;
        remainingNodes--;
        remainingBytes -= bytes;
        const children: BlockDto[] = [];
        for (const child of block.children) {
          const copied = copyTree(child);
          if (!copied) break;
          children.push(copied);
        }
        return { ...block, children };
      };
      const findBlock = (page: PageDto, id: string): BlockDto | null => {
        const stack = [...page.blocks];
        while (stack.length) {
          const block = stack.pop()!;
          if (block.id === id) return block;
          for (const child of block.children) stack.push(child);
        }
        return null;
      };
      const results = [];
      for (const spec of specs.slice(0, 64)) {
        const groups = spec.advanced
          ? (await this.runAdvancedQuery(spec.query)).groups
          : await this.runQuery(spec.query);
        const total = groups.reduce((sum, group) => sum + group.blocks.length, 0);
        const hydrated: RefGroup[] = [];
        let shown = 0;
        let omittedNodes = 0;
        for (const group of groups) {
          const page = find(group.page);
          if (!page) continue;
          const blocks: BlockDto[] = [];
          for (const shallow of group.blocks) {
            if (remainingRoots === 0) break;
            remainingRoots--;
            const source = findBlock(page, shallow.id) ?? shallow;
            const before = remainingNodes;
            const copied = copyTree(source);
            omittedNodes += Math.max(0, countTree(source) - (before - remainingNodes));
            if (copied) {
              blocks.push(copied);
              shown++;
            }
          }
          if (blocks.length) hydrated.push({ page: group.page, kind: group.kind, blocks });
          if (remainingRoots === 0) break;
        }
        results.push({ key: spec.key, groups: hydrated, shown, total, omitted_nodes: omittedNodes });
      }
      return { results, omitted_queries: Math.max(0, specs.length - 64) };
    },
    async readCustomCss(): Promise<string> {
      return (globalThis as unknown as { __tineMockCustomCss?: string }).__tineMockCustomCss ?? "";
    },
    async pageAliases(): Promise<[string, string][]> {
      return [];
    },
    async pageIcons(names: string[]): Promise<Record<string, string>> {
      const out: Record<string, string> = {};
      for (const name of names) {
        const m = all.find((p) => p.name === name)?.pre_block?.match(/^icon::\s*(.+)$/m);
        if (m) out[name] = m[1].trim();
      }
      return out;
    },
    async setFavorites(): Promise<void> {
      // no-op in the browser mock
    },
    async setPreferredWorkflow(): Promise<void> {
      // no-op in the browser mock
    },
    async setTimetrackingEnabled(): Promise<void> {
      // no-op in the browser mock
    },
    async setPreferredFormat(): Promise<void> {
      // no-op in the browser mock
    },
    async setJournalTitleFormat(): Promise<void> {
      // no-op in the browser mock
    },
    async setDefaultJournalTemplate(): Promise<void> {
      // no-op in the browser mock
    },
    async setStartOfWeek(): Promise<void> {
      // no-op in the browser mock
    },
    async openExternal(url: string): Promise<void> {
      try {
        window.open(url, "_blank", "noreferrer");
      } catch {
        // ignore
      }
    },
    async queryFacets(): Promise<[string, string[]][]> {
      const map = new Map<string, Set<string>>();
      const internal = new Set(["id", "collapsed"]);
      const walk = (bs: BlockDto[]) =>
        bs.forEach((b) => {
          for (const line of b.raw.split("\n")) {
            const m = /^([A-Za-z][\w-]*):: ?(.*)$/.exec(line.trim());
            if (m && !internal.has(m[1])) {
              const set = map.get(m[1]) ?? new Set<string>();
              if (m[2].trim()) set.add(m[2].trim());
              map.set(m[1], set);
            }
          }
          walk(b.children);
        });
      all.forEach((p) => walk(p.blocks));
      return [...map.entries()].map(([k, vs]) => [k, [...vs].sort()] as [string, string[]]);
    },
    async search(query: string, limit: number): Promise<RefGroup[]> {
      const q = canonicalFold(query.trim());
      if (!q) return [];
      let n = limit;
      const groups = collect((b) => canonicalFold(b.raw).includes(q));
      for (const g of groups) {
        if (g.blocks.length > n) g.blocks = g.blocks.slice(0, n);
        n -= g.blocks.length;
      }
      return groups.filter((g) => g.blocks.length > 0);
    },
    async runGraphSearch(source: string, pageLimit: number, blockLimit: number, _lane?: string, explain = false, scope?: import("./types").QueryPageScope): Promise<QueryExecution> {
      // Browser-preview approximation only (ADR 0016). Production matching,
      // diagnostics, and UTF-16 evidence come from Rust's QueryPlan evaluator.
      const matcher = parseSearchQuery(source);
      if (matcher.kind === "invalid") {
        return {
          hits: [],
          diagnostics: [{ code: "invalid_regex", message: matcher.error }],
          explanation: { branches: [] },
          cancelled: false,
        };
      }
      const bare = simpleTerm(matcher);
      const pages = scope ? [] : all
        .map((page) => ({ page, score: bare ? fuzzyScore(bare, canonicalFold(page.name)) : 0 }))
        .filter(({ page, score }) => bare ? score > 0 : matcherMatches(matcher, canonicalFold(page.name), page.name))
        .sort((a, b) => b.score - a.score)
        .slice(0, pageLimit)
        .map(({ page, score }) => ({
          entity: "page" as const,
          page: mockPageEntry(page),
          display_text: page.name,
          evidence: [{
            clause_id: 1,
            field: "page_name" as const,
            mode: bare ? "fuzzy" as const : matcher.kind === "regex" ? "regex" as const : "contains" as const,
            spans: matchHighlights(matcher, page.name),
            score,
          }],
          score,
          match_class: bare
            ? canonicalFold(page.name) === bare ? "exact" as const
              : canonicalFold(page.name).startsWith(bare) ? "prefix" as const
              : canonicalFold(page.name).includes(bare) ? "substring" as const
              : "fuzzy" as const
            : undefined,
        }));
      const inScope = (group: RefGroup) => {
        if (!scope) return true;
        const page = all.find((candidate) => candidate.kind === group.kind && canonicalFold(candidate.name) === canonicalFold(group.page));
        if (!page) return false;
        return scope.path
          ? mockPagePath(page) === scope.path
          : page.kind === scope.pageKind && canonicalFold(page.name) === canonicalFold(scope.name);
      };
      const blocks = collect((block) => matcherMatches(matcher, canonicalFold(block.raw), block.raw))
        .filter(inScope)
        .flatMap((group) => group.blocks.map((block) => ({ group, block })))
        .slice(0, Math.max(0, blockLimit))
        .map(({ group, block }) => {
          return {
            entity: "block" as const,
            page: group.page,
            kind: group.kind,
            block,
            display_text: block.raw,
            evidence: [{
              clause_id: 1,
              field: "visible_content" as const,
              mode: matcher.kind === "regex" ? "regex" as const : "contains" as const,
              spans: matchHighlights(matcher, block.raw),
            }],
          };
        });
      return {
        hits: [...pages, ...blocks],
        diagnostics: [],
        explanation: {
          branches: explain ? [
            { description: bare ? `Page names fuzzily match “${source}”` : `Page names match “${source}”`, children: [] },
            { description: `Block content matches “${source}”`, children: [] },
          ] : [],
        },
        cancelled: false,
      };
    },
    async listTemplates() {
      return [
        {
          name: "meeting",
          page: "Templates",
          kind: "page" as const,
          blocks: [
            { id: "t1", raw: "## Meeting [[<% today %>]]", collapsed: false, children: [] },
            { id: "t2", raw: "Attendees:", collapsed: false, children: [] },
            { id: "t3", raw: "TODO Follow up", collapsed: false, children: [] },
          ],
        },
      ];
    },
    async quickSwitch(query: string, limit: number): Promise<PageEntry[]> {
      const q = canonicalFold(query.trim());
      return all
        .filter((p) => canonicalFold(p.name).includes(q))
        .slice(0, limit)
        .map(mockPageEntry);
    },
    async captureQuickSwitch(query: string, limit: number): Promise<PageEntry[]> {
      return this.quickSwitch(query, limit);
    },
    async resolveBlock(uuid: string): Promise<RefGroup | null> {
      const find = (blocks: BlockDto[]): BlockDto | null => {
        for (const block of blocks) {
          if (block.raw.includes(`id:: ${uuid}`)) return block;
          const child = find(block.children);
          if (child) return child;
        }
        return null;
      };
      for (const p of all) {
        const found = find(p.blocks);
        if (found) return { page: p.name, kind: p.kind, blocks: [{ ...found, children: [] }] };
      }
      return null;
    },
    async resolveBlocks(uuids: string[]): Promise<(RefGroup | null)[]> {
      return Promise.all(uuids.map((u) => this.resolveBlock(u)));
    },
    async previewBlock(uuid: string, maxNodes: number): Promise<BlockPreview | null> {
      let group: RefGroup | null = null;
      const find = (blocks: BlockDto[]): BlockDto | null => {
        for (const block of blocks) {
          if (block.id === uuid || block.raw.includes(`id:: ${uuid}`)) return block;
          const child = find(block.children);
          if (child) return child;
        }
        return null;
      };
      for (const page of all) {
        const found = find(page.blocks);
        if (found) {
          group = { page: page.name, kind: page.kind, blocks: [found] };
          break;
        }
      }
      if (!group) return null;
      let emitted = 0;
      let truncated = 0;
      const count = (blocks: BlockDto[]): number => blocks.reduce((n, b) => n + 1 + count(b.children), 0);
      const copy = (blocks: BlockDto[]): BlockDto[] => {
        const out: BlockDto[] = [];
        for (const block of blocks) {
          if (emitted >= Math.max(1, maxNodes)) {
            truncated += count([block]);
            continue;
          }
          emitted++;
          out.push({ ...block, children: copy(block.children) });
        }
        return out;
      };
      return { group: { ...group, blocks: copy(group.blocks) }, truncated };
    },
    async readAsset(name: string, maxBytes?: number): Promise<Uint8Array> {
      void maxBytes;
      if (mockAssets[name]) return mockAssets[name];
      if (name === "sample.pdf") return decodeB64(SAMPLE_PDF_B64);
      if (name === "voice_memo.wav") return decodeB64(SILENT_WAV_B64);
      return new Uint8Array();
    },
    async streamAsset(name: string): Promise<string> {
      const bytes = await this.readAsset(name);
      if (!bytes.length) return "";
      const type = name.toLowerCase().endsWith(".wav") ? "audio/wav" : "application/octet-stream";
      return URL.createObjectURL(new Blob([bytes as unknown as BlobPart], { type }));
    },
    async readLocalImage(_path: string): Promise<Uint8Array> {
      // The mock has no filesystem; local-file images never resolve here.
      return new Uint8Array();
    },
    async saveAsset(name: string, bytes: Uint8Array): Promise<string> {
      mockAssets[name] = bytes;
      return name;
    },
    async pasteImage(): Promise<string | null> {
      return null; // no OS clipboard in the browser mock
    },
    async readClipboardImage(): Promise<Uint8Array | null> {
      return null; // no OS clipboard in the browser mock
    },
    async importAsset(path: string, name?: string): Promise<string> {
      return name ?? path.split("/").pop() ?? path;
    },
    async importNativeCapture(path: string, name: string): Promise<string> {
      return name || path.split("/").pop() || path;
    },
    async clipboardFiles() {
      return { files: [], skipped: 0, truncated: false };
    },
    async readTextFile(_path: string): Promise<string> {
      throw new Error("local text files are unavailable in the browser mock");
    },
    async openAsset(): Promise<void> {
      // no OS opener in the browser mock
    },
    async openPageFile(): Promise<void> {
      // no OS file manager in the browser mock
    },
    async editAssetExternal(): Promise<void> {
      // no external editor in the browser mock
    },
    async detectMediaEditor(): Promise<string> {
      return ""; // nothing to probe in the browser mock
    },
    async listOrphanAssets() {
      return [
        { name: "old_screenshot_20260601_091500.png", size: 184_320, modified: 1_748_762_100 },
        { name: "unused_clip_20260512_140233.mp4", size: 5_242_880, modified: 1_747_051_353 },
      ];
    },
    async trashAsset(): Promise<void> {
      // no-op in the browser mock
    },
    async assetTrashStats() {
      return { count: 3, bytes: 1_572_864, pages: 1, journals: 0, conflicts: 0, other: 0 };
    },
    async emptyAssetTrash(): Promise<number> {
      return 3;
    },
    async listJournalConflicts() {
      // Default demo state is clean: no sticky "duplicate journal day" toast and no
      // reconcile banner cluttering the marketing screenshots. The reconcile flow is
      // demoed on demand via `?conflicts` (mirrors the `?big` virtualization gate).
      if (typeof location !== "undefined" && !/[?&]conflicts\b/.test(location.search)) return [];
      return [
        {
          title: "Friday, 26-06-2026",
          files: [
            { name: "2026_06_26.org", path: "journals/2026_06_26.org", preview: "Tried out the Org demo graph in Tine today", canonical: true },
            { name: "Friday, 26-06-2026.org", path: "journals/Friday, 26-06-2026.org", preview: "something something", canonical: false },
          ],
        },
      ];
    },
    async trashJournalFile(): Promise<void> {
      // no-op in the browser mock
    },
    async readJournalFile(name: string): Promise<string> {
      return name.startsWith("Friday")
        ? "* something something\n*\n"
        : "* Tried out the Org demo graph in Tine today\n* TODO follow up on the [[kitchen-sink]] feature tour\nSCHEDULED: <2026-06-27 Sat>\n* DONE loaded the graph and clicked around\n";
    },
    async getPageByPath(path: string): Promise<PageDto | null> {
      const page = all.find((p) => mockPagePath(p) === path);
      if (page) return { ...page, path };
      // The duplicate-day stray opens to its own content (#21); other paths fall
      // back to the canonical page by name.
      const stray = path.includes("Friday");
      return {
        name: "Friday, 26-06-2026",
        kind: "journal",
        title: "Friday, 26-06-2026",
        pre_block: null,
        blocks: [{ id: "stray-1", raw: stray ? "something something" : "Tried out the Org demo graph in Tine today", collapsed: false, children: [] }],
        rev: "mock-rev",
        format: "org",
        read_only: false,
        path,
      };
    },
    async mergePages(): Promise<void> {
      // no-op in the browser mock
    },
    async renameFileToPage(): Promise<void> {
      // no-op in the browser mock
    },
    async listSyncConflicts() {
      // Gated on the same `?conflicts` flag as the journal-day demo, so the
      // reconcile area stays out of the marketing screenshots by default.
      if (typeof location !== "undefined" && !/[?&]conflicts\b/.test(location.search)) return [];
      return [
        {
          path: "pages/Project Plan.sync-conflict-20260705-141233-A1B2C3D.md",
          base_name: "Project Plan",
          base_path: "pages/Project Plan.md",
          kind: "page" as const,
          tag: "sync-conflict-20260705-141233-A1B2C3D",
          preview: "Milestones for the launch",
        },
      ];
    },
    async syncConflictDiff() {
      const v = (text: string) => ({ uuid: "", text, child_count: 0 });
      return {
        base_rev: "mock-sync-diff-rev",
        conflict_rev: "mock-sync-copy-rev",
        rows: [
          { id: "0", kind: "unchanged" as const, mine: v("Milestones for the launch"), theirs: v("Milestones for the launch"), children: [] },
          { id: "1", kind: "modified" as const, mine: v("TODO ship the beta by Friday"), theirs: v("TODO ship the beta by Thursday"), children: [] },
          { id: "2", kind: "added" as const, mine: v("write the release notes"), theirs: null, children: [] },
          { id: "3", kind: "removed" as const, mine: null, theirs: v("ask marketing for the banner"), children: [] },
        ],
        mine_pre: "title:: Project Plan",
        theirs_pre: "title:: Project Plan",
        pre_differs: false,
        blocks_identical: false,
      };
    },
    async resolveSyncConflict(): Promise<void> {
      // no-op in the browser mock
    },
    async trashSyncConflict(): Promise<void> {
      // no-op in the browser mock
    },
    async onConflictsChanged(): Promise<() => void> {
      return () => {};
    },
    async confirm(message: string): Promise<boolean> {
      // The browser/test env has a working global confirm (unlike the WebKitGTK
      // app), so defer to it. Read it off globalThis so test stubs (vi.stubGlobal)
      // are honoured.
      const c = (globalThis as { confirm?: (m?: string) => boolean }).confirm;
      return typeof c === "function" ? c(message) : true;
    },
    async pickFolder(_title?: string): Promise<string | null> {
      return null; // no native dialog in the browser mock
    },
    async pickGraphFolder() {
      return { status: "cancelled" as const };
    },
    async pickFile(): Promise<string | null> {
      return null;
    },
    async capturePhoto() {
      return { status: "cancelled" as const };
    },
    async startRecording() {
      return { status: "cancelled" as const };
    },
    async stopRecording() {
      return { status: "cancelled" as const };
    },
    async cancelRecording() {
      return { status: "cancelled" as const };
    },
    async writeText(text: string): Promise<void> {
      try {
        await navigator.clipboard.writeText(text);
      } catch {
        // ignore
      }
    },
    async writeRich(text: string, _html: string): Promise<void> {
      try {
        await navigator.clipboard.writeText(text);
      } catch {
        // ignore
      }
    },
    async copyImageToClipboard(): Promise<void> {
      // no OS clipboard image write in the browser mock
    },
    async onGraphChanged(): Promise<() => void> {
      return () => {}; // no external watcher in the browser mock
    },
    async getBackupKeep(): Promise<number> {
      return 12;
    },
    async setBackupKeep(): Promise<void> {
      // no-op in the browser mock
    },
    async getCaptureEnterFiles(): Promise<boolean> {
      return false;
    },
    async setCaptureEnterFiles(): Promise<void> {
      // no-op in the browser mock
    },
    async getLinkFirstMatch(): Promise<boolean> {
      return mockLinkFirstMatch;
    },
    async setLinkFirstMatch(value: boolean): Promise<void> {
      mockLinkFirstMatch = value;
    },
    async getWatchMode(): Promise<string> {
      return "inotify";
    },
    async setWatchMode(): Promise<void> {
      // no-op in the browser mock
    },
    async listBackups() {
      return [];
    },
    async restoreBackup(): Promise<void> {
      // no-op in the browser mock
    },
    async loadSession(): Promise<string | null> {
      return mockSession;
    },
    async saveSession(data: string): Promise<void> {
      mockSession = data;
    },
    async takeIdentifierMigrationNotice(): Promise<boolean> {
      return false;
    },
    async gpuEnv(): Promise<GpuEnv> {
      return { software_forced: false, appimage: false };
    },
    async getSmoothScroll(): Promise<boolean> {
      return false;
    },
    async setSmoothScroll(_value: boolean): Promise<void> {
      // no-op in the browser mock
    },
    async getAppBool(key: string, fallback: boolean): Promise<boolean> {
      const v = mockAppBools[key];
      return v === undefined ? fallback : v;
    },
    async setAppBool(key: string, value: boolean): Promise<void> {
      mockAppBools[key] = value;
    },
    async getAppString(key: string, fallback: string): Promise<string> {
      const v = mockAppStrings[key];
      return v === undefined ? fallback : v;
    },
    async setAppString(key: string, value: string): Promise<void> {
      mockAppStrings[key] = value;
    },
    async applySpellcheck(): Promise<void> {
      /* no native webview in the mock */
    },
    async listSpellcheckDictionaries(): Promise<string[]> {
      // A representative set so the picker renders in the browser mock / harness.
      return ["cs_CZ", "de_DE", "en_GB", "en_US", "fr_FR", "sk_SK"];
    },
    async debugInfo(): Promise<DebugInfo> {
      return { enabled: false, path: "" };
    },
    async debugLog(_line: string): Promise<void> {
      // no-op in the browser mock
    },
    async readHighlights(pdf: string): Promise<Highlight[]> {
      return mockHighlights[pdf]?.highlights ?? [];
    },
    async openPdf(pdf: string, label: string): Promise<PdfState> {
      const current = mockHighlights[pdf] ?? { label, highlights: [] };
      mockHighlights[pdf] = current;
      return {
        highlights: current.highlights,
        page: current.page ?? null,
        scale: current.scale ?? null,
      };
    },
    async writeHighlights(pdf: string, label: string, highlights: Highlight[], _baseIds: string[]): Promise<void> {
      mockHighlights[pdf] = { ...mockHighlights[pdf], label, highlights };
    },
    async writePdfViewState(pdf: string, page: number, scale: number): Promise<void> {
      const current = mockHighlights[pdf] ?? { label: pdf, highlights: [] };
      mockHighlights[pdf] = { ...current, page, scale };
    },
    async savePdfAreaImage(
      pdf: string,
      page: number,
      id: string,
      stamp: number,
      _bytes: Uint8Array,
    ): Promise<string> {
      return `${pdf.replace(/\.pdf$/i, "")}/${page}_${id}_${stamp}.png`;
    },
  };
}
