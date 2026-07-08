import { beforeAll, describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import { AstBody } from "./body";
import { initParser } from "./parse";

// GH #40: hovering a [[page]] link shows a read-only hover-peek card after a
// short dwell. Uses the mock backend (jsdom => mockBackend), so getPage("Tine")
// resolves to a real page with blocks. No server needed.
beforeAll(async () => {
  await initParser();
});

function mountAttached(raw: string): { root: HTMLElement; dispose: () => void } {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const dispose = render(() => (
    <div class="block-content"><AstBody raw={raw} /></div>
  ), host);
  return { root: host, dispose: () => { dispose(); host.remove(); } };
}

async function waitFor(fn: () => boolean, timeout = 2500): Promise<boolean> {
  const start = Date.now();
  while (Date.now() - start < timeout) {
    if (fn()) return true;
    await new Promise((r) => setTimeout(r, 25));
  }
  return fn();
}

function fireEnter(el: Element) {
  el.dispatchEvent(new MouseEvent("mouseenter", { bubbles: false }));
}
function fireLeave(el: Element) {
  el.dispatchEvent(new MouseEvent("mouseleave", { bubbles: false }));
}

describe("page hover-peek (GH #40)", () => {
  it("renders a peek card with the target's block lines after dwell", async () => {
    const { root, dispose } = mountAttached("A link to [[Tine]] here");
    try {
      const link = root.querySelector("a.page-ref")!;
      expect(link).toBeTruthy();
      // No card before hover.
      expect(root.querySelector(".page-ref-preview")).toBeFalsy();
      fireEnter(link);
      const shown = await waitFor(() => !!root.querySelector(".page-ref-preview-line"));
      expect(shown).toBe(true);
      const card = root.querySelector(".page-ref-preview")!;
      // Title = page name; at least one line from the Tine mock page.
      expect(card.querySelector(".page-ref-preview-title")?.textContent).toContain("Tine");
      expect(card.querySelectorAll(".page-ref-preview-line").length).toBeGreaterThan(0);
    } finally {
      dispose();
    }
  });

  it("dismisses the card on mouse leave", async () => {
    const { root, dispose } = mountAttached("go to [[Tine]] now");
    try {
      const link = root.querySelector("a.page-ref")!;
      fireEnter(link);
      await waitFor(() => !!root.querySelector(".page-ref-preview"));
      fireLeave(link);
      const gone = await waitFor(() => !root.querySelector(".page-ref-preview"), 1000);
      expect(gone).toBe(true);
    } finally {
      dispose();
    }
  });

  it("shows no card for a link whose target page does not exist", async () => {
    const { root, dispose } = mountAttached("dangling [[No Such Page 12345]] ref");
    try {
      const link = root.querySelector("a.page-ref")!;
      fireEnter(link);
      // Give dwell + fetch time; the resource resolves to null => no card.
      await new Promise((r) => setTimeout(r, 700));
      expect(root.querySelector(".page-ref-preview")).toBeFalsy();
    } finally {
      dispose();
    }
  });
});
