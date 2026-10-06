// Print / export-to-PDF: render a whole page to a self-contained HTML document in
// the Rust core (assets inlined, no sidebar — see publish::page_print_html), drop
// it into a hidden same-origin <iframe>, and print that frame. The user's native
// print dialog (WebKitGTK / WebView2 / WKWebView) then offers "Save as PDF".
//
// Why an iframe and not window.print() on the live app: the editor virtualizes
// blocks (only the on-screen ones are in the DOM), so printing the live page would
// drop most of a long page. The core-rendered document is complete and unstyled by
// the app chrome, so the PDF is the page, nothing else.
import { backend } from "./backend";
import { exportSheets } from "./sheet/exportSheets";
import { graphOwner, readOwned } from "./owned";
import { clearOnBindingInvalidated } from "./binding";
import { pushToast } from "./toasts";
import { flushAll } from "./document";
import type { PrintOpts } from "./types";
import { mainWindow } from "./windowRealm";

/** The default export options (match the Rust `PrintOpts::default`). */
export const DEFAULT_PRINT_OPTS: PrintOpts = {
  expand_collapsed: true,
  font_px: 16,
  margin_mm: 16,
};

export const PRINT_IFRAME_SANDBOX = "allow-same-origin allow-modals";
let activePrint: AbortController | null = null;
clearOnBindingInvalidated(() => activePrint?.abort());

let printRenderers: Promise<{
  katex: typeof import("katex").default;
  hljs: typeof import("highlight.js/lib/common").default;
}> | null = null;

function loadPrintRenderers() {
  if (!printRenderers) {
    printRenderers = Promise.all([
      import("katex").then(async (module) => {
        await import("katex/contrib/mhchem");
        return module.default;
      }),
      import("highlight.js/lib/common").then((module) => module.default),
    ]).then(([katex, hljs]) => ({ katex, hljs }));
  }
  return printRenderers;
}

function bundledStylesheets(): HTMLLinkElement[] {
  const current = new URL(document.baseURI);
  // The bundle's stylesheets live in the main window's <head> (popups mirror it).
  return [...mainWindow.document.querySelectorAll<HTMLLinkElement>('link[rel="stylesheet"][href]')]
    .filter((link) => {
      try {
        const url = new URL(link.href, document.baseURI);
        return url.protocol === current.protocol
          && url.host === current.host
          && /\/assets\/[A-Za-z0-9_-]+\.css$/.test(url.pathname);
      } catch {
        return false;
      }
    })
    .map((link) => link.cloneNode(true) as HTMLLinkElement);
}

class PrintQueryLimitError extends Error {
  constructor(detail: string) {
    super(`PDF export stopped at the Print query limit. ${detail}`);
    this.name = "PrintQueryLimitError";
  }
}

/**
 * Upgrade the core's inert print markup using only code already bundled with
 * Tine. The returned document contains no scripts or remote stylesheets; it
 * is safe to load in a same-origin iframe whose sandbox does not allow scripts.
 * Cost scales with the supplied markup and math/code spans. Optional renderer
 * failures leave readable raw markup. Renderer-declared query limits reject
 * before rendering: no partial page may enter the print dialog. The core owns
 * admission; this adapter reads its markup.
 */
export async function preparePrintHtml(html: string): Promise<string> {
  const parsed = new DOMParser().parseFromString(html, "text/html");
  const refusedQuery = parsed.querySelector(".query-too-large");
  if (refusedQuery) throw new PrintQueryLimitError(refusedQuery.textContent?.trim() ?? "");
  // Defense in depth against a future core regression: never pass executable or
  // remote stylesheet markup into the privileged app origin.
  parsed.querySelectorAll("script, link[rel=\"stylesheet\"]").forEach((element) => element.remove());

  try {
    const { katex, hljs } = await loadPrintRenderers();
    for (const span of parsed.querySelectorAll<HTMLElement>("span.math")) {
      const raw = span.textContent ?? "";
      const display = span.classList.contains("math-display");
      const left = display ? "\\[" : "\\(";
      const right = display ? "\\]" : "\\)";
      const tex = raw.startsWith(left) && raw.endsWith(right)
        ? raw.slice(left.length, -right.length)
        : raw;
      span.innerHTML = katex.renderToString(tex, { throwOnError: false, displayMode: display });
    }
    for (const code of parsed.querySelectorAll<HTMLElement>("pre.code-block > code")) {
      const source = code.textContent ?? "";
      const language = [...code.classList]
        .find((name) => name.startsWith("language-"))
        ?.slice("language-".length);
      try {
        code.innerHTML = language && hljs.getLanguage(language)
          ? hljs.highlight(source, { language }).value
          : hljs.highlightAuto(source).value;
      } catch {
        code.textContent = source;
      }
      code.classList.add("hljs");
    }
  } catch (error) {
    // A failed optional renderer must not make printing unavailable. The core
    // markup already contains readable raw TeX and escaped plain code.
    console.error("local print rendering failed");
  }

  for (const link of bundledStylesheets()) parsed.head.appendChild(link);
  return `<!doctype html>${parsed.documentElement.outerHTML}`;
}

