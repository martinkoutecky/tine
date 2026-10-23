// F1/F6: real wiring/HostClient/working set -> controlled JSON transport ->
// real Store/PageHost/ProductionIo, then independent cold native answers.
import { afterAll, beforeAll, expect, it, vi } from "vitest";
import { writeFileSync } from "node:fs";
import { backend } from "../../backend";
import { bumpGraphEpoch, graphEpoch } from "../../graphSession";
import { applyGraphAnswers } from "../../graphAnswers";
import { sharedQueryResult } from "../../queryResultCache";
import { doc, pageByName, setDoc } from "../model";
import { pageToDto, toFeedPage } from "../convert";
import { ensurePageLoaded, installPageIdentityNavigation, reloadPageIfStillSafe, resetStore } from "../workingSet";
import { bindHost, markDirty } from "./wiring";
import { HostClient } from "./client";
import { settle } from "./settle";
import { runSequence } from "./planner";
import { createMemo, createRoot } from "solid-js";
import type { PageDto } from "../../types";
import type { PageMail, PageRefusal } from "./protocol";
import { buildPeer, NativePeer, random, semantics, TARGET, type NativeImage, type NativeReply } from "./nativeConformance.test.support";

// Only the warm-start signal is supplied by the harness. No host, document,
// client, mirror, query evaluator, parser, serializer or filesystem is mocked.
vi.mock("../../warmCache", () => ({ waitForWarmCache: async () => true }));

type Pending = { arg: Record<string, unknown>; finish: (reply: never) => void };
let peer: NativePeer;
let client: HostClient;
let listener: (mail: PageMail) => void;
let queued: Pending[] = [];
let holdWaits = false;
let waits: (() => void)[] = [];
let initialCounts: (() => void)[] = [];
let holdCounts = false;
let mountedCount: () => number;
let disposeCount: () => void;
const counts: Record<string, number> = {};
function hit(branch: string): void { counts[branch] = (counts[branch] ?? 0) + 1; }
const ticks = async () => { for (let i = 0; i < 16; i++) await Promise.resolve(); };

function enqueue<T>(arg: Record<string, unknown>): Promise<T> {
  return new Promise(resolve => queued.push({ arg: structuredClone(arg), finish: resolve as (reply: never) => void }));
}
function deliver(mail: PageMail[]): void { for (const item of mail) listener(item); }
function windowState(name: string): unknown {
  return { dto: semantics(pageToDto(name)), disk: client.disk(name), notice: client.notice(name),
    needed: client.needs([name]), busy: client.busy(name), owes: client.owes(name),
    conflict: client.conflicted(name), consumed: client.consumed };
}

interface ModelState { s: { w: { on: boolean; text: number; pend: boolean; sent: boolean }[];
  pages: { buf: number; ver: number }[] } }
async function model(action: string, args: unknown[], name?: string, labels?: string[]): Promise<ModelState> {
  const state = await peer.rpc<ModelState>({ op: "oracle", action, args });
  if (name) {
    const window = state.s.w[0];
    expect(client.isOpen(name), `${action}: model window acquisition`).toBe(window.on);
    expect(client.busy(name), `${action}: model pending/in-flight obligation`).toBe(window.pend || window.sent);
    if (window.on && labels) expect(pageToDto(name)!.blocks[0].raw, `${action}: model window text`)
      .toBe(labels[window.text - 1]);
    hit("quint-window-boundary");
  }
  return state;
}

