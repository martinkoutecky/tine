// Configurable keyboard shortcuts. Defaults mirror OG Logseq command ids and
// bindings; users override them via config.edn `:shortcuts {:cmd "binding"}`
// (delivered in GraphMeta.shortcuts). "mod" = Ctrl (Cmd on macOS). Bindings may
// be a single chord ("mod+k") or a sequence of chords ("g j").
//
// Both the global dispatcher (this file) and the in-editor key handler
// (Block.tsx) resolve keys through the same merged binding table, so every
// listed command is remappable from config.edn.

import {
  openSwitcher,
  openCommandPalette,
  openDevtools,
  toggleTheme,
  toggleSidebar,
  openSettings,
  toggleHelpPopup,
  toggleRightSidebar,
  toggleWideMode,
  toggleDocumentMode,
  toggleFocusMode,
  toggleDimInactiveBlocks,
  focusMode,
  exitFocusMode,
  carryDays,
  pushToast,
  openPdfExport,
  pdfTarget,
  dismissMobileDrawer,
} from "./ui";
import { restoreDrawerFocus } from "./mobileDrawers";
import { dismissTopTransient } from "./transientLayers";
import { carryDaysBack } from "./carry";
import {
  openJournals,
  goBack,
  goForward,
  closeActiveTab,
  reopenClosedTab,
  activateNextTab,
  activatePrevTab,
  route,
} from "./router";
import {
  undo,
  redo,
  hasSelection,
  moveSelection,
  cycleSelectionTasks,
  moveSelectionItems,
  indentSelection,
  outdentSelection,
  deleteSelection,
  selectionMarkdown,
  clearSelection,
  selectedIds,
  blockIsGridView,
  doc,
  pageVisibleOrder,
  selectBlock,
  visibleOrder,
} from "./store";
import { editingId, startEditing } from "./editorController";
import { copyOutline } from "./clipboard";
import { openInPageFind } from "./inpageFind";
import { cellSel, enterGridSelection, handleCellSelectionKey, handleSheetPasteEvent, outlinedGridSelectionId } from "./sheet/selection";
import { decodeNavIntent } from "./navProtocol";
import {
  closePane,
  focusPane,
  focusedPaneId,
  layoutHasMultiplePanes,
  layoutPaneIds,
  layoutRoot,
  moveActiveTabToPane,
  paneRouter,
  splitPane,
  splitPaneAtSeam,
  splitRootAtEdge,
} from "./panes";
import {
  enterPaneSelect,
  exitPaneSelect,
  movePaneSelection,
  nearestPaneInDirection,
  paneSel,
  previousPaneSelectionTarget,
  readingOrderPanes,
  rememberBlockSelectionForPaneReturn,
  takeBlockSelectionForPaneReturn,
  type PaneDirection,
} from "./paneSelect";
import { openGuide } from "./guide";
import { pluginManager } from "./plugins/manager";
import { bindPluginBlockSnapshot, capturePluginGraphOwner, isPluginGraphOwnerCurrent, type OwnedPluginBlockSnapshot } from "./plugins/ownership";

function pluginFocusedBlock(): OwnedPluginBlockSnapshot | undefined {
  const owner = capturePluginGraphOwner();
  if (!owner) return undefined;
  const id = editingId();
  const node = id ? doc.byId[id] : undefined;
  if (!node) return undefined;
  let depth = 0;
  let parentId = node.parent;
  while (parentId && doc.byId[parentId] && depth < 1_000) {
    depth++;
    parentId = doc.byId[parentId].parent;
  }
  const format = doc.pages.find((page) => page.name === node.page)?.format === "org" ? "org" : "md";
  if (!isPluginGraphOwnerCurrent(owner)) return undefined;
  const owned = bindPluginBlockSnapshot({ id: node.id, raw: node.raw, parentId: node.parent, depth, format });
  if (!owned || owned.owner.graphRoot !== owner.graphRoot || owned.owner.generation !== owner.generation) return undefined;
  return owned;
}

interface Chord {
  mod: boolean;
  shift: boolean;
  alt: boolean;
  // The Super/Win key on Linux/Windows (distinct from `mod`); on macOS Cmd is
  // already `mod`, so `meta` is only meaningful off-Mac.
  meta: boolean;
  key: string; // lowercase, normalized
}

export type ShortcutScope = "global" | "editor" | "select";

interface CommandDef {
  id: string;
  binding: string;
  /** Human label for the Settings reference. */
  label: string;
  /** Global commands run from the window dispatcher; editor commands are
   *  matched by Block.tsx inside the textarea handler. */
  scope: "global" | "editor";
  run?: () => void;
  /** When false (default), a global command does not fire while typing in an
   *  editor unless its chord includes a modifier. */
  global?: boolean;
}

const isMac = typeof navigator !== "undefined" && /Mac/.test(navigator.platform);

function focusPaneByNumber(n: number) {
  if (!layoutHasMultiplePanes()) return;
  const pane = readingOrderPanes(layoutRoot())[n - 1];
  if (pane) focusPane(pane.paneId);
}

function focusPaneInDirection(dir: PaneDirection) {
  if (!layoutHasMultiplePanes()) return;
  const target = nearestPaneInDirection(layoutRoot(), focusedPaneId(), dir);
  if (target) focusPane(target);
}

function moveActiveTabInDirection(dir: PaneDirection) {
  if (!layoutHasMultiplePanes()) return;
  const target = nearestPaneInDirection(layoutRoot(), focusedPaneId(), dir);
  if (target) moveActiveTabToPane(focusedPaneId(), target);
}

function enterPaneSelectFromFocus() {
  const ids = layoutPaneIds();
  const focused = focusedPaneId();
  enterPaneSelect(ids.includes(focused) ? focused : ids[0] ?? "main");
}

