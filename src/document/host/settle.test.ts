import { describe, expect, it } from "vitest";
import { autoAnswer, mail, mailPage, opened, page, setup, textOf, tick, took } from "./fakes.test.support";
import { settle, unpublishedAfterDrain } from "./settle";

async function healthy() {
  const ctx = await setup();
  autoAnswer(ctx);
  ctx.doc.load(page("P", "a", "r1"));
  return ctx;
}

function edit(ctx: Awaited<ReturnType<typeof setup>>, name: string, text: string) {
  if (!ctx.doc.pages.has(name)) ctx.doc.load(page(name, "", "r0"));
  ctx.doc.type(name, text);
  ctx.client.noteEdit(name, "save-block", false);
}

describe("settle (S1: final synchronous quiescence proof)", () => {
  it("an empty scope succeeds without waiting on publication", async () => {
    const { host, client } = await setup();
    expect(await settle(client, [])).toBe(true);
    expect(await settle(client, "all", { assets: true, noConflict: true })).toBe(true);
    expect(host.count("wait")).toBe(0);
  });

  it("drains input, waits for the version the host took (with the witness), and succeeds", async () => {
    const ctx = await healthy();
    edit(ctx, "P", "ab");
    expect(await settle(ctx.client, ["P"], { witness: "id-1" })).toBe(true);
    const submit = ctx.host.last("submit");
    expect(ctx.host.last("wait").needs).toEqual([{ key: "pages/P.md", version: 102, witness: "id-1" }]);
    expect(submit.version).toBe(101);
    expect(ctx.client.busy("P")).toBe(false);
  });

  it("an edit during the fourth wait returns false (the bound is not success)", async () => {
    const ctx = await healthy();
    edit(ctx, "P", "a1");
    let waits = 0;
    ctx.host.onWait = () => { waits += 1; edit(ctx, "P", `a${waits + 1}`); };
    expect(await settle(ctx.client, "all")).toBe(false);
    expect([waits, ctx.client.isDirty("P")]).toEqual([4, true]);
  });

  it("an edit during the first wait is drained by the next round, which waits for its version", async () => {
    const ctx = await setup();
    const counter = autoAnswer(ctx);
    ctx.doc.load(page("P", "a", "r1"));
    edit(ctx, "P", "a1");
    ctx.host.onWait = () => { if (ctx.host.count("wait") === 1) edit(ctx, "P", "a2"); };
    expect(await settle(ctx.client, ["P"])).toBe(true);
    const submits = ctx.host.calls.filter((call) => call.cmd === "submit");
    expect(submits.map((call) => textOf(call.dto))).toEqual(["a1", "a2"]);
    // The reopened page's text was installed from the published bytes: no stale send.
    expect(submits[1].version).toBeGreaterThan(0);
    expect(ctx.host.last("wait").needs).toEqual([{ key: "pages/P.md", version: counter.version() }]);
    expect(ctx.client.isDirty("P")).toBe(false);
  });

  it("an asset write pending at entry is awaited, and the reference it adds is drained in the same round", async () => {
    const ctx = await healthy();
    edit(ctx, "P", "image");
    const finish = ctx.assets.start(() => edit(ctx, "Q", "![](../assets/x.png)"));
    setTimeout(finish, 0);
    expect(await settle(ctx.client, "all", { assets: true, rounds: 1 })).toBe(true);
    expect(ctx.client.busy("Q")).toBe(false);
  });

  it("an asset write that starts in the round and adds a late reference is new work, false at the bound", async () => {
    for (const rounds of [1, 4]) {
      const ctx = await healthy();
      edit(ctx, "P", "a");
      ctx.host.onWait = () => {
        if (ctx.host.count("wait") === 1) ctx.assets.start(() => edit(ctx, "Q", "![](../assets/x.png)"))();
      };
      expect(await settle(ctx.client, "all", { assets: true, rounds })).toBe(rounds === 4);
      expect(ctx.client.busy("Q")).toBe(rounds === 1);
    }
  });

  it("dirty again across a took answer: needs the answered version, not the coalesced page's; the next round sends the rest", async () => {
    const ctx = await setup();
    const { host, doc, client } = ctx;
    await opened(ctx, "P", 3);
    edit(ctx, "P", "a1");
    const settling = settle(client, ["P"]);
    await tick();
    const first = host.last("submit");
    doc.type("P", "a2");
    client.noteEdit("P", "save-block", false);
    // The host took a1 at 4; an external change made the page 6 before the mail.
    client.receive(mail(7, "pages/P.md", mailPage(6, { kind: "page", dto: page("P", "external") }), took(first.id, 4)));
    let second: ReturnType<typeof host.last<"submit">> | null = null;
    host.onWait = (needs) => {
      if (host.count("wait") === 1) expect(needs).toEqual([{ key: "pages/P.md", version: 4 }]);
      else expect(needs).toEqual([{ key: "pages/P.md", version: 7 }]);
    };
    for (let i = 0; i < 30 && host.count("submit") < 2; i += 1) await tick();
    second = host.last("submit");
    expect([second.version, textOf(second.dto)]).toEqual([4, "a2"]);
    client.receive(mail(7, "pages/P.md", mailPage(7, { kind: "unchanged" }, { conflict: true }), took(second.id, 7)));
    expect(await settling).toBe(true);
    expect(host.count("wait")).toBe(2);
  });

  it("an answer that beats its command reply is applied before the barrier counts the page", async () => {
    const ctx = await setup();
    const { host, client } = ctx;
    await opened(ctx, "P", 3);
    host.autoAdmit = false;
    edit(ctx, "P", "a1");
    const settling = settle(client, ["P"]);
    await tick();
    const submit = host.last("submit");
    client.receive(mail(7, "pages/P.md", mailPage(4, { kind: "unchanged" }), took(submit.id, 4)));
    await tick();
    expect(host.count("wait")).toBe(0);
    host.reply(submit.id);
    expect(await settling).toBe(true);
    expect(host.last("wait").needs).toEqual([{ key: "pages/P.md", version: 4 }]);
  });

  it("host debt that appears during the wait is new work for the next round", async () => {
    const ctx = await healthy();
    edit(ctx, "P", "a");
    ctx.host.onOwed = () => { if (!ctx.host.owedPages.length) ctx.host.owedPages = [{ key: "pages/R.md", version: 3 }]; };
    expect(await settle(ctx.client, "all", { rounds: 1 })).toBe(false);
    expect(await settle(ctx.client, "all")).toBe(true);
    expect(ctx.host.last("wait").needs).toContainEqual({ key: "pages/R.md", version: 3 });
  });

  it("noConflict fails on a conflicted page in scope", async () => {
    const ctx = await setup();
    await opened(ctx, "P", 3);
    ctx.client.receive(mail(7, "pages/P.md", mailPage(5, { kind: "unchanged" }, { conflict: true })));
    expect(await settle(ctx.client, ["P"], { noConflict: true })).toBe(false);
    expect(await settle(ctx.client, ["P"])).toBe(true);
  });
});

