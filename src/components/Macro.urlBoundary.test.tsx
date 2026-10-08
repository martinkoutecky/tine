import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { TweetMacro, VideoMacro } from "./Macro";
import { backend } from "../backend";
import { setToasts, toasts } from "../toasts";

function mount(node: () => JSX.Element) {
  const root = document.createElement("div");
  document.body.appendChild(root);
  return { root, dispose: render(node, root) };
}

afterEach(() => { vi.restoreAllMocks(); document.body.innerHTML = ""; });

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

  it("I-22: a macro link opens through the native opener, never by navigating the WebView", () => {
    const openExternal = vi.spyOn(backend(), "openExternal").mockResolvedValue();
    const { root, dispose } = mount(() => <><VideoMacro body="video https://example.com/watch" /><TweetMacro body="tweet http://example.com/post" /></>);
    try {
      for (const anchor of root.querySelectorAll("a")) {
        const event = new MouseEvent("click", { bubbles: true, cancelable: true });
        anchor.dispatchEvent(event);
        expect(event.defaultPrevented).toBe(true);
      }
      expect(openExternal.mock.calls).toEqual([["https://example.com/watch"], ["http://example.com/post"]]);
    } finally { dispose(); }
  });

  it("I-22: a refused open is shown to the user", async () => {
    vi.spyOn(backend(), "openExternal").mockRejectedValue(new Error("scheme not allowed"));
    setToasts([]);
    const { root, dispose } = mount(() => <TweetMacro body="tweet https://example.com/post" />);
    try {
      root.querySelector("a")!.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
      await vi.waitFor(() => expect(toasts().map((toast) => toast.message)).toContain("Couldn't open https://example.com/post. (Error: scheme not allowed)"));
    } finally { dispose(); }
  });
});
