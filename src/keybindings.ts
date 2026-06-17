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
  toggleTheme,
  toggleSidebar,
  closeSwitcher,
  closeSettings,
  openSettings,
  toggleRightSidebar,
  toggleWideMode,
  toggleDocumentMode,
  toggleFocusMode,
  toggleDimInactiveBlocks,
  focusMode,
  exitFocusMode,
  switcherOpen,
  settingsOpen,
} from "./ui";
import { openJournals } from "./router";
import {
  undo,
  redo,
  hasSelection,
  moveSelection,
  moveSelectionItems,
  indentSelection,
  outdentSelection,
  deleteSelection,
  selectionMarkdown,
  clearSelection,
  selectedIds,
  startEditing,
} from "./store";
import { backend } from "./backend";

interface Chord {
  mod: boolean;
  shift: boolean;
  alt: boolean;
  // The Super/Win key on Linux/Windows (distinct from `mod`); on macOS Cmd is
  // already `mod`, so `meta` is only meaningful off-Mac.
  meta: boolean;
  key: string; // lowercase, normalized
}

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

// Default command table. Editor command ids mirror OG Logseq where practical.
const COMMANDS: CommandDef[] = [
  { id: "go/search", binding: "mod+k", label: "Search / quick switch", scope: "global", run: openSwitcher, global: true },
  { id: "command-palette/toggle", binding: "mod+shift+p", label: "Command palette", scope: "global", run: openCommandPalette, global: true },
  { id: "go/journals", binding: "g j", label: "Go to journals", scope: "global", run: openJournals },
  { id: "ui/toggle-theme", binding: "t t", label: "Toggle dark / light", scope: "global", run: toggleTheme },
  { id: "ui/toggle-left-sidebar", binding: "t l", label: "Toggle left sidebar", scope: "global", run: toggleSidebar },
  { id: "ui/toggle-right-sidebar", binding: "t r", label: "Toggle right sidebar", scope: "global", run: toggleRightSidebar },
  { id: "ui/open-settings", binding: "t s", label: "Open settings", scope: "global", run: openSettings },
  { id: "ui/toggle-wide-mode", binding: "t w", label: "Toggle wide mode", scope: "global", run: toggleWideMode },
  { id: "ui/toggle-document-mode", binding: "t d", label: "Toggle document mode", scope: "global", run: toggleDocumentMode },
  { id: "ui/toggle-focus-mode", binding: "t f", label: "Toggle focus mode", scope: "global", run: toggleFocusMode },
  { id: "ui/toggle-dim-blocks", binding: "t b", label: "Toggle dim inactive blocks", scope: "global", run: toggleDimInactiveBlocks },
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
  // Inline formatting toggles.
  { id: "editor/bold", binding: "mod+b", label: "Bold", scope: "editor" },
  { id: "editor/italics", binding: "mod+i", label: "Italic", scope: "editor" },
  { id: "editor/strike-through", binding: "mod+shift+s", label: "Strikethrough", scope: "editor" },
  { id: "editor/highlight", binding: "mod+shift+h", label: "Highlight", scope: "editor" },
  { id: "editor/insert-link", binding: "mod+shift+l", label: "Insert link", scope: "editor" },
  { id: "editor/clear-block", binding: "alt+l", label: "Clear block content", scope: "editor" },
  // Emacs-style cursor/kill motions.
  { id: "editor/kill-line-before", binding: "alt+u", label: "Delete to line start", scope: "editor" },
  { id: "editor/kill-line-after", binding: "alt+k", label: "Delete to line end", scope: "editor" },
  { id: "editor/backward-word", binding: "alt+b", label: "Cursor word backward", scope: "editor" },
  { id: "editor/forward-word", binding: "alt+f", label: "Cursor word forward", scope: "editor" },
  { id: "editor/backward-kill-word", binding: "alt+w", label: "Delete word backward", scope: "editor" },
  { id: "editor/forward-kill-word", binding: "alt+d", label: "Delete word forward", scope: "editor" },
];

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

/** Runnable global commands for the command palette / Ctrl-K Commands group:
 *  every global command with a run handler, with its effective binding. The
 *  switcher itself is excluded (no point launching the launcher). */
export function paletteCommands(): { id: string; label: string; binding: string; run: () => void }[] {
  return COMMANDS.filter((c) => c.scope === "global" && c.run && c.id !== "go/search")
    .map((c) => ({
      id: c.id,
      label: c.label,
      binding: overridesApplied[c.id] ?? c.binding,
      run: c.run!,
    }))
    .filter((c) => c.binding !== "false");
}

/** Merged shortcuts for the Settings reference. */
export function currentShortcuts(): { id: string; label: string; binding: string }[] {
  return COMMANDS.map((c) => ({
    id: c.id,
    label: c.label,
    binding: overridesApplied[c.id] ?? c.binding,
  })).filter((c) => c.binding !== "false");
}

/** Built-in command defaults (id + label + default binding) for the Settings
 *  remap UI, which computes the effective binding reactively from these plus
 *  config.edn and the user's local overrides. */
export function commandDefaults(): { id: string; label: string; binding: string }[] {
  return COMMANDS.map((c) => ({ id: c.id, label: c.label, binding: c.binding }));
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

// Keyboard handling while in block-selection mode (no editor focused).
function handleSelectionKey(e: KeyboardEvent): boolean {
  if (e.key === "Escape") return clearSelection(), true;
  if (matchesCommand(e, "editor/outdent")) return outdentSelection(), true;
  if (matchesCommand(e, "editor/indent")) return indentSelection(), true;
  if (matchesCommand(e, "editor/move-block-down")) return moveSelectionItems(1), true;
  if (matchesCommand(e, "editor/move-block-up")) return moveSelectionItems(-1), true;
  if (e.key === "ArrowDown") return moveSelection(1, e.shiftKey), true;
  if (e.key === "ArrowUp") return moveSelection(-1, e.shiftKey), true;
  if (e.key === "Backspace" || e.key === "Delete") return deleteSelection(), true;
  const mod = isMac ? e.metaKey : e.ctrlKey;
  if (mod && e.key.toLowerCase() === "c") return void backend().writeText(selectionMarkdown()), true;
  if (mod && e.key.toLowerCase() === "x") {
    void backend().writeText(selectionMarkdown());
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
  for (const c of COMMANDS) {
    const b = overrides[c.id] ?? c.binding;
    if (b !== "false") bindings[c.id] = parseBinding(b);
  }

  // Global dispatch list (sequences + global chords).
  const commands = COMMANDS.filter((c) => c.scope === "global" && c.run)
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
    if (suspended) return;
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
      if (switcherOpen() || settingsOpen()) {
        closeSwitcher();
        closeSettings();
        e.preventDefault();
        resetSeq();
        return;
      }
      if (editing) return; // defer to the editor's own Esc (capture phase)
      if (hasSelection()) {
        clearSelection();
        if (focusMode()) void exitFocusMode();
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
      return;
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
      // phase (WebKitGTK grabs it before the textarea can), but still let the
      // event reach the editor so it can indent/outdent.
      if (e.code === "Tab" || chord.key === "tab") e.preventDefault();
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

  window.addEventListener("keydown", handler, true);
  window.addEventListener("keydown", superTracker, true);
  window.addEventListener("keyup", superTracker, true);
  window.addEventListener("blur", clearSuper);
  return () => {
    window.removeEventListener("keydown", handler, true);
    window.removeEventListener("keydown", superTracker, true);
    window.removeEventListener("keyup", superTracker, true);
    window.removeEventListener("blur", clearSuper);
  };
}
