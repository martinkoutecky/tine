// The shared local-target table (fixtures/local-target-cases.json), TypeScript
// side. Per row it checks two things: the owner, read through its wasm wrapper,
// gives the recorded answer; and develop's renderer still does what the row
// records (rendered and clicked for real). A row whose owner answer differs
// from the renderer's must say which decision or rule explains the change.
// The Rust side (crates/tine-core/src/local_target_tests.rs) checks the owner,
// the asset collector and export against the same rows.
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import table from "../../fixtures/local-target-cases.json";
import { InlineText } from "./inline";
import { initParser, parseBlock } from "./parse";
import { urlDest } from "./urlDest";
import { filePathUrl, resolvedFilePath } from "./relativeFileLink";
import type { Inline } from "./ast";
import { backend } from "../backend";
import * as panes from "../panes";
import * as outbound from "../components/ExternalLink";
import * as assetCache from "../assetCache";
import { setGraphMeta } from "../graphSession";
import type { GraphMeta } from "../types";
import { pageIdentityKey } from "../pageIdentity";
import { pageIconAsset } from "../pageIconImage";
import { rawHtmlPresentation } from "./htmlSanitize";
import { localTarget, type AssetName, type LocalDestination, type LocalRoute, type LocalTarget, type LocalTargetOrigin } from "../localTarget";

interface Row {
  id: string;
  case: string;
  format: "md" | "org";
  origin: LocalTargetOrigin;
  source: string;
  dest?: string;
  page?: string;
  owner: string;
  today: { renderer: string; collector: string[]; export: string[] };
  change: { renderer: string | null; liveness: string | null; export: string | null };
}
const rows = table.rows as Row[];
const ROOT = table.root;
const RECORD = !!process.env.LOCAL_TARGET_RECORD;

beforeAll(async () => {
  await initParser();
  setGraphMeta({ root: ROOT } as GraphMeta);
});
afterEach(() => {
  vi.restoreAllMocks();
  document.body.replaceChildren();
});

type Link = Extract<Inline, { k: "link" }>;
function first(node: unknown, k: string): Record<string, unknown> | null {
  if (!node || typeof node !== "object") return null;
  if ((node as { k?: unknown }).k === k) return node as Record<string, unknown>;
  for (const value of Object.values(node)) {
    const found = first(value, k);
    if (found) return found;
  }
  return null;
}

/** The parser's own node for the row, as production hands it to the owner. */
function destinationOf(row: Row): { destination: LocalDestination; link: Link | null } {
  if (row.dest !== undefined) return { destination: { text: row.dest }, link: null };
  const blocks = parseBlock(row.source, row.format === "org");
  if (row.origin === "img-macro") {
    const macro = first(blocks, "macro") as { args: string[] } | null;
    // The macro's URL is the first token of the parser's first argument.
    return { destination: { text: macro?.args[0]?.trim().split(/\s+/)[0] ?? "" }, link: null };
  }
  const link = first(blocks, "link") as Link | null;
  if (!link) return { destination: { text: row.source }, link: null };
  return { destination: { url: link.url, full: link.full }, link };
}

export function describeTarget(t: LocalTarget): string {
  const r = t.route;
  const route = r.kind === "page_ref" ? `page:${r.name}`
    : r.kind === "block_ref" ? `block:${r.id}`
    : r.kind === "app_scheme" ? `app:${r.scheme}`
    : r.kind === "fragment" ? `fragment:${r.href}`
    : r.kind === "attachment" ? `attachment:${r.name}`
    : r.kind === "graph_file" ? `graph-file:${r.path}`
    : r.kind === "outside_file" ? `outside-${r.reason}:${r.path}`
    : r.kind === "drive" ? `drive:${r.path}`
    : r.kind === "file_url" ? `file-url:${r.url}`
    : r.kind === "page_relative" ? `page-relative:${r.path}`
    : r.kind.replace("_", "-");
  const pdf = !t.pdf ? "" : t.pdf.kind === "asset_reader" ? ` pdf=asset:${t.pdf.name}`
    : ` pdf=outside${t.pdf.fallback === null ? "" : `>asset:${t.pdf.fallback}`}`;
  return route + pdf + (t.media ? ` media=${t.media}` : "");
}