function firstVisibleBlockInFocusedPane(): string | null {
  const currentRoute = paneRouter(focusedPaneId()).route();
  return currentRoute.kind === "page"
    ? pageVisibleOrder(currentRoute.name)[0] ?? null
    : visibleOrder()[0] ?? null;
}

function restoreBlockSelectionAfterPaneReturn(previous: string | null) {
  if (hasSelection()) return;
  const target = previous && doc.byId[previous] ? previous : firstVisibleBlockInFocusedPane();
  if (target) selectBlock(target);
}

// Materialize a split at the selected seam/edge. Two flavors (Martin's Jul 8
// ruling): Enter = a plain MIRROR split (the new pane keeps the duplicated
// content, no dialog — the quick "same thing side by side"); typing = an
// embryo split with the switcher prefilled with the typed char (the quick
// "open/create THAT page in a new split"; embryo mode already hides commands).
function materializePaneSelection(prefill: string | null) {
  const target = paneSel();
  if (!target || target.kind === "pane") return;
  const source = previousPaneSelectionTarget() ?? focusedPaneId();
  const paneId =
    target.kind === "seam"
      ? splitPaneAtSeam(target.path, source)
      : target.kind === "pane-edge"
        ? // Split ONLY that pane, new pane on the chosen side.
          splitPane(target.paneId, target.side === "left" || target.side === "right" ? "row" : "col", {
            position: target.side === "left" || target.side === "top" ? "before" : "after",
          })
        : splitRootAtEdge(target.side, source);
  if (!paneId) return;
  exitPaneSelect();
  if (prefill === null) focusPane(paneId); // mirror split — done
  else openSwitcher({ mode: "embryo", paneId, prefill });
}

// Exported for src/navModel.contract.test.ts — the shared nav-model invariants
// (ADR 0034) drive this handler and the sheet's handleCellSelectionKey with the
// same key sequences.
export function handlePaneSelectKey(e: KeyboardEvent): boolean {
  const target = paneSel();
  if (!target) return false;
  // Ctrl+K on a seam/edge = split with an empty embryo switcher (same gesture
  // as typing, for people who reach for Ctrl+K by reflex). On a pane target it
  // falls through to the global handler, which now acts on the selected pane.
  // A mod-chord, so it is a surface command, not nav — handled before decoding.
  if (matchesCommand(e, "go/search")) {
    if (target.kind !== "pane") {
      materializePaneSelection("");
      return true;
    }
    exitPaneSelect(); // switcher takes over; a lingering ring would lie
    return false;
  }
  const intent = decodeNavIntent(e);
  if (!intent) return false;
  switch (intent.kind) {
    // Span extension (tree-aligned widening rungs) is backlogged; until it
    // exists, shift+arrow steps like a plain arrow rather than going dead.
    case "extend":
    case "step": {
      const next = movePaneSelection(layoutRoot(), intent.dir);
      // Focus follows pane selection: Ctrl+K (and every focused-pane command)
      // acts on the pane you SEE selected, not a stale earlier focus.
      if (next.kind === "pane") focusPane(next.paneId);
      return true;
    }
    case "dismiss":
      const previous = takeBlockSelectionForPaneReturn();
      exitPaneSelect();
      restoreBlockSelectionAfterPaneReturn(previous);
      return true;
    case "activate":
      if (target.kind === "pane") {
        const previous = takeBlockSelectionForPaneReturn();
        exitPaneSelect();
        focusPane(target.paneId);
        restoreBlockSelectionAfterPaneReturn(previous);
      } else {
        materializePaneSelection(null); // Enter on a seam/edge = mirror split
      }
      return true;
    case "remove":
      if (target.kind !== "pane") return false;
      if (closePane(target.paneId)) enterPaneSelect(focusedPaneId()); // stay in the mode on the survivor
      return true;
    case "overtype":
      if (target.kind !== "pane") materializePaneSelection(intent.char);
      return true; // swallow stray typing even on a pane target
  }
}

