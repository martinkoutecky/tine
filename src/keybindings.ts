import { shortcutKey } from "./shortcutKey";
import { openWorkspaceCreation } from "./workspaceCreation";
import { adjustSidebarWidth } from "./sidebarSizing";
import { openCssEditor } from "./cssEditor";
import { openWorkspaceWindow, workspaceWindowsSupported } from "./workspaceWindows";
import { browserPlatform } from "./browserPlatform";
// Configurable keyboard shortcuts. Defaults mirror OG Logseq command ids and
// bindings; users override them via config.edn `:shortcuts {:cmd "binding"}`
// (delivered in GraphMeta.shortcuts). "mod" = Ctrl (Cmd on macOS). Bindings may
// be a single chord ("mod+k") or a sequence of chords ("g j").
//
// Both the global dispatcher (this file) and the in-editor key handler
// (Block.tsx) resolve keys through the same merged binding table, so every
// listed command is remappable from config.edn.

import { openSwitcher, closeSwitcher, openCommandPalette, openDevtools, toggleTheme, toggleSidebar, openSettings, toggleHelpPopup, toggleRightSidebar, toggleWideMode, toggleDocumentMode, toggleFocusMode, toggleDimInactiveBlocks, focusMode, exitFocusMode, carryDays, showBrackets, changeShowBrackets, openPdfExport, dismissMobileDrawer } from "./ui";
import { pushToast } from "./toasts";
import { msg, render, renderEnglish, type UiText, type Msg } from "./i18n";
import { customCssDisabled, setCustomCssDisabled } from "./customCss";
import { restoreDrawerFocus } from "./mobileDrawers";
import { zoomReset } from "./zoom";
import { activeWindowId, MAIN_WINDOW_ID, onEachWindow, queryAllWindows } from "./windowRealm";
import { dismissTopTransient } from "./transientLayers";
import { carryDaysBack } from "./carry";
import {
  openJournals,
  openQueryInNewTab,
  openPage,
  goBack,
  goForward,
  closeActiveTab,
  reopenClosedTab,
  activateNextTab,
  activatePrevTab,
  route,
} from "./router";
import { journalTitle, parseJournalTitle, appNow } from "./journal";
import { undo, redo, hasSelection, moveSelection, cycleSelectionTasks, expandBlockSelection, moveSelectionItems, indentSelection, outdentSelection, deleteSelection, selectionMarkdown, clearSelection, selectedIds, blockIsGridView, pageVisibleOrder, selectBlock, visibleOrder, toggleUndoRedoMode, buildClipboardPayload, node as docNode, loadedPage } from "./document";
import { editingId, startEditing } from "./editorController";
import { copyBlockOutline } from "./clipboard";
import { cutBlocks } from "./cut";
import { deleteRenderedTextSelection } from "./editor/renderedSelectionDelete";
import { followLinkUnderCaret, openLinkUnderCaretInSidebar } from "./followLink";
import { openInPageFind } from "./inpageFind";
import { cellSel, enterGridSelection, handleCellSelectionKey, handleSheetPasteEvent, outlinedGridSelectionId } from "./sheet/selection";
import { decodeNavIntent } from "./navProtocol";
import {
  closePane,
  adjustPaneSize,
  focusPane,
  focusedPaneId,
  focusedRouter,
  layoutHasMultiplePanes,
  layoutPaneIds,
  layoutRoot,
  moveActiveTabInDirection,
  paneRouter,
  splitPane,
  splitPaneAtSeam,
  splitRootAtEdge,
  togglePaneMaximize,
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
import { copyTargetBlockRefs, duplicateTargetBlocks, insertChildIntoTarget, searchSelectedText, setTargetChildrenCollapsed } from "./blockRefActions";
import { dispatchFocusedEditorCommand } from "./editorCommandBridge";
import { goHome } from "./homePage";
import { pluginManager } from "./plugins/manager";
import { bindPluginBlockSnapshot, capturePluginGraphOwner, isPluginGraphOwnerCurrent, type OwnedPluginBlockSnapshot } from "./plugins/ownership";

/** Open the adjacent journal route (+1 next, -1 previous) from the current
 * journal date, or today's local date on another route. O(1) before navigation;
 * the page loader owns any read error. No page is written here. */
export function goAdjacentJournal(dir: 1 | -1): void {
  const current = route();
  const anchor = current.kind === "page" && current.pageKind === "journal"
    ? parseJournalTitle(current.name) ?? appNow()
    : appNow();
  openPage(journalTitle(new Date(anchor.getFullYear(), anchor.getMonth(), anchor.getDate() + dir)), "journal");
}
function pluginFocusedBlock(): OwnedPluginBlockSnapshot | undefined {
  const owner = capturePluginGraphOwner();
  if (!owner) return undefined;
  const id = editingId();
  const node = id ? docNode(id) : undefined;
  if (!node) return undefined;
  let depth = 0;
  let parentId = node.parent;
  while (parentId && docNode(parentId) && depth < 1_000) {
    depth++;
    parentId = docNode(parentId).parent;
  }
  const format = loadedPage(node.page)?.format === "org" ? "org" : "md";
  if (!isPluginGraphOwnerCurrent(owner)) return undefined;
  const owned = bindPluginBlockSnapshot({ id: node.id, raw: node.raw, parentId: node.parent, depth, format });
  if (!owned || owned.owner.graphRoot !== owner.graphRoot || owned.owner.generation !== owner.generation) return undefined;
  return owned;
}

interface Chord {
  mod: boolean;
  // Physical Control is distinct from portable `mod` on macOS. Elsewhere the
  // same key remains `mod`, preserving existing cross-platform bindings (GH #378).
  ctrl: boolean;
  shift: boolean;
  alt: boolean;
  // The Super/Win key on Linux/Windows (distinct from `mod`); on macOS Cmd is
  // already `mod`, so `meta` is only meaningful off-Mac.
  meta: boolean;
  key: string; // lowercase, normalized
}

export type ShortcutScope = "global" | "editor" | "select";

interface CommandDef {
  /** Keep bindable commands in Shortcuts while omitting them from launchers. */
  paletteHidden?: boolean;
  id: string;
  binding: string;
  /** Human label for the palette and the Settings reference: a catalog message
   *  (`command-` + the id with `/` as `-`, D4), or a plugin's own title. */
  label: UiText;
  /** Global commands run from the window dispatcher; editor commands are
   *  matched by Block.tsx inside the textarea handler. */
  scope: "global" | "editor";
  run?: () => void;
  /** Offered in the palette only while this holds (e.g. a desktop-only command). */
  available?: () => boolean;
  /** A SECOND default chord that runs the same command: a platform convention a
   *  user will certainly try and that Logseq does not bind, so the key reaches
   *  the command it obviously means without displacing Logseq's own binding or
   *  taking a second row in Settings. An alias is a default, not an override: the
   *  moment the user binds the command themselves, their chord is the only one,
   *  because an alias they can neither see in the recorder nor reset must not
   *  outlive their choice. Global scope only; it loses every collision with a
   *  primary binding. */
  alias?: string;
  /** When false (default), a global command does not fire while typing in an
   *  editor unless its chord includes a modifier. */
  global?: boolean;
}

const isMac = typeof navigator !== "undefined" && browserPlatform("", navigator.platform).macKeyboard;
function focusPaneByNumber(n: number) {
  if (!layoutHasMultiplePanes()) return;
  const pane = readingOrderPanes(layoutRoot())[n - 1];
  if (pane) focusPane(pane.paneId);
}

function editSelectedBlock(): boolean {
  const ids = selectedIds();
  if (ids.length !== 1 || !docNode(ids[0])) return false;
  clearSelection();
  startEditing(ids[0]);
  return true;
}

function focusPaneInDirection(dir: PaneDirection) {
  if (!layoutHasMultiplePanes()) return;
  const target = nearestPaneInDirection(layoutRoot(), focusedPaneId(), dir);
  if (target) focusPane(target);
}

// With no pane in the requested direction, panes.ts grows the layout instead
// of no-op'ing (GH #282), so the focus commands' single-pane gate does not apply.
function moveActiveTab(dir: PaneDirection) {
  moveActiveTabInDirection(focusedPaneId(), dir);
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
  const target = previous && docNode(previous) ? previous : firstVisibleBlockInFocusedPane();
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
  { id: "editor/edit-selected-block", binding: "", label: msg("command-editor-edit-selected-block"), scope: "global", run: () => { editSelectedBlock(); }, global: true },
  { id: "go/search", binding: "mod+k", label: msg("command-go-search"), scope: "global", run: () => openSwitcher({ pluginBlock: pluginFocusedBlock() ?? null }), global: true },
  { id: "go/search-pages", binding: "", label: msg("command-go-search-pages"), scope: "global", run: () => openSwitcher({ mode: "pages" }), global: true },
  { id: "go/search-content", binding: "", label: msg("command-go-search-content"), scope: "global", run: () => openSwitcher({ mode: "content" }), global: true },
  { id: "go/search-current-page", binding: "mod+shift+k", label: msg("command-go-search-current-page"), scope: "global", run: () => openSwitcher({ mode: "current-page", pluginBlock: pluginFocusedBlock() ?? null }), global: true },
  // GH #633: search for the selected text. Unbound: mod+shift+f is OG's
  // :command/toggle-favorite (config.cljs:493), so the chord is left to the user.
  { id: "go/search-selection", binding: "", label: msg("command-go-search-selection"), scope: "global", run: searchSelectedText, global: true },
  { id: "guide/open", binding: "", label: msg("command-guide-open"), scope: "global", run: () => void openGuide(), global: true },
  { id: "go/find-in-page", binding: "mod+f", label: msg("command-go-find-in-page"), scope: "global", run: openInPageFind, global: true },
  // GH #274 / OG parity: `:editor/follow-link` (mod+o) and
  // `:editor/open-link-in-sidebar` (mod+shift+o) in OG's shortcut config.
  // Global rather than editor-scoped because the handler reads the focused
  // textarea itself and is a no-op when nothing is being edited.
  { id: "editor/follow-link", binding: "mod+o", label: msg("command-editor-follow-link"), scope: "global", run: () => { followLinkUnderCaret(); }, global: true },
  { id: "editor/open-link-in-sidebar", binding: "mod+shift+o", label: msg("command-editor-open-link-in-sidebar"), scope: "global", run: () => { openLinkUnderCaretInSidebar(); }, global: true },
  { id: "command-palette/toggle", binding: "mod+shift+p", label: msg("command-command-palette-toggle"), scope: "global", run: () => openCommandPalette(pluginFocusedBlock() ?? null), global: true },
  // Toggle the WebKit Web Inspector for theme/CSS debugging (GH #31). The usual
  // Ctrl+Shift+I / F12 / Ctrl+Shift+C are all swallowed by WebKitGTK itself (its
  // built-in inspector keys, handled in the web process below where the app can
  // intercept), so they never reach this dispatcher. Ctrl+Shift+J — Chrome's other
  // devtools shortcut (console) — is NOT grabbed by WebKit, so it works here. A
  // mod-chord, so it fires even while editing; remap it in Settings if you like.
  { id: "ui/toggle-devtools", binding: "mod+shift+j", label: msg("command-ui-toggle-devtools"), scope: "global", run: openDevtools, global: true },
  { id: "ui/toggle-custom-css", binding: "", label: msg("command-ui-toggle-custom-css"), scope: "global", run: () => setCustomCssDisabled(!customCssDisabled()), global: true },
  { id: "go/journals", binding: "g j", label: msg("command-go-journals"), scope: "global", run: openJournals },
  { id: "go/home", binding: "g h", label: msg("command-go-home"), scope: "global", run: goHome },
  { id: "go/journal-next", binding: "g n", label: msg("command-go-journal-next"), scope: "global", run: () => goAdjacentJournal(1) },
  { id: "go/journal-prev", binding: "g p", label: msg("command-go-journal-prev"), scope: "global", run: () => goAdjacentJournal(-1) },
  { id: "go/keyboard-shortcuts", binding: "g s", label: msg("command-go-keyboard-shortcuts"), scope: "global", run: () => openSettings("shortcuts") },
  // Browser-style history nav (per-tab back/forward). Special-cased in the
  // dispatcher so they fire even while editing a block; remappable like any other.
  { id: "go/backward", binding: "alt+left", label: msg("command-go-backward"), scope: "global", run: goBack, global: true },
  { id: "go/forward", binding: "alt+right", label: msg("command-go-forward"), scope: "global", run: goForward, global: true },
  // mod+w is a mod-chord, so it clears the while-editing guard and reaches the
  // generic dispatch loop below — closes the active tab even mid-edit (like a
  // browser). The last tab can't be closed, so this never quits the app.
  { id: "tab/close", binding: "mod+w", label: msg("command-tab-close"), scope: "global", run: closeActiveTab, global: true },
  // Reopen the most-recently-closed tab (browser-style). mod-chord ⇒ fires even
  // mid-edit, like mod+w.
  { id: "tab/reopen-closed", binding: "mod+shift+t", label: msg("command-tab-reopen-closed"), scope: "global", run: reopenClosedTab, global: true },
  // Browser-style tab cycling (Ctrl+PgDn / Ctrl+PgUp on Linux/Windows; Cmd on
  // macOS). mod-chords, so they fire mid-edit; remappable like everything here.
  { id: "tab/next", binding: "mod+pagedown", label: msg("command-tab-next"), scope: "global", run: activateNextTab, global: true },
  { id: "tab/previous", binding: "mod+pageup", label: msg("command-tab-previous"), scope: "global", run: activatePrevTab, global: true },
  // mod+1..9 belong to pane focus. Leave these browser-style slots bindable.
  ...Array.from({ length: 9 }, (_, i): CommandDef => ({
    id: `tab/activate-${i + 1}`,
    paletteHidden: true,
    binding: "",
    label: i === 8 ? msg("command-tab-activate-last") : msg("command-tab-activate-n", { index: i + 1 }),
    scope: "global",
    global: true,
    run: () => {
      const router = paneRouter(focusedPaneId());
      const tabs = router.tabs();
      const tab = tabs[i === 8 ? tabs.length - 1 : i];
      if (tab) router.setActiveTab(tab.id);
    },
  })),
  { id: "pane/split-right", binding: "mod+alt+\\", label: msg("command-pane-split-right"), scope: "global", run: () => void splitPane(focusedPaneId(), "row"), global: true },
  { id: "pane/split-down", binding: "mod+alt+shift+\\", label: msg("command-pane-split-down"), scope: "global", run: () => void splitPane(focusedPaneId(), "col"), global: true },
  { id: "pane/close", binding: "", label: msg("command-pane-close"), scope: "global", run: () => void closePane(focusedPaneId()), global: true },
  { id: "pane/toggle-maximize", binding: "mod+alt+m", label: msg("command-pane-toggle-maximize"), scope: "global", run: () => { togglePaneMaximize(); }, global: true },
  { id: "window/open-in-new-window", binding: "", label: msg("command-window-open-in-new-window"), scope: "global", available: () => workspaceWindowsSupported(), run: () => { if (workspaceWindowsSupported()) openWorkspaceWindow({ snapshot: focusedRouter().duplicateActiveSnapshot() }); }, global: true },
  { id: "sidebar/grow-width", paletteHidden: true, binding: "", label: msg("command-sidebar-grow-width"), scope: "global", run: () => adjustSidebarWidth("left", true), global: true },
  { id: "sidebar/shrink-width", paletteHidden: true, binding: "", label: msg("command-sidebar-shrink-width"), scope: "global", run: () => adjustSidebarWidth("left", false), global: true },
  { id: "right-sidebar/grow-width", paletteHidden: true, binding: "", label: msg("command-right-sidebar-grow-width"), scope: "global", run: () => adjustSidebarWidth("right", true), global: true },
  { id: "right-sidebar/shrink-width", paletteHidden: true, binding: "", label: msg("command-right-sidebar-shrink-width"), scope: "global", run: () => adjustSidebarWidth("right", false), global: true },
  { id: "go/search-tab", binding: "", label: msg("command-go-search-tab"), scope: "global", run: () => { closeSwitcher(); openQueryInNewTab("", "search", true); }, global: true },
  { id: "pane/grow-width", paletteHidden: true, binding: "", label: msg("command-pane-grow-width"), scope: "global", run: () => { adjustPaneSize(focusedPaneId(), "width", true); }, global: true },
  { id: "pane/shrink-width", paletteHidden: true, binding: "", label: msg("command-pane-shrink-width"), scope: "global", run: () => { adjustPaneSize(focusedPaneId(), "width", false); }, global: true },
  { id: "pane/grow-height", paletteHidden: true, binding: "", label: msg("command-pane-grow-height"), scope: "global", run: () => { adjustPaneSize(focusedPaneId(), "height", true); }, global: true },
  { id: "pane/shrink-height", paletteHidden: true, binding: "", label: msg("command-pane-shrink-height"), scope: "global", run: () => { adjustPaneSize(focusedPaneId(), "height", false); }, global: true },
  // Palette-discoverable entry into pane-select (it's otherwise only reachable
  // via Esc-with-nothing-open, which users won't guess — Martin didn't).
  { id: "pane/select-mode", binding: "", label: msg("command-pane-select-mode"), scope: "global", run: enterPaneSelectFromFocus, global: true },
  ...Array.from({ length: 9 }, (_, i): CommandDef => ({
    id: `pane/focus-${i + 1}`,
    paletteHidden: true,
    binding: `mod+${i + 1}`,
    label: msg("command-pane-focus-n", { index: i + 1 }),
    scope: "global",
    run: () => focusPaneByNumber(i + 1),
    global: true,
  })),
  { id: "pane/focus-left", binding: "mod+alt+left", label: msg("command-pane-focus-left"), scope: "global", run: () => focusPaneInDirection("left"), global: true },
  { id: "pane/focus-right", binding: "mod+alt+right", label: msg("command-pane-focus-right"), scope: "global", run: () => focusPaneInDirection("right"), global: true },
  { id: "pane/focus-up", binding: "mod+alt+up", label: msg("command-pane-focus-up"), scope: "global", run: () => focusPaneInDirection("up"), global: true },
  { id: "pane/focus-down", binding: "mod+alt+down", label: msg("command-pane-focus-down"), scope: "global", run: () => focusPaneInDirection("down"), global: true },
  { id: "pane/move-tab-left", binding: "mod+alt+shift+left", label: msg("command-pane-move-tab-left"), scope: "global", run: () => moveActiveTab("left"), global: true },
  { id: "pane/move-tab-right", binding: "mod+alt+shift+right", label: msg("command-pane-move-tab-right"), scope: "global", run: () => moveActiveTab("right"), global: true },
  { id: "pane/move-tab-up", binding: "mod+alt+shift+up", label: msg("command-pane-move-tab-up"), scope: "global", run: () => moveActiveTab("up"), global: true },
  { id: "pane/move-tab-down", binding: "mod+alt+shift+down", label: msg("command-pane-move-tab-down"), scope: "global", run: () => moveActiveTab("down"), global: true },
  { id: "ui/toggle-theme", binding: "t t", label: msg("command-ui-toggle-theme"), scope: "global", run: toggleTheme },
  { id: "ui/reset-zoom", binding: "", label: msg("command-ui-reset-zoom"), scope: "global", run: zoomReset, global: true },
  { id: "ui/toggle-brackets", binding: "mod+c mod+b", label: msg("command-ui-toggle-brackets"), scope: "global", run: () => changeShowBrackets(!showBrackets()), global: true },
  { id: "ui/toggle-left-sidebar", binding: "t l", label: msg("command-ui-toggle-left-sidebar"), scope: "global", run: toggleSidebar },
  { id: "ui/toggle-right-sidebar", binding: "t r", label: msg("command-ui-toggle-right-sidebar"), scope: "global", run: toggleRightSidebar },
  { id: "ui/open-settings", binding: "t s", label: msg("command-ui-open-settings"), scope: "global", run: openSettings },
  { id: "appearance/edit-custom-css", binding: "", label: msg("command-appearance-edit-custom-css"), scope: "global", run: () => void openCssEditor(false) },
  { id: "appearance/edit-global-css", binding: "", label: msg("command-appearance-edit-global-css"), scope: "global", run: () => void openCssEditor(true) },
  { id: "graph/export-html", binding: "", label: msg("command-graph-export-html"), scope: "global", run: () => openSettings("graph") },
  {
    id: "page/export-pdf",
    binding: "",
    label: msg("command-page-export-pdf"),
    scope: "global",
    run: () => {
      const r = route();
      if (r.kind === "page") openPdfExport(r.name);
      else pushToast(msg("commands-open-a-page-first-to-export-it"), "warn");
    },
  },
  { id: "ui/toggle-wide-mode", binding: "t w", label: msg("command-ui-toggle-wide-mode"), scope: "global", run: toggleWideMode },
  { id: "ui/toggle-document-mode", binding: "t d", label: msg("command-ui-toggle-document-mode"), scope: "global", run: toggleDocumentMode },
  { id: "ui/toggle-focus-mode", binding: "t f", label: msg("command-ui-toggle-focus-mode"), scope: "global", run: toggleFocusMode },
  { id: "ui/toggle-dim-blocks", binding: "t b", label: msg("command-ui-toggle-dim-blocks"), scope: "global", run: toggleDimInactiveBlocks },
  // Web KeyboardEvent reports Shift+/ as key "?", so the binding must use the
  // shifted character. OG stores "shift+/" and rewrites it for display; Tine
  // matches eventToChord's actual output instead.
  { id: "ui/toggle-help", binding: "shift+?", label: msg("command-ui-toggle-help"), scope: "global", run: toggleHelpPopup },
  // Saved-workspace management belongs to the main window; secondary workspace
  // windows have no switcher or saved-layout registry controls.
  { id: "workspace/new", binding: "", label: msg("command-workspace-new"), scope: "global", available: () => activeWindowId() === MAIN_WINDOW_ID, run: openWorkspaceCreation },
  // Carry the Settings-configured number of days.
  { id: "task/carry-n", binding: "", label: msg("command-task-carry-n"), scope: "global", run: () => void carryDaysBack(carryDays()) },
  { id: "editor/undo", binding: "mod+z", label: msg("command-editor-undo"), scope: "global", run: undo, global: true },
  // Logseq binds redo to mod+shift+z alone, and that stays the binding Settings
  // shows and remaps. Ctrl+Y is the Windows/Linux convention for redo and Logseq
  // leaves it unbound, so pressing it was silence (GH #491, master 2721c0126). It
  // rides along as an alias so the shortcuts list keeps one Redo row. Not on
  // macOS: there Ctrl+Y is the system's Emacs-style yank inside every text field.
  { id: "editor/redo", binding: "mod+shift+z", alias: isMac ? "" : "ctrl+y", label: msg("command-editor-redo"), scope: "global", run: redo, global: true },
  // Palette-only, matching OG's empty binding and mode report at
  // `src/main/frontend/modules/shortcut/config.cljs:355-356` and
  // `src/main/frontend/modules/editor/undo_redo.cljs:232-237`
  // (OG commit 6e7afa8eb).
  {
    id: "editor/toggle-undo-redo-mode",
    binding: "",
    label: msg("command-editor-toggle-undo-redo-mode"),
    scope: "global",
    run: () => pushToast(msg("commands-undo-redo-mode", { mode: toggleUndoRedoMode() })),
  },
  // Block commands (R2 B1). OG binds none of these, so none has a default key;
  // each acts on the block selection, else on the block being edited.
  { id: "select/copy-block-ref", binding: "", label: msg("command-select-copy-block-ref"), scope: "global", run: () => { copyTargetBlockRefs(); }, global: true },
  { id: "editor/duplicate-block", binding: "", label: msg("command-editor-duplicate-block"), scope: "global", run: () => void duplicateTargetBlocks(), global: true },
  { id: "editor/duplicate-block-above", binding: "", label: msg("command-editor-duplicate-block-above"), scope: "global", run: () => void duplicateTargetBlocks("above"), global: true },
  { id: "editor/insert-child-block", binding: "", label: msg("command-editor-insert-child-block"), scope: "global", run: () => { insertChildIntoTarget(); }, global: true },
  { id: "editor/select-embedding-block", binding: "", label: msg("command-editor-select-embedding-block"), scope: "global", run: () => { dispatchFocusedEditorCommand("editor/select-embedding-block"); }, global: true },
  { id: "editor/collapse-children", binding: "", label: msg("command-editor-collapse-children"), scope: "global", run: () => { setTargetChildrenCollapsed(true); }, global: true },
  { id: "editor/expand-children", binding: "", label: msg("command-editor-expand-children"), scope: "global", run: () => { setTargetChildrenCollapsed(false); }, global: true },
  // GH #448: the clipboard's URL or block ref becomes a link around the selection.
  {
    id: "editor/paste-as-link",
    binding: "",
    label: msg("command-editor-paste-as-link"),
    scope: "global",
    run: () => { if (!dispatchFocusedEditorCommand("editor/paste-as-link")) pushToast(msg("commands-select-text-in-a-block-then-use"), "warn"); },
    global: true,
  },
  // Editor commands (resolved in Block.tsx / selection handler).
  { id: "editor/indent", binding: "tab", label: msg("command-editor-indent"), scope: "editor" },
  { id: "editor/outdent", binding: "shift+tab", label: msg("command-editor-outdent"), scope: "editor" },
  // OG's non-Mac default (alt+shift+arrow). Super+arrow (the old "meta+up"
  // binding) is grabbed by most Linux compositors for window tiling, so it never
  // reached the app.
  { id: "editor/move-block-up", binding: "alt+shift+up", label: msg("command-editor-move-block-up"), scope: "editor" },
  { id: "editor/move-block-down", binding: "alt+shift+down", label: msg("command-editor-move-block-down"), scope: "editor" },
  { id: "editor/collapse", binding: "mod+up", label: msg("command-editor-collapse"), scope: "editor" },
  { id: "editor/expand", binding: "mod+down", label: msg("command-editor-expand"), scope: "editor" },
  { id: "editor/select-block-up", binding: "shift+up", label: msg("command-editor-select-block-up"), scope: "editor" },
  { id: "editor/select-block-down", binding: "shift+down", label: msg("command-editor-select-block-down"), scope: "editor" },
  // Ctrl/Cmd+A ladder (GH #262): first press selects the block's text natively,
  // then the block's subtree, then each ancestor's subtree, then the outline.
  { id: "editor/select-all", binding: "mod+a", label: msg("command-editor-select-all"), scope: "editor" },
  { id: "editor/cycle-todo", binding: "mod+enter", label: msg("command-editor-cycle-todo"), scope: "editor" },
  // Quick-capture mini-window only: file the capture to today's journal. Acts
  // only when CaptureCtx is present (Block.tsx); a no-op in the main app. Default
  // mod+shift+enter avoids mod+enter (cycle-todo) and shift+enter (soft newline).
  { id: "editor/quick-capture-file", binding: "mod+shift+enter", label: msg("command-editor-quick-capture-file"), scope: "editor" },
  // Inline formatting toggles.
  { id: "editor/bold", binding: "mod+b", label: msg("command-editor-bold"), scope: "editor" },
  { id: "editor/italics", binding: "mod+i", label: msg("command-editor-italics"), scope: "editor" },
  { id: "editor/strike-through", binding: "mod+shift+s", label: msg("command-editor-strike-through"), scope: "editor" },
  { id: "editor/highlight", binding: "mod+shift+h", label: msg("command-editor-highlight"), scope: "editor" },
  { id: "editor/insert-link", binding: "mod+l", label: msg("command-editor-insert-link"), scope: "editor" },
  // GH #279: the embed counterpart of builtin Mod+C (copy block ref) — with a
  // text selection the handler declines so the platform's ordinary copy runs.
  { id: "editor/copy-embed", binding: "mod+shift+c", label: msg("command-editor-copy-embed"), scope: "editor" },
  { id: "editor/clear-block", binding: "alt+l", label: msg("command-editor-clear-block"), scope: "editor" },
  // Emacs-style cursor/kill motions.
  { id: "editor/kill-line-before", binding: "alt+u", label: msg("command-editor-kill-line-before"), scope: "editor" },
  { id: "editor/kill-line-after", binding: "alt+k", label: msg("command-editor-kill-line-after"), scope: "editor" },
  { id: "editor/backward-word", binding: "alt+b", label: msg("command-editor-backward-word"), scope: "editor" },
  { id: "editor/forward-word", binding: "alt+f", label: msg("command-editor-forward-word"), scope: "editor" },
  { id: "editor/backward-kill-word", binding: "alt+w", label: msg("command-editor-backward-kill-word"), scope: "editor" },
  { id: "editor/forward-kill-word", binding: "alt+d", label: msg("command-editor-forward-kill-word"), scope: "editor" },
];

export interface BuiltinKeyDef {
  id: string;
  scope: ShortcutScope;
  binding: string;
  label: Msg;
  details?: Msg;
}

// Hardcoded keys that are intentionally not remappable. Keep this next to
// COMMANDS so shortcut behavior and shortcut documentation drift together.
export const BUILTIN_KEYS: BuiltinKeyDef[] = [
  {
    id: "builtin/global/escape",
    scope: "global",
    binding: "esc",
    label: msg("command-builtin-global-escape"),
    details: msg("commands-closes-search-settings-first-outside-overlays"),
  },
  {
    id: "builtin/editor/enter",
    scope: "editor",
    binding: "enter",
    label: msg("command-builtin-editor-enter"),
    details: msg("commands-normally-splits-the-current-block-continues-an"),
  },
  {
    id: "builtin/editor/soft-newline",
    scope: "editor",
    binding: "shift+enter",
    label: msg("command-builtin-editor-soft-newline"),
  },
  {
    id: "builtin/editor/escape",
    scope: "editor",
    binding: "esc",
    label: msg("command-builtin-editor-escape"),
  },
  {
    id: "builtin/editor/backspace-start",
    scope: "editor",
    binding: "backspace",
    label: msg("command-builtin-editor-backspace-start"),
    details: msg("commands-at-the-start-of-a-block-merges"),
  },
  {
    id: "builtin/editor/arrow-cross-block",
    scope: "editor",
    binding: "up / down",
    label: msg("command-builtin-editor-arrow-cross-block"),
    details: msg("commands-from-the-first-or-last-visual-row"),
  },
  {
    id: "builtin/editor/copy-block-ref",
    scope: "editor",
    binding: "mod+c",
    label: msg("command-builtin-editor-copy-block-ref"),
  },
  // GH #513 / #81. OG: `:editor/paste-text-in-one-block-at-point` (mod+shift+v,
  // handler/paste.cljs editor-on-paste-raw!). Not a remappable command: the key
  // has to reach the platform's own paste so the clipboard can be read (Block.tsx
  // keeps a one-shot latch between the keydown and the paste event).
  {
    id: "builtin/editor/paste-plain",
    scope: "editor",
    binding: "mod+shift+v",
    label: msg("command-builtin-editor-paste-plain"),
    details: msg("commands-inserts-the-clipboard-s-plain-text-as-typed"),
  },
  {
    id: "builtin/editor/ac-nav",
    scope: "editor",
    binding: "up / down",
    label: msg("command-builtin-editor-ac-nav"),
  },
  {
    id: "builtin/editor/ac-accept",
    scope: "editor",
    binding: "enter / tab / shift+tab",
    label: msg("command-builtin-editor-ac-accept"),
  },
  {
    id: "builtin/editor/ac-close",
    scope: "editor",
    binding: "esc",
    label: msg("command-builtin-editor-ac-close"),
  },
  {
    id: "builtin/select/escape",
    scope: "select",
    binding: "esc",
    label: msg("command-builtin-select-escape"),
  },
  {
    id: "builtin/select/move",
    scope: "select",
    binding: "up / down",
    label: msg("command-builtin-select-move"),
  },
  {
    id: "builtin/select/extend",
    scope: "select",
    binding: "shift+up / shift+down",
    label: msg("command-builtin-select-extend"),
  },
  {
    id: "builtin/select/delete",
    scope: "select",
    binding: "backspace / delete",
    label: msg("command-builtin-select-delete"),
  },
  {
    id: "builtin/select/copy",
    scope: "select",
    binding: "mod+c",
    label: msg("command-builtin-select-copy"),
  },
  {
    id: "builtin/select/cut",
    scope: "select",
    binding: "mod+x",
    label: msg("command-builtin-select-cut"),
  },
  {
    id: "builtin/select/edit",
    scope: "select",
    binding: "enter",
    label: msg("command-builtin-select-edit"),
  },
  {
    id: "builtin/sheet/select-cell",
    scope: "select",
    binding: "click",
    label: msg("command-builtin-sheet-select-cell"),
    details: msg("commands-double-click-enter-or-f2-edits-the-selected"),
  },
  {
    id: "builtin/sheet/move",
    scope: "select",
    binding: "arrow keys",
    label: msg("command-builtin-sheet-move"),
    details: msg("commands-esc-leaves-a-sheet-typing-replaces-the"),
  },
  {
    id: "builtin/sheet/range",
    scope: "select",
    binding: "drag / shift+click / shift+arrows",
    label: msg("command-builtin-sheet-range"),
  },
  {
    id: "builtin/sheet/copy-cut",
    scope: "select",
    binding: "mod+c / mod+x / paste",
    label: msg("command-builtin-sheet-copy-cut"),
  },
  {
    id: "builtin/sheet/fill",
    scope: "select",
    binding: "mod+d / mod+r",
    label: msg("command-builtin-sheet-fill"),
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
    label: msg("command-plugin", { title: contribution.title }),
    scope: "global",
    global: true,
    run: () => {
      void pluginManager
        .invokeCommand(pluginId, contribution.id, pluginFocusedBlock())
        .catch((error) => pushToast(msg("commands-plugin-command-failed", { error: String(error) }), "error"));
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
  const chord: Chord = { mod: false, ctrl: false, shift: false, alt: false, meta: false, key: "" };
  for (const p of parts) {
    if (p === "mod" || p === "cmd") chord.mod = true;
    else if (p === "ctrl") {
      if (isMac) chord.ctrl = true;
      else chord.mod = true;
    }
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
  let key = e.code === "Tab" ? "tab" : shortcutKey(e);
  // Real WebKitGTK reports Shift+/ as key "?"; Playwright/Chromium's synthetic
  // Shift+/ reports key "/". Normalize both to the binding string we expose.
  if (e.shiftKey && e.code === "Slash" && key === "/") key = "?";
  key = normKey(key);
  return {
    mod: isMac ? e.metaKey : e.ctrlKey,
    ctrl: isMac && e.ctrlKey,
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
  return a.mod === b.mod && a.ctrl === b.ctrl && a.shift === b.shift && a.alt === b.alt && a.meta === b.meta && a.key === b.key;
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
): { id: string; label: string; english: string; binding: string; run: () => void }[] {
  const builtIn = COMMANDS.filter((c) => c.scope === "global" && c.run && !c.paletteHidden && c.id !== "go/search" && (!c.available || c.available()))
    .map((c) => ({
      id: c.id,
      label: render(c.label),
      english: renderEnglish(c.label),
      binding: (overridesApplied[c.id] ?? c.binding) === "false" ? "" : overridesApplied[c.id] ?? c.binding,
      run: c.run!,
    }));
  const plugins = pluginManager.commands().map(({ pluginId, contribution }) => ({
    id: `plugin:${pluginId}:${contribution.id}`,
    label: contribution.title,
    english: contribution.title,
    binding: overridesApplied[`plugin:${pluginId}:${contribution.id}`] === "false"
      ? ""
      : overridesApplied[`plugin:${pluginId}:${contribution.id}`] ?? contribution.defaultBinding ?? "",
    run: () => {
      void pluginManager
        .invokeCommand(pluginId, contribution.id, focusedPluginBlock ?? undefined)
        .catch((error) => pushToast(msg("commands-plugin-command-failed", { error: String(error) }), "error"));
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

/** Built-in command defaults (id + label + default binding) for the Settings
 *  remap UI, which computes the effective binding reactively from these plus
 *  config.edn and the user's local overrides. Call from a reactive consumer:
 *  `label` is in the active UI language, `english` is for search. */
export function commandDefaults(): { id: string; label: string; english: string; binding: string; alias?: string; scope: ShortcutScope }[] {
  return [...COMMANDS, ...pluginCommandDefs()].map((c) => ({ id: c.id, label: render(c.label), english: renderEnglish(c.label), binding: c.binding, alias: c.alias, scope: shortcutScope(c) }));
}

/** Turn a keyboard event into a binding string like "mod+shift+down". Returns
 *  null for a bare modifier press (keep waiting). */
export function eventToBindingString(e: KeyboardEvent): string | null {
  if (isModifierKey(e)) return null;
  const c = eventToChord(e);
  if (!c.key) return null;
  const parts: string[] = [];
  if (c.ctrl) parts.push("ctrl");
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
  const pane = queryAllWindows<HTMLElement>(`[data-pane-id="${esc(paneId)}"]`);
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
  if (matchesCommand(e, "editor/edit-selected-block")) return editSelectedBlock();
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
  if (mod && shortcutKey(e) === "c") {
    const ids = selectedIds();
    const text = selectionMarkdown();
    void copyBlockOutline("copy", text, buildClipboardPayload(ids))
      .catch(() => pushToast(msg("commands-couldn-t-copy-selection-clipboard-write-failed"), "error"));
    return true;
  }
  if (mod && shortcutKey(e) === "x") {
    const ids = selectedIds();
    // Cut deletes whole subtrees, so its public text carries them regardless of the copy preference.
    const text = selectionMarkdown(true);
    void cutBlocks(ids, text, () =>
      JSON.stringify(selectedIds()) === JSON.stringify(ids) ? selectionMarkdown(true) : "",
      () => deleteSelection())
      .catch(() => pushToast(msg("commands-couldn-t-cut-selection-clipboard-write-failed"), "error"));
    return true;
  }
  if (mod && !e.shiftKey && !e.altKey && shortcutKey(e) === "a") {
    return expandBlockSelection(), true;
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
  const knownIds = new Set(allCommands.map((command) => command.id));
  if (Object.keys(overrides).some((id) => !knownIds.has(id))) {
    // Keep user-authored config values out of console sinks (I-5). The original
    // entries remain in config.edn/device overrides for inspection and repair.
    console.warn("[shortcuts] Ignoring unknown command ids; check shortcut configuration.");
  }
  for (const c of allCommands) {
    const b = overrides[c.id] ?? c.binding;
    if (b !== "false") bindings[c.id] = parseBinding(b);
  }

  // Global dispatch list (sequences + global chords). Aliases come last, so a
  // primary binding always wins a collision, and are dropped for any command the
  // user has bound themselves.
  const primary = allCommands.filter((c) => c.scope === "global" && c.run)
    .map((c) => ({ ...c, chords: bindings[c.id] }))
    .filter((c) => c.chords);
  const aliased = allCommands
    .filter((c) => c.scope === "global" && c.run && c.alias && !(c.id in overrides))
    .map((c) => ({ ...c, chords: parseBinding(c.alias!) }));
  const commands = [...primary, ...aliased];

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
        if (matchesCommand(e, "editor/edit-selected-block") && editSelectedBlock()) {
          e.preventDefault(); resetSeq(); return;
        }
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

    // OG contenteditable parity: Delete/Backspace over a RENDERED (not-editing)
    // text selection deletes that text from the block's source; without this the
    // keypress reaches no editor and dies silently.
    if (!editing && (e.key === "Delete" || e.key === "Backspace") && !e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey
        && deleteRenderedTextSelection()) {
      e.preventDefault();
      resetSeq();
      return;
    }

    // While typing, only modifier chords are eligible (so "g j" doesn't fire).
    // Alt counts as a modifier here (GH #461): Chrome and VS Code fire Alt
    // shortcuts with a text field focused. Only `scope: "global"` commands with
    // a `run` are matched below, so the editor's own bare-Alt bindings are
    // untouched, and an Alt chord nothing is bound to still reaches the textarea
    // unprevented (dead keys and Option-composed characters keep working).
    if (editing && !chord.mod && !chord.ctrl && !chord.alt) {
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
        if (cmd.id === "go/find-in-page" && paneRouter(focusedPaneId()).route().kind === "pdf") {
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

  // One set of listeners per Tine window (OG-MULTIWINDOW P1): a shortcut typed
  // in a workspace window reaches the same handler, and its window's listeners
  // go away with it.
  const uninstallWindows = onEachWindow((win) => {
    win.addEventListener("keydown", handler, true);
    win.addEventListener("paste", pasteHandler, true);
    win.addEventListener("keydown", superTracker, true);
    win.addEventListener("keyup", superTracker, true);
    win.addEventListener("blur", clearSuper);
    win.addEventListener("auxclick", mouseNav, true);
    return () => {
      win.removeEventListener("keydown", handler, true);
      win.removeEventListener("paste", pasteHandler, true);
      win.removeEventListener("keydown", superTracker, true);
      win.removeEventListener("keyup", superTracker, true);
      win.removeEventListener("blur", clearSuper);
      win.removeEventListener("auxclick", mouseNav, true);
    };
  });
  return () => {
    disposed = true;
    unlistenNativeMouseHistory();
    uninstallWindows();
  };
}
