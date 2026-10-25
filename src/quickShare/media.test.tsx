import { afterEach, beforeAll, expect, it } from "vitest";
import { render } from "solid-js/web";
import { initParser, parseBlock } from "../render/parse";
import { renderBlocks } from "../render/body";
beforeAll(() => initParser());
let dispose: (() => void) | undefined;
afterEach(() => { dispose?.(); document.body.replaceChildren(); document.querySelector('meta[name="tine-published"]')?.remove(); });
it("suppresses remote images before creating a resource element in share mode", () => {
  const meta = document.createElement("meta"); meta.name = "tine-published"; meta.content = "quick-share:1"; document.head.append(meta);
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(() => renderBlocks(parseBlock('![photo](https://remote.example/photo.png)', false)), root);
  expect(root.querySelector("img")).toBeNull();
  expect(root.textContent).toContain("remote.example");
});
