import { afterEach, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import katex from "katex";
import { MathView } from "./inline";

afterEach(() => vi.restoreAllMocks());

it("typesets once per formula change, with no extra work on pane resize or local scrolling", async () => {
  const typeset = vi.spyOn(katex, "renderToString");
  const [tex, setTex] = createSignal(String.raw`\mathrm{K_2Cr_2O_7 + 6FeSO_4 + 7H_2SO_4 \rightarrow K_2SO_4 + Cr_2(SO_4)_3 + 3Fe_2(SO_4)_3 + 7H_2O}`);
  const [display, setDisplay] = createSignal(false);
  const root = document.createElement("div");
  document.body.append(root);
  let renders = 0;
  const dispose = render(() => {
    renders++;
    return <><MathView tex={tex()} display={display()} /><MathView tex={tex()} display={!display()} /></>;
  }, root);
  try {
    await vi.waitFor(() => expect(root.querySelectorAll(".katex")).toHaveLength(2));
    expect(typeset).toHaveBeenCalledTimes(2);
    expect(renders).toBe(1);
    for (const width of [390, 640, 1450, 390]) {
      root.style.width = `${width}px`;
      window.dispatchEvent(new Event("resize"));
      for (const scroller of root.querySelectorAll(".math > span")) {
        scroller.scrollLeft = 120;
        scroller.dispatchEvent(new Event("scroll"));
      }
    }
    await vi.dynamicImportSettled();
    expect(typeset).toHaveBeenCalledTimes(2);
    expect(renders).toBe(1);
    setTex("x^2");
    expect(typeset).toHaveBeenCalledTimes(4);
    expect(renders).toBe(1);
    expect(typeset.mock.calls.slice(2).map(call => call[0])).toEqual(["x^2", "x^2"]);
    setDisplay(true);
    expect(typeset).toHaveBeenCalledTimes(6);
    expect(typeset.mock.calls.slice(4).map(call => call[1]?.displayMode)).toEqual([true, false]);
  } finally {
    dispose();
    root.remove();
  }
});