// Default command table. Editor command ids mirror OG Logseq where practical.
const COMMANDS: CommandDef[] = [
  { id: "go/search", binding: "mod+k", label: "Search / quick switch", scope: "global", run: () => openSwitcher({ pluginBlock: pluginFocusedBlock() ?? null }), global: true },
  { id: "go/search-current-page", binding: "mod+shift+k", label: "Search blocks in current page", scope: "global", run: () => openSwitcher({ mode: "current-page", pluginBlock: pluginFocusedBlock() ?? null }), global: true },
  { id: "guide/open", binding: "", label: "Open Guide", scope: "global", run: () => void openGuide(), global: true },
  { id: "go/find-in-page", binding: "mod+f", label: "Find in page", scope: "global", run: openInPageFind, global: true },
  { id: "command-palette/toggle", binding: "mod+shift+p", label: "Command palette", scope: "global", run: () => openCommandPalette(pluginFocusedBlock() ?? null), global: true },
  // Toggle the WebKit Web Inspector for theme/CSS debugging (GH #31). The usual
  // Ctrl+Shift+I / F12 / Ctrl+Shift+C are all swallowed by WebKitGTK itself (its
  // built-in inspector keys, handled in the web process below where the app can
  // intercept), so they never reach this dispatcher. Ctrl+Shift+J — Chrome's other
  // devtools shortcut (console) — is NOT grabbed by WebKit, so it works here. A
  // mod-chord, so it fires even while editing; remap it in Settings if you like.
  { id: "ui/toggle-devtools", binding: "mod+shift+j", label: "Toggle developer tools", scope: "global", run: openDevtools, global: true },
  { id: "go/journals", binding: "g j", label: "Go to journals", scope: "global", run: openJournals },
  { id: "go/keyboard-shortcuts", binding: "g s", label: "Go to keyboard shortcuts", scope: "global", run: () => openSettings("shortcuts") },
  // Browser-style history nav (per-tab back/forward). Special-cased in the
  // dispatcher so they fire even while editing a block; remappable like any other.
  { id: "go/backward", binding: "alt+left", label: "Go back", scope: "global", run: goBack, global: true },
  { id: "go/forward", binding: "alt+right", label: "Go forward", scope: "global", run: goForward, global: true },
  // mod+w is a mod-chord, so it clears the while-editing guard and reaches the
  // generic dispatch loop below — closes the active tab even mid-edit (like a
  // browser). The last tab can't be closed, so this never quits the app.
  { id: "tab/close", binding: "mod+w", label: "Close current tab", scope: "global", run: closeActiveTab, global: true },
  // Reopen the most-recently-closed tab (browser-style). mod-chord ⇒ fires even
  // mid-edit, like mod+w.
  { id: "tab/reopen-closed", binding: "mod+shift+t", label: "Reopen closed tab", scope: "global", run: reopenClosedTab, global: true },
  // Browser-style tab cycling (Ctrl+PgDn / Ctrl+PgUp on Linux/Windows; Cmd on
  // macOS). mod-chords, so they fire mid-edit; remappable like everything here.
  { id: "tab/next", binding: "mod+pagedown", label: "Next tab", scope: "global", run: activateNextTab, global: true },
  { id: "tab/previous", binding: "mod+pageup", label: "Previous tab", scope: "global", run: activatePrevTab, global: true },
  { id: "pane/split-right", binding: "mod+alt+\\", label: "Split right", scope: "global", run: () => void splitPane(focusedPaneId(), "row"), global: true },
  { id: "pane/split-down", binding: "mod+alt+shift+\\", label: "Split down", scope: "global", run: () => void splitPane(focusedPaneId(), "col"), global: true },
  { id: "pane/close", binding: "", label: "Close pane", scope: "global", run: () => void closePane(focusedPaneId()), global: true },
  // Palette-discoverable entry into pane-select (it's otherwise only reachable
  // via Esc-with-nothing-open, which users won't guess — Martin didn't).
  { id: "pane/select-mode", binding: "", label: "Pane select mode (arrows move, Enter opens/splits)", scope: "global", run: enterPaneSelectFromFocus, global: true },
  ...Array.from({ length: 9 }, (_, i): CommandDef => ({
    id: `pane/focus-${i + 1}`,
    binding: `mod+${i + 1}`,
    label: `Focus pane ${i + 1}`,
    scope: "global",
    run: () => focusPaneByNumber(i + 1),
    global: true,
  })),
  { id: "pane/focus-left", binding: "mod+alt+left", label: "Focus pane left", scope: "global", run: () => focusPaneInDirection("left"), global: true },
  { id: "pane/focus-right", binding: "mod+alt+right", label: "Focus pane right", scope: "global", run: () => focusPaneInDirection("right"), global: true },
  { id: "pane/focus-up", binding: "mod+alt+up", label: "Focus pane up", scope: "global", run: () => focusPaneInDirection("up"), global: true },
  { id: "pane/focus-down", binding: "mod+alt+down", label: "Focus pane down", scope: "global", run: () => focusPaneInDirection("down"), global: true },
  { id: "pane/move-tab-left", binding: "mod+alt+shift+left", label: "Move tab to pane left", scope: "global", run: () => moveActiveTabInDirection("left"), global: true },
  { id: "pane/move-tab-right", binding: "mod+alt+shift+right", label: "Move tab to pane right", scope: "global", run: () => moveActiveTabInDirection("right"), global: true },
  { id: "pane/move-tab-up", binding: "mod+alt+shift+up", label: "Move tab to pane up", scope: "global", run: () => moveActiveTabInDirection("up"), global: true },
  { id: "pane/move-tab-down", binding: "mod+alt+shift+down", label: "Move tab to pane down", scope: "global", run: () => moveActiveTabInDirection("down"), global: true },
  { id: "ui/toggle-theme", binding: "t t", label: "Toggle dark / light", scope: "global", run: toggleTheme },
  { id: "ui/toggle-left-sidebar", binding: "t l", label: "Toggle left sidebar", scope: "global", run: toggleSidebar },
  { id: "ui/toggle-right-sidebar", binding: "t r", label: "Toggle right sidebar", scope: "global", run: toggleRightSidebar },
  { id: "ui/open-settings", binding: "t s", label: "Open settings", scope: "global", run: openSettings },
  {
    id: "page/export-pdf",
    binding: "",
    label: "Export current page to PDF…",
    scope: "global",
    run: () => {
      const r = route();
      if (r.kind === "page") openPdfExport(r.name);
      else pushToast("Open a page first to export it to PDF", "warn");
    },
  },
  { id: "ui/toggle-wide-mode", binding: "t w", label: "Toggle wide mode", scope: "global", run: toggleWideMode },
  { id: "ui/toggle-document-mode", binding: "t d", label: "Toggle document mode", scope: "global", run: toggleDocumentMode },
  { id: "ui/toggle-focus-mode", binding: "t f", label: "Toggle focus mode", scope: "global", run: toggleFocusMode },
  { id: "ui/toggle-dim-blocks", binding: "t b", label: "Toggle dim inactive blocks", scope: "global", run: toggleDimInactiveBlocks },
  // Web KeyboardEvent reports Shift+/ as key "?", so the binding must use the
  // shifted character. OG stores "shift+/" and rewrites it for display; Tine
  // matches eventToChord's actual output instead.
  { id: "ui/toggle-help", binding: "shift+?", label: "Toggle help", scope: "global", run: toggleHelpPopup },
  // Carry unfinished tasks forward. Palette-only (no default binding); the presets
  // and the settings-configured N are all surfaced in Ctrl-K.
  { id: "task/carry-7", binding: "", label: "Carry unfinished tasks: last 7 days", scope: "global", run: () => void carryDaysBack(7) },
  { id: "task/carry-30", binding: "", label: "Carry unfinished tasks: last 30 days", scope: "global", run: () => void carryDaysBack(30) },
  { id: "task/carry-365", binding: "", label: "Carry unfinished tasks: last 365 days", scope: "global", run: () => void carryDaysBack(365) },
  { id: "task/carry-n", binding: "", label: "Carry unfinished tasks: last N days (Settings)", scope: "global", run: () => void carryDaysBack(carryDays()) },
  { id: "editor/undo", binding: "mod+z", label: "Undo", scope: "global", run: undo, global: true },
  { id: "editor/redo", binding: "mod+shift+z", label: "Redo", scope: "global", run: redo, global: true },
  // Editor commands (resolved in Block.tsx / selection handler).
  { id: "editor/indent", binding: "tab", label: "Indent block", scope: "editor" },
  { id: "editor/outdent", binding: "shift+tab", label: "Outdent block", scope: "editor" },
  // OG's non-Mac default (alt+shift+arrow). Super+arrow (the old "meta+up"
  // binding) is grabbed by most Linux compositors for window tiling, so it never
  // reached the app.
  { id: "editor/move-block-up", binding: "alt+shift+up", label: "Move block up", scope: "editor" },
  { id: "editor/move-block-down", binding: "alt+shift+down", label: "Move block down", scope: "editor" },
  { id: "editor/collapse", binding: "mod+up", label: "Collapse block", scope: "editor" },
  { id: "editor/expand", binding: "mod+down", label: "Expand block", scope: "editor" },
  { id: "editor/select-block-up", binding: "shift+up", label: "Select block up", scope: "editor" },
  { id: "editor/select-block-down", binding: "shift+down", label: "Select block down", scope: "editor" },
  { id: "editor/cycle-todo", binding: "mod+enter", label: "Cycle TODO / DOING / DONE", scope: "editor" },
  // Quick-capture mini-window only: file the capture to today's journal. Acts
  // only when CaptureCtx is present (Block.tsx); a no-op in the main app. Default
  // mod+shift+enter avoids mod+enter (cycle-todo) and shift+enter (soft newline).
  { id: "editor/quick-capture-file", binding: "mod+shift+enter", label: "Quick-capture: file to today's journal", scope: "editor" },
  // Inline formatting toggles.
  { id: "editor/bold", binding: "mod+b", label: "Bold", scope: "editor" },
  { id: "editor/italics", binding: "mod+i", label: "Italic", scope: "editor" },
  { id: "editor/strike-through", binding: "mod+shift+s", label: "Strikethrough", scope: "editor" },
  { id: "editor/highlight", binding: "mod+shift+h", label: "Highlight", scope: "editor" },
  { id: "editor/insert-link", binding: "mod+l", label: "Insert link", scope: "editor" },
  { id: "editor/clear-block", binding: "alt+l", label: "Clear block content", scope: "editor" },
  // Emacs-style cursor/kill motions.
  { id: "editor/kill-line-before", binding: "alt+u", label: "Delete to line start", scope: "editor" },
  { id: "editor/kill-line-after", binding: "alt+k", label: "Delete to line end", scope: "editor" },
  { id: "editor/backward-word", binding: "alt+b", label: "Cursor word backward", scope: "editor" },
  { id: "editor/forward-word", binding: "alt+f", label: "Cursor word forward", scope: "editor" },
  { id: "editor/backward-kill-word", binding: "alt+w", label: "Delete word backward", scope: "editor" },
  { id: "editor/forward-kill-word", binding: "alt+d", label: "Delete word forward", scope: "editor" },
];

