import { beforeAll, expect, it } from "vitest";
import { initParser } from "./render/parse";
import { pageIconAsset, pageIconImageLink, PAGE_ICON_PROPERTY } from "./pageIconImage";
import { isStaticIconImage } from "./staticImage";

beforeAll(initParser);
it("uses the shared property policy", () => expect(PAGE_ICON_PROPERTY).toBe("icon"));
it.each([
  ["![](https://example.com/a.png)", "pages/A.md", true],
  ["![](../images/a.gif)", "pages/A.md", true],
  ["![](../assets/a.png)", "", true],
  ["[[file:https://example.com/a.png]]", "pages/A.org", true],
  ["[[../images/a.gif]]", "pages/A.org", true],
  ["[[Page]]", "pages/A.org", false],
  ["[label](../assets/a.png)", "pages/A.md", false],
  ["🚩", "pages/A.md", false],
  ["AB", "pages/A.md", false],
])("classifies %s independently of asset eligibility", (value, owner, image) => {
  expect(!!pageIconImageLink(value, owner)).toBe(image);
});
it.each([
  ["![](../assets/logo.png)", "pages/A.md", "logo.png"],
  ['![](../assets/logo.JPG){:width "90%" :height "9999"}', "pages/A.md", "logo.JPG"],
  ["![](../../assets/sub/a.JpEg)", "pages/nested/A.md", "sub/a.JpEg"],
  ["![](assets/logo.png)", "A.md", "logo.png"],
  ["![](../assets/a%20b.png)", "pages/A.md", "a b.png"],
  ["[[file:../assets/logo.png]]", "pages/A.org", "logo.png"],
  ["[[../assets/logo.png]]", "pages/A.org", "logo.png"],
])("accepts parser-owned assets link %s", (value, owner, expected) => {
  expect(pageIconAsset(value, owner)).toBe(expected);
});
it.each([
  "![](../logo.png)", "![](../images/logo.png)", "![](../../assets/logo.png)",
  "![](../assets/../../logo.png)", "![](../assets/%2e%2e/logo.png)",
  "![](https://example.com/assets/logo.png)", "![](//example.com/assets/logo.png)",
  "![](data:image/png;base64,AAAA)", "![](file:../assets/logo.png)",
  "![](custom:../assets/logo.png)", "![](/assets/logo.png)",
  "![](../assets/a.gif)", "![](../assets/a.svg)", "![](../assets/a.webp)",
  "![](../assets/a.png?x=1)", "![](../assets/a.png#fragment)",
  "[label](../assets/a.png)", "`![](../assets/a.png)`", "text ![](../assets/a.png)",
  "![](../assets/a.png) ![](../assets/b.png)", "🚩", "AB", "",
  "[[file:https://example.com/assets/logo.png]]",
  "[[file:../assets/logo.png]]", "[[../assets/logo.png]]",
  "![](../assets/%ZZ.png)",
])("refuses %s without transport access", value => {
  expect(pageIconAsset(value, "pages/A.md")).toBeNull();
});
const png = Uint8Array.from(atob("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aFOkAAAAASUVORK5CYII="), c => c.charCodeAt(0));
it("permits static PNG/JPEG bytes and refuses animation and disguised formats", () => {
  expect(isStaticIconImage(png)).toBe(true);
  expect(isStaticIconImage(new Uint8Array([255, 216, 255, 224]))).toBe(true);
  const animated = new Uint8Array(png.length + 12);
  animated.set(png.subarray(0, 33));
  animated.set([0, 0, 0, 0, 97, 99, 84, 76, 0, 0, 0, 0], 33);
  animated.set(png.subarray(33), 45);
  expect(isStaticIconImage(animated)).toBe(false);
  expect(isStaticIconImage(png.subarray(0, 32))).toBe(false);
  expect(isStaticIconImage(new TextEncoder().encode("GIF89a"))).toBe(false);
  expect(isStaticIconImage(new TextEncoder().encode("<svg/>"))).toBe(false);
});
