// Configurable keyboard shortcuts. Defaults mirror OG Logseq command ids and
// bindings; users override them via config.edn `:shortcuts {:cmd "binding"}`
// (delivered in GraphMeta.shortcuts). "mod" = Ctrl (Cmd on macOS). Bindings may
// be a single chord ("mod+k") or a sequence of chords ("g j").

import { openSwitcher, toggleTheme, toggleSidebar, closeSwitcher } from "./ui";
import { openJournals } from "./router";
import {
  undo,
  redo,
  hasSelection,
  moveSelection,
  indentSelection,
  outdentSelection,
  deleteSelection,
  selectionMarkdown,
  clearSelection,
  selectedIds,
  startEditing,
} from "./store";
import { backend } from "./backend";

// Keyboard handling while in block-selection mode (no editor focused).
function handleSelectionKey(e: KeyboardEvent, mod: boolean): boolean {
  const k = e.key;
  if (k === "Escape") return clearSelection(), true;
  if (e.code === "Tab" && e.shiftKey) return outdentSelection(), true;
  if (e.code === "Tab") return indentSelection(), true;
  if (k === "ArrowDown") return moveSelection(1, e.shiftKey), true;
  if (k === "ArrowUp") return moveSelection(-1, e.shiftKey), true;
  if (k === "Backspace" || k === "Delete") return deleteSelection(), true;
  if (mod && k.toLowerCase() === "c") return void backend().writeText(selectionMarkdown()), true;
  if (mod && k.toLowerCase() === "x") {
    void backend().writeText(selectionMarkdown());
    deleteSelection();
    return true;
  }
  if (k === "Enter") {
    const ids = selectedIds();
    const last = ids[ids.length - 1];
    if (last) startEditing(last, 1e9);
    return true;
  }
  return false;
}

interface Chord {
  mod: boolean;
  shift: boolean;
  alt: boolean;
  key: string; // lowercase
}

export interface Command {
  id: string;
  binding: string;
  run: () => void;
  /** When false (default), the command does not fire while typing in an editor
   *  unless its chord includes a modifier. */
  global?: boolean;
}

const isMac = typeof navigator !== "undefined" && /Mac/.test(navigator.platform);

const DEFAULTS: Command[] = [
  { id: "go/search", binding: "mod+k", run: openSwitcher, global: true },
  { id: "go/journals", binding: "g j", run: openJournals },
  { id: "ui/toggle-theme", binding: "t t", run: toggleTheme },
  { id: "ui/toggle-left-sidebar", binding: "t l", run: toggleSidebar },
  { id: "editor/undo", binding: "mod+z", run: undo, global: true },
  { id: "editor/redo", binding: "mod+shift+z", run: redo, global: true },
];

function parseChord(s: string): Chord {
  const parts = s.toLowerCase().split("+");
  const chord: Chord = { mod: false, shift: false, alt: false, key: "" };
  for (const p of parts) {
    if (p === "mod" || p === "ctrl" || p === "cmd" || p === "meta") chord.mod = true;
    else if (p === "shift") chord.shift = true;
    else if (p === "alt" || p === "option") chord.alt = true;
    else chord.key = p;
  }
  return chord;
}

function parseBinding(b: string): Chord[] {
  return b.trim().split(/\s+/).map(parseChord);
}

function eventToChord(e: KeyboardEvent): Chord {
  return {
    mod: isMac ? e.metaKey : e.ctrlKey,
    shift: e.shiftKey,
    alt: e.altKey,
    key: e.key.toLowerCase(),
  };
}

function chordEq(a: Chord, b: Chord): boolean {
  return a.mod === b.mod && a.shift === b.shift && a.alt === b.alt && a.key === b.key;
}

function isEditableTarget(t: EventTarget | null): boolean {
  const el = t as HTMLElement | null;
  if (!el) return false;
  const tag = el.tagName;
  return tag === "TEXTAREA" || tag === "INPUT" || el.isContentEditable;
}

/** Install the global shortcut handler, merging config overrides over defaults.
 *  Returns a disposer. */
export function installKeybindings(overrides: Record<string, string> = {}): () => void {
  const commands = DEFAULTS.map((c) => ({
    ...c,
    chords: parseBinding(overrides[c.id] ?? c.binding),
  })).filter((c) => (overrides[c.id] ?? c.binding) !== "false");

  let seq: Chord[] = [];
  let seqTimer: ReturnType<typeof setTimeout> | null = null;
  const resetSeq = () => {
    seq = [];
    if (seqTimer) clearTimeout(seqTimer);
    seqTimer = null;
  };

  const handler = (e: KeyboardEvent) => {
    // Ignore bare modifier presses.
    if (["Control", "Shift", "Alt", "Meta"].includes(e.key)) return;

    const chord = eventToChord(e);
    const editing = isEditableTarget(e.target);

    // Block-selection mode keys (no editor focused).
    if (!editing && hasSelection()) {
      if (handleSelectionKey(e, chord.mod)) {
        e.preventDefault();
        resetSeq();
        return;
      }
    }

    // Escape closes the switcher regardless of context.
    if (e.key === "Escape") {
      closeSwitcher();
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
        cmd.run();
        return;
      }
    }
  };

  window.addEventListener("keydown", handler, true);
  return () => window.removeEventListener("keydown", handler, true);
}

export function defaultCommands(): Command[] {
  return DEFAULTS;
}