export interface BuiltinKeyDef {
  id: string;
  scope: ShortcutScope;
  binding: string;
  label: string;
  details?: string;
}

// Hardcoded keys that are intentionally not remappable. Keep this next to
// COMMANDS so shortcut behavior and shortcut documentation drift together.
export const BUILTIN_KEYS: BuiltinKeyDef[] = [
  {
    id: "builtin/global/escape",
    scope: "global",
    binding: "esc",
    label: "Close overlays or exit focus",
    details: "Closes Search / Settings first; outside overlays, peels off block selection and focus mode.",
  },
  {
    id: "builtin/editor/enter",
    scope: "editor",
    binding: "enter",
    label: "New block or continue list",
    details: "Splits the current block, continues an in-block list, or adds a sibling note under PDF annotations.",
  },
  {
    id: "builtin/editor/soft-newline",
    scope: "editor",
    binding: "shift+enter",
    label: "Insert a newline inside the block",
  },
  {
    id: "builtin/editor/escape",
    scope: "editor",
    binding: "esc",
    label: "Leave editing and select the block",
  },
  {
    id: "builtin/editor/backspace-start",
    scope: "editor",
    binding: "backspace",
    label: "Merge at block start",
    details: "At the start of a block, merges with the previous block or removes an empty block/list marker.",
  },
  {
    id: "builtin/editor/arrow-cross-block",
    scope: "editor",
    binding: "up / down",
    label: "Move across blocks at visual edges",
    details: "From the first or last visual row, moves the caret to the previous or next visible block.",
  },
  {
    id: "builtin/editor/copy-block-ref",
    scope: "editor",
    binding: "mod+c",
    label: "Copy block reference when no text is selected",
  },
  {
    id: "builtin/editor/ac-nav",
    scope: "editor",
    binding: "up / down",
    label: "Autocomplete: move highlight",
  },
  {
    id: "builtin/editor/ac-accept",
    scope: "editor",
    binding: "enter / tab / shift+tab",
    label: "Autocomplete: accept highlighted item",
  },
  {
    id: "builtin/editor/ac-close",
    scope: "editor",
    binding: "esc",
    label: "Autocomplete: close the popup",
  },
  {
    id: "builtin/select/escape",
    scope: "select",
    binding: "esc",
    label: "Clear block selection",
  },
  {
    id: "builtin/select/move",
    scope: "select",
    binding: "up / down",
    label: "Move the selected block range",
  },
  {
    id: "builtin/select/extend",
    scope: "select",
    binding: "shift+up / shift+down",
    label: "Extend block selection",
  },
  {
    id: "builtin/select/delete",
    scope: "select",
    binding: "backspace / delete",
    label: "Delete selected blocks",
  },
  {
    id: "builtin/select/copy",
    scope: "select",
    binding: "mod+c",
    label: "Copy selected outline",
  },
  {
    id: "builtin/select/cut",
    scope: "select",
    binding: "mod+x",
    label: "Cut selected outline",
  },
  {
    id: "builtin/select/edit",
    scope: "select",
    binding: "enter",
    label: "Edit the last selected block",
  },
  {
    id: "builtin/sheet/select-cell",
    scope: "select",
    binding: "click",
    label: "Select a sheet cell",
    details: "Double-click, Enter, or F2 edits the selected cell.",
  },
  {
    id: "builtin/sheet/move",
    scope: "select",
    binding: "arrow keys",
    label: "Move sheet cell selection",
    details: "Esc leaves a sheet; typing replaces the focused cell.",
  },
  {
    id: "builtin/sheet/range",
    scope: "select",
    binding: "drag / shift+click / shift+arrows",
    label: "Select a sheet range",
  },
  {
    id: "builtin/sheet/copy-cut",
    scope: "select",
    binding: "mod+c / mod+x / paste",
    label: "Copy, cut, or paste sheet cells",
  },
  {
    id: "builtin/sheet/fill",
    scope: "select",
    binding: "mod+d / mod+r",
    label: "Fill sheet selection down or right",
  },
];