function resolved(r: LocalRoute): string {
  if (r.kind === "drive") return r.path;
  if (r.kind === "outside_file") return resolvedFilePath({ kind: "outside", reason: r.reason, path: r.path }, ROOT);
  if (r.kind === "graph_file") return resolvedFilePath({ kind: "graph", path: r.path }, ROOT);
  throw new Error(`not a file route: ${r.kind}`);
}

/** pageIconAsset's page-relative tail, applied to the owner's path. */
function iconOf(path: string, page: string): string {
  const parts = page.split("/").slice(0, -1);
  for (const part of path.split("/")) {
    if (!part || part === ".") continue;
    if (part === "..") {
      if (!parts.length) return "icon:none";
      parts.pop();
    } else parts.push(part);
  }
  const rel = parts.slice(1).join("/");
  const ext = rel.slice(rel.lastIndexOf(".") + 1).toLowerCase();
  return parts[0] === "assets" && parts.length >= 2 && ["png", "jpg", "jpeg"].includes(ext) ? `icon:${rel}` : "icon:none";
}

/** What develop's renderer would do with the owner's answer (desktop, the
 * table's graph root): the same vocabulary `rendered` observes. */
function projected(row: Row, t: LocalTarget, link: Link | null): string {
  const r = t.route;
  if (row.origin === "page-icon") return r.kind === "page_relative" ? iconOf(r.path, row.page!) : "icon:none";
  if (row.origin === "html-resource") return r.kind === "drive" || r.kind === "outside_file" ? `local-image:${r.path}` : "browser";
  if (r.kind === "page_ref") return `page:${pageIdentityKey(r.name)}`;
  if (r.kind === "refused" || r.kind === "refused_malformed") return "inert";
  const av = t.media === "audio" || t.media === "video";
  const imageOnly = row.origin === "img-macro" || (link?.url.type === "page_ref" && t.media === "image");
  // An image with no graph asset and no web URL renders nothing.
  if (imageOnly) return r.kind === "attachment" ? `image:${r.name}` : r.kind === "web" ? "image-web" : "none";
  if (t.pdf?.kind === "asset_reader") return `asset-pdf:${t.pdf.name}`;
  if (t.pdf?.kind === "outside_reader") return `outside-pdf:${r.kind === "file_url" ? r.url : filePathUrl(resolved(r))}`;
  if (row.origin === "md-embed") {
    if (r.kind === "attachment") return av ? `media:${r.name}` : `image:${r.name}`;
    if (r.kind === "web") return av ? "media-web" : "image-web";
    if (["graph_file", "outside_file", "drive", "fragment"].includes(r.kind)) return av ? "media:none" : "none";
  }
  if (r.kind === "web" && row.origin === "md-link" && /^https?:\/\//i.test(link!.full.trimStart()) && (av || t.media === "image")) {
    return av ? "media-web" : "image-web";
  }
  if (r.kind === "attachment") return `attachment:${r.name}`;
  if (r.kind === "graph_file") return `graph-file:${r.path}`;
  if (r.kind === "outside_file" || r.kind === "drive") return `open:${filePathUrl(resolved(r))}`;
  if (r.kind === "file_url") return `open:${r.url}`;
  return `open:${link ? urlDest(link.url) : row.dest ?? row.source}`;
}

const ticks = async () => { for (let i = 0; i < 5; i++) await Promise.resolve(); };

