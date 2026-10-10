import { describe, expect, it } from "vitest";
import { reviewedDiskToken } from "./client";
import { applied, mail, mailPage, opened, page, setup, textOf, tick, took } from "./fakes.test.support";

const KEY = "pages/P.md";

describe("§4.2 send and answer (model wSend/wRecv)", () => {
  it("sends input on the version its text is typed on and adopts the version the host took", async () => {
    const ctx = await setup();
    const { host, doc, client } = ctx;
    await opened(ctx, "P", 3);
    expect(client.versionOf("P")).toBe(3);
    doc.type("P", "ab");
    client.noteEdit("P", "save-block", false);
    client.sendNow("P");
    await tick();
    const submit = host.last("submit");
    expect([submit.version, textOf(submit.dto), submit.kinds]).toEqual([3, "ab", ["save-block"]]);
    client.receive(mail(7, KEY, mailPage(4, { kind: "unchanged" }), took(submit.id, 4)));
    expect([client.versionOf("P"), client.isDirty("P"), client.busy("P")]).toEqual([4, false, false]);
  });

  it("applies an answer that beats its command reply at admission, exactly once", async () => {
    const ctx = await setup();
    const { host, doc, client } = ctx;
    await opened(ctx, "P", 3);
    host.autoAdmit = false;
    doc.type("P", "ab");
    client.noteEdit("P", "save-block", false);
    client.sendNow("P");
    const submit = host.last("submit");
    client.receive(mail(7, KEY, mailPage(4, { kind: "unchanged" }), took(submit.id, 4)));
    expect([client.versionOf("P"), client.busy("P")]).toEqual([3, true]);
    host.reply(submit.id);
    await tick();
    expect([client.versionOf("P"), client.busy("P")]).toEqual([4, false]);
    // The same answer delivered again is a push of that state, not a second answer.
    client.receive(mail(7, KEY, mailPage(4, { kind: "unchanged" }), took(submit.id, 4)));
    expect(client.versionOf("P")).toBe(4);
  });

  it("keeps the text and the dirty state when the host does not admit a command", async () => {
    const ctx = await setup();
    const { host, doc, client } = ctx;
    await opened(ctx, "P", 3);
    host.refuse = { reason: "not-admitted" };
    doc.type("P", "ab");
    client.noteEdit("P", "save-block", false);
    client.sendNow("P");
    await tick();
    expect([client.isDirty("P"), client.busy("P"), doc.text("P"), client.versionOf("P")]).toEqual([true, true, "ab", 3]);
  });

  it("debounces automatic sends", async () => {
    const { vi } = await import("vitest");
    vi.useFakeTimers();
    try {
      const ctx = await setup();
      const { host, client } = ctx;
      await opened(ctx, "P", 3);
      client.noteEdit("P", "save-block");
      client.noteEdit("P", "save-block");
      expect(host.count("submit")).toBe(0);
      await vi.advanceTimersByTimeAsync(400);
      expect(host.count("submit")).toBe(1);
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("S4 installation gate and the took-answer rule", () => {
  async function sending(text = "ab") {
    const ctx = await setup();
    await opened(ctx, "P", 3);
    ctx.doc.type("P", text);
    ctx.client.noteEdit("P", "save-block", false);
    ctx.client.sendNow("P");
    await tick();
    return { ...ctx, id: ctx.host.last("submit").id };
  }

  it("took + newer input: the input moves to the answer version, not a coalesced newer page version", async () => {
    const { host, doc, client, id } = await sending();
    doc.type("P", "abc");
    client.noteEdit("P", "save-block", false);
    client.receive(mail(7, KEY, mailPage(6, { kind: "page", dto: page("P", "external") }), took(id, 4)));
    expect([client.versionOf("P"), doc.text("P"), client.isDirty("P")]).toEqual([4, "abc", true]);
    client.sendNow("P");
    await tick();
    expect(host.last("submit").version).toBe(4);
  });

  it("took coalesced with a newer external page, no newer input: adopts the page at its version", async () => {
    const { doc, client, id } = await sending();
    client.receive(mail(7, KEY, mailPage(6, { kind: "page", dto: page("P", "external", "r6") }), took(id, 4)));
    expect([client.versionOf("P"), doc.text("P")]).toEqual([6, "external"]);
  });

  it("not-took (discard) + newer input keeps the version the input was typed on", async () => {
    const ctx = await setup();
    const { host, doc, client } = ctx;
    await opened(ctx, "P", 3);
    const answer = client.discard("P");
    await tick();
    const discard = host.last("discard");
    doc.type("P", "typed");
    client.noteEdit("P", "save-block", false);
    client.receive(mail(7, KEY, mailPage(5, { kind: "page", dto: page("P", "disk") }), applied(discard.id, 5)));
    expect((await answer).took).toBe(false);
    expect([client.versionOf("P"), doc.text("P"), client.isDirty("P")]).toEqual([3, "typed", true]);
  });

  it("a push held under a pin keeps the pin's start version for its commit; unpinned without input, it installs", async () => {
    const ctx = await setup();
    const { host, doc, client } = ctx;
    await opened(ctx, "P", 3);
    const unpin = client.acquire("P", { pin: true });
    client.receive(mail(7, KEY, mailPage(5, { kind: "page", dto: page("P", "external", "r5") })));
    expect([client.versionOf("P"), doc.text("P"), client.heldVersion("P")]).toEqual([3, "a", 5]);
    unpin();
    expect([client.versionOf("P"), doc.text("P"), client.heldVersion("P")]).toEqual([5, "external", null]);

    const unpin2 = client.acquire("P", { pin: true });
    client.receive(mail(7, KEY, mailPage(7, { kind: "page", dto: page("P", "external2", "r7") })));
    doc.type("P", "cell commit");
    client.noteEdit("P", "save-block", false);
    unpin2();
    client.sendNow("P");
    await tick();
    expect([host.last("submit").version, doc.text("P"), client.heldVersion("P")]).toEqual([5, "cell commit", null]);
  });

  it("took under a pin: the version advances to the answer, the coalesced page waits for the unpin", async () => {
    const { doc, client, id } = await sending();
    const unpin = client.acquire("P", { pin: true });
    client.receive(mail(7, KEY, mailPage(6, { kind: "page", dto: page("P", "external", "r6") }), took(id, 4)));
    expect([client.versionOf("P"), doc.text("P"), client.heldVersion("P")]).toEqual([4, "ab", 6]);
    unpin();
    expect([client.versionOf("P"), doc.text("P")]).toEqual([6, "external"]);
  });

  it("notice-only mail during an Always-ask hold never releases the held text/version pair", async () => {
    const ctx = await setup();
    const { doc, client } = ctx;
    await opened(ctx, "P", 3);
    doc.pushHeld.add("P");
    client.receive(mail(7, KEY, mailPage(5, { kind: "page", dto: page("P", "external", "r5") })));
    client.receive(mail(7, KEY, mailPage(5, { kind: "unchanged" })));
    client.release("P");
    expect([client.versionOf("P"), doc.text("P"), client.heldVersion("P")]).toEqual([3, "a", 5]);
    client.release("P", true);
    expect([client.versionOf("P"), doc.text("P")]).toEqual([5, "external"]);
  });

  it("an editor on a different file or a block move holds content like a pin", async () => {
    const ctx = await setup();
    const { doc, client } = ctx;
    await opened(ctx, "P", 3);
    doc.held.add("P");
    client.receive(mail(7, KEY, mailPage(5, { kind: "page", dto: page("P", "external", "r5") })));
    expect(doc.text("P")).toBe("a");
    doc.held.delete("P");
    client.release("P");
    expect([client.versionOf("P"), doc.text("P")]).toEqual([5, "external"]);
  });

  it("freeze awaits a composition that commits after blur, and fails on a cell that cannot commit", async () => {
    const ctx = await setup();
    const { doc, client } = ctx;
    await opened(ctx, "P", 3);
    let end!: () => void;
    const composition = client.acquire("P", { pin: true, commit: () => new Promise<boolean>((resolve) => {
      end = () => { doc.type("P", "composed"); client.noteEdit("P", "save-block", false); resolve(true); };
    }) });
    const freezing = client.freeze(["P"]);
    await tick();
    expect(client.isDirty("P")).toBe(false);
    end();
    const frozen = await freezing;
    expect([frozen.ok, client.isDirty("P"), client.isFrozen("P")]).toEqual([true, true, true]);
    frozen.unfreeze();
    composition();

    await opened(ctx, "Q", 2);
    client.acquire("Q", { pin: true, commit: () => false });
    const failed = await client.freeze(["P", "Q"]);
    expect([failed.ok, failed.failed, client.isFrozen("P"), client.isFrozen("Q")]).toEqual([false, ["Q"], false, false]);
  });
});

describe("S5 edit-intent acquisition and baseline provenance", () => {
  it("focus without mutation opens, sends nothing, and closes on release", async () => {
    const ctx = await setup();
    const { host, client } = ctx;
    const release = await opened(ctx, "P", 3);
    expect([client.isOpen("P"), host.count("submit"), host.count("close")]).toEqual([true, 0, 0]);
    release();
    await tick();
    expect(host.count("close")).toBe(1);
  });

  it("a pinned cell acquires the page before blur, and a kept clean editor stays open", async () => {
    const ctx = await setup();
    const { host, doc, client } = ctx;
    doc.load(page("P", "a", "r3"));
    const unpin = client.acquire("P", { pin: true });
    expect(host.last("open").name).toBe("P");
    await tick();
    client.receive(mail(7, KEY, mailPage(3, { kind: "page", dto: page("P", "a", "r3") }, { disk: { kind: "file", rev: "r3" } }),
      applied(host.last("open").id, 3)));
    const editing = client.acquire("P");
    doc.type("P", "b");
    client.noteEdit("P", "save-block", false);
    client.sendNow("P");
    await tick();
    client.receive(mail(7, KEY, mailPage(4, { kind: "unchanged", rev: "r4" }), took(host.last("submit").id, 4)));
    unpin();
    await tick();
    expect([client.isOpen("P"), host.count("close")]).toEqual([true, 0]);
    editing();
    await tick();
    expect(host.count("close")).toBe(1);
  });

  async function reopenWithInput(ctx: Awaited<ReturnType<typeof setup>>, answer: Parameters<typeof mailPage>) {
    const { host, doc, client } = ctx;
    doc.type("P", "typed early");
    client.noteEdit("P", "save-block", false);
    await tick();
    const open = host.last("open");
    client.receive(mail(7, KEY, mailPage(...answer), applied(open.id, answer[0])));
    client.sendNow("P");
    await tick();
    return host.last("submit").version;
  }

  async function closed(ctx: Awaited<ReturnType<typeof setup>>, before?: () => void) {
    const release = await opened(ctx, "P", 3, "a", "r3");
    before?.();
    release();
    await tick();
  }

  it("a reopen the host still holds at the text's version grants it", async () => {
    const ctx = await setup();
    await closed(ctx);
    expect(await reopenWithInput(ctx, [3, { kind: "page", dto: page("P", "a", "r3") }, { risk: true, disk: { kind: "file", rev: "r3" } }])).toBe(3);
  });

  it("E101: a version remembered under one session never grants under the next", async () => {
    const ctx = await setup();
    const { host, doc, client } = ctx;
    await closed(ctx);
    host.session = 8;
    host.nextId = 50;
    await client.rebind();
    doc.type("P", "typed early");
    client.noteEdit("P", "save-block", false);
    await tick();
    const open = host.last("open");
    expect(open.id).toBe(50);
    // The new host instance happens to answer at the old number (3); only the
    // reopen clause could grant it, and the remembered 3 is the old session's.
    client.receive(mail(8, KEY, mailPage(3, { kind: "page", dto: page("P", "a", "r3") }, { risk: true, disk: { kind: "file", rev: "r3" } }),
      applied(open.id, 3)));
    client.sendNow("P");
    await tick();
    expect(host.last("submit").version).toBe(0);
  });

  it("a clean Open whose disk is the installed bytes grants its version", async () => {
    const ctx = await setup();
    await closed(ctx);
    expect(await reopenWithInput(ctx, [9, { kind: "page", dto: page("P", "a", "r3") }, { disk: { kind: "file", rev: "r3" } }])).toBe(9);
  });

  it("a held push never advances the baseline: the first-dirty Open after it sends stale", async () => {
    const ctx = await setup();
    await closed(ctx, () => {
      ctx.doc.pushHeld.add("P");
      ctx.client.receive(mail(7, KEY, mailPage(5, { kind: "page", dto: page("P", "external", "r5") })));
    });
    expect(await reopenWithInput(ctx, [5, { kind: "page", dto: page("P", "external", "r5") }, { disk: { kind: "file", rev: "r5" } }])).toBe(0);
    expect(ctx.doc.text("P")).toBe("typed early");
  });

  it("the same name and bytes on another entry, a twin format, or a buffer that is not disk keep no version", async () => {
    for (const [entry, dto, extra] of [
      [false, page("P", "a", "r3"), {}],
      [true, { ...page("P", "a", "r3"), format: "org" as const }, {}],
      [true, page("P", "a", "r4"), {}],
      [true, page("P", "a", "r3"), { conflict: true }],
    ] as const) {
      const ctx = await setup();
      ctx.doc.load(page("P", "a", "r3"));
      ctx.host.baselineEntry = entry;
      ctx.client.acquire("P");
      expect(await reopenWithInput(ctx, [9, { kind: "page", dto }, { disk: { kind: "file", rev: "r3" }, ...extra }])).toBe(0);
    }
  });

  it("a spelling-only move of the same entry (the host's identity says so) still grants", async () => {
    const ctx = await setup();
    ctx.doc.load(page("P", "a", "r3"), "pages/p.md");
    ctx.host.baselineEntry = true;
    ctx.client.acquire("P");
    expect(await reopenWithInput(ctx, [9, { kind: "page", dto: page("P", "a", "r3") }, { disk: { kind: "file", rev: "r3" } }])).toBe(9);
  });

  it("an unknown revision is not a proved absence", async () => {
    const unknown = await setup();
    unknown.doc.load(page("P", "a"), null, { kind: "unknown" });
    unknown.client.acquire("P");
    expect(await reopenWithInput(unknown, [9, { kind: "no-file" }, { disk: { kind: "no-file" } }])).toBe(0);
    const absent = await setup();
    absent.doc.load(page("P", "a", null), null, { kind: "no-file" });
    absent.client.acquire("P");
    expect(await reopenWithInput(absent, [9, { kind: "no-file" }, { disk: { kind: "no-file" } }])).toBe(9);
  });
});

describe("S7 Concord review capture", () => {
  async function conflicted() {
    const ctx = await setup();
    await opened(ctx, "P", 3);
    ctx.client.receive(mail(7, KEY, mailPage(5, { kind: "unchanged" }, { conflict: true, disk: { kind: "no-file" } })));
    return ctx;
  }

  it("maps the absent sentinel to NoFile and submits the reviewed merge on the review's version", async () => {
    expect(reviewedDiskToken("absent")).toEqual({ kind: "no-file" });
    expect(reviewedDiskToken("abc")).toEqual({ kind: "file", rev: "abc" });
    const { host, doc, client } = await conflicted();
    const ticket = client.reviewTicket("P", "absent")!;
    const result = client.submitReviewed(ticket, async () => page("P", "merged"));
    await tick(10);
    const submit = host.last("submit");
    expect([submit.resolve, submit.version, submit.kinds, textOf(submit.dto), doc.text("P")])
      .toEqual([{ kind: "no-file" }, 3, ["replace-page"], "merged", "merged"]);
    client.receive(mail(7, KEY, mailPage(6, { kind: "unchanged" }), took(submit.id, 6)));
    expect(await result).toEqual({ took: true, refusal: null });
  });

  it("input typed while the merge is produced makes the review stale; nothing is submitted", async () => {
    const { host, doc, client } = await conflicted();
    const ticket = client.reviewTicket("P", "r9")!;
    let finish!: (dto: ReturnType<typeof page>) => void;
    const result = client.submitReviewed(ticket, () => new Promise((resolve) => { finish = resolve; }));
    doc.type("P", "typed during merge");
    client.noteEdit("P", "save-block", false);
    finish(page("P", "merged"));
    expect(await result).toBe("review-stale");
    expect([host.count("submit"), doc.text("P")]).toEqual([0, "typed during merge"]);
  });
});

describe("R7 cut grants carry the host page and session", () => {
  it("checks instance, file, key, session, tombstone, conflict and outstanding work", async () => {
    const ctx = await setup();
    const { doc, client } = ctx;
    await opened(ctx, "P", 3);
    const facts = doc.facts("P")!;
    const [source] = client.stampCutSources([{ name: "P", kind: "page", path: facts.path!, generation: facts.instance }]);
    expect([client.cutSourcesUsable([source]), client.cutSourcesRetired([source])]).toEqual([true, true]);
    client.noteEdit("P", "delete-blocks", false);
    expect([client.cutSourcesUsable([source]), client.cutSourcesRetired([source])]).toEqual([true, false]);
    expect(client.cutSourcesUsable([{ ...source, session: 99 }])).toBe(false);
    expect(client.cutSourcesUsable([{ ...source, key: "pages/other.md" }])).toBe(false);
    expect(client.cutSourcesUsable([{ ...source, generation: facts.instance + 1 }])).toBe(false);
    doc.tombs.add("P");
    expect(client.cutSourcesUsable([source])).toBe(false);
  });

  it("a source cut before its first save or Open is pinned by its instance", async () => {
    const ctx = await setup();
    const { host, doc, client } = ctx;
    doc.load(page("P", "a", null), null, { kind: "no-file" });
    client.acquire("P");
    const [source] = client.stampCutSources([{ name: "P", kind: "page", generation: doc.facts("P")!.instance }]);
    expect(source.key).toBe(null);
    await tick();
    client.receive(mail(7, KEY, mailPage(1, { kind: "no-file" }, { disk: { kind: "no-file" } }), applied(host.last("open").id, 1)));
    doc.pages.get("P")!.path = KEY;
    expect(client.cutSourcesRetired([source])).toBe(true);
  });
});

describe("R4 createPage", () => {
  it("creates only over no file, submits due now, and returns at publication", async () => {
    const ctx = await setup();
    const { host, client } = ctx;
    const created = client.createPage("New", page("New", "hello"));
    await tick();
    client.receive(mail(7, "pages/New.md", mailPage(1, { kind: "no-file" }, { disk: { kind: "no-file" } }), applied(host.last("open").id, 1)));
    await tick();
    const submit = host.last("submit");
    expect([submit.version, submit.kinds, textOf(submit.dto)]).toEqual([1, ["create-page"], "hello"]);
    client.receive(mail(7, "pages/New.md", mailPage(2, { kind: "unchanged" }), took(submit.id, 2)));
    expect(await created).toEqual({ kind: "created", key: "pages/New.md", version: 2 });
    expect(host.last("wait").needs).toEqual([{ key: "pages/New.md", version: 2 }]);
  });

  it("an existing file is 'exists' with nothing written; a changed expected file is sent stale", async () => {
    const ctx = await setup();
    const { host, client } = ctx;
    const exists = client.createPage("Old", page("Old", "x"));
    await tick();
    client.receive(mail(7, "pages/Old.md", mailPage(1, { kind: "page", dto: page("Old", "disk", "r1") }, { disk: { kind: "file", rev: "r1" } }),
      applied(host.last("open").id, 1)));
    expect(await exists).toEqual({ kind: "exists" });
    expect(host.count("submit")).toBe(0);

    const favorites = client.createPage("Fav", page("Fav", "arranged"), { expected: { kind: "file", rev: "r0" } });
    await tick();
    client.receive(mail(7, "pages/Fav.md", mailPage(4, { kind: "page", dto: page("Fav", "disk", "r1") }, { disk: { kind: "file", rev: "r1" } }),
      applied(host.last("open").id, 4)));
    await tick();
    expect(host.last("submit").version).toBe(0);
    client.receive(mail(7, "pages/Fav.md", mailPage(5, { kind: "unchanged" }, { conflict: true }), took(host.last("submit").id, 5)));
    expect(await favorites).toEqual({ kind: "created", key: "pages/Fav.md", version: 5 });
  });
});

describe("S8 session rebind", () => {
  it("drops the old session's mail, replays early new-session mail, and reopens acquired pages", async () => {
    const ctx = await setup();
    const { host, doc, client } = ctx;
    await opened(ctx, "P", 3);
    host.session = 8;
    host.nextId = 50;
    const rebinding = client.rebind();
    client.receive(mail(7, KEY, mailPage(9, { kind: "page", dto: page("P", "old host") })));
    await rebinding;
    const open = host.last("open");
    expect([client.session, open.id]).toEqual([8, 50]);
    client.receive(mail(7, KEY, mailPage(9, { kind: "page", dto: page("P", "late old answer") }), applied(open.id, 9)));
    expect(client.isOpen("P")).toBe(false);
    await tick();
    client.receive(mail(8, KEY, mailPage(2, { kind: "page", dto: page("P", "restored", "r2") }, { disk: { kind: "file", rev: "r2" } }),
      applied(open.id, 2)));
    expect([client.isOpen("P"), client.versionOf("P"), doc.text("P")]).toEqual([true, 2, "restored"]);
  });

  it("mail for the new session that arrives before the reload reply is kept", async () => {
    const ctx = await setup();
    const { host, client } = ctx;
    await opened(ctx, "P", 3);
    host.session = 8;
    host.nextId = 50;
    const rebinding = client.rebind();
    client.receive(mail(8, KEY, mailPage(2, { kind: "page", dto: page("P", "restored", "r2") }, { disk: { kind: "file", rev: "r2" } }),
      applied(50, 2)));
    await rebinding;
    await tick();
    expect([client.isOpen("P"), client.versionOf("P")]).toEqual([true, 2]);
  });
});

describe("close and move edges", () => {
  it("mail after a close while the host keeps the page leaves a closed view (no close loop)", async () => {
    const ctx = await setup();
    const { host, client } = ctx;
    const release = await opened(ctx, "P", 3);
    release();
    await tick();
    client.receive(mail(7, KEY, mailPage(3, { kind: "unchanged" }, { risk: true })));
    await tick();
    expect([client.isOpen("P"), host.count("close"), client.names()]).toEqual([false, 1, []]);
  });

  it("a close is never answered (model wClose): admitted, the view ends, and later input reopens and is sent", async () => {
    const ctx = await setup();
    const { host, doc, client } = ctx;
    const release = await opened(ctx, "P", 3, "a", "r3");
    release();
    await tick();
    expect(host.count("close")).toBe(1);
    // No mail follows a close: the real host only unsubscribes (upClose).
    expect([client.isOpen("P"), client.busy("P"), client.names()]).toEqual([false, false, []]);
    const editing = client.acquire("P");
    doc.type("P", "later");
    client.noteEdit("P", "save-block", false);
    await tick();
    expect(host.count("open")).toBe(2);
    client.receive(mail(7, KEY, mailPage(3, { kind: "page", dto: page("P", "a", "r3") }, { disk: { kind: "file", rev: "r3" } }),
      applied(host.last("open").id, 3)));
    client.sendNow("P");
    await tick();
    expect(textOf(host.last("submit").dto)).toBe("later");
    editing();
  });

  it("keeps a page open while a version the host took from it is unpublished (J6); a true wait lets go", async () => {
    const ctx = await setup();
    const { host, doc, client } = ctx;
    let publish: (published: boolean | null) => void = () => {};
    host.onWait = () => new Promise<boolean | null>((resolve) => { publish = resolve; });
    const release = await opened(ctx, "P", 3);
    doc.type("P", "ab");
    client.noteEdit("P", "save-block", false);
    client.sendNow("P");
    await tick();
    client.receive(mail(7, KEY, mailPage(4, { kind: "unchanged" }), took(host.last("submit").id, 4)));
    release();
    await tick();
    expect([client.isOpen("P"), host.count("close"), host.last("wait").needs]).toEqual([true, 0, [{ key: KEY, version: 4 }]]);
    // A bounded wait passing is not publication: the page stays open.
    publish(null);
    await tick();
    expect([client.isOpen("P"), host.count("close")]).toEqual([true, 0]);
    publish(true);
    await tick();
    expect([host.count("close"), client.names()]).toEqual([1, []]);
  });

  it("a page whose taken version is unpublished lets go when the session ends (J6)", async () => {
    const ctx = await setup();
    const { host, doc, client } = ctx;
    host.onWait = () => new Promise<boolean | null>(() => {});
    const release = await opened(ctx, "P", 3);
    doc.type("P", "ab");
    client.noteEdit("P", "save-block", false);
    client.sendNow("P");
    await tick();
    client.receive(mail(7, KEY, mailPage(4, { kind: "unchanged" }), took(host.last("submit").id, 4)));
    release();
    client.forget("P");
    await tick();
    expect([client.isOpen("P"), host.count("close")]).toEqual([true, 0]);
    host.session = 8;
    await client.rebind();
    expect([client.names(), host.count("close")]).toEqual([[], 0]);
  });

  it("a move the host does not admit restores both endpoints, clean, so no half is ever sent", async () => {
    const ctx = await setup();
    const { host, doc, client } = ctx;
    await opened(ctx, "A", 3, "x");
    await opened(ctx, "B", 4, "y");
    host.refuse = { reason: "not-admitted" };
    const answer = await client.move("A", "B", page("A", ""), page("B", "y\nx"), ["move-blocks"]);
    expect([answer.took, doc.text("A"), doc.text("B"), client.isDirty("A"), client.isDirty("B")])
      .toEqual([false, "x", "y", false, false]);
  });
});
