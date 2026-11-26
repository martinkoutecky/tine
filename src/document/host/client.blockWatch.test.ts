// `HostClient.watchBlocks`: proved only by an own-save version EQUAL to the
// newest version the host took, from a text that held the watched blocks.
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { mail, mailPage, opened, setup, tick, took } from "./fakes.test.support";

beforeEach(() => { vi.useFakeTimers(); });
afterEach(() => { vi.useRealTimers(); });

const KEY = "pages/P.md";
type Ctx = Awaited<ReturnType<typeof setup>>;

/** Type `text` (the fake document's one block is `P-1`), send it, and let the host take it at `version`. */
async function sendAndTake(ctx: Ctx, text: string, version: number): Promise<void> {
  ctx.doc.type("P", text);
  ctx.client.noteEdit("P", "save-block");
  ctx.client.sendNow("P");
  await tick();
  ctx.client.receive(mail(7, KEY, mailPage(version, { kind: "unchanged" }), took(ctx.host.last("submit").id, version)));
}

/** The host's wait answers that its own save wrote `version`. */
async function ownSave(ctx: Ctx, version: number): Promise<void> {
  ctx.host.onWait = () => [{ key: KEY, version, rev: `r${version}` }];
  expect(await ctx.client.published([{ key: KEY, version: 1 }])).toBe(true);
}

it("proves when the host's own save wrote the version it took from a text holding the blocks", async () => {
  const ctx = await setup();
  await opened(ctx, "P", 3);
  ctx.doc.type("P", "ab");
  const watch = ctx.client.watchBlocks("P", ["P-1"])!;
  await sendAndTake(ctx, "ab", 4);
  expect(watch.proved).toBe(false);
  await ownSave(ctx, 4);
  expect(watch.proved).toBe(true);
  // Sticky: a later version without the blocks does not take it back.
  await sendAndTake(ctx, "", 5);
  await ownSave(ctx, 5);
  expect(watch.proved).toBe(true);
});

it("a later or an earlier own-save version proves nothing; the exact one still does", async () => {
  const ctx = await setup();
  await opened(ctx, "P", 3);
  const watch = ctx.client.watchBlocks("P", ["P-1"])!;
  await sendAndTake(ctx, "ab", 4);
  await ownSave(ctx, 3); // the watermark before this text: an older save
  expect(watch.proved).toBe(false);
  await ownSave(ctx, 4);
  expect(watch.proved).toBe(true);

  const other = await setup();
  await opened(other, "P", 3);
  const lost = other.client.watchBlocks("P", ["P-1"])!;
  await sendAndTake(other, "ab", 4);
  await ownSave(other, 5); // a later version (the page moved on without a save of 4)
  expect(lost.proved).toBe(false);
  await ownSave(other, 4); // a watermark never goes back; if it did, 4 was pruned
  expect(lost.proved).toBe(false);
});

it("a taken text that omits a watched block is not a holding version", async () => {
  const ctx = await setup();
  await opened(ctx, "P", 3);
  const watch = ctx.client.watchBlocks("P", ["P-1", "elsewhere"])!;
  await sendAndTake(ctx, "ab", 4);
  await ownSave(ctx, 4);
  expect(watch.proved).toBe(false);
});

it("a text sent before the watch began, a refused send and a host that did not take prove nothing", async () => {
  const ctx = await setup();
  const { host, doc, client } = ctx;
  await opened(ctx, "P", 3);
  doc.type("P", "ab");
  client.noteEdit("P", "save-block");
  client.sendNow("P");
  await tick();
  const early = client.watchBlocks("P", ["P-1"])!; // registered while the submit is in flight
  client.receive(mail(7, KEY, mailPage(4, { kind: "unchanged" }), took(host.last("submit").id, 4)));
  await ownSave(ctx, 4);
  expect(early.proved).toBe(false);

  // Not taken (a stale conflict answers `took: false`): its version is not holding.
  doc.type("P", "abc");
  client.noteEdit("P", "save-block");
  client.sendNow("P");
  await tick();
  client.receive(mail(7, KEY, mailPage(6, { kind: "unchanged" }, { conflict: true }),
    { id: host.last("submit").id, version: 6, took: false, outcome: { kind: "applied" } }));
  await ownSave(ctx, 6);
  expect(early.proved).toBe(false);
});

