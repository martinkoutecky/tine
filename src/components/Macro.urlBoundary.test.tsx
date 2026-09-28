import { afterEach, describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { TweetMacro, VideoMacro } from "./Macro";

function mount(node: () => JSX.Element) {
  const root = document.createElement("div");
  document.body.appendChild(root);
  return { root, dispose: render(node, root) };
}

afterEach(() => { document.body.innerHTML = ""; });

describe("graph macro URL boundary", () => {
  it.each([
    ["video javascript:alert(1)", VideoMacro],
    ["video javascript:alert(1).mp4", VideoMacro],
    ["tweet javascript:alert(1)", TweetMacro],
  ])("does not put an unsafe URL in href or src: %s", (body, Component) => {
    const { root, dispose } = mount(() => <Component body={body} />);
    try {
      expect([...root.querySelectorAll("[href], [src]")]).toEqual([]);
      expect(root.textContent).toContain("javascript:alert(1)");
    } finally { dispose(); }
  });

  it("retains http and https links", () => {
    const { root, dispose } = mount(() => <><VideoMacro body="video https://example.com/watch" /><TweetMacro body="tweet http://example.com/post" /></>);
    try {
      expect([...root.querySelectorAll("a")].map((anchor) => anchor.getAttribute("href")))
        .toEqual(["https://example.com/watch", "http://example.com/post"]);
    } finally { dispose(); }
  });
});