beforeAll(async () => {
  peer = new NativePeer(buildPeer());
  await peer.rpc({ op: "reset" });
  const originalBind = HostClient.prototype.bind;
  vi.spyOn(HostClient.prototype, "bind").mockImplementation(async function (this: HostClient) {
    client = this;
    await originalBind.call(this);
  });
  const b = backend();
  vi.spyOn(b, "onPageMail").mockImplementation(async cb => { listener = cb; return () => {}; });
  vi.spyOn(b, "pageWindowReloaded").mockImplementation(() => peer.rpc({ op: "reload" }));
  vi.spyOn(b, "pageOpen").mockImplementation((session, id, page) => enqueue({ op: "open", session, id,
    name: page.name, path: page.path ?? `pages/${page.name}.md` }));
  vi.spyOn(b, "pageSubmit").mockImplementation((session, id, key, dto, version, resolve, kinds) =>
    enqueue({ op: "submit", session, id, key, dto, version, resolve, kinds }));
  vi.spyOn(b, "pageClose").mockImplementation((session, id, key) => enqueue({ op: "close", session, id, key }));
  vi.spyOn(b, "pageDiscard").mockImplementation((session, id, key) => enqueue({ op: "discard", session, id, key }));
  vi.spyOn(b, "pageSaveNow").mockImplementation(async (session, keys) => { await peer.rpc({ op: "save", session, keys }); });
  vi.spyOn(b, "pageWait").mockImplementation((session, needs) => new Promise(resolve => {
    const start = () => { void peer.rpc<boolean | null>({ op: "wait", session, needs }).then(resolve); };
    if (holdWaits) waits.push(start); else start();
  }));
  vi.spyOn(b, "pageOwed").mockImplementation(session => peer.rpc({ op: "owed", session }));
  vi.spyOn(b, "getPageByPath").mockImplementation(async path => {
    const image = await peer.image();
    const page = image.pages.find(p => p.path === path);
    return page ? { ...page.dto, id: page.path } : null;
  });
  vi.spyOn(b, "getBlockRefCounts").mockImplementation(async () => {
    const image = await peer.image();
    if (holdCounts) await new Promise<void>(resolve => initialCounts.push(resolve));
    return image.counts;
  });
  const { blockRefCount } = await import("../../blockRefCounts");
  createRoot(dispose => { disposeCount = dispose; mountedCount = createMemo(() => blockRefCount(TARGET)); });
  installPageIdentityNavigation(() => {});
}, 600_000);

// REG-HOST-REBIND-ACQUISITION-RELEASE: production fixes are outside this lane.
// Opt in to the ignored native repro with TINE_WINDOW_REBIND_REPRO=1 and -t.
it.skipIf(process.env.TINE_WINDOW_REBIND_REPRO !== "1")("known bug: releasing the last acquisition after rebind closes the native page", async () => {
  resetStore(); queued = []; holdCounts = false;
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
  const initial = await peer.rpc<NativeImage>({ op: "reset" });
  bumpGraphEpoch();
  for (const page of initial.pages) expect(ensurePageLoaded({ ...page.dto, id: page.path })).toBeNull();
  await bindHost();
  const release = client.acquire("Alpha");
  await step(1);
  await client.rebind(); await pump(1);
  expect(client.isOpen("Alpha")).toBe(true);
  release(); await pump(1);
  expect(client.isOpen("Alpha"), "the final release must close the rebound native page").toBe(false);
});

afterAll(async () => {
  holdWaits = false;
  waits.splice(0).forEach(start => start());
  initialCounts.splice(0).forEach(finish => finish());
  resetStore();
  disposeCount?.();
  vi.useRealTimers();
  await peer?.close();
  vi.restoreAllMocks();
});

async function step(order: number, during?: () => void, nativeArg?: Record<string, unknown>): Promise<PageMail[]> {
  await ticks();
  const request = queued.shift();
  expect(request, "a real client command must be outstanding").toBeDefined();
  const arg = nativeArg ?? request!.arg;
  const submitting = arg.op === "submit";
  const name = submitting ? (request!.arg.dto as PageDto).name : request!.arg.name as string;
  if (submitting) {
    expect(client.isDirty(name), "no acknowledgement before native custody").toBe(true);
  }
  during?.();
  const result = await peer.rpc<NativeReply | PageRefusal | null>(arg);
  let reply = result;
  if (submitting) {
    const native = result as NativeReply;
    reply = native.reply as PageRefusal | null;
    expect(native.custody).toBe(!reply);
    if (!reply) {
      // The fixture is deliberately an interior serializable DTO; native checks
      // its comparison-source serialization against the actual admitted bytes.
      expect(native.bytes).toContain((request!.arg.dto as PageDto).blocks[0].raw.split("\n")[0]);
      hit("custody-bytes");
    }
    expect(client.isDirty(name), "custody alone is not a delivered admission").toBe(true);
  }
  if (reply && typeof reply === "object" && "reason" in reply) {
    request!.finish(reply as never);
    await peer.rpc({ op: "resume" });
    await ticks();
    expect(client.isDirty(name)).toBe(true);
    hit(`refusal:${reply.reason}`);
    return [];
  }
  await peer.rpc({ op: "resume" });
  if (arg.op === "close") {
    request!.finish(reply as never); await ticks(); hit("close"); return [];
  }
  const mail = await peer.mail(Number(arg.id));
  if (order === 0) {
    deliver(mail); await ticks();
    if (submitting) expect(client.isDirty(name), "mail is stashed until the command reply").toBe(true);
    request!.finish(reply as never); hit("mail-before-reply");
  } else {
    request!.finish(reply as never); await ticks();
    if (submitting) expect(client.busy(client.nameOfKey(String(arg.key)) ?? name), "admission without an answer stays in flight").toBe(true);
    deliver(mail); hit("reply-before-mail");
  }
  await ticks();
  return mail;
}