it("a released watch and a retired client prove nothing afterwards", async () => {
  const ctx = await setup();
  await opened(ctx, "P", 3);
  const released = ctx.client.watchBlocks("P", ["P-1"])!;
  const kept = ctx.client.watchBlocks("P", ["P-1"])!;
  await sendAndTake(ctx, "ab", 4);
  released.release();
  await ownSave(ctx, 4);
  expect([released.proved, kept.proved]).toEqual([false, true]);

  const gone = await setup();
  await opened(gone, "P", 3);
  const watch = gone.client.watchBlocks("P", ["P-1"])!;
  await sendAndTake(gone, "ab", 4);
  gone.client.retire();
  gone.host.onWait = () => [{ key: KEY, version: 4, rev: "r4" }];
  expect(await gone.client.published([{ key: KEY, version: 4 }])).toBe(false);
  expect(watch.proved).toBe(false);
});

it("there is nothing to watch on a page the client does not know, or for no blocks", async () => {
  const ctx = await setup();
  await opened(ctx, "P", 3);
  expect(ctx.client.watchBlocks("Unknown", ["x"])).toBeNull();
  expect(ctx.client.watchBlocks("P", [])).toBeNull();
});

it("only the newest taken text can prove: a watch keeps one version however long the save is blocked", async () => {
  const ctx = await setup();
  await opened(ctx, "P", 3);
  const watch = ctx.client.watchBlocks("P", ["P-1"])!;
  // A blocked save: the host takes each edit, and publishes none of them.
  for (let version = 4; version < 1004; version++) await sendAndTake(ctx, `ab${version}`, version);
  const kept = (ctx.client as unknown as { pages: Map<string, { watches: object[] }> }).pages.get("P")!.watches;
  expect(kept).toHaveLength(1);
  expect(JSON.stringify(kept[0]).length).toBeLessThan(200);
  // The host can still write only its current buffer: an older taken version is superseded.
  await ownSave(ctx, 500);
  expect(watch.proved).toBe(false);
  await ownSave(ctx, 1003);
  expect(watch.proved).toBe(true);
});

it("a newer taken text without the blocks takes the proof away from an older one that held them", async () => {
  const ctx = await setup();
  await opened(ctx, "P", 3);
  const watch = ctx.client.watchBlocks("P", ["P-1"])!;
  await sendAndTake(ctx, "ab", 4);
  await sendAndTake(ctx, "", 5); // the block is gone from the text the host holds now
  await ownSave(ctx, 4);
  expect(watch.proved).toBe(false);
});

it("released watches cost a send nothing: the page's text is walked once per live watch", async () => {
  const ctx = await setup();
  await opened(ctx, "P", 3);
  const walks = async (version: number): Promise<number> => {
    ctx.doc.type("P", `ab${version}`);
    let reads = 0;
    const dto = ctx.doc.dto("P")!;
    const blocks = dto.blocks;
    Object.defineProperty(dto, "blocks", { get() { reads += 1; return blocks; }, configurable: true, enumerable: true });
    ctx.client.noteEdit("P", "save-block");
    ctx.client.sendNow("P");
    await tick();
    ctx.client.receive(mail(7, KEY, mailPage(version, { kind: "unchanged" }), took(ctx.host.last("submit").id, version)));
    return reads;
  };
  const bare = await walks(4);
  const abandoned = Array.from({ length: 100 }, () => ctx.client.watchBlocks("P", ["P-1"])!);
  const live = ctx.client.watchBlocks("P", ["P-1"])!;
  for (const watch of abandoned) watch.release();
  expect(await walks(5)).toBe(bare + 1);
  const pages = (ctx.client as unknown as { pages: Map<string, { watches: object[] }> }).pages;
  expect(pages.get("P")!.watches).toHaveLength(1);
  live.release();
  expect(await walks(6)).toBe(bare);
  expect(pages.get("P")!.watches).toHaveLength(0);
  expect(abandoned.some((watch) => watch.proved) || live.proved).toBe(false);
});
