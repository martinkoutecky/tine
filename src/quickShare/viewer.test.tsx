import { afterEach, beforeAll, expect, it } from "vitest";
import { render } from "solid-js/web";
import { initParser, parseBlock } from "../render/parse";
import { renderBlocks } from "../render/body";
import { renderRawHtml, renderSanitizedHtml } from "../render/rawHtml";
import { QuickShareFailure, QuickShareBanner } from "./viewer";
import { QuickShareError } from "./quickShare";

let dispose: (() => void) | undefined;
afterEach(() => { dispose?.(); document.body.replaceChildren(); document.querySelector('meta[name="tine-published"]')?.remove(); });
beforeAll(() => initParser());
function mount(node: () => import("solid-js").JSX.Element) {
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(node, root); return root;
}
function shareMode() {
  const meta = document.createElement("meta"); meta.name = "tine-published"; meta.content = "quick-share:1"; document.head.append(meta);
}
it.each(["bad-link", "unsupported-browser", "not-found", "unreachable", "too-large", "not-a-share", "newer-viewer", "unauthentic", "invalid", "blocked"] as const)("explains %s without reflecting exception details", (reason) => {
  const root = mount(() => <QuickShareFailure error={new QuickShareError(reason, "SECRET-FRAGMENT")} />);
  expect(root.querySelector('[role="alert"]')?.textContent).toBeTruthy();
  expect(root.textContent).not.toContain("SECRET-FRAGMENT");
  if (reason === "unsupported-browser") expect(root.textContent).toContain("Firefox");
});
it("marks the reader as user content and offers a secret-free abuse contact", () => {
  const root = mount(() => <QuickShareBanner />);
  expect(root.textContent).toContain("User content");
  expect(root.querySelector("a")?.getAttribute("href")).toBe("mailto:support@tine.page");
});
it("turns Markdown, raw HTML and macro media into placeholders and links before mounting resources", () => {
  shareMode();
  const raw = '![photo](https://remote.example/photo.png) ![local](../assets/local.png) [file](../assets/a.pdf) [attachment](../assets/report.txt)\n\n{{youtube dQw4w9WgXcQ}}\n\n<iframe src="https://remote.example/frame"></iframe>\n\n<img src="https://remote.example/raw.png">';
  const root = mount(() => renderBlocks(parseBlock(raw, false)));
  expect(root.querySelector("img,iframe,video,audio,source")).toBeNull();
  expect(root.textContent).toContain("remote.example");
  expect(root.textContent).toContain("local.png");
  expect(root.textContent).toContain("report.txt — not included");
  expect(root.textContent).toContain("not included");
  expect(root.querySelector('a[href^="https://"]')).toBeTruthy();
});
it.each([renderRawHtml, renderSanitizedHtml])("suppresses raw and macro-expanded HTML resources before live insertion", (renderer) => {
  shareMode();
  const root = mount(() => renderer('<div><img src="https://remote.example/raw.png"><img src="../assets/raw.png"><img src="file:///home/example/local-photo.png"></div>'));
  expect(root.querySelector("img")).toBeNull();
  expect(root.textContent).toContain("remote.example");
  expect(root.textContent).toContain("not included");
  expect(root.textContent).toContain("local-photo.png — not included");
});