function type(name: string, text: string): void {
  const page = pageByName(name)!;
  setDoc("byId", page.roots[0], "raw", text);
  markDirty(name, "save-block");
}

async function pump(order: number): Promise<void> {
  for (let n = 0; n < 12; n++) {
    await ticks();
    if (!queued.length) { deliver(await peer.mail()); await ticks(); if (!queued.length) return; }
    if (queued.length) await step(order);
  }
  throw new Error("client did not drain in twelve actual commands");
}

async function settled(names: string[], order: number): Promise<void> {
  // Use the actual barrier: its native waits prove both save and index debt.
  let done = false;
  const result = settle(client, names).then(ok => { done = true; return ok; });
  for (let n = 0; !done && n < 200; n++) {
    if (queued.length) await step(order);
    else { await peer.rpc({ op: "resume" }); deliver(await peer.mail()); await ticks(); }
  }
  expect(done, "settle completed within 200 native transport rounds").toBe(true);
  expect(await result, "native durable/indexed witness").toBe(true);
  for (const name of names) expect(client.busy(name) || client.owes(name)).toBe(false);
  hit("durable-indexed");
}

async function mirrors(order: number): Promise<void> {
  const changes = await peer.changes();
  const ordered = order ? changes.toReversed() : changes;
  for (const change of ordered) applyGraphAnswers(change);
  if (changes.length) { hit(order ? "publication-reversed" : "publication-ordered"); }
  if (order && changes.length > 1) hit("publication-reversed-pair");
  if (initialCounts.length) hit("initial-count-behind-deltas");
  initialCounts.splice(0).forEach(finish => finish());
  await ticks();
  const live = await peer.image();
  const cold = await peer.image(true);
  const query = await sharedQueryResult(`native:${graphEpoch()}`, `target:${live.rev}`, async () => live.query);
  expect(semantics(query), "cached query vs cold native window").toEqual(semantics(cold.query));
  expect(mountedCount(), "mounted count mirror vs cold native window").toBe(cold.counts[TARGET] ?? 0);
  // A separate frontend document assembled only from the cold native DTOs.
  const freshNodes = {};
  const freshPages = cold.pages.map(p => ({ ...toFeedPage(p.dto, freshNodes), id: p.path }));
  const names = freshPages.map(p => [p.name, p.id]).sort();
  expect(doc.pages.map(p => [p.name, p.id]).sort(), "working-set name slots vs fresh window").toEqual(names);
  const authored = (dto: PageDto) => {
    const blocks = (list: PageDto["blocks"]): unknown => list.map(b => [b.raw, b.collapsed, blocks(b.children)]);
    return [dto.name, dto.title, dto.kind, dto.format ?? "md", dto.pre_block, blocks(dto.blocks)];
  };
  for (const page of cold.pages) expect(authored(pageToDto(page.dto.name)!), "working-set authored text vs cold native window")
    .toEqual(authored(page.dto));
  hit("live-fresh-mirrors");
}

