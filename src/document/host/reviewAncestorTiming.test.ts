// #964: conflict review falls back to a two-way review (the saved ancestor is
// lost) when a newer acknowledgment replaces an older saved revision before
// that revision is proven on disk. The first two cases are the merge
// reviewer's boundary probes on 2b554a32c, kept verbatim; the last two are
// their siblings (an acknowledgment whose own disk already holds it; a late
// older wait after a newer proof).
import { afterEach, expect, it, vi } from "vitest";
import { mail, mailPage, opened, setup, tick, took } from "./fakes.test.support";

afterEach(() => vi.useRealTimers());

it.each(["pending", "later-conflict", "conflicted-answer"])("retains a proven older publication when a newer input is %s", async (mode) => {
  vi.useFakeTimers();
  const ctx = await setup();
  const { host, doc, client } = ctx;
  const key = host.keyOf("P");
  await opened(ctx, "P", 3);
  doc.type("P", "saved first edit");
  client.noteEdit("P", "save-block");
  client.sendNow("P");
  await tick();
  client.receive(mail(7, key, mailPage(4, { kind: "unchanged", rev: "saved-r4" }), took(host.last("submit").id, 4)));
  let finish!: (answer: boolean) => void;
  host.onWait = () => new Promise<boolean>(resolve => { finish = resolve; });
  const first = client.published([{ key, version: 4 }]);
  doc.type("P", "unsaved second edit");
  client.noteEdit("P", "save-block");
  client.sendNow("P");
  await tick();
  client.receive(mail(7, key, mailPage(5, { kind: "unchanged", rev: "draft-r5" }, { conflict: mode === "conflicted-answer" }), took(host.last("submit").id, 5)));
  if (mode === "later-conflict") client.receive(mail(7, key, mailPage(5, { kind: "unchanged" }, { conflict: true, risk: true })));
  finish(true);
  expect(await first).toBe(true);
  expect(doc.publishedRevs).toEqual([["P", "saved-r4"]]);
  client.retire();
});

it("keeps an older saved candidate when the next answer proves it on disk before the debounce starts a wait", async () => {
  vi.useFakeTimers();
  const ctx = await setup();
  const { host, doc, client } = ctx;
  const key = host.keyOf("P");
  await opened(ctx, "P", 3);
  for (const version of [4, 5]) {
    doc.type("P", `edit ${version}`);
    client.noteEdit("P", "save-block");
    client.sendNow("P");
    await tick();
    client.receive(mail(7, key, mailPage(version, { kind: "unchanged", rev: `r${version}` }, {
      disk: { kind: "file", rev: version === 4 ? "r3" : "r4" },
    }), took(host.last("submit").id, version)));
  }
  client.receive(mail(7, key, mailPage(5, { kind: "unchanged" }, { conflict: true, risk: true, disk: { kind: "file", rev: "cli" } })));
  host.onWait = needs => needs.every(need => need.version <= 4);
  expect(await client.published([{ key, version: 5 }])).toBe(false);
  expect(doc.publishedRevs).toEqual([["P", "r4"]]);
  client.retire();
});

it("an acknowledgment whose disk already holds its revision makes that revision the ancestor", async () => {
  vi.useFakeTimers();
  const ctx = await setup();
  const { host, doc, client } = ctx;
  const key = host.keyOf("P");
  await opened(ctx, "P", 3);
  doc.type("P", "saved edit");
  client.noteEdit("P", "save-block");
  client.sendNow("P");
  await tick();
  client.receive(mail(7, key, mailPage(4, { kind: "unchanged", rev: "r4" }, { disk: { kind: "file", rev: "r4" } }),
    took(host.last("submit").id, 4)));
  client.receive(mail(7, key, mailPage(4, { kind: "unchanged" }, { conflict: true, risk: true, disk: { kind: "file", rev: "cli" } })));
  expect(doc.publishedRevs).toEqual([["P", "r4"]]);
  client.retire();
});

it("a late older publication wait never moves the ancestor back behind a newer proven one", async () => {
  vi.useFakeTimers();
  const ctx = await setup();
  const { host, doc, client } = ctx;
  const key = host.keyOf("P");
  await opened(ctx, "P", 3);
  doc.type("P", "first edit");
  client.noteEdit("P", "save-block");
  client.sendNow("P");
  await tick();
  client.receive(mail(7, key, mailPage(4, { kind: "unchanged", rev: "r4" }), took(host.last("submit").id, 4)));
  let finish!: (answer: boolean) => void;
  host.onWait = () => new Promise<boolean>(resolve => { finish = resolve; });
  const older = client.published([{ key, version: 4 }]);
  doc.type("P", "second edit");
  client.noteEdit("P", "save-block");
  client.sendNow("P");
  await tick();
  client.receive(mail(7, key, mailPage(5, { kind: "unchanged", rev: "r5" }, { disk: { kind: "file", rev: "r5" } }),
    took(host.last("submit").id, 5)));
  finish(true);
  expect(await older).toBe(true);
  expect(doc.publishedRevs).toEqual([["P", "r5"]]);
  client.retire();
});
