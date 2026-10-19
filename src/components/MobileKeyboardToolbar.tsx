import { msg, t, render, type Msg } from "../i18n";
import { For, Show, createEffect, createSignal, onCleanup, untrack, type Component, type JSX } from "solid-js";
import { Dynamic } from "solid-js/web";
import { dispatchFocusedEditorCommand, blurFocusedEditor, focusedEditorCommandBridge, type MobileEditorCommandId } from "../editorCommandBridge";
import { runGlobalCommand } from "../keybindings";
import { touchPrimary } from "../inputCapability";
import { isRecordingAudio } from "../mediaCapture";

type ToolbarIcon =
  | "outdent"
  | "indent"
  | "move-up"
  | "move-down"
  | "soft-newline"
  | "todo"
  | "camera"
  | "mic"
  | "stop-recording"
  | "undo"
  | "redo"
  | "date"
  | "page-ref"
  | "block-ref"
  | "slash";

type ToolbarAction =
  | { label: Msg; icon: ToolbarIcon; kind: "editor"; command: MobileEditorCommandId }
  | { label: Msg; icon: ToolbarIcon; kind: "global"; command: "editor/undo" | "editor/redo" };

const ACTIONS: ToolbarAction[] = [
  { label: msg("mobile-keyboard-toolbar-outdent"), icon: "outdent", kind: "editor", command: "editor/outdent" },
  { label: msg("mobile-keyboard-toolbar-indent"), icon: "indent", kind: "editor", command: "editor/indent" },
  { label: msg("mobile-keyboard-toolbar-move-block-up"), icon: "move-up", kind: "editor", command: "editor/move-block-up" },
  { label: msg("mobile-keyboard-toolbar-move-block-down"), icon: "move-down", kind: "editor", command: "editor/move-block-down" },
  { label: msg("mobile-keyboard-toolbar-soft-newline"), icon: "soft-newline", kind: "editor", command: "editor/soft-newline" },
  { label: msg("mobile-keyboard-toolbar-todo"), icon: "todo", kind: "editor", command: "editor/cycle-todo" },
  { label: msg("mobile-keyboard-toolbar-camera"), icon: "camera", kind: "editor", command: "editor/capture-photo" },
  { label: msg("mobile-keyboard-toolbar-voice-memo"), icon: "mic", kind: "editor", command: "editor/voice-memo" },
  { label: msg("mobile-keyboard-toolbar-undo"), icon: "undo", kind: "global", command: "editor/undo" },
  { label: msg("mobile-keyboard-toolbar-redo"), icon: "redo", kind: "global", command: "editor/redo" },
  { label: msg("mobile-keyboard-toolbar-date-picker"), icon: "date", kind: "editor", command: "editor/open-date-picker" },
  { label: msg("mobile-keyboard-toolbar-page-reference"), icon: "page-ref", kind: "editor", command: "editor/insert-page-ref" },
  { label: msg("mobile-keyboard-toolbar-block-reference"), icon: "block-ref", kind: "editor", command: "editor/insert-block-ref" },
  { label: msg("mobile-keyboard-toolbar-slash-menu"), icon: "slash", kind: "editor", command: "editor/open-slash-menu" },
];

const KEYBOARD_GAP_THRESHOLD = 48;
/** Dispatched on window by Android's MainActivity with `detail: true|false`
 *  whenever the soft keyboard's visibility changes (GH #716). */
export const NATIVE_IME_EVENT = "tine-ime";
const EDITOR_TOOLBAR_MARGIN = 8;

function activeEditorHasFocus(): boolean {
  const el = document.activeElement;
  return el instanceof HTMLTextAreaElement && el.classList.contains("block-editor");
}

function viewportKeyboardTop(): number {
  const vv = window.visualViewport;
  return vv ? vv.height + vv.offsetTop : window.innerHeight;
}

/** Scroll the focused block editor's nearest scroller (or the window) just enough
 *  that the editor's bottom sits 8px above `toolbarTop` (GH #384), or above the
 *  editor's docked formatting row when one is shown, which sits above the dock
 *  (GH #375: measuring only the dock left a selection just above it under the
 *  formatting row). Returns true only when it scrolled; a non-editor focus or an
 *  already-visible editor is a no-op returning false. O(1) layout reads. */
