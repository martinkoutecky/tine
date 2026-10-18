// GH #417 on touch: holding an inline block reference opens its menu (the touch
// route to Edit link / Remove reference), through GH #231's deliberate hold.
import { afterEach, beforeAll, expect, it, vi } from "vitest";
vi.hoisted(() => { (globalThis as { __TINE_PLATFORM__?: string }).__TINE_PLATFORM__ = "android"; });
import { render } from "solid-js/web";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import { Block } from "./Block";
import { ContextMenu } from "./ContextMenu";
import { closeContextMenu, contextMenu } from "../ui";
import { isMobilePlatform } from "../nativeChrome";
import { LONG_PRESS_DELAY } from "../render/longPress";

const T = "41700000-0000-4000-8000-000000000003";
beforeAll(async () => { await initParser(); });
afterEach(() => { closeContextMenu(); resetStore(); vi.restoreAllMocks(); document.body.innerHTML = ""; });

it("Android: holding an inline block reference offers Edit link and Remove reference, and Remove works", async () => {
  expect(isMobilePlatform).toBe(true);
  vi.spyOn(backend(), "resolveBlocks").mockImplementation(async ids => ids.map(() => ({
    page: "Source", kind: "page" as const, blocks: [{ id: "target", raw: "target text", collapsed: false, children: [] }],
  })));
  loadSingle({ name: "Host", title: "Host", kind: "page", pre_block: null,
    blocks: [{ id: "host", raw: `see ((${T}))`, collapsed: false, children: [] }] });
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <><Block id="host" /><ContextMenu /></>, root);
  try {
    await new Promise(r => setTimeout(r, 100));
    const ref = root.querySelector(".block-ref")!;
    ref.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, cancelable: true, pointerId: 7, pointerType: "touch", isPrimary: true, clientX: 20, clientY: 20 }));
    await new Promise(r => setTimeout(r, LONG_PRESS_DELAY + 100));
    expect(contextMenu()).toMatchObject({ kind: "blockref" });
    const items = [...document.querySelectorAll<HTMLElement>(".ctx-item")];
    expect(items.map((el) => el.textContent)).toEqual(expect.arrayContaining(["Edit link", "Remove reference"]));
    items.find((el) => el.textContent === "Remove reference")!.click();
    expect(doc.byId.host.raw).toBe("see");
  } finally { dispose(); }
});
