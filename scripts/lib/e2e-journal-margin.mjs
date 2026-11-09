// GH #891: native WebKitGTK journal margins and synthetic iOS tap-to-edit.
// This checks the shared JS entry and saved bytes, not physical WKWebView input.
import fs from "node:fs";
import { openJournals } from "./e2e-navigation.mjs";
import { waitForFileText } from "../e2e-file-poll.mjs";

const dayKey = date => `${date.getFullYear()}_${String(date.getMonth() + 1).padStart(2, "0")}_${String(date.getDate()).padStart(2, "0")}`;
const ROOT = "A journal comment";
const REPLY = "A prior journal reply";
const AUTHOR = "tine.author:: Ada Test  \t";
const FUTURE = "TiNe.future:: opaque value  \t";

export function seedJournalMargin(graph, pageName) {
  const today = new Date(); const yesterday = new Date(today); yesterday.setDate(today.getDate() - 1);
  const file = `${graph}/journals/${dayKey(today)}.md`;
  const previous = `${graph}/journals/${dayKey(yesterday)}.md`;
  // More replies than prose: the next day must start below the whole thread.
  const extra = Array.from({ length: 5 }, (_, i) => `\t\t- Additional journal response ${i + 1}\n`).join("");
  fs.writeFileSync(file, `- open [[${pageName}]]\n- A journal passage for margin review.\n\t- ${ROOT}\n\t  tine.quote:: journal passage  \t\n\t  ${AUTHOR}\n\t  ${FUTURE}\n\t\t- ${REPLY}\n\t\t  ${AUTHOR}\n\t\t  ${FUTURE}\n${extra}`);
  fs.writeFileSync(previous, "- Earlier journal passage.\n\t- Earlier journal comment\n\t  tine.quote:: journal passage\n");
  return { file, previous };
}

function journalGeometry() {
  return [...document.querySelectorAll(".page-section")].filter(section => section.querySelector(".margin-column")).map(section => {
    const rect = section.getBoundingClientRect();
    const threads = [...section.querySelectorAll(".margin-thread")].map(thread => {
      const r = thread.getBoundingClientRect();
      return { left: r.left, top: r.top, bottom: r.bottom, text: thread.textContent };
    });
    const outline = section.querySelector(".page-blocks").getBoundingClientRect();
    return { top: rect.top, bottom: rect.bottom, outlineRight: outline.right, threads,
      inline: !!section.querySelector(".page-blocks .comment-in-thread") };
  });
}

function syntheticTap(text) {
  const target = [...document.querySelectorAll(".comment-in-thread > .block-main .block-content")].find(el => el.textContent.startsWith(text));
  if (!target) throw new Error(`Missing comment body ${text}`);
  target.scrollIntoView({ block: "center" });
  const rect = target.getBoundingClientRect();
  const point = { identifier: 1, target, clientX: rect.left + 8, clientY: rect.top + rect.height / 2 };
  let real = true;
  for (const type of ["touchstart", "touchend"]) {
    let event;
    try {
      const touch = new Touch(point);
      const live = type === "touchend" ? [] : [touch];
      event = new TouchEvent(type, { bubbles: true, cancelable: true, touches: live, targetTouches: live, changedTouches: [touch] });
    } catch {
      real = false;
      event = new Event(type, { bubbles: true, cancelable: true });
      Object.defineProperties(event, { touches: { value: type === "touchend" ? [] : [point] }, changedTouches: { value: [point] } });
    }
    target.dispatchEvent(event);
  }
  // WebDriver's injected TouchEvents do not synthesize compatibility mouse
  // events. Deliver the tap's mousedown explicitly to the real edit handler.
  target.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0, clientX: point.clientX, clientY: point.clientY }));
  document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
  return { events: real ? "TouchEvent" : "Event with touch fields", compatibilityMouse: "explicit", gesturePlatform: window.__TINE_E2E_TOUCH_GESTURES__ };
}