function shortcutScope(c: CommandDef): ShortcutScope {
  if (c.scope === "editor") return "editor";
  if (!c.global && c.binding.trim()) return "select";
  return "global";
}

function pluginCommandDefs(): CommandDef[] {
  return pluginManager.commands().map(({ pluginId, contribution }) => ({
    id: `plugin:${pluginId}:${contribution.id}`,
    binding: contribution.defaultBinding ?? "",
    label: `Plugin: ${contribution.title}`,
    scope: "global",
    global: true,
    run: () => {
      void pluginManager
        .invokeCommand(pluginId, contribution.id, pluginFocusedBlock())
        .catch((error) => pushToast(`Plugin command failed: ${String(error)}`, "error"));
    },
  }));
}

function normKey(k: string): string {
  switch (k) {
    case "arrowup": return "up";
    case "arrowdown": return "down";
    case "arrowleft": return "left";
    case "arrowright": return "right";
    case "escape": return "esc";
    case " ":
    case "spacebar": return "space";
    default: return k;
  }
}

// A bare modifier-key press (so chord recording / sequence matching keeps
// waiting for the real key). Covers Super/Windows — WebKitGTK reports its
// `key` as "Super"/"OS", not "Meta" — plus the standard names; also matches by
// `code` as a fallback.
const MODIFIER_KEYS = new Set([
  "control", "shift", "alt", "meta", "super", "hyper", "os", "altgraph", "capslock",
]);
function isModifierKey(e: KeyboardEvent): boolean {
  if (MODIFIER_KEYS.has(e.key.toLowerCase())) return true;
  return /^(Control|Shift|Alt|Meta|OS|Super|Hyper)(Left|Right)?$/.test(e.code);
}

function parseChord(s: string): Chord {
  const parts = s.toLowerCase().split("+");
  const chord: Chord = { mod: false, shift: false, alt: false, meta: false, key: "" };
  for (const p of parts) {
    if (p === "mod" || p === "ctrl" || p === "cmd") chord.mod = true;
    else if (p === "meta" || p === "super" || p === "win") chord.meta = true;
    else if (p === "shift") chord.shift = true;
    else if (p === "alt" || p === "option") chord.alt = true;
    else chord.key = normKey(p);
  }
  return chord;
}

function parseBinding(b: string): Chord[] {
  return b.trim().split(/\s+/).map(parseChord);
}

// Is this the Super/Windows key itself? (its own keydown/keyup, used to track
// held state — WebKitGTK/Wayland doesn't reliably set e.metaKey for it.)
function isSuperKey(e: KeyboardEvent): boolean {
  const k = e.key.toLowerCase();
  return k === "super" || k === "os" || k === "meta" || /^(Meta|OS|Super)(Left|Right)?$/.test(e.code);
}
// Whether the Super key is currently held. Maintained from keydown/keyup because
// e.metaKey is unreliable on this platform; reset on blur so a missed keyup
// (e.g. the WM grabbed the combo) doesn't leave it stuck.
let superDown = false;

function eventToChord(e: KeyboardEvent): Chord {
  // WebKitGTK reports Shift+Tab with e.key != "Tab"; e.code is reliable.
  let key = e.code === "Tab" ? "tab" : e.key.toLowerCase();
  // Real WebKitGTK reports Shift+/ as key "?"; Playwright/Chromium's synthetic
  // Shift+/ reports key "/". Normalize both to the binding string we expose.
  if (e.shiftKey && e.code === "Slash" && key === "/") key = "?";
  key = normKey(key);
  return {
    mod: isMac ? e.metaKey : e.ctrlKey,
    shift: e.shiftKey,
    alt: e.altKey,
    // Super/Win on non-Mac (on Mac, metaKey is already `mod`). Fall back to the
    // tracked held state when the event's own metaKey flag is missing.
    meta: !isMac && (e.metaKey || superDown),
    key,
  };
}

/** WebKitGTK can report Shift+Tab with a non-Tab key, but preserves its code. */
export function isTabLikeEvent(e: KeyboardEvent, chord = eventToChord(e)): boolean {
  return e.code === "Tab" || chord.key === "tab";
}

/** Bare Tab permits Shift but declines every platform modifier.
 *
 * Raw Control is deliberate: on macOS eventToChord maps `mod` from Meta, so
 * Ctrl+Tab must not normalize into a plain editor Tab. */
