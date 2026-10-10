import { describe, expect, it } from "vitest";
import type { PageDto } from "../../types";
import { autoAnswer, mail, mailPage, page, setup, textOf } from "./fakes.test.support";
import { runSequence, type SequenceStep } from "./planner";

async function carry() {
  const ctx = await setup();
  const counter = autoAnswer(ctx);
  ctx.doc.load(page("A", "a", "rA"));
  ctx.doc.load(page("C", "c", "rC"));
  ctx.doc.load(page("B", "b", "rB"));
  return { ...ctx, counter };
}

/** Carry A's and C's text into B: step i's receiver holds the drained receiver
 * plus sources 1..i only. */
function plan(drained: ReadonlyMap<string, PageDto>): SequenceStep[] {
  let receiver = textOf(drained.get("B"));
  return ["A", "C"].map((source) => {
    receiver = `${receiver}\n${textOf(drained.get(source))}`;
    return { source, receiver: "B", sourceDto: page(source, ""), receiverDto: page("B", receiver) };
  });
}

describe("§8 planned sequences", () => {
  it("drains every endpoint, then runs each move on the versions the previous step left", async () => {
    const { host, doc, client } = await carry();
    doc.type("A", "a typed");
    client.noteEdit("A", "save-block", false);
    const outcome = await runSequence(client, ["A", "C", "B"], plan, ["move-blocks"]);
    expect(outcome).toEqual({ ok: true, completed: 2 });
    const submit = host.last("submit");
    const moves = host.calls.filter((call) => call.cmd === "move");
    expect(textOf(submit.dto)).toBe("a typed");
    expect(moves.map((call) => textOf(call.receiver[1]))).toEqual(["b\na typed", "b\na typed\nc"]);
    // Step 2's receiver version is the one step 1's answer left.
    expect(moves[1].receiver[2]).toBeGreaterThan(moves[0].receiver[2]);
    expect(doc.text("B")).toBe("b\na typed\nc");
    expect(client.isFrozen("A")).toBe(false);
  });

  it("stops at a refused step: completed steps stand, the receiver takes the host's state", async () => {
    const { host, doc, client } = await carry();
    const respond = host.respond!;
    host.respond = (call) => {
      if (call.cmd !== "move" || host.count("move") === 1) return respond(call);
      return [call.source[0], call.receiver[0]].map((key) => mail(host.session, key,
        mailPage(150, { kind: "page", dto: page(key === "pages/B.md" ? "B" : "C", key === "pages/B.md" ? "b\na" : "c") }),
        { id: call.id, version: 150, took: false, outcome: { kind: "refused", reason: "stale" } }));
    };
    const outcome = await runSequence(client, ["A", "C", "B"], plan, ["move-blocks"]);
    expect(outcome).toEqual({ ok: false, completed: 1, reason: "refused", pages: ["C"], refusal: "stale" });
    expect([doc.text("A"), doc.text("C"), doc.text("B")]).toEqual(["", "c", "b\na"]);
    expect(client.isDirty("B")).toBe(false);
  });

  it("moves nothing when a component cannot commit its input, or an endpoint stays conflicted", async () => {
    const { host, client } = await carry();
    client.acquire("B", { pin: true, commit: () => false });
    expect(await runSequence(client, ["A", "C", "B"], plan, ["move-blocks"]))
      .toEqual({ ok: false, completed: 0, reason: "commit-failed", pages: ["B"] });

    const other = await carry();
    other.client.acquire("C");
    await new Promise((resolve) => setTimeout(resolve, 0));
    other.client.receive(mail(7, "pages/C.md", mailPage(160, { kind: "unchanged" }, { conflict: true })));
    expect(await runSequence(other.client, ["A", "C", "B"], plan, ["move-blocks"]))
      .toEqual({ ok: false, completed: 0, reason: "not-drained", pages: ["C"] });
    expect(host.count("move") + other.host.count("move")).toBe(0);
  });
});
