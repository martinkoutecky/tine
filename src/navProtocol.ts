// ONE spatial-navigation key protocol for the app's two tiling surfaces
// (ADR 0034): sheet cell selection (src/sheet/selection.ts) and pane-select
// mode (src/keybindings.ts). Both decode navigation keys through this table so
// the surfaces cannot drift apart silently; src/navModel.contract.test.ts pins
// the behavioral invariants both must uphold.
//
// Mod-chords (ctrl/meta/alt) are surface commands, never navigation — this
// decoder declines them, and each surface handles its own chords itself.

export type NavDirection = "up" | "down" | "left" | "right";

export type NavIntent =
  | { kind: "step"; dir: NavDirection }
  | { kind: "extend"; dir: NavDirection } // shift+arrow — selection/span growth
  | { kind: "activate" } // Enter (+F2 where the surface opts in): edit / materialize
  | { kind: "remove"; side: "before" | "after" } // Backspace / Delete
  | { kind: "overtype"; char: string } // printable char: create-and-type here
  | { kind: "dismiss" }; // Escape: down one mode rung

export function navDirectionForKey(key: string): NavDirection | null {
  switch (key) {
    case "ArrowUp":
      return "up";
    case "ArrowDown":
      return "down";
    case "ArrowLeft":
      return "left";
    case "ArrowRight":
      return "right";
    default:
      return null;
  }
}

export function decodeNavIntent(e: KeyboardEvent, opts?: { acceptF2?: boolean }): NavIntent | null {
  if (e.ctrlKey || e.metaKey || e.altKey) return null;
  const dir = navDirectionForKey(e.key);
  if (dir) return e.shiftKey ? { kind: "extend", dir } : { kind: "step", dir };
  if (e.key === "Escape") return { kind: "dismiss" };
  // Shifted non-character keys (Shift+Enter, Shift+Delete, …) are not nav;
  // shifted printables (uppercase letters) are.
  if (e.shiftKey && e.key.length > 1) return null;
  if (e.key === "Enter" || (opts?.acceptF2 && e.key === "F2")) return { kind: "activate" };
  if (e.key === "Backspace") return { kind: "remove", side: "before" };
  if (e.key === "Delete") return { kind: "remove", side: "after" };
  if (e.key.length === 1 && !e.isComposing) return { kind: "overtype", char: e.key };
  return null;
}