export function isPermittedTabGesture(e: KeyboardEvent, chord = eventToChord(e)): boolean {
  return isTabLikeEvent(e, chord) && !e.ctrlKey && !chord.mod && !chord.alt && !chord.meta;
}

function chordEq(a: Chord, b: Chord): boolean {
  return a.mod === b.mod && a.shift === b.shift && a.alt === b.alt && a.meta === b.meta && a.key === b.key;
}

// Merged binding table (defaults + config overrides), populated by
// installKeybindings and consulted by matchesCommand.
let bindings: Record<string, Chord[]> = {};
let overridesApplied: Record<string, string> = {};

// When true, the global dispatcher ignores all keys — used while the Settings
// modal is recording a new binding so chords like "mod+k" get captured instead
// of firing their command.
let suspended = false;
export function setKeybindingsSuspended(b: boolean) {
  suspended = b;
}

/** Does the event match the configured binding for `id`? (single-chord only). */
export function matchesCommand(e: KeyboardEvent, id: string): boolean {
  const cs = bindings[id];
  if (!cs || cs.length !== 1) return false;
  return chordEq(eventToChord(e), cs[0]);
}

/** The editor-scoped command id whose configured (single-chord) binding matches
 *  the event, or null. Lets the block editor dispatch via a handler table instead
 *  of a long sequence of matchesCommand checks. */
export function editorCommandFor(e: KeyboardEvent): string | null {
  const chord = eventToChord(e);
  for (const c of COMMANDS) {
    if (c.scope !== "editor") continue;
    const cs = bindings[c.id];
    if (cs && cs.length === 1 && chordEq(chord, cs[0])) return c.id;
  }
  return null;
}

/** Runnable global commands for the command palette / Ctrl-K Commands group:
 *  every global command with a run handler, with its effective binding. The
 *  switcher itself is excluded (no point launching the launcher). */
export function paletteCommands(
  focusedPluginBlock: OwnedPluginBlockSnapshot | null = pluginFocusedBlock() ?? null
): { id: string; label: string; binding: string; run: () => void }[] {
  const builtIn = COMMANDS.filter((c) => c.scope === "global" && c.run && c.id !== "go/search")
    .map((c) => ({
      id: c.id,
      label: c.label,
      binding: overridesApplied[c.id] ?? c.binding,
      run: c.run!,
    }))
    .filter((c) => c.binding !== "false");
  const plugins = pluginManager.commands().map(({ pluginId, contribution }) => ({
    id: `plugin:${pluginId}:${contribution.id}`,
    label: contribution.title,
    binding: overridesApplied[`plugin:${pluginId}:${contribution.id}`] ?? contribution.defaultBinding ?? "",
    run: () => {
      void pluginManager
        .invokeCommand(pluginId, contribution.id, focusedPluginBlock ?? undefined)
        .catch((error) => pushToast(`Plugin command failed: ${String(error)}`, "error"));
    },
  }));
  return [...builtIn, ...plugins];
}

export function runGlobalCommand(id: string): boolean {
  const cmd = COMMANDS.find((c) => c.id === id && c.scope === "global" && c.run);
  if (!cmd?.run) return false;
  cmd.run();
  return true;
}

/** Merged shortcuts for the Settings reference. */
export function currentShortcuts(): { id: string; label: string; binding: string; scope: ShortcutScope }[] {
  return [...COMMANDS, ...pluginCommandDefs()].map((c) => ({
    id: c.id,
    label: c.label,
    binding: overridesApplied[c.id] ?? c.binding,
    scope: shortcutScope(c),
  })).filter((c) => c.binding !== "false");
}

/** Built-in command defaults (id + label + default binding) for the Settings
 *  remap UI, which computes the effective binding reactively from these plus
 *  config.edn and the user's local overrides. */
export function commandDefaults(): { id: string; label: string; binding: string; scope: ShortcutScope }[] {
  return [...COMMANDS, ...pluginCommandDefs()].map((c) => ({ id: c.id, label: c.label, binding: c.binding, scope: shortcutScope(c) }));
}

/** Turn a keyboard event into a binding string like "mod+shift+down". Returns
 *  null for a bare modifier press (keep waiting). */
export function eventToBindingString(e: KeyboardEvent): string | null {
  if (isModifierKey(e)) return null;
  const c = eventToChord(e);
  if (!c.key) return null;
  const parts: string[] = [];
  if (c.mod) parts.push("mod");
  if (c.meta) parts.push("super"); // the Super/Windows key (clearer than "meta")
  if (c.alt) parts.push("alt");
  if (c.shift) parts.push("shift");
  parts.push(c.key);
  return parts.join("+");
}

function isEditableTarget(t: EventTarget | null): boolean {
  const el = t as HTMLElement | null;
  if (!el) return false;
  const tag = el.tagName;
  return tag === "TEXTAREA" || tag === "INPUT" || el.isContentEditable;
}

function isOutlineBlockEditorTarget(t: EventTarget | null): boolean {
  const el = t as { tagName?: unknown; classList?: { contains?: (name: string) => boolean } } | null;
  return el?.tagName === "TEXTAREA" && el.classList?.contains?.("block-editor") === true;
}

