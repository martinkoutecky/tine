// In-memory mock backend seeded with a fixture graph. Used only when running
// outside Tauri (browser dev / Playwright screenshots). Mirrors the real
// backend's shape so the UI behaves identically.

import type { Backend } from "./backend";
import type { BlockDto, GraphMeta, Highlight, PageDto, PageEntry, RefGroup } from "./types";
import { SAMPLE_PDF_B64 } from "./sample-pdf";
import { hlsPageName } from "./pdf";

function pageRefs(raw: string): string[] {
  const out: string[] = [];
  const re = /\[\[([^\]]+)\]\]|#\[\[([^\]]+)\]\]|#([\w/_.-]+)/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(raw))) out.push(m[1] ?? m[2] ?? m[3]);
  return out;
}
function leadingMarker(raw: string): string | null {
  const m = /^(TODO|DOING|DONE|NOW|LATER|WAITING|WAIT|CANCELED|CANCELLED|IN-PROGRESS)\b/.exec(raw);
  return m ? m[1] : null;
}

let _id = 0;
const nid = () => `mock-${_id++}`;

function b(raw: string, children: BlockDto[] = [], collapsed = false): BlockDto {
  return { id: nid(), raw, collapsed, children };
}

const PAGES: PageDto[] = [
  {
    name: "Jun 14th, 2026",
    kind: "journal",
    title: "Jun 14th, 2026",
    pre_block: null,
    blocks: [
      b("## Today"),
      b("Started the [[logseq-claude]] rewrite — aiming for a #fast native feel.", [
        b("The outliner is the core; everything hangs off **blocks**."),
        b("Reading the OG source for the *exact* file format and `mldoc` quirks."),
      ]),
      b("TODO [#A] Ship the M0 vertical slice\nSCHEDULED: <2026-06-15 Mon>"),
      b("DOING Wire up the [[block editor]] with caret preservation"),
      b("A code block:\n```rust\nfn main() {\n    println!(\"hello, tine\");\n}\n```"),
      b("A table:\n| Feature | Status |\n| --- | --- |\n| Outliner | done |\n| Queries | partial |"),
      b("DONE Validate round-trip on the real `shui-graph`"),
      b("Inline math works too: $E = mc^2$ and references like ((arch-1))."),
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
      b("Set up the [[logseq-claude]] repo and Rust core.", [
        b("Round-trip tests pass on the real graph."),
      ]),
      b("DONE Decide the stack: **Tauri** + *SolidJS*"),
    ],
  },
];

const NAMED: PageDto[] = [
  {
    name: "logseq-claude",
    kind: "page",
    title: "logseq-claude",
    pre_block: "title:: logseq-claude\ntags:: project, tooling",
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
      b("DONE finished task with a logbook drawer\n:LOGBOOK:\nCLOCK: [2026-06-16 Tue 09:00]--[2026-06-16 Tue 09:30] =>  0:30\n:END:"),
      b("Task markers: TODO a, DOING b, NOW c, LATER d, WAIT e, DONE f"),
      b("TODO [#A] high-priority task"),
      b("Inline styles: **bold**, *italic*, ~~strike~~, ==highlight==, `code`"),
      b("Footnote reference[^1] in a sentence.\n[^1]: the footnote definition."),
      b("Video embed: {{video https://www.youtube.com/watch?v=dQw4w9WgXcQ}}"),
      b("Tweet embed: {{tweet https://twitter.com/logseq/status/123}}"),
    ],
  },
];

const mockHighlights: Record<string, { label: string; highlights: Highlight[] }> = {};
const mockAssets: Record<string, Uint8Array> = {};

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

