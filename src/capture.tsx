// Quick-capture mini window. A separate Tauri webview (capture.html) that the
// running app pops on a `tine --capture` signal. It renders the SAME block tree
// the main app uses (Block + Editor) over an isolated scratch store — so all
// autocomplete (`[[`/`#`/`((`/`/`), slash commands, the date picker and
// formatting behave identically, and Enter can create real new blocks. On submit
// the scratch is serialized to outline markdown and emitted; the MAIN window
// appends it to today's journal (App.tsx), keeping the single-writer design (this
// window never writes the graph itself — it never sets doc.loaded, so the
// persistence engine's saves are no-ops here).
import { render } from "solid-js/web";
import { For, Show, createSignal, createEffect, onCleanup, onMount } from "solid-js";
import { initParser } from "./render/parse";
import { initLinkDefault } from "./editor/linkDefault";
import { Block, CaptureCtx, type CaptureApi } from "./components/Block";
import { DatePicker } from "./components/DatePicker";
import { datePicker } from "./ui";
import {
  ensurePageLoaded,
  pageByName,
  blockSubtreeMarkdown,
  deleteBlock,
  setRaw,
  doc,
} from "./store";
import { startEditing } from "./editorController";
import { installKeybindings, eventToBindingString } from "./keybindings";
import { backend } from "./backend";
import { initSpellcheckSettings } from "./spellcheckSettings";
import { initRefCompletionSettings } from "./refCompletionSettings";
import { createCaptureBlurGate, resettleIfVisible } from "./captureVisibility";
import {
  QUICK_CAPTURE_ACK_TIMEOUT_MS,
  createQuickCaptureRequestId,
  quickCaptureAckMatches,
  shouldRetryQuickCapture,
  type QuickCaptureAck,
  type QuickCaptureRequest,
} from "./quickCaptureAck";
import { CAPTURE_SCRATCH_NAME, createCaptureScratchPage } from "./captureSeed";
// theme.css MUST come first: it defines every CSS variable (--bg-primary,
// --bullet-color, --ls-block-bullet-size, --selection-bg) AND the
// html[data-theme="dark"] overrides. Without it the capture webview had no
// variable definitions → white background, invisible bullets (size 0), no
// selection highlight, and data-theme="dark" did nothing.
import "./styles/theme.css";
import "./lsShimInstall";
import "./styles/app.css";
import "./styles/capture.css";

const SCRATCH = CAPTURE_SCRATCH_NAME;

// Pretty-print a binding string ("mod+shift+enter") for a hint ("Ctrl-Shift-Enter").
// "mod" is Ctrl on Linux/Windows (Cmd on macOS — but this app targets Linux).
function formatBinding(binding: string): string {
  const label: Record<string, string> = {
    mod: "Ctrl", ctrl: "Ctrl", control: "Ctrl", cmd: "Cmd", meta: "Cmd",
    super: "Super", shift: "Shift", alt: "Alt", enter: "Enter", esc: "Esc",
    space: "Space", tab: "Tab", up: "↑", down: "↓", left: "←", right: "→",
  };
  return binding
    .split("+")
    .map((k) => label[k.toLowerCase()] ?? (k.length === 1 ? k.toUpperCase() : k[0].toUpperCase() + k.slice(1)))
    .join("-");
}

