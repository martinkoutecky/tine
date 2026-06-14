// In-memory mock backend seeded with a fixture graph. Used only when running
// outside Tauri (browser dev / Playwright screenshots). Mirrors the real
// backend's shape so the UI behaves identically.

import type { Backend } from "./backend";
import type { BlockDto, GraphMeta, PageDto, PageEntry, RefGroup } from "./types";

function pageRefs(raw: string): string[] {
  const out: string[] = [];
  const re = /\[\[([^\]]+)\]\]|#\[\[([^\]]+)\]\]|#([\w/_.-]+)/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(raw))) out.push(m[1] ?? m[2] ?? m[3]);
  return out;
}
function leadingMarker(raw: string): string | null {
  const m = /^(TODO|DOING|DONE|NOW|LATER|WAITING|CANCELED|CANCELLED|IN-PROGRESS)\b/.exec(raw);
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
      b("TODO Ship the M0 vertical slice"),
      b("DOING Wire up the [[block editor]] with caret preservation"),
      b("DONE Validate round-trip on the real `shui-graph`"),
      b("Inline math works too: $E = mc^2$ and references like ((arch-1))."),
      b("Open tasks across the graph:"),
      b("{{query (todo TODO DOING)}}"),
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
    ],
  },
];

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
      return { root: "/mock/graph", journals_dir: "journals", pages_dir: "pages", shortcuts: {} };
    },
    async listPages(): Promise<PageEntry[]> {
      return all.map((p) => ({ name: p.name, kind: p.kind, date_key: null }));
    },
    async journalsDesc(limit: number, offset: number): Promise<PageDto[]> {
      return PAGES.slice(offset, offset + limit);
    },
    async getPage(name: string): Promise<PageDto | null> {
      return find(name);
    },
    async savePage(): Promise<void> {
      // no-op in mock
    },
    async getBacklinks(name: string): Promise<RefGroup[]> {
      const n = name.toLowerCase();
      return collect((b) => pageRefs(b.raw).some((r) => r.toLowerCase() === n), name);
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
  };
}
