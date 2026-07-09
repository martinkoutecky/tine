// installPaneTracker's focusin handling: focus moving INTO a pane retargets
// pane focus; focus landing OUTSIDE any pane (the Ctrl+K / palette overlay
// input) must NOT steal it — the old "?? main" default reset pane focus on
// every switcher open, so palette splits and Ctrl+K picks always landed in
// "main" (Martin's Jul 8 wrong-pane report).
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { installPaneTracker, registerPaneFocusSetter } from "./ui";

describe("pane focus tracker", () => {
  let dispose: (() => void) | null = null;
  let calls: string[] = [];

  beforeEach(() => {
    calls = [];
    registerPaneFocusSetter((id) => calls.push(id));
    dispose = installPaneTracker();
    document.body.innerHTML = `
      <div data-pane-id="pane-7"><input id="inside" /></div>
      <div class="overlay"><input id="outside" /></div>
    `;
  });

  afterEach(() => {
    dispose?.();
    dispose = null;
    document.body.innerHTML = "";
  });

  it("focus inside a pane retargets pane focus", () => {
    (document.getElementById("inside") as HTMLInputElement).focus();
    expect(calls).toContain("pane-7");
  });

  it("focus in a pane-neutral overlay does NOT steal pane focus (Ctrl+K input)", () => {
    (document.getElementById("inside") as HTMLInputElement).focus();
    calls = [];
    (document.getElementById("outside") as HTMLInputElement).focus();
    expect(calls).toEqual([]);
  });

  it("a pointer click outside panes still falls back to main (deliberate act)", () => {
    const outside = document.getElementById("outside") as HTMLInputElement;
    outside.dispatchEvent(new Event("pointerdown", { bubbles: true }));
    expect(calls).toContain("main");
  });
});