export function revealFocusedEditorAboveToolbar(toolbarTop: number): boolean {
  const editor = document.activeElement;
  if (!(editor instanceof HTMLTextAreaElement) || !editor.classList.contains("block-editor")) {
    return false;
  }
  const formatting = document.querySelector<HTMLElement>("[data-mobile-selection-toolbar]")?.getBoundingClientRect();
  const top = formatting && formatting.height > 0 ? Math.min(toolbarTop, formatting.top) : toolbarTop;
  const overlap = editor.getBoundingClientRect().bottom + EDITOR_TOOLBAR_MARGIN - top;
  if (overlap <= 0) return false;
  const scroller = editor.closest<HTMLElement>(
    ".main-content, .right-sidebar-scroll, .left-sidebar-scroll",
  );
  if (scroller) scroller.scrollTop += overlap;
  else window.scrollBy({ top: overlap });
  return true;
}

function Icon(props: { name: ToolbarIcon }): JSX.Element {
  switch (props.name) {
    case "outdent":
      return <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M5 6h14M5 12h8M5 18h14M15 9l-3 3 3 3" /></svg>;
    case "indent":
      return <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M5 6h14M11 12h8M5 18h14M6 9l3 3-3 3" /></svg>;
    case "move-up":
      return <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 18V6M7 11l5-5 5 5M6 20h12" /></svg>;
    case "move-down":
      return <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 6v12M7 13l5 5 5-5M6 4h12" /></svg>;
    case "soft-newline":
      return <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M17 5v5a4 4 0 0 1-4 4H6M9 10l-4 4 4 4" /></svg>;
    case "todo":
      return <svg viewBox="0 0 24 24" aria-hidden="true"><rect x="4" y="4" width="16" height="16" rx="3" /><path d="M8 12l3 3 5-7" /></svg>;
    case "camera":
      return <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 8h3l1.5-2h7L17 8h3v11H4z" /><circle cx="12" cy="13" r="3.2" /></svg>;
    case "mic":
      return <svg viewBox="0 0 24 24" aria-hidden="true"><rect x="9" y="3" width="6" height="11" rx="3" /><path d="M6 11a6 6 0 0 0 12 0M12 17v4M8 21h8" /></svg>;
    case "stop-recording":
      return <svg viewBox="0 0 24 24" aria-hidden="true"><rect x="6" y="6" width="12" height="12" rx="2" /></svg>;
    case "undo":
      return <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M9 14l-4-4 4-4M5 10h10a4 4 0 0 1 0 8h-2" /></svg>;
    case "redo":
      return <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M15 14l4-4-4-4M19 10H9a4 4 0 0 0 0 8h2" /></svg>;
    case "date":
      return <svg viewBox="0 0 24 24" aria-hidden="true"><rect x="4" y="5" width="16" height="15" rx="2" /><path d="M8 3v4M16 3v4M4 10h16" /></svg>;
    case "page-ref":
      return <span class="mobile-keyboard-toolbar-text" aria-hidden="true">[[ ]]</span>;
    case "block-ref":
      return <span class="mobile-keyboard-toolbar-text" aria-hidden="true">(( ))</span>;
    case "slash":
      return <span class="mobile-keyboard-toolbar-text mobile-keyboard-toolbar-slash" aria-hidden="true">/</span>;
  }
}

export interface MobileKeyboardToolbarProps {
  /** The App's Sidebar / Search / Back / Forward group, shown at the toolbar's
   *  leading end when "Navigation buttons at the bottom" is on (GH #598,
   *  D-20261019-N2). The toolbar only hosts it: a tap ends the edit through the
   *  editor's normal blur save, then runs the group's own handler. */
  navigation?: Component;
}

