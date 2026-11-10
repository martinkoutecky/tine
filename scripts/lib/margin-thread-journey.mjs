// GH #891 / #110: routed-page proof for anchoring, flat threads and wide layout.
import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { openPageByName } from "./e2e-navigation.mjs";
import { waitForFileText } from "../e2e-file-poll.mjs";
import { saveJourneyScreenshot } from "./e2e-screenshot.mjs";

const PAGE = "Margin Thread Order";
const IDS = {
  parent: "89100000-0000-4000-8000-000000000001",
  late: "89100000-0000-4000-8000-000000000002",
  early: "89100000-0000-4000-8000-000000000003",
  tie: "89100000-0000-4000-8000-000000000004",
  reply: "89100000-0000-4000-8000-000000000005",
  deep: "89100000-0000-4000-8000-000000000006",
};
const repeated = "Duis aute irure dolor in reprehenderit in voluptate laborum.";
const RAW = [
  "- Opening passage. " + Array(4).fill(repeated).join(" "),
  "  id:: " + IDS.parent,
  "  - Late response",
  "    id:: " + IDS.late,
  "    tine.quote:: laborum.",
  "    tine.quote-prefix:: in reprehenderit in voluptate",
  "    tine.quote-suffix:: ",
  "  - Early response",
  "    id:: " + IDS.early,
  "    tine.quote:: Opening passage.",
  "    - First reply",
  "      id:: " + IDS.reply,
  "      - Deep reply",
  "        id:: " + IDS.deep,
  "        tine.quote:: First",
  "  - Equal anchor",
  "    id:: " + IDS.tie,
  "    tine.quote:: Opening passage.",
  "- word repeated word",
  "  - Ambiguous response",
  "    tine.quote:: word",
  "- Replacement text",
  "  - Missing response",
  "    tine.quote:: vanished",
  "",
].join("\n");

export function seedMarginThreads(graph) {
  fs.writeFileSync(path.join(graph, "pages", PAGE + ".md"), RAW);
}