it("seeded real window custody and native-publication mirrors equal a cold window at every settled step", async () => {
  const number = Number(process.env.TINE_WINDOW_SEEDS ?? 6);
  const start = performance.now();
  for (let seed = 1; seed <= number; seed++) {
    const choose = random(seed);
    peer.trace.length = 0;
    try {
      resetStore(); queued = [];
      vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
      holdCounts = seed % 2 === 1;
      const initial = await peer.rpc<NativeImage>({ op: "reset" });
      bumpGraphEpoch();
      for (const page of initial.pages) expect(ensurePageLoaded({ ...page.dto, id: page.path })).toBeNull();
      await bindHost();
      if (!holdCounts) { await ticks(); hit("initial-count-before-deltas"); }
      let name = "Alpha";
      const labels = ["initial [[Target]]", `seed ${seed} unsent Ω [[Target]]`,
        `seed ${seed} newer\n((${TARGET})) [[Target]]`];
      let release = client.acquire(name);
      await model("wOpen", [0], name);
      await step(choose(2));
      await model("deliverUp", [true]); await model("wRecv", [0], name, labels);
      expect(mountedCount(), "new native graph epoch clears the prior graph's counts").toBe(0);
      hit("count-epoch");
      // A stale-session refusal is produced by the real admission check, then retried
      // with the client's still-current session and unchanged local input.
      type(name, labels[1]); await model("wEdit", [0, 2], name, labels); client.sendNow(name);
      const stale = { ...queued[0].arg, session: client.session + 99999 };
      await step(choose(2), undefined, stale);
      expect(client.isDirty(name)).toBe(true);
      client.sendNow(name);
      await model("wSend", [0], name, labels);
      type(name, labels[2]); await model("wEdit", [0, 3], name, labels);
      const mail = await step(choose(2));
      await model("deliverUp", [true]); await model("wRecv", [0], name, labels);
      expect(pageToDto(name)!.blocks[0].raw).toContain("newer");
      expect(client.isDirty(name), "newer generation survives the previous answer").toBe(true);
      hit("typing-in-flight");
      client.sendNow(name); await model("wSend", [0], name, labels); await step(choose(2));
      await model("deliverUp", [true]); await model("wRecv", [0], name, labels);
      await settled([name], choose(2));
      await model("flush", [0]); await model("check", []); await model("rename", []);
      const publishedModel = await model("dirSync", [true], name, labels);
      const native = await peer.rpc<{ buffers: Record<string, { version: number; bytes: string }> }>({ op: "inspect" });
      expect(native.buffers["pages/Alpha.md"].version).toBe(publishedModel.s.pages[0].ver);
      expect(native.buffers["pages/Alpha.md"].bytes).toContain(labels[publishedModel.s.pages[0].buf - 1].split("\n")[0]);
      holdCounts = false;
      await mirrors(choose(2));

      // Two actual publications change the SAME count target to different
      // values before either reaches the window. Reversing these must retain
      // the later count, not merely survive a reversal of disjoint keys.
      await peer.rpc({ op: "external", path: "pages/Beta.md",
        bytes: `- wave one [[Target]] ((${TARGET}))\n` });
      const finalBeta = choose(2) ? `- wave final [[Target]]\n` :
        `- wave final [[Target]] ((${TARGET}))\n- second ((${TARGET}))\n`;
      await peer.rpc({ op: "external", path: "pages/Beta.md", bytes: finalBeta });
      const beta = (await peer.image()).pages.find(p => p.path === "pages/Beta.md")!;
      expect(reloadPageIfStillSafe("Beta", { ...beta.dto, id: beta.path })).toBe(true);
      await mirrors(1); hit("same-target-publication-wave");

      type(name, `coalesced ${seed} [[Target]]`); client.sendNow(name);
      const coalescing = { ...queued[0].arg, coalesceBytes: `- native observation ${seed} [[Target]]\n` };
      const coalesced = await step(choose(2), () => type(name, `coalesced newer ${seed} [[Target]]`), coalescing);
      expect(coalesced.some(m => m.answer?.took && m.page?.text.kind === "page"
        && m.page.version > m.answer.version), "real native answer includes newer coalesced content").toBe(true);
      expect(pageToDto(name)!.blocks[0].raw).toContain("coalesced newer");
      expect(client.isDirty(name)).toBe(true); hit("coalesced-newer-buffer");
      client.sendNow(name); await step(choose(2));
      expect(client.conflicted(name), "newer input on the old authoring version becomes a conflict").toBe(true);
      const disk = client.disk(name)!;
      const ticket = client.reviewTicket(name, disk.kind === "file" ? disk.rev : "absent")!;
      const merge = client.submitReviewed(ticket, async () => structuredClone(pageToDto(name)!));
      await ticks(); await step(choose(2));
      expect(await merge).toMatchObject({ took: true }); hit("reviewed-retry");
      await settled([name], choose(2)); await mirrors(choose(2));

      // Refusal by native DTO validation: it must not consume the unsent edit.
      type(name, `seed ${seed} retry [[Target]]`); client.sendNow(name);
      const invalid = structuredClone(queued[0].arg);
      (invalid.dto as PageDto).guide = true;
      await step(choose(2), undefined, invalid);
      client.sendNow(name); await step(choose(2));
      await settled([name], choose(2)); await mirrors(choose(2));

      // Random title identity changes are native publications. The real wiring
      // rekeys its working set/client while newer unsent input stays in the slot.
      for (let n = 0; n < 2 + choose(3); n++) {
        const next = `Renamed-${seed}-${n}`;
        setDoc("pages", p => p.name === name, "preBlock", `title:: ${next}`);
        type(name, `title ${n} [[Target]]${choose(2) ? `\n((${TARGET}))` : ""}`);
        client.sendNow(name);
        await step(choose(2), () => type(name, `title newer ${n} [[Target]]${choose(2) ? `\n((${TARGET}))` : ""}`));
        name = next;
        expect(pageByName(name)?.id).toBe("pages/Alpha.md");
        hit("title-rekey");
        await settled([name], choose(2)); await mirrors(choose(2));
      }

      // Same-name replacement may not steal a slot containing unsent input.
      type(name, `retained ${seed} [[Target]]`);
      await peer.rpc({ op: "external", path: "pages/replacement.md", bytes: `title:: ${name}\n\n- replacement\n` });
      const replacement = (await peer.image()).pages.find(p => p.path === "pages/replacement.md")!;
      expect(ensurePageLoaded({ ...replacement.dto, id: replacement.path })?.reason).toBe("unsaved-work");
      expect(pageToDto(name)!.blocks[0].raw).toContain("retained");
      hit("replacement-retains-unsent");
      // Remove the temporary claimant through a native test-environment command.
      await peer.rpc({ op: "remove", path: replacement.path });
      client.sendNow(name); await step(choose(2)); await settled([name], choose(2));

      // An empty real planner sequence still freezes, opens and drains its endpoints.
      expect(await runSequence(client, [name], () => [], ["save-block"])).toEqual({ ok: true, completed: 0 });
      hit("planner-drain");

      // Close with publication debt: withholding the native witness must retain
      // the page. Only a true native answer allows the actual close command.
      type(name, `close ${seed} [[Target]]\n((${TARGET}))`); client.sendNow(name); await step(choose(2));
      holdWaits = true;
      release(); await ticks();
      expect(client.owes(name)).toBe(true);
      expect(client.isOpen(name)).toBe(true);
      expect(queued.some(q => q.arg.op === "close")).toBe(false);
      hit("close-publication-debt");
      holdWaits = false; waits.splice(0).forEach(start => start());
      await settled([name], choose(2)); await pump(choose(2));

      // Old real mail after both reload and saved restore/relaunch must do nothing.
      const oldMail = mail;
      release = client.acquire(name); await step(choose(2));
      await client.rebind(); await pump(choose(2));
      const before = windowState(name);
      deliver(oldMail); await ticks();
      expect(windowState(name)).toEqual(before); hit("old-session-mail");
      const staleReply = await peer.rpc({ op: "close", session: oldMail[0].session,
        id: 999999, key: "pages/Alpha.md" });
      expect(staleReply).toEqual({ reason: "not-admitted" });
      await peer.rpc({ op: "restore", consumed: client.consumed });
      await bindHost(); await pump(choose(2));
      const restored = windowState(name);
      deliver(oldMail); expect(windowState(name)).toEqual(restored);
      hit("restore-relaunch");
      await mirrors(choose(2));

      // Initial replies crossing an epoch are read-owned: the retired result
      // cannot repopulate either mirror even if its revision number is reused.
      const oldQuery = (await peer.image()).query;
      let answerOld!: (q: object) => void;
      const late = sharedQueryResult(`native:${graphEpoch()}`, "late:0", () => new Promise<object>(r => { answerOld = r; }));
      const oldEpoch = graphEpoch();
      bumpGraphEpoch();
      const freshQuery = await sharedQueryResult(`native:${graphEpoch()}`, "late:0", async () => []);
      answerOld(oldQuery); await late;
      expect(await sharedQueryResult(`native:${graphEpoch()}`, "late:0", async () => oldQuery)).toBe(freshQuery);
      expect(graphEpoch()).toBeGreaterThan(oldEpoch); hit("late-query-epoch");
      release(); await pump(choose(2));
      client.retire(); resetStore(); vi.useRealTimers();
      if (seed % 50 === 0) console.log(`native conformance completed ${seed}/${number} seeds`);
    } catch (error) { peer.capsule(seed, error); throw error; }
  }
  for (const branch of ["mail-before-reply", "reply-before-mail", "refusal:not-admitted", "refusal:invalid-target",
    "custody-bytes", "typing-in-flight", "coalesced-newer-buffer", "reviewed-retry", "title-rekey", "replacement-retains-unsent", "planner-drain",
    "close-publication-debt", "old-session-mail", "restore-relaunch", "publication-reversed",
    "publication-ordered", "publication-reversed-pair", "same-target-publication-wave", "live-fresh-mirrors", "quint-window-boundary",
    "initial-count-behind-deltas", "initial-count-before-deltas", "count-epoch", "late-query-epoch", "durable-indexed", "close"])
    expect(counts[branch], `unvisited declared branch: ${branch}`).toBeGreaterThan(0);
  const receipt = { seeds: number, milliseconds: performance.now() - start, branches: counts };
  writeFileSync("artifacts/conformance-f1-f6/branches.json", JSON.stringify(receipt, null, 2));
  console.log("native window conformance", receipt);
}, 600_000);
