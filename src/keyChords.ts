// Key chords: the parsed form of a binding string ("mod+shift+down") and of a
// keyboard event. Platform-free and pure; the platform's modifier mapping,
// binding parsing and the dispatcher live in keybindings.ts.

/** One chord of a binding: its modifiers and its normalized key. */
export interface Chord {
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

/** The binding spelling of an event key (`arrowup` -> `up`, `" "` -> `space`). */
export function normKey(k: string): string {
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
export function isModifierKey(e: KeyboardEvent): boolean {
  if (MODIFIER_KEYS.has(e.key.toLowerCase())) return true;
  return /^(Control|Shift|Alt|Meta|OS|Super|Hyper)(Left|Right)?$/.test(e.code);
}

/** Whether two chords are the same keys. */
export function chordEq(a: Chord, b: Chord): boolean {
  return a.mod === b.mod && a.ctrl === b.ctrl && a.shift === b.shift && a.alt === b.alt && a.meta === b.meta && a.key === b.key;
}
