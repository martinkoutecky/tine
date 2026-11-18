// #964: live conflict review keeps the newest revision of this page instance
// proven on disk as its ancestor. Proof is a publication wait's host witness
// or an observed disk token whose bytes are a saved revision's. A newer
// acknowledgment never discards an older saved revision before either proof,
// and a late older proof never moves the ancestor back. Losing the ancestor
// loses no data, but the user gets a two-way review instead of a three-way one.
import { afterEach, expect, it, vi } from "vitest";
import { mail, mailPage, opened, setup, tick, took } from "./fakes.test.support";
import type { PublishedRevision } from "./protocol";
import { PendingBases } from "./client";

afterEach(() => vi.useRealTimers());

type Ctx = Awaited<ReturnType<typeof setup>>;

/** Type, send and acknowledge one save of `P` at `version` with buffer `rev`. */
async function save(ctx: Ctx, version: number, rev: string, extra: { conflict?: boolean; disk?: string } = {}) {
  const { host, doc, client } = ctx;
  doc.type("P", `edit ${version}`);
  client.noteEdit("P", "save-block");
  client.sendNow("P");
  await tick();
  client.receive(mail(host.session, host.keyOf("P"), mailPage(version, { kind: "unchanged", rev }, {
    conflict: extra.conflict ?? false, risk: extra.conflict ?? false,
    disk: extra.disk ? { kind: "file", rev: extra.disk } : null,
  }), took(host.last("submit").id, version)));
}

/** A publication wait the test answers later with the host's witnesses. */
function heldWait(ctx: Ctx): (answer: PublishedRevision[] | false) => void {
  let finish!: (answer: PublishedRevision[] | false) => void;
  ctx.host.onWait = () => new Promise<PublishedRevision[] | false>(resolve => { finish = resolve; });
  return answer => finish(answer);
}

function cliConflict(ctx: Ctx, version: number) {
  ctx.client.receive(mail(ctx.host.session, ctx.host.keyOf("P"), mailPage(version, { kind: "unchanged" },
    { conflict: true, risk: true, disk: { kind: "file", rev: "cli" } })));
}

it.each(["pending", "later-conflict", "conflicted-answer"])("a wait started for an older save proves it when a newer input is %s", async (mode) => {
  vi.useFakeTimers();
  const ctx = await setup();
  const { host, doc, client } = ctx;
  const key = host.keyOf("P");
  await opened(ctx, "P", 3);
  await save(ctx, 4, "saved-r4");
  const finish = heldWait(ctx);
  const first = client.published([{ key, version: 4 }]);
  await save(ctx, 5, "draft-r5", { conflict: mode === "conflicted-answer" });
  if (mode === "later-conflict") cliConflict(ctx, 5);
  finish([{ key, version: 4, rev: "saved-r4" }]);
  expect(await first).toBe(true);
  expect(doc.publishedRevs).toEqual([["P", "saved-r4"]]);
  client.retire();
});

it("an older save the next answer shows on disk stays the ancestor although no wait for it ran", async () => {
  vi.useFakeTimers();
  const ctx = await setup();
  const { host, doc, client } = ctx;
  const key = host.keyOf("P");
  await opened(ctx, "P", 3);
  await save(ctx, 4, "r4", { disk: "r3" });
  await save(ctx, 5, "r5", { disk: "r4" });
  cliConflict(ctx, 5);
  // The only wait asks for the conflicted newest save: the host refuses it.
  host.onWait = () => false;
  expect(await client.published([{ key, version: 5 }])).toBe(false);
  expect(doc.publishedRevs).toEqual([["P", "r4"]]);
  client.retire();
});

it("an acknowledgment whose disk already holds its revision makes that revision the ancestor", async () => {
  vi.useFakeTimers();
  const ctx = await setup();
  const { doc, client } = ctx;
  await opened(ctx, "P", 3);
  await save(ctx, 4, "r4", { disk: "r4" });
  cliConflict(ctx, 4);
  expect(doc.publishedRevs).toEqual([["P", "r4"]]);
  client.retire();
});

it("a late older publication wait never moves the ancestor back behind a newer proven one", async () => {
  vi.useFakeTimers();
  const ctx = await setup();
  const { host, doc, client } = ctx;
  const key = host.keyOf("P");
  await opened(ctx, "P", 3);
  await save(ctx, 4, "r4");
  const finish = heldWait(ctx);
  const older = client.published([{ key, version: 4 }]);
  await save(ctx, 5, "r5", { disk: "r5" });
  finish([{ key, version: 4, rev: "r4" }]);
  expect(await older).toBe(true);
  expect(doc.publishedRevs).toEqual([["P", "r5"]]);
  client.retire();
});

