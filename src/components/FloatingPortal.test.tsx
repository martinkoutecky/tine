// GH #619: a press on a portalled surface (the query sheet, the autocomplete
// popup, the tab menu, the peek popup) must not reach the block that rendered it.
import { afterEach, describe, expect, it } from "vitest";
import { render, Portal } from "solid-js/web";
import { FloatingPortal } from "./FloatingPortal";

let dispose: (() => void) | undefined;
afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = "";
});

function mountWith(portal: "bare" | "floating") {
  const seen: string[] = [];
  const root = document.createElement("div");
  document.body.appendChild(root);
  const inner = (
    <button type="button" class="surface-button">
      inside
    </button>
  );
  dispose = render(
    () => (
      <div
        class="block-content-wrapper"
        onMouseDown={() => seen.push("mousedown")}
        onClick={() => seen.push("click")}
        onKeyDown={() => seen.push("keydown")}
        onContextMenu={() => seen.push("contextmenu")}
      >
        {portal === "bare" ? <Portal>{inner}</Portal> : <FloatingPortal>{inner}</FloatingPortal>}
      </div>
    ),
    root,
  );
  return seen;
}

function press(button: Element) {
  button.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
  button.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
  button.dispatchEvent(new KeyboardEvent("keydown", { key: "a", bubbles: true, cancelable: true }));
  button.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
}

describe("FloatingPortal (GH #619)", () => {
  it("control: Solid's bare Portal delivers every press to the logical parent", () => {
    const seen = mountWith("bare");
    press(document.querySelector(".surface-button")!);
    expect(seen).toEqual(["mousedown", "click", "keydown", "contextmenu"]);
  });

  it("a press, key or right-click on a floating surface never reaches the block that rendered it", () => {
    const seen = mountWith("floating");
    const button = document.querySelector(".surface-button")!;
    expect(button.closest(".block-content-wrapper")).toBeNull(); // really portalled
    press(button);
    expect(seen).toEqual([]);
  });

  it("handlers inside the floating surface itself still run", () => {
    const seen: string[] = [];
    const root = document.createElement("div");
    document.body.appendChild(root);
    dispose = render(
      () => (
        <FloatingPortal>
          <button type="button" class="own" onClick={() => seen.push("own-click")}>
            x
          </button>
        </FloatingPortal>
      ),
      root,
    );
    document.querySelector(".own")!.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    expect(seen).toEqual(["own-click"]);
  });
});
