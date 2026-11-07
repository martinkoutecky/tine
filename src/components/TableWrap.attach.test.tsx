import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { TableWrap } from "./TableWrap";

// GH #802: a table mounted before its block is attached to the page (a Linked
// References copy in the right sidebar) must still get its pane geometry once
// it is attached, not give up because no pane was found at mount.
describe("TableWrap attach", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("measures a table that mounts detached once it is laid out in a pane", async () => {
    const observers: { callback: ResizeObserverCallback; targets: Element[] }[] = [];
    vi.stubGlobal("ResizeObserver", class {
      targets: Element[] = [];
      constructor(public callback: ResizeObserverCallback) { observers.push(this); }
      observe(target: Element) { this.targets.push(target); }
      unobserve() {}
      disconnect() { this.targets = []; }
    });
    const detached = document.createElement("div");
    const dispose = render(() => <TableWrap><table class="md-table"><tbody><tr><td>a</td></tr></tbody></table></TableWrap>, detached);
    const wrap = detached.querySelector<HTMLElement>(".md-table-wrap")!;
    expect(wrap.style.getPropertyValue("--table-column-width")).toBe("");

    const pane = document.createElement("div");
    pane.className = "rs-item-body";
    document.body.append(pane);
    pane.append(detached);
    for (const o of [...observers]) if (o.targets.includes(wrap)) o.callback([], o as unknown as ResizeObserver);
    await new Promise(resolve => setTimeout(resolve, 40));
    expect(wrap.style.getPropertyValue("--table-column-width")).not.toBe("");
    dispose();
    pane.remove();
  });
});
