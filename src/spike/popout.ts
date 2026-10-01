// Throwaway spike: every function and Solid owner here runs in the main JS heap.
import { createComponent } from "solid-js";
import { clearDelegatedEvents, delegateEvents, DelegatedEvents, render } from "solid-js/web";
import { invoke } from "@tauri-apps/api/core";
import { PageView } from "../components/Page";
import { SurfaceContext } from "../components/Block";
import { PaneContext } from "../paneContext";
import { mainPaneRouter, openPage } from "../router";
import { graphMeta } from "../graphSession";
import { installKeybindings } from "../keybindings";
import { endEdit } from "../editorController";

class CheckFailure extends Error { constructor(readonly detail: unknown) { super(JSON.stringify(detail)); } }
type Check = { status: "pass" | "fail" | "error"; detail: unknown };
type Realm = Window & typeof globalThis;
const native = <T = unknown>(action: string, value?: unknown) => invoke<T>("spike_mw", { action, value: value ?? null });
const sleep = (ms: number, realm: Window = window) => new Promise<void>(r => realm.setTimeout(r, ms));
async function until(test: () => boolean | Promise<boolean>, ms = 5000, realm: Window = window) {
  const start = performance.now();
  while (performance.now() - start < ms) { if (await test()) return; await sleep(50, realm); }
  throw new Error(`condition timed out after ${ms}ms`);
}

const log = (message: string, detail?: unknown) => native("log", { message, detail });
export async function openPopout() {
  await log("window.open calling", { userActivation: navigator.userActivation?.isActive });
  const popup = window.open("about:blank", "spike-popup", "popup,width=780,height=800,left=1200,top=50") as Realm | null;
  await log("window.open returned", { nonNull: !!popup, closed: popup?.closed });
  if (!popup) throw new Error("window.open returned null");
  await until(() => !!popup.document.body);
  await log("popup document ready");
  const doc = popup.document;
  doc.title = "Tine Spike Popup";
  const mirror = () => {
    doc.head.querySelectorAll("link[rel=stylesheet], style").forEach(n => n.remove());
    document.head.querySelectorAll<HTMLLinkElement | HTMLStyleElement>("link[rel=stylesheet], style").forEach(n => {
      const copy = n.cloneNode(true) as HTMLLinkElement | HTMLStyleElement;
      if (n instanceof HTMLLinkElement) (copy as HTMLLinkElement).href = n.href;
      doc.head.append(copy);
    });
    for (const name of doc.documentElement.getAttributeNames()) doc.documentElement.removeAttribute(name);
    for (const attr of document.documentElement.attributes) doc.documentElement.setAttribute(attr.name, attr.value);
    doc.body.className = document.body.className;
  };
  mirror();
  const styles = new MutationObserver(mirror);
  styles.observe(document.head, { childList: true, subtree: true, characterData: true, attributes: true });
  styles.observe(document.documentElement, { attributes: true });
  styles.observe(document.body, { attributes: true });
  delegateEvents([...DelegatedEvents], doc);
  const root = doc.createElement("div");
  root.className = "main-content";
  root.style.cssText = "height:100vh;overflow:auto;padding:24px";
  doc.body.append(root);
  const dispose = render(() => createComponent(PaneContext.Provider, {
    value: { paneId: "spike-popup", router: mainPaneRouter },
    get children() { return createComponent(SurfaceContext.Provider, {
      value: "pane:spike-popup", get children() { return createComponent(PageView, {}); }
    }); }
  }), root);
  await log("popup Solid rendered", { rows: rows(doc).length });
  const keys = installKeybindings({}, popup);
  let disposed = false;
  const close = () => {
    if (disposed) return;
    disposed = true;
    endEdit("page-navigation");
    keys(); dispose(); styles.disconnect(); clearDelegatedEvents(doc);
  };
  popup.addEventListener("pagehide", close);
  popup.addEventListener("beforeunload", close);
  return { popup, dispose: close };
}

function rows(doc: Document) { return Array.from(doc.querySelectorAll<HTMLElement>(".ls-block[data-block-id]")); }
function texts(doc: Document) { return rows(doc).map(row => row.querySelector<HTMLTextAreaElement>("textarea.block-editor")?.value ?? row.querySelector(".block-content-wrapper")?.textContent?.trim()); }
async function edit(realm: Realm, index: number) {
  endEdit("page-navigation");
  await sleep(80, realm);
  const target = rows(realm.document)[index]?.querySelector<HTMLElement>(".block-content-wrapper");
  if (!target) throw new Error(`missing block ${index}`);
  target.dispatchEvent(new realm.MouseEvent("mousedown", { bubbles: true, button: 0 }));
  realm.document.dispatchEvent(new realm.MouseEvent("mouseup", { bubbles: true, button: 0 }));
  await until(() => !!realm.document.querySelector("textarea.block-editor"), 5000, realm);
  return realm.document.querySelector<HTMLTextAreaElement>("textarea.block-editor")!;
}
function input(realm: Realm, editor: HTMLTextAreaElement, text: string) {
  editor.value = text; editor.setSelectionRange(text.length, text.length);
  editor.dispatchEvent(new realm.InputEvent("input", { bubbles: true, inputType: "insertText", data: text }));
}
function key(realm: Realm, editor: HTMLTextAreaElement, name: string, extra = {}) {
  return !editor.dispatchEvent(new realm.KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: name, ...extra }));
}

