import { expect, it, vi } from "vitest";

it("cold capture Block mounts before parser init and publishes identity when ready", async () => {
  // Capture paints its seeded empty editor before this WebView initializes wasm.
  vi.resetModules();
  const { render } = await import("solid-js/web");
  const { Block } = await import("../components/Block");
  const { ensurePageLoaded, resetStore } = await import("../document/workingSet");
  const { startEditing, endEdit } = await import("../editorController");
  const { initParser, parserReady } = await import("../render/parse");
  expect(parserReady()).toBe(false);
  ensurePageLoaded({ name: "ColdCapture", title: "ColdCapture", kind: "page", pre_block: null,
    blocks: [{ id: "scratch", raw: "", collapsed: false, children: [] }] });
  startEditing("scratch", 0);
  const root = document.createElement("div");
  document.body.append(root);
  let dispose: (() => void) | undefined;
  try {
    dispose = render(() => Block({ id: "scratch" }), root);
    expect(root.querySelector("textarea.block-editor")).not.toBeNull();
    expect(root.querySelector(".ls-block")?.hasAttribute("data-block-ref")).toBe(false);
    await initParser();
    expect(root.querySelector(".ls-block")?.getAttribute("data-block-ref")).toBe("scratch");
    expect(root.querySelector("textarea.block-editor")).not.toBeNull();
  } finally {
    dispose?.();
    endEdit("page-navigation");
    resetStore();
    root.remove();
  }
});