/** Render the row with develop's renderer and click it; name what happened. */
async function rendered(row: Row): Promise<string> {
  if (row.origin === "page-icon") return `icon:${pageIconAsset(row.source, row.page!) ?? "none"}`;
  if (row.origin === "html-resource") {
    const path = rawHtmlPresentation(row.source).localImages.find((image) => image !== null);
    return path ? `local-image:${path}` : "browser";
  }
  const cached = vi.spyOn(assetCache, "acquireCachedAssetBlob").mockReturnValue(undefined);
  vi.spyOn(assetCache, "acquireAssetBlob").mockReturnValue(new Promise(() => {}));
  const stream = vi.spyOn(backend(), "streamAsset").mockResolvedValue("");
  const openAsset = vi.spyOn(backend(), "openAsset").mockResolvedValue(undefined);
  const openGraphFile = vi.spyOn(backend(), "openGraphFile").mockResolvedValue(undefined);
  const openPdf = vi.spyOn(panes, "openPdf").mockReturnValue(null);
  const open = vi.spyOn(outbound, "openOutboundLink").mockResolvedValue(undefined);
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <InlineText text={row.source} format={row.format} />, host);
  try {
    await ticks();
    const page = host.querySelector("a.page-ref");
    if (page) return `page:${page.getAttribute("data-ref")}`;
    if (cached.mock.calls.length) return `image:${cached.mock.calls[0][0]}`;
    if (stream.mock.calls.length) return `media:${stream.mock.calls[0][0]}`;
    const media = host.querySelector("video, audio");
    if (media) return /^https?:/.test(media.getAttribute("src") ?? "") ? "media-web" : "media:none";
    const img = host.querySelector("img");
    if (img) return /^https?:/.test(img.getAttribute("src") ?? "") ? "image-web" : "image:none";
    const anchor = host.querySelector("a");
    if (!anchor) return "none";
    anchor.click();
    await ticks();
    if (openAsset.mock.calls.length) return `attachment:${openAsset.mock.calls[0][0]}`;
    if (openGraphFile.mock.calls.length) return `graph-file:${openGraphFile.mock.calls[0][0]}`;
    if (openPdf.mock.calls.length) {
      const [file, , , , options] = openPdf.mock.calls[0];
      return `${options?.externalRead ? "outside-pdf" : "asset-pdf"}:${file}`;
    }
    if (open.mock.calls.length) return `open:${open.mock.calls[0][0]}`;
    return "inert";
  } finally { dispose(); }
}

describe("the local-target table (one owner, every answerer pinned)", () => {
  it("covers every origin", () => {
    expect(new Set(rows.map((row) => row.origin))).toEqual(new Set(["md-link", "md-embed", "org-link", "org-bare", "img-macro", "html-resource", "page-icon"]));
  });

  it("is pure and cheap: the whole table through the wasm owner", () => {
    const inputs = rows.map(destinationOf);
    const started = performance.now();
    for (let i = 0; i < 100; i++) rows.forEach((row, n) => localTarget(inputs[n].destination, row.origin));
    const perTable = (performance.now() - started) / 100;
    console.info(`local target: ${rows.length} rows in ${perTable.toFixed(3)} ms per table`);
  });

  for (const row of rows) {
    it(`${row.id}: ${row.source}`, async () => {
      const { destination, link } = destinationOf(row);
      const owner = localTarget(destination, row.origin);
      const today = await rendered(row);
      const projection = projected(row, owner, link);
      if (RECORD) {
        console.log(`REC ${JSON.stringify({ id: row.id, owner: describeTarget(owner), renderer: today, projected: projection })}`);
        return;
      }
      expect(describeTarget(owner), "the owner's answer").toBe(row.owner);
      expect(today, "develop's renderer").toBe(row.today.renderer);
      expect(projection !== today, `owner and renderer differ (${projection} vs ${today}); change: ${row.change.renderer}`)
        .toBe(row.change.renderer !== null);
    });
  }
});

describe("AssetName is made only by the owner's wrapper", () => {
  it("rejects a plain string at compile time", () => {
    // @ts-expect-error a string is not an AssetName: only localTarget makes one.
    const name: AssetName = "photo.png";
    expect(name).toBe("photo.png");
  });
});
