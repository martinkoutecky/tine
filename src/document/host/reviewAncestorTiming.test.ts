// #964: live conflict review keeps the newest revision of this page instance
// proven on disk as its ancestor. Proof is a publication wait's host witness
// or an observed disk token whose bytes are a saved revision's. A newer
// acknowledgment never discards an older saved revision before either proof,
// and a late older proof never moves the ancestor back. Losing the ancestor
// loses no data, but the user gets a two-way review instead of a three-way one.
import { afterEach, expect, it, vi } from "vitest";
import { mail, mailPage, opened, setup, tick, took } from "./fakes.test.support";
import type { PublishedRevision } from "./protocol";

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