describe("rename's drain (S6)", () => {
  it("reports every unpublished page even when an unrelated conflict fails the barrier", async () => {
    const ctx = await healthy();
    const { host, client } = ctx;
    await opened(ctx, "Q", 3);
    client.receive(mail(7, "pages/Q.md", mailPage(5, { kind: "unchanged" }, { conflict: true })));
    edit(ctx, "P", "[[Old]] new reference");
    host.owedPages = [{ key: "pages/Q.md", version: 5 }, { key: "pages/Recovered.md", version: 2 }];
    host.onWait = (needs) => {
      // P's save lands; only the conflicted Q fails the barrier.
      host.owedPages = host.owedPages.filter((owed) => owed.key !== "pages/P.md");
      return !needs.some((need) => need.key === "pages/Q.md");
    };
    expect(await unpublishedAfterDrain(client)).toEqual([
      { key: "pages/Q.md", name: "Q", state: "owed", conflict: true },
      { key: "pages/Recovered.md", name: null, state: "owed", conflict: false },
    ]);
    expect(client.busy("P")).toBe(false);
  });
});

describe("publication results are the session's (REVIEW-3b-P1 F1)", () => {
  it("asks again while each bounded host wait passes, and succeeds when the needs publish", async () => {
    const ctx = await healthy();
    edit(ctx, "P", "ab");
    ctx.host.onWait = () => (ctx.host.count("wait") < 3 ? null : true);
    expect(await settle(ctx.client, ["P"])).toBe(true);
    expect(ctx.host.count("wait")).toBe(3);
  });

  it("a rebind during a pending wait fails the barrier: no result of session N vouches for N+1", async () => {
    const ctx = await healthy();
    edit(ctx, "P", "ab");
    ctx.host.onWait = async () => {
      ctx.host.session = 8;
      await ctx.client.rebind();
      return true;
    };
    expect(await settle(ctx.client, ["P"])).toBe(false);
    expect(ctx.host.count("wait")).toBe(1);
  });

  it("debt listed for an earlier session fails the barrier instead of reading as none", async () => {
    const { host, client } = await setup();
    expect(await settle(client, "all")).toBe(true);
    host.session = 9;
    expect(await settle(client, "all")).toBe(false);
    expect(host.count("wait")).toBe(0);
  });
});
