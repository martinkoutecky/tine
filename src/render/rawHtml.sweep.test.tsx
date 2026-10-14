import { expect, it } from "vitest";
import { render } from "solid-js/web";
import { renderRawHtml } from "./rawHtml";

it.each([
  ['width="100%" height="833" style="background: transparent; border: 1px solid #ccc;"', "100%", "833px"],
  ['style="width: 350px; height: 630px"', "350px", "630px"],
  ['style="width: 1px; height: 1px"', "1px", "1px"],
  ['width="700"', "700px", ""],
  ['height="833"', "", "833px"],
  ['width="100%" height="833" style="width: 350px; height: 630px"', "350px", "630px"],
] as const)("GH #54: author size reaches the visible iframe box (%s)", (attrs, width, height) => {
  const root = document.createElement("div");
  const dispose = render(() => renderRawHtml(`<iframe src="https://airtable.com/embed/example" ${attrs}></iframe>`), root);
  try {
    const wrap = root.querySelector<HTMLElement>(".embed-iframe-wrap")!;
    expect(wrap.style.width).toBe(width);
    expect(wrap.style.height).toBe(height);
    expect(wrap.style.maxWidth).toBe("none");
    if (height) expect(wrap.style.aspectRatio).toBe("auto");
    expect(root.querySelector("iframe")?.getAttribute("sandbox")).toContain("allow-scripts");
  } finally { dispose(); }
});

it("GH #54: invalid dimensions leave the ordinary default embed box", () => {
  const root = document.createElement("div");
  const dispose = render(() => renderRawHtml('<iframe src="https://example.com" width="833garbage" height="-1"></iframe>'), root);
  try {
    const wrap = root.querySelector<HTMLElement>(".embed-iframe-wrap")!;
    expect(wrap.style.width).toBe("");
    expect(wrap.style.height).toBe("");
    expect(wrap.style.maxWidth).toBe("");
    expect(wrap.style.aspectRatio).toBe("");
  } finally { dispose(); }
});