export async function checkMarginThreads(browser, graph, artifact) {
  const file = path.join(graph, "pages", PAGE + ".md");
  const shots = path.join(artifact, "shots");
  fs.mkdirSync(shots, { recursive: true });
  const assert = (ok, message, detail) => {
    if (!ok) throw Object.assign(new Error(message + (detail ? ": " + JSON.stringify(detail) : "")), { classification: "product" });
  };
  const proof = { page: PAGE, ids: IDS, testedCommit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(), steps: [] };
  try {
    await openPageByName(browser, PAGE);
    await browser.$(".margin-column .margin-thread").waitForExist({ timeout: 10000 });
    await browser.waitUntil(() => browser.execute(() => document.querySelectorAll(".margin-thread").length === 4), { timeout: 10000 });
    if (process.env.E2E_MARGIN_BASE_CSS === "1") {
      const css = execFileSync("git", ["show", "f391144efb930724415efa2fc380b525e38d6d88:src/styles/comments.css"], { encoding: "utf8" });
      await browser.execute(text => {
        const style = document.createElement("style");
        style.textContent = text;
        document.head.appendChild(style);
      }, css);
      proof.reconstruction = "assigned-base comment stylesheet over candidate app";
    }
    await saveJourneyScreenshot(browser, path.join(shots, "threads-initial.png"));
    const initial = await browser.execute((ids) => {
      const row = id => document.querySelector('.ls-block[data-block-ref="' + id + '"]');
      const thread = id => row(id).closest(".margin-thread");
      const p = row(ids.parent);
      const content = p.querySelector(":scope > .block-main .block-content");
      const mark = content.querySelector('[data-comment-id="' + thread(ids.late).dataset.threadId + '"]');
      const before = document.createRange();
      before.selectNodeContents(content);
      if (mark) before.setEndBefore(mark);
      const bodyX = id => row(id).querySelector(":scope > .block-main .block-content").getBoundingClientRect().left;
      return {
        order: [...document.querySelectorAll(".margin-thread")].slice(0, 2).flatMap(el => [...el.querySelectorAll(".comment-card")].map(row => row.dataset.blockRef)),
        beforeLast: mark ? before.toString() : null, parentText: content.textContent,
        xs: [ids.early, ids.reply, ids.deep].map(bodyX),
        count: p.querySelector("button.margin-marker")?.textContent ?? null,
        ambiguous: [...document.querySelectorAll(".comment-quote")].some(el => el.textContent.includes("quoted text ambiguous")),
        missing: [...document.querySelectorAll(".comment-quote")].some(el => el.textContent.includes("quoted text changed")),
      };
    }, IDS);
    proof.initial = initial;
    assert(JSON.stringify(initial.order) === JSON.stringify([IDS.early, IDS.tie, IDS.late]), "threads do not follow stable passage order", initial);
    assert(initial.beforeLast !== null && initial.beforeLast.split("laborum.").length === 4, "the last laborum did not receive the highlight", initial);
    assert(Math.max(...initial.xs) - Math.min(...initial.xs) <= 2, "responses still draw at different nesting levels", initial);
    assert(initial.count === null, "expanded wide block duplicated its visible threads with a badge", initial);
    assert(initial.ambiguous && initial.missing, "uncertain quotes were silently anchored or dropped", initial);
    assert(fs.readFileSync(file, "utf8") === RAW, "viewing hand-nested replies rewrote their file");
    proof.steps.push("last laborum, stable passage order, flat hand nesting, thread-only count and uncertain quotes");

    // Focus the later thread's draft. It must pin to its own passage even when
    // preceding cards would otherwise displace it; neighbors cannot overlap.
    await browser.execute((ids) => {
      const root = document.querySelector('.ls-block[data-block-ref="' + ids.late + '"]');
      root.closest(".margin-thread").dispatchEvent(new MouseEvent("mouseenter"));
      root.querySelector(".comment-reply-draft").focus();
    }, IDS);
    await browser.waitUntil(() => browser.execute((ids) => {
      const root = document.querySelector('.ls-block[data-block-ref="' + ids.late + '"]');
      const thread = root.closest(".margin-thread");
      const mark = document.querySelector('.page-blocks [data-comment-id="' + thread.dataset.threadId + '"]');
      return Math.abs(thread.getBoundingClientRect().top - mark.getBoundingClientRect().top) <= 2;
    }, IDS), { timeout: 5000, timeoutMsg: "focused thread moved away from its anchor" });
    await saveJourneyScreenshot(browser, path.join(shots, "threads-focused.png"));
    const cards = await browser.execute(() => [...document.querySelectorAll(".margin-thread")].map(el => {
      const r = el.getBoundingClientRect(); return { top: r.top, bottom: r.bottom };
    }));
    for (let i = 1; i < cards.length; i++) assert(cards[i].top >= cards[i - 1].bottom, "focused cards overlap", cards);
    proof.steps.push("focused card pinned to passage; other cards cascade without overlap");
    await browser.execute(() => document.activeElement?.blur());
    await browser.waitUntil(() => browser.execute((ids) => {
      const deep = document.querySelector('.ls-block[data-block-ref="' + ids.deep + '"] > .block-main');
      return deep.getBoundingClientRect().top > document.querySelector(".main-content").getBoundingClientRect().top;
    }, IDS), { timeout: 5000, timeoutMsg: "leaving the draft did not restore the earlier thread" });

    // Comment on a deeply nested response through its editor. The quote remains
    // compact, but the newly persisted response must be a direct root child.
    await browser.$('.ls-block[data-block-ref="' + IDS.deep + '"] > .block-main .block-content').click();
    await browser.$(".margin-column textarea.block-editor").waitForExist({ timeout: 5000 });
    await browser.execute(() => {
      const ta = document.querySelector(".margin-column textarea.block-editor");
      ta.focus(); ta.setSelectionRange(0, 4);
    });
    await browser.keys(["Control", "r"]);
    await browser.waitUntil(() => browser.execute(() =>
      !!document.querySelector(".comment-name-prompt") || document.querySelector(".margin-column textarea.block-editor")?.value === ""),
    { timeout: 5000, timeoutMsg: "in-thread Comment opened neither a name prompt nor an empty editor" });
    if (await browser.$(".comment-name-prompt").isExisting()) await browser.$(".comment-name-prompt .export-btn-secondary").click();
    await browser.waitUntil(() => browser.execute(() => {
      const ta = document.querySelector(".margin-column textarea.block-editor");
      return ta?.value === "" && document.activeElement === ta;
    }), { timeout: 5000, timeoutMsg: "the new quoted reply did not receive editor focus" });
    // WebKitWebDriver drops adjacent repeated keys ("oo" becomes "o").
    const body = "New flat reply";
    await browser.keys(body);
    await browser.keys(["Escape"]);
    const saved = await waitForFileText(file, text => text.includes(body), { timeoutMs: 20000 });
    const indentOf = text => saved.split("\n").find(line => line.trim() === "- " + text)?.match(/^[ \t]*/)[0];
    assert(indentOf(body) === indentOf("First reply"), "in-thread Comment nested its reply under the selected response", saved);
    assert(saved.includes("      - Deep reply\n        id:: " + IDS.deep), "existing nested reply was moved", saved);
    await saveJourneyScreenshot(browser, path.join(shots, "threads-new-reply.png"));
    proof.steps.push("Ctrl+R in a deep reply saves a root child and leaves existing nesting intact");
    proof.persistedBytes = { before: Buffer.byteLength(RAW), after: Buffer.byteLength(saved), replyDelta: Buffer.byteLength(saved) - Buffer.byteLength(RAW) };
    proof.result = "pass";
    console.log("PASS: GH #891 / #110 margin thread journey");
  } catch (error) {
    proof.result = "fail";
    proof.failure = { message: String(error), classification: error.classification ?? "ambiguous", expected: "stable passage order, flat replies, root-child writes, pinned focus", observed: String(error) };
    await saveJourneyScreenshot(browser, path.join(shots, "threads-failure.png")).catch(() => {});
    throw error;
  } finally {
    fs.writeFileSync(path.join(artifact, "thread-proof.json"), JSON.stringify(proof, null, 2) + "\n");
  }
}