function Capture() {
  const [ready, setReady] = createSignal(false);
  const [enterFiles, setEnterFiles] = createSignal(false);
  // Optional page title: filled → the capture becomes a NEW page; empty → it's
  // appended to today's journal. Plus the live merged shortcut map (from the main
  // window) so the hint text shows the user's ACTUAL submit shortcut.
  const [title, setTitle] = createSignal("");
  const [shortcuts, setShortcuts] = createSignal<Record<string, string>>({});
  const [captureStatus, setCaptureStatus] = createSignal<"idle" | "saving" | "error">("idle");
  const [captureMessage, setCaptureMessage] = createSignal("");
  const submitShortcut = () =>
    formatBinding(shortcuts()["editor/quick-capture-file"] || "mod+shift+enter");
  const bulletHint = () => `Edit as usual, ${submitShortcut()} to submit`;
  let titleRef: HTMLInputElement | undefined;

  const roots = () => pageByName(SCRATCH)?.roots ?? [];

  const seed = () => {
    ensurePageLoaded(createCaptureScratchPage());
    const root = pageByName(SCRATCH)?.roots[0];
    if (root) startEditing(root, 0, null);
    setReady(true);
  };

  // Reset to a single empty block (the page stays loaded; we just clear it).
  const clearScratch = () => {
    const page = pageByName(SCRATCH);
    const rid = page?.roots[0];
    if (!page || !rid) return;
    for (const r of page.roots.slice(1)) deleteBlock(r);
    const first = doc.byId[rid];
    if (first) for (const c of [...first.children]) deleteBlock(c);
    setRaw(rid, "");
    startEditing(rid, 0, null);
  };

  // The window is created hidden at startup, so the editor's onMount fit to a
  // 0-height layout and focus was a no-op. Re-fit + focus the live textarea when
  // the window is actually shown (the *first editing* textarea — capture starts
  // with the caret in the first/only block).
  const refit = () => {
    const ta = document.querySelector<HTMLTextAreaElement>(".capture-shell textarea");
    if (!ta) return;
    ta.focus();
    ta.style.height = "auto";
    ta.style.height = `${ta.scrollHeight}px`;
  };

  // --- auto-grow the window to fit its content + any open popup --------------
  // The capture window starts tiny (one line). As you add blocks, wrap lines, or
  // open an autocomplete popup / date picker, paint would overflow the OS window
  // and get clipped — so we measure what's actually rendered and resize the
  // window to fit (capped at HALF the screen). Rust resets it to the base size
  // on each show, so this only ever grows from the small base.
  let lastH = 0;
  const measureHeight = (): number => {
    const PAD = 18; // breathing room below the content
    const shell = document.querySelector<HTMLElement>(".capture-shell");
    let h = shell ? shell.getBoundingClientRect().bottom : 72;
    const popup = document.querySelector<HTMLElement>(".autocomplete");
    if (popup) h = Math.max(h, popup.getBoundingClientRect().bottom);
    // The format toolbar floats just below the editing line (see capture.css) —
    // grow the window so it isn't clipped at the bottom edge.
    const toolbar = document.querySelector<HTMLElement>(".sel-toolbar");
    if (toolbar) h = Math.max(h, toolbar.getBoundingClientRect().bottom);
    // The date picker is fixed-positioned and clamps to the *current* window
    // height, so reserve a fixed slab tall enough to show it (it then positions
    // itself once the resize takes effect — its placement is window-size-aware).
    if (datePicker()) h = Math.max(h, 392);
    return Math.ceil(h + PAD);
  };

  const fitWindow = async () => {
    try {
      const { getCurrentWindow, LogicalSize } = await import("@tauri-apps/api/window");
      const win = getCurrentWindow();
      if (!(await win.isVisible())) return;
      // Cap the auto-grow at HALF the screen, AND at a fixed 700px ceiling. The
      // fixed ceiling guards against an over-large `availHeight` reading on HiDPI
      // displays (WebKitGTK can report it in device pixels, while setSize takes
      // logical pixels — so `0.5 * availHeight` could exceed the real screen).
      const screenMax = Math.round((window.screen?.availHeight ?? 1000) * 0.5);
      const measured = measureHeight();
      // Floor of 140px: a touch taller than the bare title+first-bullet content,
      // so the page-title field and the "…to submit" hint are always fully
      // visible with breathing room (Martin: the tight fit clipped the hint).
      const h = Math.max(140, Math.min(measured, screenMax, 700));
      // Diagnostic (visible when launched from a terminal): if the window is ever
      // mis-sized, this shows whether the fault is the measurement or setSize.
      console.log("[capture] fit", { measured, screenMax, h, lastH });
      if (Math.abs(h - lastH) < 2) return; // avoid churn / feedback loops
      lastH = h;
      await win.setSize(new LogicalSize(600, h));
    } catch {
      // not in Tauri (dev preview)
    }
  };

  // Re-fit the window to its CURRENT content. Run on every (re)show — both the
  // Rust `capture-shown` event and a focus-gained event — because this window is
  // created `focus: false` and frameless, so a WM may not deliver a focus event
  // on show; relying on focus alone left the window stuck at a previously-grown
  // size. Clearing `lastH` lets it shrink back down (the churn guard would else
  // suppress it), and re-fitting across a few ticks catches layout settling.
  const resettle = () => {
    lastH = 0;
    scheduleFit();
    requestAnimationFrame(() => { refit(); scheduleFit(); });
    setTimeout(() => { refit(); scheduleFit(); }, 80);
    setTimeout(scheduleFit, 220);
  };
  const activateNativeWindow = async () => {
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      await invoke("capture_frontend_ready");
    } catch {
      // not in Tauri, hidden again, or shutting down
    }
  };
  let activationGeneration = 0;
  const activateWhenEditorReady = () => {
    const generation = ++activationGeneration;
    const tryActivate = (attempt: number) => {
      if (generation !== activationGeneration) return;
      const editor = document.querySelector<HTMLTextAreaElement>(".capture-shell textarea");
      if (editor) {
        refit();
        void activateNativeWindow();
        return;
      }
      // The editor deliberately ends its editing session when the always-on-top
      // capture window is hidden. Re-enter edit mode on show; otherwise the
      // native window can be focused while the block is still rendered as
      // read-only text, and typing has no destination until the user clicks it.
      if (attempt === 0) {
        const root = roots()[0];
        const block = root ? doc.byId[root] : undefined;
        if (root) startEditing(root, block?.raw.length ?? 0, null);
      }
      // The capture shell mounts before its Block editor on a cold WebView.
      // Do not ask the WM for focus until real keyboard input has somewhere to
      // land. A short bounded retry also covers the first parser initialization.
      if (attempt < 300) setTimeout(() => tryActivate(attempt + 1), 16);
    };
    tryActivate(0);
  };
  const resettleAndActivate = () => {
    resettle();
    activateWhenEditorReady();
  };
  // Quick Capture is a separate WebView, so its module-local completion-policy
  // signal starts at the default and it has no graph binding. Do not acknowledge
  // an activation until this WebView has read the persisted policy *and* leased
  // the selected graph for quickSwitch; otherwise a fast first `[[…]]` can
  // either use adaptive or issue its query without any graph candidates.
  // A later show wins if refreshes overlap while the window is being hidden or
  // re-shown, so stale reads cannot activate an older lifecycle.
  let policyRefreshGeneration = 0;
  const refreshPolicyThenResettleAndActivate = async () => {
    const generation = ++policyRefreshGeneration;
    await Promise.all([initLinkDefault(), backend().bindCaptureGraph()]);
    if (generation !== policyRefreshGeneration) return;
    resettleAndActivate();
  };
  const blurGate = createCaptureBlurGate();
  let fitRaf: number | undefined;
  const scheduleFit = () => {
    if (fitRaf !== undefined) return;
    fitRaf = requestAnimationFrame(() => {
      fitRaf = undefined;
      void fitWindow();
    });
  };

  const hideWindow = async () => {
    blurGate.disarm();
    try {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      await getCurrentWindow().hide();
    } catch {
      // not in Tauri (dev preview)
    }
  };

  const loadPref = () => {
    void backend().getCaptureEnterFiles().then(setEnterFiles).catch(() => {});
  };

  type PendingCapture = {
    id: string;
    attemptsStarted: number;
    payload: QuickCaptureRequest;
    target: string;
    unlisten?: () => void;
    timer?: number;
  };
  let pendingCapture: PendingCapture | null = null;
  const disposePendingCapture = () => {
    if (!pendingCapture) return;
    if (pendingCapture.timer !== undefined) window.clearTimeout(pendingCapture.timer);
    pendingCapture.unlisten?.();
    pendingCapture = null;
  };
  onCleanup(disposePendingCapture);

  // The capture webview can't read the main window's chosen theme (localStorage
  // isn't shared across WebKitGTK webviews, and prefers-color-scheme doesn't
  // track the app theme). Ask the running main window for it; it replies (and
  // also broadcasts on every change) with `capture-apply-theme`.
  const applyThemeAttr = (t: string) => {
    if (t === "dark" || t === "light") document.documentElement.setAttribute("data-theme", t);
  };
  const requestTheme = async () => {
    try {
      const { emitTo } = await import("@tauri-apps/api/event");
      const target = await backend().captureTarget();
      await emitTo(target, "capture-request-theme", { target });
    } catch {
      // not in Tauri
    }
  };

  // Editor shortcuts (incl. the configurable `editor/quick-capture-file` that
  // files the capture) are merged from config.edn + the user's Settings
  // overrides, which live in the MAIN window's localStorage — unreadable here.
  // Ask the main window for the merged map and re-install our keybindings with
  // it, so a remapped shortcut is honored in the capture window too.
  let disposeKeys: () => void = () => {};
  const requestShortcuts = async () => {
    try {
      const { emitTo } = await import("@tauri-apps/api/event");
      const target = await backend().captureTarget();
      await emitTo(target, "capture-request-shortcuts", { target });
    } catch {
      // not in Tauri
    }
  };

  const submit = () => {
    if (pendingCapture) return;
    const md = roots().map((r) => blockSubtreeMarkdown(r)).join("\n").trim();
    const pageTitle = title().trim();
    void (async () => {
      if (!md) {
        setCaptureStatus("idle");
        setCaptureMessage("");
        clearScratch();
        setTitle("");
        await hideWindow();
        return;
      }
      const id = createQuickCaptureRequestId();
      let target: string;
      try {
        target = await backend().captureTarget();
      } catch {
        setCaptureStatus("error");
        setCaptureMessage("No graph window is ready — text kept");
        scheduleFit();
        return;
      }
      const pending: PendingCapture = {
        id,
        attemptsStarted: 0,
        payload: { id, target, text: md, title: pageTitle },
        target,
      };
      const finish = async (ok: boolean) => {
        if (pendingCapture !== pending) return;
        disposePendingCapture();
        if (ok) {
          setCaptureStatus("idle");
          setCaptureMessage("");
          clearScratch();
          setTitle(""); // reset for the next capture — don't carry over the filed text
          await hideWindow();
        } else {
          setCaptureStatus("error");
          setCaptureMessage("Capture couldn't be saved — text kept");
          scheduleFit();
        }
      };
      const giveUp = () => {
        if (pendingCapture !== pending) return;
        disposePendingCapture();
        setCaptureStatus("error");
        setCaptureMessage("graph window not ready — try again");
        scheduleFit();
      };
      setCaptureStatus("saving");
      setCaptureMessage("saving…");
      pendingCapture = pending;
      let eventApi: typeof import("@tauri-apps/api/event");
      try {
        eventApi = await import("@tauri-apps/api/event");
      } catch {
        giveUp();
        return;
      }
      if (pendingCapture !== pending) return;
      const { emitTo, listen } = eventApi;
      const scheduleTimeout = () => {
        pending.timer = window.setTimeout(() => {
          if (pendingCapture !== pending) return;
          if (shouldRetryQuickCapture(pending.attemptsStarted)) {
            void emitAttempt();
          } else {
            giveUp();
          }
        }, QUICK_CAPTURE_ACK_TIMEOUT_MS);
      };
      const emitAttempt = async () => {
        if (pending.timer !== undefined) {
          window.clearTimeout(pending.timer);
          pending.timer = undefined;
        }
        pending.attemptsStarted += 1;
        try {
          // `title` set → the main window files this as a NEW page; empty → today.
          await emitTo(pending.target, "quick-capture", pending.payload);
        } catch {
          giveUp();
          return;
        }
        scheduleTimeout();
      };
      try {
        const unlisten = await listen<QuickCaptureAck>("quick-capture-ack", (e) => {
          if (quickCaptureAckMatches(pending.id, e.payload)) void finish(e.payload.ok);
        });
        if (pendingCapture !== pending) {
          unlisten();
          return;
        }
        pending.unlisten = unlisten;
      } catch {
        giveUp();
        return;
      }
      await emitAttempt();
    })();
  };

  const cancel = () => {
    disposePendingCapture();
    setCaptureStatus("idle");
    setCaptureMessage("");
    clearScratch();
    setTitle("");
    void hideWindow();
  };

  const captureApi: CaptureApi = {
    submit,
    cancel,
    enterFiles,
    bulletHint,
    quickSwitch: (query, limit) => backend().captureQuickSwitch(query, limit),
  };

  // Grow/shrink the window whenever the rendered content changes: new/removed
  // blocks and popups (childList), or a textarea autosizing (inline `style`).
  // Plus the date-picker signal, which a MutationObserver on the DOM also catches
  // but the effect makes the dependency explicit.
  createEffect(() => {
    datePicker();
    captureMessage();
    scheduleFit();
  });

  onMount(() => {
    // Populate the keybinding table so editor shortcuts (bold/italic/…) work the
    // same as the main window — defaults until the main window sends the merged
    // map (see the capture-apply-shortcuts listener). Its global handler is
    // harmless here.
    disposeKeys = installKeybindings();
    onCleanup(() => disposeKeys());
    loadPref();
    // Mirror the main window's spellcheck pref in this separate webview context.
    void initSpellcheckSettings();
    void initRefCompletionSettings();
    seed();

    const root = document.getElementById("capture-root");
    if (root) {
      const mo = new MutationObserver(scheduleFit);
      mo.observe(root, {
        childList: true,
        subtree: true,
        attributes: true,
        attributeFilter: ["style"],
      });
      onCleanup(() => mo.disconnect());
    }

    void (async () => {
      try {
        const { listen } = await import("@tauri-apps/api/event");
        const unTheme = await listen<{ theme: string }>("capture-apply-theme", (e) =>
          applyThemeAttr(e.payload.theme)
        );
        const unKeys = await listen<Record<string, string>>("capture-apply-shortcuts", (e) => {
          disposeKeys();
          disposeKeys = installKeybindings(e.payload ?? {});
          setShortcuts(e.payload ?? {}); // keep the hint's submit shortcut current
        });
        // Rust emits this every time it shows the window — the reliable re-fit
        // trigger (the focus event isn't guaranteed for a `focus:false` frameless
        // window). Also refresh the pref/theme/shortcuts that may have changed
        // while we were hidden.
        const unShown = await listen("capture-shown", () => {
          loadPref();
          void refreshPolicyThenResettleAndActivate();
          void requestTheme();
          void requestShortcuts();
        });
        const unFocusEditor = await listen("capture-focus-editor", refit);
        // Cold `tine --capture` can show + emit before this asynchronously
        // imported listener exists. Reconcile the current window state once the
        // listener is installed, so that missed first event cannot leave the
        // visible capture editor unfocused (GH #117). A later show still follows
        // the normal event path above.
        const { getCurrentWindow } = await import("@tauri-apps/api/window");
        await resettleIfVisible(getCurrentWindow(), refreshPolicyThenResettleAndActivate);
        onCleanup(() => {
          unTheme();
          unKeys();
          unShown();
          unFocusEditor();
        });
      } catch {
        // not in Tauri
      }
    })();
    void requestTheme();
    void requestShortcuts();

    void (async () => {
      try {
        const { getCurrentWindow } = await import("@tauri-apps/api/window");
        await getCurrentWindow().onFocusChanged(({ payload: focused }) => {
          if (focused) {
            blurGate.focusChanged(true);
            loadPref(); // pick up a Settings change made while we were hidden
            void requestTheme(); // and a theme change
            void requestShortcuts(); // and a shortcut remap
            resettle();
          } else {
            // Dismiss on blur. The draft is preserved (only Esc/submit clear it)
            // so an accidental focus loss can't lose text. A newly mapped
            // window may emit an initial false transition before the WM honors
            // activation; wait until this show has actually held focus.
            if (blurGate.focusChanged(false)) void hideWindow();
          }
        });
      } catch {
        // not in Tauri
      }
    })();
  });

  return (
    <CaptureCtx.Provider value={captureApi}>
      <div class="capture-shell">
        <Show when={ready()}>
          {/* Optional page title. Filled → the capture becomes a NEW page; empty →
              appended to today. Plain Enter drops into the bullet; the submit
              shortcut / Esc work here too. */}
          <input
            ref={titleRef}
            class="capture-title"
            type="text"
            value={title()}
            placeholder="Page Title (optional, if empty → appended to Today)"
            onInput={(e) => setTitle(e.currentTarget.value)}
            onKeyDown={(e) => {
              const want = shortcuts()["editor/quick-capture-file"] || "mod+shift+enter";
              if (eventToBindingString(e) === want) {
                e.preventDefault();
                submit();
              } else if (e.key === "Escape") {
                e.preventDefault();
                cancel();
              } else if (e.key === "Enter" && !e.shiftKey && !e.ctrlKey && !e.metaKey) {
                e.preventDefault();
                document.querySelector<HTMLTextAreaElement>(".capture-shell .page-blocks textarea")?.focus();
              }
            }}
          />
          <div class="page-blocks">
            <For each={roots()}>{(rid) => <Block id={rid} />}</For>
          </div>
          <Show when={captureMessage()}>
            <div
              class="capture-status"
              classList={{ "capture-status-error": captureStatus() === "error" }}
            >
              {captureMessage()}
            </div>
          </Show>
        </Show>
      </div>
      <DatePicker />
    </CaptureCtx.Provider>
  );
}

// Render the capture window IMMEDIATELY — it opens to an empty editing block, which
// needs no parser for first paint, so we must NOT gate capture-open latency on the
// wasm init. The parser loads in the background; any block that renders before it's
// ready falls back to raw text (AstBody/InlineText) and swaps in once `parserReady`
// flips (typically tens of ms, well before you finish typing the first block).
void initParser().catch((e) => console.error("lsdoc-wasm init failed:", e));
render(() => <Capture />, document.getElementById("capture-root")!);