export async function checkJournalMargin(browser, fixture, shot, proof) {
  await browser.setWindowSize(1600, 1000);
  await openJournals(browser);
  await browser.waitUntil(async () => (await browser.$$(".margin-column")).length === 2, { timeout: 10_000, timeoutMsg: "Journal days did not get their own wide margin columns" });
  await browser.waitUntil(async () => {
    const days = await browser.execute(journalGeometry);
    return days.length === 2 && days.every(day => !day.inline && day.threads.length === 1
      && day.threads[0].left >= day.outlineRight && day.bottom >= day.threads[0].bottom - 2)
      && days[1].top >= days[0].threads[0].bottom - 2;
  }, { timeout: 5000, timeoutMsg: "Journal margins overlap a later day or are inside the prose" });
  proof.steps.push({ outcome: "two journal days own separate margins; long first thread leaves room for the next day", geometry: await browser.execute(journalGeometry) });
  await shot("06-journal-per-day-margins");

  for (const [index, body] of [ROOT, REPLY].entries()) {
    const tap = await browser.execute(syntheticTap, body);
    if (tap.gesturePlatform !== "ios") throw new Error("The iOS touch-gesture injection was not enabled");
    const editor = await browser.$("textarea.block-editor");
    await editor.waitForExist({ timeout: 5000 });
    if (await editor.getValue() !== body) throw new Error(`Comment editor exposed metadata after tap: ${await editor.getValue()}`);
    proof.steps.push({ outcome: `${body}: iOS gesture injection edits only words`, ...tap });
    await shot(`07-${index}-ios-comment-editor`);
    await browser.execute(() => { const ta = document.querySelector("textarea.block-editor"); ta.focus(); ta.setSelectionRange(ta.value.length, ta.value.length); });
    await browser.keys(" revised"); await browser.keys(["Escape"]);
    const saved = await waitForFileText(fixture.file, text => text.includes(`${body} revised`), { timeoutMs: 20_000 });
    if (saved.split(AUTHOR).length !== 3 || saved.split(FUTURE).length !== 3 || !saved.includes("tine.quote:: journal passage  \t\n")) {
      throw new Error("A journal comment edit lost or changed hidden property bytes");
    }
  }
  const saved = fs.readFileSync(fixture.file, "utf8");
  await browser.setWindowSize(1000, 820);
  // D-20261108-MC7/MC8: no counted markers; each day's highlight opens its thread on hover.
  await browser.waitUntil(async () => (await browser.$$(".margin-column")).length === 0
    && (await browser.$$(".page-blocks .comment-quote-anchor")).length === 2
    && (await browser.$$(".page-blocks .margin-marker")).length === 0, { timeout: 5000 });
  await browser.$(".page-blocks .comment-quote-anchor").moveTo();
  await browser.$(".comment-thread-popup .comment-card").waitForExist({ timeout: 5000 });
  await shot("08-journal-narrow-popover");
  await browser.$(".comment-popup-close").click();
  if (fs.readFileSync(fixture.file, "utf8") !== saved) throw new Error("Changing width rewrote the journal");
  await browser.setWindowSize(1600, 1000);
  await browser.refresh();
  await browser.waitUntil(async () => (await browser.$$(".margin-column")).length === 2, { timeout: 20_000 });
  await browser.waitUntil(() => browser.execute((root, reply) => {
    const bodies = [...document.querySelectorAll(".margin-column .block-content")].map(el => el.textContent);
    return bodies.includes(`${root} revised`) && bodies.includes(`${reply} revised`);
  }, ROOT, REPLY), { timeout: 5000, timeoutMsg: "Reload lost a journal comment's edited words" });
  if (fs.readFileSync(fixture.file, "utf8") !== saved) throw new Error("Reload rewrote the journal");
  await shot("09-journal-reloaded");
  proof.steps.push("journal comment and reply edits keep exact mixed-case tine.* rows and author name; narrow/wide transition and reload preserve bytes");
}