export function mockBackend(): Backend {
  const all = [...PAGES, ...NAMED];
  const find = (name: string) =>
    all.find((p) => p.name.toLowerCase() === name.toLowerCase()) ?? null;

  // Collect (page, matching blocks) where keep() holds, grouped by page.
  const collect = (keep: (b: BlockDto) => boolean, exclude?: string): RefGroup[] => {
    const groups: RefGroup[] = [];
    for (const p of all) {
      if (exclude && p.name.toLowerCase() === exclude.toLowerCase()) continue;
      const matched: BlockDto[] = [];
      const walk = (bs: BlockDto[]) => bs.forEach((b) => (keep(b) && matched.push(b), walk(b.children)));
      walk(p.blocks);
      if (matched.length) groups.push({ page: p.name, kind: p.kind, blocks: matched });
    }
    return groups;
  };

  return {
    async loadGraph(): Promise<GraphMeta> {
      return {
        root: "/mock/graph",
        journals_dir: "journals",
        pages_dir: "pages",
        preferred_workflow: "todo",
        shortcuts: {},
        start_of_week: 0,
        block_hidden_properties: [],
        default_journal_template: null,
        favorites: [],
      };
    },
    async listPages(): Promise<PageEntry[]> {
      return all.map((p) => ({ name: p.name, kind: p.kind, date_key: null }));
    },
    async journalsDesc(limit: number, offset: number): Promise<PageDto[]> {
      return PAGES.slice(offset, offset + limit);
    },
    async journalContentDays(): Promise<number[]> {
      return [];
    },
    async getPage(name: string): Promise<PageDto | null> {
      if (name.startsWith("hls__")) return hlsPageDto(name);
      return find(name);
    },
    async savePage(_page: PageDto, _force?: boolean): Promise<void> {
      // no-op in mock
    },
    async getBacklinks(name: string): Promise<RefGroup[]> {
      const n = name.toLowerCase();
      return collect((b) => pageRefs(b.raw).some((r) => r.toLowerCase() === n), name);
    },
    async getUnlinkedRefs(name: string): Promise<RefGroup[]> {
      const n = name.toLowerCase();
      const re = new RegExp(`\\b${n.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\b`);
      return collect(
        (b) => re.test(b.raw.toLowerCase()) && !pageRefs(b.raw).some((r) => r.toLowerCase() === n),
        name
      );
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
      const ref = pageRefs(query)[0];
      if (ref) {
        const n = ref.toLowerCase();
        return collect((b) => pageRefs(b.raw).some((r) => r.toLowerCase() === n));
      }
      return [];
    },
    async readCustomCss(): Promise<string> {
      return "";
    },
    async pageAliases(): Promise<[string, string][]> {
      return [];
    },
    async setFavorites(): Promise<void> {
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
      const q = query.trim().toLowerCase();
      if (!q) return [];
      let n = limit;
      const groups = collect((b) => b.raw.toLowerCase().includes(q));
      for (const g of groups) {
        if (g.blocks.length > n) g.blocks = g.blocks.slice(0, n);
        n -= g.blocks.length;
      }
      return groups.filter((g) => g.blocks.length > 0);
    },
    async listTemplates() {
      return [
        {
          name: "meeting",
          blocks: [
            { id: "t1", raw: "## Meeting [[<% today %>]]", collapsed: false, children: [] },
            { id: "t2", raw: "Attendees:", collapsed: false, children: [] },
            { id: "t3", raw: "TODO Follow up", collapsed: false, children: [] },
          ],
        },
      ];
    },
    async quickSwitch(query: string, limit: number): Promise<PageEntry[]> {
      const q = query.trim().toLowerCase();
      return all
        .filter((p) => p.name.toLowerCase().includes(q))
        .slice(0, limit)
        .map((p) => ({ name: p.name, kind: p.kind, date_key: null }));
    },
    async resolveBlock(uuid: string): Promise<RefGroup | null> {
      for (const p of all) {
        let found: BlockDto | null = null;
        const walk = (bs: BlockDto[]) =>
          bs.forEach((b) => {
            if (!found && b.raw.includes(`id:: ${uuid}`)) found = b;
            walk(b.children);
          });
        walk(p.blocks);
        if (found) return { page: p.name, kind: p.kind, blocks: [found] };
      }
      return null;
    },
    async readAsset(name: string): Promise<Uint8Array> {
      if (mockAssets[name]) return mockAssets[name];
      if (name === "sample.pdf") return decodeB64(SAMPLE_PDF_B64);
      return new Uint8Array();
    },
    async saveAsset(name: string, bytes: Uint8Array): Promise<string> {
      mockAssets[name] = bytes;
      return name;
    },
    async pasteImage(): Promise<string | null> {
      return null; // no OS clipboard in the browser mock
    },
    async importAsset(path: string): Promise<string> {
      return path.split("/").pop() ?? path;
    },
    async pickFolder(): Promise<string | null> {
      return null; // no native dialog in the browser mock
    },
    async pickFile(): Promise<string | null> {
      return null;
    },
    async writeText(text: string): Promise<void> {
      try {
        await navigator.clipboard.writeText(text);
      } catch {
        // ignore
      }
    },
    async onGraphChanged(): Promise<() => void> {
      return () => {}; // no external watcher in the browser mock
    },
    async readHighlights(pdf: string): Promise<Highlight[]> {
      return mockHighlights[pdf]?.highlights ?? [];
    },
    async writeHighlights(pdf: string, label: string, highlights: Highlight[]): Promise<void> {
      mockHighlights[pdf] = { label, highlights };
    },
  };
}