export async function runSpike() {
  let config: { graph: string; oskeys: boolean };
  try { config = await native("config"); } catch (error) {
    const result = Object.fromEntries(["C1", "C2", "C3", "C4", "C5", "C6", "C7", "C8", "C9", "R"].map(id => [id, { status: "error", detail: String(error) }]));
    await native("finish", result);
    return;
  }
  const result: Record<string, Check> = {};
  const check = async (id: string, fn: () => Promise<unknown>) => {
    try { result[id] = { status: "pass", detail: await fn() }; }
    catch (error) { result[id] = { status: error instanceof CheckFailure ? "fail" : "error", detail: error instanceof CheckFailure ? error.detail : String(error) }; }
    await log(`check ${id}`, result[id]);
  };
  const assert = (ok: boolean, detail: unknown) => { if (!ok) throw new CheckFailure(detail); return detail; };
  let aux: Awaited<ReturnType<typeof openPopout>> | undefined;
  let popup: Realm;
  try {
    await log("self-test loaded", { config, graph: graphMeta(), href: location.href });
    await until(() => graphMeta()?.root === config.graph, 30000);
    await log("graph ready", graphMeta());
    openPage("Spike");
    await until(() => rows(document).length === 20, 15000);
    await log("main page loaded", { rows: rows(document).length });
    await check("C1", async () => {
      aux = await openPopout(); popup = aux.popup;
      const windows = await native<Array<{label: string; visible: boolean}>>("windows");
      return assert(popup.document !== document && windows.filter(w => w.label === "main" || w.label === "spike-popup").length === 2, { windows, distinctDocument: popup.document !== document, opener: popup.opener === window, note: "capture is the pre-existing hidden third webview" });
    });
    if (!aux) throw new Error("No scriptable native popup; later checks cannot run");
    popup = aux.popup;
    await check("C2", async () => {
      await until(() => rows(popup.document).length === 20, 10000);
      await sleep(1000);
      return assert(JSON.stringify(texts(document)) === JSON.stringify(texts(popup.document)), { main: texts(document), popup: texts(popup.document) });
    });
    await check("C3", async () => {
      const editor = await edit(popup, 0);
      const before = rows(popup.document).length;
      const prevented = key(popup, editor, "Enter");
      await until(() => rows(popup.document).length === before + 1, 5000, popup);
      return assert(prevented, { mouseEnteredEditor: true, enterPrevented: prevented, before, after: rows(popup.document).length });
    });
    await check("C4", async () => {
      input(popup, await edit(popup, 2), "popup shared model");
      await until(() => texts(document).includes("popup shared model"));
      input(window, await edit(window, 3), "main shared model");
      await until(() => texts(popup.document).includes("main shared model"));
      return { popupToMain: true, mainToPopup: true, noReload: true };
    });
    await check("C5", async () => {
      const start = performance.now();
      input(popup, await edit(popup, 2), "popup saved on disk");
      await until(async () => (await native<string>("read")).includes("popup saved on disk"), 5000, popup);
      return { elapsedMs: performance.now() - start, text: "popup saved on disk" };
    });
    await check("C8", async () => {
      const editor = await edit(window, 4);
      const before = editor.value;
      input(window, editor, before + " main undo marker");
      await sleep(250);
      const popupEditor = await edit(popup, 6);
      const prevented = key(popup, popupEditor, "z", { ctrlKey: !navigator.platform.includes("Mac"), metaKey: navigator.platform.includes("Mac") });
      await until(() => !texts(document).some(t => t?.includes("main undo marker")), 5000, popup);
      return assert(prevented && texts(document).includes(before), { popupUndoShortcut: prevented, mainEditReverted: true, before });
    });
    await native("screenshots");
    await sleep(8000); // external runner captures the two native windows
    await check("C9", async () => {
      const capture = await native<Check>("screenshot-result");
      const windows = await native<Array<{ label: string; decorated: boolean }>>("windows");
      const mainStyle = window.getComputedStyle(rows(document)[0]);
      const popupStyle = popup.getComputedStyle(rows(popup.document)[0]);
      return assert(capture.status === "pass" && windows.some(w => w.label === "spike-popup" && w.decorated)
        && mainStyle.fontFamily === popupStyle.fontFamily && mainStyle.fontSize === popupStyle.fontSize,
        { capture, popupNativeDecorations: true, mainFont: [mainStyle.fontFamily, mainStyle.fontSize], popupFont: [popupStyle.fontFamily, popupStyle.fontSize], note: "External screenshot requires human visual inspection." });
    });
    await check("C6", async () => {
      const counts = { mainTimer: 0, popupTimer: 0, mainRaf: 0, popupRaf: 0, mutation: 0, resize: 0, intersection: 0 };
      let running = true;
      const mainInterval = window.setInterval(() => counts.mainTimer++, 100);
      const popupInterval = popup.setInterval(() => counts.popupTimer++, 100);
      const frame = (realm: Window, field: "mainRaf" | "popupRaf") => realm.requestAnimationFrame(() => { counts[field]++; if (running) frame(realm, field); });
      frame(window, "mainRaf"); frame(popup, "popupRaf");
      const observer = new popup.MutationObserver(() => counts.mutation++);
      observer.observe(popup.document.body, { subtree: true, childList: true, characterData: true });
      const probe = popup.document.createElement("div");
      probe.style.cssText = "position:fixed;top:0;left:0;width:1px;height:1px";
      popup.document.body.append(probe);
      const resize = new popup.ResizeObserver(() => counts.resize++);
      resize.observe(probe);
      const intersection = new popup.IntersectionObserver(() => counts.intersection++);
      intersection.observe(probe);
      try {
        await native("minimize");
        await native("focus-popup");
        await sleep(250, popup);
        for (const field of Object.keys(counts) as Array<keyof typeof counts>) counts[field] = 0;
        const editor = await edit(popup, 7);
        const start = performance.now();
        input(popup, editor, "minimized main popup save");
        let savedMs: number | null = null;
        for (let i = 0; i < 50; i++) {
          probe.style.width = `${i + 1}px`;
          probe.style.left = i % 2 ? "-100px" : "0px";
          probe.textContent = String(i);
          if (savedMs === null && (await native<string>("read")).includes("minimized main popup save")) savedMs = performance.now() - start;
          await sleep(100, popup);
        }
        return assert(savedMs !== null && savedMs <= 5000 && counts.popupTimer > 0 && counts.popupRaf > 0 && counts.mutation > 0 && counts.resize > 0 && counts.intersection > 0 && counts.mainRaf > 0, { counts: { ...counts }, savedMs, intervalMs: performance.now() - start, mainSchedulerThrottled: counts.mainRaf === 0, note: "Five-second probe only; does not settle ten-minute suspension risk." });
      } finally { running = false; window.clearInterval(mainInterval); popup.clearInterval(popupInterval); observer.disconnect(); resize.disconnect(); intersection.disconnect(); probe.remove(); await native("restore"); }
    });
    await check("R", async () => {
      if (!config.oskeys) throw new Error("OS input not requested; rerun with TINE_SPIKE_MW_OSKEYS=1");
      const editor = await edit(popup, 8);
      editor.setSelectionRange(editor.value.length, editor.value.length);
      await native("focus-popup");
      const before = rows(popup.document).length;
      await native("ready");
      await until(async () => (await native<string>("read")).includes("spike123") && rows(popup.document).length > before, 30000, popup);
      return { textReachedDisk: true, enterCreatedBlock: true, before, after: rows(popup.document).length };
    });
    await check("C7", async () => {
      aux!.dispose(); popup.close();
      await sleep(500);
      const jsCloseLeftNativeFrame = (await native<Array<{label: string}>>("windows")).some(w => w.label === "spike-popup");
      if (jsCloseLeftNativeFrame) await native("close-popup");
      await until(async () => !(await native<Array<{label: string}>>("windows")).some(w => w.label === "spike-popup"));
      input(window, await edit(window, 10), "main healthy after popup close");
      await until(async () => (await native<string>("read")).includes("main healthy after popup close"));
      aux = await openPopout(); // final main close is observed on Rust's native event
      return { popupCloseLeavesMainHealthy: true, reopenedForMainClose: true, jsCloseLeftNativeFrame, workaround: jsCloseLeftNativeFrame ? "Native window.close() through Tauri after JS popup.close()" : "none" };
    });
  } catch (error) {
    await log("startup failed", { error: String(error), graph: graphMeta(), config });
    for (const id of ["C1", "C2", "C3", "C4", "C5", "C6", "C7", "C8", "C9", "R"]) result[id] ??= { status: "error", detail: String(error) };
  }
  await native("finish", result);
}