it("an older save whose publication is observed only after a newer acknowledgment stays the ancestor", async () => {
  vi.useFakeTimers();
  const ctx = await setup();
  const { host, doc, client } = ctx;
  await opened(ctx, "P", 3);
  await save(ctx, 4, "r4", { disk: "r3" });
  await save(ctx, 5, "r5", { disk: "r3" });
  // The host's mail after its write of the older save (no text: the window holds newer input).
  client.receive(mail(host.session, host.keyOf("P"), mailPage(5, { kind: "unchanged" }, { disk: { kind: "file", rev: "r4" } })));
  cliConflict(ctx, 5);
  expect(doc.publishedRevs).toEqual([["P", "r4"]]);
  client.retire();
});

// Review round 2 (f49cff162): the host's disk token is its retained observation,
// which a completed save does not refresh. A repeated token is not news, and a
// new one proves the oldest pending revision with those bytes.
it("an input returning to the loaded bytes is not proved by the repeated load-time disk token", async () => {
  vi.useFakeTimers();
  const ctx = await setup();
  const { host, doc, client } = ctx;
  await opened(ctx, "P", 3);
  await save(ctx, 4, "r4", { disk: "r3" });
  await save(ctx, 5, "r3", { disk: "r3" });
  client.receive(mail(host.session, host.keyOf("P"), mailPage(5, { kind: "unchanged" }, { disk: { kind: "file", rev: "r4" } })));
  cliConflict(ctx, 5);
  expect(doc.publishedRevs).toEqual([["P", "r4"]]);
  client.retire();
});

it("a new disk token proves the oldest pending input with its bytes, so a later one still settles the next", async () => {
  vi.useFakeTimers();
  const ctx = await setup();
  const { host, doc, client } = ctx;
  await opened(ctx, "P", 3);
  await save(ctx, 4, "r4", { disk: "r3" });
  await save(ctx, 5, "r5", { disk: "r3" });
  await save(ctx, 6, "r4", { disk: "r4" });
  client.receive(mail(host.session, host.keyOf("P"), mailPage(6, { kind: "unchanged" }, { disk: { kind: "file", rev: "r5" } })));
  cliConflict(ctx, 6);
  expect(doc.publishedRevs).toEqual([["P", "r4"], ["P", "r5"]]);
  client.retire();
});

it("an input returning to already proved bytes is not proved by the repeated token", async () => {
  vi.useFakeTimers();
  const ctx = await setup();
  const { host, doc, client } = ctx;
  await opened(ctx, "P", 3);
  await save(ctx, 4, "r4", { disk: "r4" });
  await save(ctx, 5, "r5", { disk: "r4" });
  await save(ctx, 6, "r4", { disk: "r4" });
  client.receive(mail(host.session, host.keyOf("P"), mailPage(6, { kind: "unchanged" }, { disk: { kind: "file", rev: "r5" } })));
  cliConflict(ctx, 6);
  expect(doc.publishedRevs).toEqual([["P", "r4"], ["P", "r5"]]);
  client.retire();
});

it("a wait started after a newer acknowledgment still proves the older save it asked for", async () => {
  vi.useFakeTimers();
  const ctx = await setup();
  const { host, doc, client } = ctx;
  const key = host.keyOf("P");
  await opened(ctx, "P", 3);
  await save(ctx, 4, "r4", { disk: "r3" });
  await save(ctx, 5, "r5", { disk: "r3" });
  const finish = heldWait(ctx);
  const late = client.published([{ key, version: 4 }]);
  cliConflict(ctx, 5);
  finish([{ key, version: 4, rev: "r4" }]);
  expect(await late).toBe(true);
  expect(doc.publishedRevs).toEqual([["P", "r4"]]);
  client.retire();
});

it("acknowledgments while saves fail cost constant work each; a proof settles the prefix once", async () => {
  vi.useFakeTimers();
  const ctx = await setup();
  const { host, doc, client } = ctx;
  const key = host.keyOf("P");
  await opened(ctx, "P", 3);
  const shift = vi.spyOn(PendingBases.prototype as unknown as { shift: () => unknown }, "shift");
  for (let version = 4; version < 204; version++) await save(ctx, version, `r${version}`, { disk: "r3" });
  expect(shift).toHaveBeenCalledTimes(0);
  host.onWait = () => [{ key, version: 203, rev: "r203" }];
  expect(await client.published([{ key, version: 203 }])).toBe(true);
  expect(shift).toHaveBeenCalledTimes(200);
  expect(doc.publishedRevs).toEqual([["P", "r203"]]);
  client.retire();
});
