// REG-QS3-SHARE-PENDING-MARKER (D-20261018-QS3): a quick share whose upload outcome was unknown, confirmed later in
// Settings → Sharing after the dialog that knew its query block is gone, still gets `tine.share:: <login>/<gist_id>`
// on that block. The dialog and Settings run over the document layer's real edits and save, against a fake disk;
// the native commands are scripted. Asserted on the bytes that reach the disk.
import { afterEach, beforeAll, beforeEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { clearSeededFacets } from "../render/facets";
import { backend } from "../backend";
import { flushAll, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import { bindTestHost } from "../document/host/wiring.test.support";
import { installDiskHost } from "../diskHost.test.support";
import { QuickShareDialog } from "./QuickShareDialog";
import { QuickShareSettings } from "./QuickShareSettings";
import { closeQuickShare, openQuickShare, quickShareRequest } from "../shareState";
import { clearTransientLayersForTest } from "../transientLayers";
import { setToasts } from "../toasts";
import type { BlockDto, PageDto, PageRead, QueryPublicationRequest } from "../types";
import type { QuickShareListed, QuickShareListing } from "../quickShare/commands";

vi.mock("../exportsPage", async (original) => ({
  ...await original<typeof import("../exportsPage")>(),
  ensureExportsPage: vi.fn(async () => "Exports"),
}));

const tick = () => new Promise((r) => setTimeout(r, 0));
const settle = async () => { for (let i = 0; i < 12; i++) await tick(); };
const RECORD = "Rec0rd-id_0123456789xy";
const GIST = "0123456789abcdef0123";
const QUERY = "{{query (task TODO)}}";
const REQUEST: QueryPublicationRequest = { argument: "(task TODO)", dialect: "og", properties: [],
  currentPage: "Notes", hostBlockId: null, name: "Notes", includeQueryText: false };

let counter = 0;
const props = (raw: string) => raw.split("\n").slice(1).map((line) => /^([^\s:]+):: (.*)$/.exec(line)).filter((m) => !!m)
  .map((m) => [m![1], m![2]] as [string, string]);
const blk = (raw: string, children: BlockDto[] = []): BlockDto => ({ id: `q${counter++}`, raw, collapsed: false, children, properties: props(raw) });
const page = (name: string, blocks: BlockDto[]): PageRead => ({
  name, kind: "page", title: name, id: `pages/${name}.md`, rev: "r0", format: "md", pre_block: null, blocks,
} as PageRead);

let files: Map<string, PageRead>;
const raws = (path: string): string[] => {
  const out: string[] = [];
  const walk = (blocks: BlockDto[]) => { for (const b of blocks) { out.push(b.raw); walk(b.children); } };
  walk(files.get(path)!.blocks);
  return out;
};
let shares: QuickShareListed[];
const listing = (): QuickShareListing => ({ enabled: true, account: { login: "octo", accountId: 7 }, shares, error: null });
const pendingRow = (state: "pending" | "live"): QuickShareListed => ({ shareId: RECORD, state, login: "octo",
  gistId: state === "live" ? GIST : null, createdAt: "2026-10-19T10:00:00Z", title: "Notes", pages: 1, accountMatches: true });

async function bindDisk(initial: PageRead[]) {
  files = new Map(initial.map((dto) => [dto.id, structuredClone(dto)]));
  let rev = 0;
  const host = await bindTestHost();
  installDiskHost(host, {
    read: (path) => files.get(path) ?? null,
    write: (path, dto) => { files.set(path, { ...structuredClone(dto), id: path, rev: `rev${++rev}` }); return `rev${rev}`; },
  });
  const b = backend();
  vi.spyOn(b, "getPage").mockImplementation(async (name: string) => structuredClone([...files.values()].find((dto) => dto.name === name) ?? null));
  // The engine's `(property tine.share)`: every block on disk carrying it, with its parsed properties.
  vi.spyOn(b, "parseQuery").mockResolvedValue({ query: { anchor: "block" }, view: {} } as never);
  vi.spyOn(b, "queryRun").mockImplementation(async () => ({
    anchor: "block", total: 0, exceeded: false, report: { supported: true },
    groups: [...files.values()].map((dto) => {
      const flat: BlockDto[] = [];
      const walk = (blocks: BlockDto[]) => { for (const x of blocks) { flat.push({ ...x, properties: props(x.raw) }); walk(x.children); } };
      walk(dto.blocks);
      return { page: dto.name, kind: "page", blocks: flat.filter((x) => x.raw.includes("tine.share::")) };
    }).filter((group) => group.blocks.length),
  }) as never);
  vi.spyOn(b, "quickShareList").mockImplementation(async () => listing());
  vi.spyOn(b, "quickSharePrepare").mockResolvedValue({ prepareId: "p1", shareId: RECORD, preview: { title: "Notes",
    pages: [{ name: "Notes", path: "pages/Notes.md", journal: false }], leftOut: [], payloadBytes: 10, envelopeBytes: 20 } });
  vi.spyOn(b, "quickShareDiscard").mockResolvedValue();
  vi.spyOn(b, "confirm").mockResolvedValue(true);
}

function load(dto: PageRead): PageDto {
  loadSingle(structuredClone(dto));
  clearSeededFacets();
  return dto;
}
const idOf = (name: string, raw: string) => Object.values(doc.byId).find((n) => n.page === name && n.raw === raw)!.id;

let dispose: (() => void) | undefined;
async function mount(view: () => unknown) {
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(view as never, root);
  await settle();
  return root;
}
function unmount() { dispose?.(); dispose = undefined; document.body.innerHTML = ""; }

beforeAll(() => initParser());
beforeEach(() => { counter = 0; resetStore(); shares = []; });
afterEach(() => { unmount(); closeQuickShare(); clearTransientLayersForTest(); setToasts([]); vi.restoreAllMocks(); });

/** Open the share dialog on the Notes query block and press Create. */
async function startShare() {
  const notes = page("Notes", [blk("intro"), blk(QUERY)]);
  await bindDisk([notes, page("Exports", [blk("{{query (or (property export) (property tine.share))}}")])]);
  load(notes);
  openQuickShare(REQUEST, { hostBlockId: idOf("Notes", QUERY), macro: QUERY });
  const root = await mount(() => <QuickShareDialog request={quickShareRequest} />);
  root.querySelector<HTMLButtonElement>(".quick-share-create")!.click();
  await settle();
  return root;
}

it("an unknown upload outcome confirmed later by Settings → Check writes the share onto its query block", async () => {
  const create = vi.spyOn(backend(), "quickShareCreate").mockImplementation(async () => {
    // The marker reached disk before the upload was asked for.
    expect(raws("pages/Notes.md")).toEqual(["intro", `${QUERY}\ntine.share:: pending/${RECORD}`]);
    shares = [pendingRow("pending")];
    throw { kind: "maybeUploaded", shareId: RECORD };
  });
  await startShare();
  expect(create).toHaveBeenCalledWith("p1");
  expect(raws("pages/Notes.md"), "an unknown outcome keeps the marker").toEqual(["intro", `${QUERY}\ntine.share:: pending/${RECORD}`]);
  // The dialog is closed (the query it knew is gone with it); the block is edited meanwhile, and only its own
  // text goes: the marker is found by the property, not by a remembered position or text.
  unmount(); closeQuickShare();
  const id = Object.values(doc.byId).find((n) => n.raw.includes("pending/"))!.id;
  const { setBlockProperty } = await import("../document");
  setBlockProperty(id, "status", "draft");
  expect(await flushAll()).toBe(true);

  vi.spyOn(backend(), "quickShareCheck").mockImplementation(async () => { shares = [pendingRow("live")]; return "live"; });
  const settings = await mount(() => <QuickShareSettings />);
  settings.querySelector<HTMLButtonElement>(`[data-share-id="${RECORD}"] .quick-share-row-check`)!.click();
  await settle();
  expect(await flushAll()).toBe(true);
  expect(raws("pages/Notes.md")).toEqual(["intro", `${QUERY}\ntine.share:: octo/${GIST}\nstatus:: draft`]);
});

it("Settings → Check on the pending markers finishes a share this device recorded and leaves another device's alone", async () => {
  const OTHER = "0ther-device_record_id";
  const notes = page("Notes", [blk(`${QUERY}\ntine.share:: pending/${RECORD}`), blk(`{{query (task DONE)}}\ntine.share:: pending/${OTHER}`),
    blk(`{{query (task NOW)}}\ntine.share:: pending/${RECORD}`)]);
  await bindDisk([notes]);
  shares = [pendingRow("pending")];
  const check = vi.spyOn(backend(), "quickShareCheck").mockImplementation(async () => { shares = [pendingRow("live")]; return "live"; });
  const settings = await mount(() => <QuickShareSettings />);
  const marker = (id: string) => settings.querySelector<HTMLElement>(`.quick-share-marker[data-share-id="${id}"]`)!;
  expect(marker(RECORD).textContent).toContain("On Notes");
  expect(marker(OTHER).textContent).toContain("Pending on another device");
  expect(marker(RECORD).querySelector(".quick-share-marker-remove"), "a recorded share is finished, not removed").toBeNull();
  settings.querySelector<HTMLButtonElement>(".quick-share-markers-check")!.click();
  await settle();
  expect(await flushAll()).toBe(true);
  expect(check, "the gist is looked for first").toHaveBeenCalledWith(RECORD);
  // Both blocks carrying this record's marker are definitions of the share; the foreign marker stays.
  expect(raws("pages/Notes.md")).toEqual([`${QUERY}\ntine.share:: octo/${GIST}`, `{{query (task DONE)}}\ntine.share:: pending/${OTHER}`,
    `{{query (task NOW)}}\ntine.share:: octo/${GIST}`]);
  expect(marker(RECORD)).toBeNull();
  // A marker no record here stands behind can be removed, after a confirmation.
  marker(OTHER).querySelector<HTMLButtonElement>(".quick-share-marker-remove")!.click();
  await settle();
  expect(await flushAll()).toBe(true);
  expect(raws("pages/Notes.md")[1]).toBe("{{query (task DONE)}}");
  expect(settings.querySelector(".quick-share-markers")).toBeNull();
});

it("a definite failure removes the marker; a success replaces it; a deleted block is said, not written", async () => {
  vi.spyOn(backend(), "quickShareCreate").mockRejectedValueOnce({ kind: "network" });
  let root = await startShare();
  expect(await flushAll()).toBe(true);
  expect(raws("pages/Notes.md"), "no gist was made: no marker").toEqual(["intro", QUERY]);
  expect(root.querySelector("[role=alert]")!.textContent).toContain("Tine could not reach GitHub");
  unmount(); closeQuickShare(); resetStore();

  vi.spyOn(backend(), "quickShareCreate").mockResolvedValueOnce({ shareId: RECORD, gistId: GIST, login: "octo", copied: true });
  root = await startShare();
  expect(await flushAll()).toBe(true);
  expect(raws("pages/Notes.md")).toEqual(["intro", `${QUERY}\ntine.share:: octo/${GIST}`]);
  expect(root.textContent).toContain("Listed on Notes");
  unmount(); closeQuickShare(); resetStore();

  // The block is deleted while the upload runs.
  let finish!: (value: { shareId: string; gistId: string; login: string; copied: boolean }) => void;
  vi.spyOn(backend(), "quickShareCreate").mockReturnValueOnce(new Promise((r) => { finish = r; }));
  root = await startShare();
  const { deleteBlock } = await import("../document");
  deleteBlock(Object.values(doc.byId).find((n) => n.raw.includes("pending/"))!.id);
  expect(await flushAll()).toBe(true);
  finish({ shareId: RECORD, gistId: GIST, login: "octo", copied: true });
  await settle();
  expect(await flushAll()).toBe(true);
  expect(raws("pages/Notes.md")).toEqual(["intro"]);
  expect(root.querySelector("[role=alert]")!.textContent).toContain("no block carries its pending marker any more");
});

it("a marker that cannot be saved starts no upload and leaves no marker behind", async () => {
  const create = vi.spyOn(backend(), "quickShareCreate");
  const documentLayer = await import("../document");
  vi.spyOn(documentLayer, "flushPage").mockResolvedValueOnce(false);
  const root = await startShare();
  expect(create, "the share does not start without its marker").not.toHaveBeenCalled();
  expect(root.querySelector("[role=alert]")!.textContent).toContain("Tine could not mark the query block before the upload, so nothing was sent");
  expect(Object.values(doc.byId).some((n) => n.raw.includes("pending/")), "taken back out of the working set").toBe(false);
  expect(await flushAll()).toBe(true);
  expect(raws("pages/Notes.md")).toEqual(["intro", QUERY]);
  expect(root.querySelector<HTMLButtonElement>(".quick-share-create")!.disabled, "the preview stays for another try").toBe(false);
});

it("closing the dialog while the marker is saved cancels the share before its upload and removes the marker", async () => {
  const create = vi.spyOn(backend(), "quickShareCreate");
  const documentLayer = await import("../document");
  const real = documentLayer.flushPage;
  let release!: () => void;
  vi.spyOn(documentLayer, "flushPage").mockImplementationOnce(async (name) => { await new Promise<void>((r) => { release = r; }); return real(name); });
  const notes = page("Notes", [blk("intro"), blk(QUERY)]);
  await bindDisk([notes]);
  load(notes);
  openQuickShare(REQUEST, { hostBlockId: idOf("Notes", QUERY), macro: QUERY });
  const root = await mount(() => <QuickShareDialog request={quickShareRequest} />);
  root.querySelector<HTMLButtonElement>(".quick-share-create")!.click();
  await settle();
  closeQuickShare();
  await settle();
  release();
  await settle();
  expect(await flushAll()).toBe(true);
  expect(create).not.toHaveBeenCalled();
  expect(raws("pages/Notes.md")).toEqual(["intro", QUERY]);
});