function focusedGridSurface(gridId: string): string | null {
  const paneId = focusedPaneId();
  const expected = paneId === "main" ? "main" : `pane:${paneId}`;
  if (typeof document === "undefined") return expected;
  const esc = (value: string) =>
    typeof CSS !== "undefined" && CSS.escape ? CSS.escape(value) : value.replace(/["\\]/g, "\\$&");
  const pane = document.querySelector<HTMLElement>(`[data-pane-id="${esc(paneId)}"]`);
  if (!pane) return expected;
  const surfaces = new Set(
    [...pane.querySelectorAll<HTMLElement>(`[data-sheet-grid-id="${esc(gridId)}"][data-sheet-surface-id]`)]
      .map((grid) => grid.dataset.sheetSurfaceId)
      .filter((surface): surface is string => !!surface)
  );
  return surfaces.size === 1 ? [...surfaces][0] : null;
}

// Keyboard handling while in block-selection mode (no editor focused).
function handleSelectionKey(e: KeyboardEvent): boolean {
  if (e.key === "Escape") return clearSelection(), true;
  // Resolve the configured command before generic Enter handling. This keeps
  // Ctrl/Cmd+Enter (and user remaps) in block-selection mode instead of opening
  // the last selected block's editor.
  if (matchesCommand(e, "editor/cycle-todo")) return cycleSelectionTasks(), true;
  if (matchesCommand(e, "editor/outdent")) return outdentSelection(), true;
  if (matchesCommand(e, "editor/indent")) return indentSelection(), true;
  if (matchesCommand(e, "editor/move-block-down")) return moveSelectionItems(1), true;
  if (matchesCommand(e, "editor/move-block-up")) return moveSelectionItems(-1), true;
  if (e.key === "Enter" || e.key === "ArrowRight") {
    const ids = selectedIds();
    const gridId = ids.length === 1 && blockIsGridView(ids[0]) ? ids[0] : ids.length === 0 ? outlinedGridSelectionId() : null;
    if (gridId) {
      const surface = focusedGridSurface(gridId);
      if (surface !== null && enterGridSelection(gridId, surface)) return true;
    }
  }
  if (e.key === "ArrowDown") return moveSelection(1, e.shiftKey), true;
  if (e.key === "ArrowUp") return moveSelection(-1, e.shiftKey), true;
  if (e.key === "Backspace" || e.key === "Delete") return deleteSelection(), true;
  const mod = isMac ? e.metaKey : e.ctrlKey;
  if (mod && e.key.toLowerCase() === "c") return void copyOutline(selectionMarkdown()), true;
  if (mod && e.key.toLowerCase() === "x") {
    void copyOutline(selectionMarkdown());
    deleteSelection();
    return true;
  }
  if (e.key === "Enter") {
    const ids = selectedIds();
    const last = ids[ids.length - 1];
    if (last) startEditing(last, 1e9);
    return true;
  }
  return false;
}

export interface Command {
  id: string;
  binding: string;
  run: () => void;
  global?: boolean;
}

/** Install the global shortcut handler, merging config overrides over defaults.
 *  Returns a disposer. */
export function installKeybindings(overrides: Record<string, string> = {}): () => void {
  overridesApplied = overrides;
  bindings = {};
  const allCommands = [...COMMANDS, ...pluginCommandDefs()];
  for (const c of allCommands) {
    const b = overrides[c.id] ?? c.binding;
    if (b !== "false") bindings[c.id] = parseBinding(b);
  }

  // Global dispatch list (sequences + global chords).
  const commands = allCommands.filter((c) => c.scope === "global" && c.run)
    .map((c) => ({ ...c, chords: bindings[c.id] }))
    .filter((c) => c.chords);

  let seq: Chord[] = [];
  let seqTimer: ReturnType<typeof setTimeout> | null = null;
  const resetSeq = () => {
    seq = [];
    if (seqTimer) clearTimeout(seqTimer);
    seqTimer = null;
  };

  const handler = (e: KeyboardEvent) => {
    // IME composition owns Escape.  It must not fall through to a transient,
    // shortcut recorder, editor, or pane handler.
    if (e.isComposing || e.keyCode === 229) return;
    // Ignore bare modifier presses (incl. Super/Windows).
    if (isModifierKey(e)) return;

    const chord = eventToChord(e);
    const editing = isEditableTarget(e.target);

    // Escape, in priority order, so focus mode peels off one layer at a time
    // (Logseq-like): overlays first; then if editing a block's text let the
    // editor exit text-editing (don't exit focus yet); then a selected block
    // deselects (and in focus mode that same Esc exits focus); else exit focus.
    // Net: editing → Esc (to block-select) → Esc (exit focus) = twice, not once.
    if (e.key === "Escape") {
      if (dismissTopTransient("escape")) {
        e.preventDefault(); e.stopImmediatePropagation(); resetSeq(); return;
      }
      if (dismissMobileDrawer("escape")) {
        restoreDrawerFocus("escape");
        e.preventDefault(); e.stopImmediatePropagation(); resetSeq(); return;
      }
      // Settings shortcut recording suspends only the ordinary shortcut/editor/
      // pane ladder. Escape's transient/drawer prefix above remains available,
      // but an unconsumed Escape must be stopped at capture so it cannot reach a
      // target-local editor handler.
      if (suspended) {
        e.preventDefault(); e.stopImmediatePropagation(); resetSeq(); return;
      }
      if (paneSel() && !editing && handlePaneSelectKey(e)) {
        e.preventDefault(); resetSeq(); return;
      }
      if (editing) return; // defer to the editor's own Esc (capture phase)
      if (cellSel() && handleCellSelectionKey(e)) {
        e.preventDefault();
        resetSeq();
        return;
      }
      if (hasSelection()) {
        const previous = selectedIds().at(-1) ?? null;
        if (!focusMode()) rememberBlockSelectionForPaneReturn(previous);
        clearSelection();
        // Martin's 2-rung ladder (Jul 8): block-select climbs STRAIGHT to
        // pane-select — the old "cleared but nothing selected" state between
        // them was an invisible dead rung. Focus mode still peels first.
        if (focusMode()) void exitFocusMode();
        else enterPaneSelectFromFocus();
        e.preventDefault();
        resetSeq();
        return;
      }
      if (focusMode()) {
        void exitFocusMode();
        e.preventDefault();
        resetSeq();
        return;
      }
      enterPaneSelectFromFocus();
      e.preventDefault();
      resetSeq();
      return;
    }

    // Settings shortcut recording still declines non-Escape input so its own
    // capture listener can record ordinary chords.
    if (suspended) return;

    // !editing guard: pane-select must NEVER eat ordinary keys while a text
    // field has focus. Escape itself was handled after transient/drawer.
    if (paneSel() && !editing && handlePaneSelectKey(e)) {
      e.preventDefault();
      resetSeq();
      return;
    }

    // Browser-style history nav, handled BEFORE the while-editing guard so
    // Alt+Left / Alt+Right work from inside a block too (like a browser navigating
    // from a focused field). preventDefault stops the textarea's own alt-arrow.
    if (matchesCommand(e, "go/backward")) {
      e.preventDefault();
      resetSeq();
      goBack();
      return;
    }
    if (matchesCommand(e, "go/forward")) {
      e.preventDefault();
      resetSeq();
      goForward();
      return;
    }

    // Cell-selection mode keys (no editor focused).
    if (!editing && cellSel()) {
      if (handleCellSelectionKey(e)) {
        e.preventDefault();
        resetSeq();
        return;
      }
    }

    // Block-selection mode keys (no editor focused).
    if (!editing && hasSelection()) {
      if (handleSelectionKey(e)) {
        e.preventDefault();
        resetSeq();
        return;
      }
    }

    // While typing, only modifier chords are eligible (so "g j" doesn't fire).
    if (editing && !chord.mod) {
      // Cancel GTK/browser focus traversal on Tab/Shift+Tab in the capture
      // phase (WebKitGTK grabs it before an outline editor can), but still let
      // that editor receive its owned gesture. Native form controls retain
      // their browser focus traversal and blur behavior.
      if (isOutlineBlockEditorTarget(e.target) && isPermittedTabGesture(e, chord)) e.preventDefault();
      resetSeq();
      return;
    }

    seq.push(chord);
    if (seqTimer) clearTimeout(seqTimer);
    seqTimer = setTimeout(resetSeq, 800);

    for (const cmd of commands) {
      const cs = cmd.chords;
      if (cs.length > seq.length) continue;
      const tail = seq.slice(seq.length - cs.length);
      if (cs.every((c, i) => chordEq(c, tail[i]))) {
        if (cmd.id === "go/find-in-page" && pdfTarget()) {
          resetSeq();
          return;
        }
        e.preventDefault();
        resetSeq();
        cmd.run!();
        return;
      }
    }
  };

  // Track the Super key's held state separately (e.metaKey is unreliable here).
  // Runs even while the dispatcher is suspended (shortcut recording) so a
  // Super+key chord can be captured.
  const superTracker = (e: KeyboardEvent) => {
    if (isSuperKey(e)) superDown = e.type === "keydown";
  };
  const clearSuper = () => {
    superDown = false;
  };
  const pasteHandler = (e: ClipboardEvent) => {
    if (isEditableTarget(e.target)) return;
    if (!cellSel()) return;
    if (handleSheetPasteEvent(e)) e.preventDefault();
  };
  type MouseHistoryDirection = "back" | "forward";
  type MouseHistorySource = "dom" | "native";
  let lastMouseHistory: { direction: MouseHistoryDirection; source: MouseHistorySource; at: number } | undefined;
  const navigateMouseHistory = (direction: MouseHistoryDirection, source: MouseHistorySource) => {
    const now = performance.now();
    // A platform that exposes both its native command and DOM auxclick must not
    // advance twice for one physical release. Repeated events from the same
    // source remain distinct clicks and are never collapsed.
    if (lastMouseHistory?.direction === direction && lastMouseHistory.source !== source && now - lastMouseHistory.at < 100) {
      return;
    }
    lastMouseHistory = { direction, source, at: now };
    if (direction === "back") goBack();
    else goForward();
  };
  // Mouse side buttons: X1 (DOM button 3) navigates back, X2 (button 4) forward,
  // reusing the same history ops as Alt+Left / Alt+Right (GH #156). `auxclick`
  // fires once per non-primary button release, so a single listener never
  // double-navigates; preventDefault suppresses any webview built-in nav.
  // Middle-click (button 1) is left untouched for new-tab handlers.
  const mouseNav = (e: MouseEvent) => {
    if (e.button === 3) {
      e.preventDefault();
      navigateMouseHistory("back", "dom");
    } else if (e.button === 4) {
      e.preventDefault();
      navigateMouseHistory("forward", "dom");
    }
  };

  let disposed = false;
  let unlistenNativeMouseHistory = () => {};
  if ("__TAURI_INTERNALS__" in window) {
    void Promise.all([import("@tauri-apps/api/event"), import("@tauri-apps/api/window")]).then(
      async ([{ listen }, { getCurrentWindow }]) => {
        const target = getCurrentWindow().label;
        const unlisten = await listen<{ direction: MouseHistoryDirection; target: string }>(
          "history-navigate",
          (e) => {
            if (e.payload?.target !== target) return;
            if (e.payload.direction === "back" || e.payload.direction === "forward") {
              navigateMouseHistory(e.payload.direction, "native");
            }
          },
        );
        if (disposed) unlisten();
        else unlistenNativeMouseHistory = unlisten;
      },
    );
  }

  window.addEventListener("keydown", handler, true);
  window.addEventListener("paste", pasteHandler, true);
  window.addEventListener("keydown", superTracker, true);
  window.addEventListener("keyup", superTracker, true);
  window.addEventListener("blur", clearSuper);
  window.addEventListener("auxclick", mouseNav, true);
  return () => {
    disposed = true;
    unlistenNativeMouseHistory();
    window.removeEventListener("keydown", handler, true);
    window.removeEventListener("paste", pasteHandler, true);
    window.removeEventListener("keydown", superTracker, true);
    window.removeEventListener("keyup", superTracker, true);
    window.removeEventListener("blur", clearSuper);
    window.removeEventListener("auxclick", mouseNav, true);
  };
}