/** Export through a hidden print frame and the OS dialog. First flush every
 * dirty page in the graph, costing O(dirty pages) writes. A failed save or any
 * unresolved conflict shows an error toast and opens no dialog. Missing pages
 * and backend errors also toast; this function does not reject. It resolves
 * when the frame is attached, before its load/fonts/print dialog complete.
 * Renderer-declared query limits show their reason and attach no frame. HTML
 * preparation scales with the rendered page; a newer call supersedes pending
 * preparation or an attached frame.
 * Supersession, graph retirement and window teardown silently settle pending
 * preparation. A frame releases on afterprint, load/print failure, retirement,
 * teardown, supersession or a 60-second watchdog; native print exceptions toast and remove it. */
export async function exportPagePdf(name: string, opts: PrintOpts = DEFAULT_PRINT_OPTS): Promise<void> {
  activePrint?.abort();
  const controller = new AbortController();
  activePrint = controller;
  const owner = graphOwner(() => activePrint === controller && !controller.signal.aborted);
  const abort = () => controller.abort();
  mainWindow.addEventListener("pagehide", abort);
  window.addEventListener("beforeunload", abort);
  let iframe: HTMLIFrameElement | undefined;
  let watchdog: ReturnType<typeof setTimeout> | undefined;
  let done = false;
  let cancel!: () => void;
  const cancelled = new Promise<undefined>((resolve) => { cancel = () => resolve(undefined); });
  const cleanup = () => {
    if (done) return;
    done = true;
    clearTimeout(watchdog);
    iframe?.remove();
    controller.signal.removeEventListener("abort", cleanup);
    mainWindow.removeEventListener("pagehide", abort);
    window.removeEventListener("beforeunload", abort);
    if (activePrint === controller) activePrint = null;
    cancel();
  };
  controller.signal.addEventListener("abort", cleanup, { once: true });
  const owned = <T,>(work: Promise<T>) => Promise.race([readOwned(owner, work), cancelled]);
  let html: string;
  try {
    const savedResult = await owned(flushAll());
    const saved = savedResult?.kind === "current" && savedResult.value;
    if (!owner()) { cleanup(); return; }
    if (!saved) {
      pushToast("PDF export stopped because some page edits could not be saved. Resolve the save conflict and try again.", "error");
      cleanup();
      return;
    }
    const sheets = await owned(exportSheets([name]));
    if (!sheets || sheets.kind === "stale" || !owner()) { cleanup(); return; }
    const result = await Promise.race([
      readOwned(owner, backend().pagePrintHtml(name, opts, sheets.value)), cancelled,
    ]);
    if (!result || result.kind === "stale") { cleanup(); return; }
    const prepared = await owned(preparePrintHtml(result.value));
    if (!prepared || prepared.kind === "stale") { cleanup(); return; }
    html = prepared.value;
    if (!owner()) { cleanup(); return; }
  } catch (e) {
    const current = owner();
    cleanup();
    if (!current) return;
    // `no-page` (deleted mid-action) or any core error — never leave a dangling frame.
    pushToast(e instanceof PrintQueryLimitError ? e.message : `Couldn't prepare “${name}” for PDF`, "error");
    console.error("pagePrintHtml failed");
    return;
  }

  try {
    iframe = document.createElement("iframe");
    iframe.setAttribute("aria-hidden", "true");
    // Keep same-origin DOM access so the parent can wait for fonts and invoke the
    // native print dialog, but categorically disable child scripts. The core also
    // emits script-src 'none'; neither graph markup nor a remote dependency can
    // reach Tauri's privileged parent/IPC surface.
    iframe.setAttribute("sandbox", PRINT_IFRAME_SANDBOX);
    // Off-screen + hidden: the print engine paginates the document at page width
    // regardless of the iframe's on-screen box, so a 0-size hidden frame prints fine.
    iframe.style.cssText =
      "position:fixed;right:0;bottom:0;width:0;height:0;border:0;visibility:hidden";
    iframe.srcdoc = html;

    iframe.onload = async () => {
      if (done) return;
      const win = iframe!.contentWindow;
      if (!win) {
        cleanup();
        return;
      }
      try {
        // Let the locally bundled styles/fonts settle so pagination measures the
        // final, already-typeset static layout.
        const fonts = iframe!.contentDocument?.fonts;
        if (fonts?.ready) await fonts.ready;
        await new Promise((r) => setTimeout(r, 400));
        if (done) return;
        if (!owner()) { cleanup(); return; }
        win.addEventListener("afterprint", cleanup, { once: true });
        win.focus();
        win.print();
      } catch (e) {
        if (owner()) pushToast("Print failed", "error");
        console.error("iframe print failed");
        cleanup();
      }
    };

    iframe.onerror = cleanup;
    // A failed load, stalled font, or missing afterprint must release the guard.
    watchdog = setTimeout(cleanup, 60_000);
    // The print frame is the main window's; the OS print dialog belongs to it.
    mainWindow.document.body.appendChild(iframe);
  } catch {
    const current = owner();
    cleanup();
    if (current) pushToast("Print failed", "error");
    console.error("iframe setup failed");
  }
}