export function MobileKeyboardToolbar(props: MobileKeyboardToolbarProps = {}): JSX.Element {
  let toolbarRef: HTMLDivElement | undefined;
  const [dockTop, setDockTop] = createSignal(typeof window !== "undefined" ? window.innerHeight : 0);
  const [keyboardVisible, setKeyboardVisible] = createSignal(false);
  const [focusedFallback, setFocusedFallback] = createSignal(false);
  let revealFrame = 0;
  // Android pads the WebView itself above the IME (MainActivity), so there the
  // keyboard looks exactly like a window resize. MainActivity reports the real
  // IME state as a `tine-ime` event; until one arrives (iOS, a browser, or an
  // Android page that has not seen a change yet) the visual viewport decides.
  let nativeIme: boolean | undefined;
  // A focused editor with no keyboard keeps the dock (hardware keyboard) only
  // until a keyboard has been seen: once Back or the IME's hide key closes it,
  // the editor stays focused but the dock yields to the navigation row, like
  // OG's keyboardWillHide (show-toolbar? false, show-tabbar? true) (GH #716).
  let keyboardSeen = false;

  const updateDock = () => {
    const top = viewportKeyboardTop();
    const keyboardUp = nativeIme ?? window.innerHeight - top > KEYBOARD_GAP_THRESHOLD;
    const focused = activeEditorHasFocus();
    keyboardSeen = focused && (keyboardSeen || keyboardUp);
    setDockTop(top);
    setKeyboardVisible(keyboardUp);
    setFocusedFallback(focused && !keyboardSeen);
  };

  // The dock belongs to a touch-first device (D-20261018-W1: input capability,
  // not the OS name); the listeners follow `touchPrimary()` if it flips.
  createEffect(() => {
    if (!touchPrimary()) return;
    untrack(attachViewportListeners);
  });
  function attachViewportListeners() {
    const vv = window.visualViewport;
    const scheduleEditorReveal = () => {
      if (revealFrame) return;
      revealFrame = requestAnimationFrame(() => {
        revealFrame = 0;
        if (toolbarRef && visible()) {
          revealFocusedEditorAboveToolbar(toolbarRef.getBoundingClientRect().top);
        }
      });
    };
    const updateViewport = () => {
      updateDock();
      scheduleEditorReveal();
    };
    const updateAfterFocusChange = () => setTimeout(updateViewport, 0);
    const revealAfterEditorInput = (event: Event) => {
      if (event.target instanceof HTMLTextAreaElement && event.target.classList.contains("block-editor")) {
        scheduleEditorReveal();
      }
    };
    const updateFromNativeIme = (event: Event) => {
      nativeIme = (event as CustomEvent<unknown>).detail === true;
      updateViewport();
    };
    updateDock();
    window.addEventListener(NATIVE_IME_EVENT, updateFromNativeIme);
    vv?.addEventListener("resize", updateViewport);
    vv?.addEventListener("scroll", updateViewport);
    window.addEventListener("resize", updateViewport);
    window.addEventListener("focusin", updateAfterFocusChange);
    window.addEventListener("focusout", updateAfterFocusChange);
    document.addEventListener("input", revealAfterEditorInput, true);
    onCleanup(() => {
      window.removeEventListener(NATIVE_IME_EVENT, updateFromNativeIme);
      vv?.removeEventListener("resize", updateViewport);
      vv?.removeEventListener("scroll", updateViewport);
      window.removeEventListener("resize", updateViewport);
      window.removeEventListener("focusin", updateAfterFocusChange);
      window.removeEventListener("focusout", updateAfterFocusChange);
      document.removeEventListener("input", revealAfterEditorInput, true);
      if (revealFrame) cancelAnimationFrame(revealFrame);
    });
  }

  createEffect(() => {
    focusedEditorCommandBridge();
    if (touchPrimary()) queueMicrotask(updateDock);
  });

  // GH #336: the hide button's pointerdown blurs the editor, which drops the
  // keyboard and would unmount this toolbar MID-GESTURE — Android then delivers
  // the gesture's synthesized pointerup/click to whatever moved underneath
  // ("touch-through" onto the page). Stay mounted until the trailing click is
  // consumed here, with a short safety window for pointercancel/lost events.
  const [hideGesture, setHideGesture] = createSignal(false);
  const [hideGestureDockTop, setHideGestureDockTop] = createSignal<number | null>(null);
  let hideGestureTimer: ReturnType<typeof setTimeout> | undefined;
  const cancelHideGesture = () => {
    if (hideGestureTimer !== undefined) clearTimeout(hideGestureTimer);
    hideGestureTimer = undefined;
    setHideGestureDockTop(null);
    setHideGesture(false);
  };
  onCleanup(() => {
    if (hideGestureTimer !== undefined) clearTimeout(hideGestureTimer);
  });

  const visible = () =>
    (!!focusedEditorCommandBridge() && (keyboardVisible() || focusedFallback())) || hideGesture();

  // Publish the toolbar's on-screen top as a CSS var so the fixed help "?" FAB
  // (and any other bottom-anchored chrome) can lift ABOVE it instead of
  // overlapping. Re-runs when the dock position or visibility changes; measures
  // the real rendered box (robust to whatever the keyboard does to the viewport).
  createEffect(() => {
    const shown = touchPrimary() && visible();
    dockTop(); // re-measure when the keyboard/dock moves
    const root = typeof document !== "undefined" ? document.documentElement : null;
    if (!root) return;
    // The editing dock replaces the bottom navigation row while it is up.
    root.toggleAttribute("data-mobile-editing-dock", shown);
    if (!shown) {
      root.style.removeProperty("--mobile-kb-toolbar-lift");
      return;
    }
    requestAnimationFrame(() => {
      // The dock may have hidden since (the keyboard closed in the same
      // resize); measuring it now would publish a lift from a 0 top (GH #716).
      if (!toolbarRef || !visible()) return;
      const top = toolbarRef.getBoundingClientRect().top;
      const lift = Math.max(0, window.innerHeight - top) + 8;
      root.style.setProperty("--mobile-kb-toolbar-lift", `${lift}px`);
      revealFocusedEditorAboveToolbar(top);
    });
  });
  onCleanup(() => {
    if (typeof document !== "undefined") {
      document.documentElement.style.removeProperty("--mobile-kb-toolbar-lift");
      document.documentElement.removeAttribute("data-mobile-editing-dock");
    }
  });

  const style = () => ({
    // The viewport grows while the keyboard closes. Keep the button under the
    // active pointer until its trailing click is consumed; otherwise a mounted
    // toolbar can still move away and expose the note before pointer-up.
    top: `calc(${Math.max(0, hideGestureDockTop() ?? dockTop())}px - var(--system-inset-bottom))`,
  });
  const keepEditorFocus = (e: Event) => e.preventDefault();

  // A structural command can briefly unregister the editor and retarget the
  // browser's compatibility click (#495/#496). Own the completed pointer at
  // toolbar scope, and consume its click before any new target can act. A new
  // primary pointerdown starts a new gesture; keyboard/AT clicks have detail 0.
  // No elapsed-time window may suppress an independent second activation.
  let completedPointer: number | null = null;
  createEffect(() => {
    if (!touchPrimary()) return;
    untrack(attachCompletedPointerGuard);
  });
  function attachCompletedPointerGuard() {
    const beginPointer = (e: PointerEvent) => {
      if (e.isPrimary) completedPointer = null;
    };
    const consumeClick = (e: MouseEvent) => {
      if (completedPointer === null || e.detail === 0) return;
      const clickPointer = "pointerId" in e ? (e as PointerEvent).pointerId : null;
      if (clickPointer !== null && clickPointer !== completedPointer) return;
      completedPointer = null;
      e.preventDefault();
      e.stopImmediatePropagation();
    };
    document.addEventListener("pointerdown", beginPointer, true);
    document.addEventListener("click", consumeClick, true);
    onCleanup(() => {
      document.removeEventListener("pointerdown", beginPointer, true);
      document.removeEventListener("click", consumeClick, true);
    });
  }

  // Pointer-up remains the primary activation path: iOS WebKit can omit the
  // compatibility click after our focus-preserving canceled pointerdown (#434).
  function tapActivation(run: () => void) {
    let pointer: number | null = null;
    return {
      onPointerDown(e: PointerEvent) {
        e.preventDefault();
        if (!e.isPrimary || e.button !== 0) return;
        pointer = e.pointerId;
        (e.currentTarget as HTMLElement).setPointerCapture?.(e.pointerId);
      },
      onPointerUp(e: PointerEvent) {
        if (pointer !== e.pointerId) return;
        pointer = null;
        completedPointer = e.pointerId;
        const box = (e.currentTarget as HTMLElement).getBoundingClientRect();
        // Sliding off cancels activation, including a possible trailing click.
        // jsdom has no layout and reports a zero box.
        const off =
          (box.width > 0 || box.height > 0) &&
          (e.clientX < box.left || e.clientX > box.right || e.clientY < box.top || e.clientY > box.bottom);
        if (!off) run();
      },
      onPointerCancel(e: PointerEvent) {
        if (pointer === e.pointerId) pointer = null;
      },
      onClick(e: MouseEvent) {
        e.preventDefault();
        e.stopPropagation();
        run();
      },
    };
  }

  const run = (action: ToolbarAction) => {
    if (action.kind === "global") {
      runGlobalCommand(action.command);
      return;
    }
    dispatchFocusedEditorCommand(action.command);
  };
  const beginHideGesture = (e: Event) => {
    e.preventDefault();
    if (hideGestureTimer !== undefined) clearTimeout(hideGestureTimer);
    setHideGestureDockTop(dockTop());
    hideGestureTimer = setTimeout(cancelHideGesture, 700);
    setHideGesture(true);
    blurFocusedEditor();
  };
  const endHideGesture = () => {
    // An AT/keyboard activation arrives without a pointer gesture, and still
    // needs the blur itself.
    if (!hideGesture()) blurFocusedEditor();
    cancelHideGesture();
  };
  // The hide button's own two-phase gesture starts on pointerdown, so it keeps
  // that handler and borrows only the completion half of tapActivation.
  const hideTap = tapActivation(endHideGesture);

  // GH #598: the navigation group is the App's own buttons (their onClick is the
  // action), so activation is delegated: take the tap like tapActivation does
  // (pointer-up first, the trailing click swallowed), then click the button. The
  // capture-phase click listener below ends the edit BEFORE that handler runs, for
  // every route in (tap, keyboard, assistive technology): blur is the editor's
  // normal save path, so the text is committed before the page changes.
  let navPointer: number | null = null;
  const navButtonOf = (e: Event): HTMLButtonElement | null => {
    const button = (e.target as Element | null)?.closest?.("button");
    return button && !button.disabled ? button : null;
  };
  const navTap = {
    onPointerDown(e: PointerEvent) {
      const button = navButtonOf(e);
      if (!button) return;
      e.preventDefault(); // keep the editor focused until the tap completes
      if (!e.isPrimary || e.button !== 0) return;
      navPointer = e.pointerId;
      button.setPointerCapture?.(e.pointerId);
    },
    onPointerUp(e: PointerEvent) {
      const button = navButtonOf(e);
      if (!button || navPointer !== e.pointerId) return;
      navPointer = null;
      completedPointer = e.pointerId;
      const box = button.getBoundingClientRect();
      const off =
        (box.width > 0 || box.height > 0) &&
        (e.clientX < box.left || e.clientX > box.right || e.clientY < box.top || e.clientY > box.bottom);
      if (!off) button.click();
    },
    onPointerCancel(e: PointerEvent) {
      if (navPointer === e.pointerId) navPointer = null;
    },
  };
  const endEditBeforeNavigation = (e: MouseEvent) => {
    if (navButtonOf(e)) blurFocusedEditor();
  };
  const navRef = (el: HTMLDivElement) => {
    el.addEventListener("click", endEditBeforeNavigation, true);
    onCleanup(() => el.removeEventListener("click", endEditBeforeNavigation, true));
  };

  return (
    // The toolbar belongs to the mobile shell, not a particular editor DOM node.
    // Preserve its buttons and horizontal scroll through bridge handoffs; only
    // its visibility follows focus/keyboard state.
    <Show when={touchPrimary()}>
      <div
        ref={toolbarRef}
        class="mobile-keyboard-toolbar"
        data-mobile-keyboard-toolbar
        role="toolbar"
        aria-label={t("mobile-keyboard-toolbar-editor-toolbar")}
        hidden={!visible()}
        style={{ ...style(), display: visible() ? undefined : "none" }}
      >
        <Show when={props.navigation && visible()}>
          <div
            ref={navRef}
            class="mobile-keyboard-toolbar-nav"
            onPointerDown={navTap.onPointerDown}
            onPointerUp={navTap.onPointerUp}
            onPointerCancel={navTap.onPointerCancel}
            onMouseDown={keepEditorFocus}
          >
            <Dynamic component={props.navigation} />
          </div>
        </Show>
        <div class="mobile-keyboard-toolbar-strip" data-lenis-prevent>
          <For each={ACTIONS}>
            {(action) => {
              const tap = tapActivation(() => run(action));
              return (
              <button
                type="button"
                class="mobile-keyboard-toolbar-btn"
                classList={{ recording: action.icon === "mic" && isRecordingAudio() }}
                title={action.icon === "mic" && isRecordingAudio() ? t("mobile-keyboard-toolbar-stop-recording") : render(action.label)}
                aria-label={action.icon === "mic" && isRecordingAudio() ? t("mobile-keyboard-toolbar-stop-recording") : render(action.label)}
                onPointerDown={tap.onPointerDown}
                onPointerUp={tap.onPointerUp}
                onPointerCancel={tap.onPointerCancel}
                onMouseDown={keepEditorFocus}
                onClick={tap.onClick}
              >
                <Show
                  when={action.icon === "mic" && isRecordingAudio()}
                  fallback={<Icon name={action.icon} />}
                >
                  <Icon name="stop-recording" />
                </Show>
              </button>
              );
            }}
          </For>
        </div>
        <button
          type="button"
          class="mobile-keyboard-toolbar-btn mobile-keyboard-toolbar-hide"
          title={t("mobile-keyboard-toolbar-hide-keyboard")}
          aria-label={t("mobile-keyboard-toolbar-hide-keyboard")}
          onPointerDown={(e) => {
            beginHideGesture(e);
            hideTap.onPointerDown(e);
          }}
          onPointerUp={hideTap.onPointerUp}
          onPointerCancel={hideTap.onPointerCancel}
          onMouseDown={keepEditorFocus}
          onClick={hideTap.onClick}
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <rect x="3" y="5" width="18" height="10" rx="2" />
            <path d="M7 9h.01M11 9h.01M15 9h.01M8 12h8M8 18l4 3 4-3" />
          </svg>
        </button>
      </div>
    </Show>
  );
}
