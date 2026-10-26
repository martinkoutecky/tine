import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { VideoMacro } from "./Macro";

afterEach(() => { vi.unstubAllGlobals(); document.body.innerHTML = ""; });

// Diagnostic evidence for GH #600: the shared markup is identical on macOS
// and Linux. Linux's native WebProcess hook supplies its HTTP Referer; this
// iframe policy cannot manufacture a Referer from a tauri: parent on macOS.
it.each(["Mozilla/5.0 (Macintosh; Intel Mac OS X)", "Mozilla/5.0 (X11; Linux x86_64)"])("documents the generated YouTube iframe identity boundary on %s", (userAgent) => {
  vi.stubGlobal("navigator", { userAgent });
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <VideoMacro body="video https://youtu.be/JI-AyLv68Xs" />, root);
  const iframe = root.querySelector("iframe")!;
  const url = new URL(iframe.src);
  expect(url.origin).toBe("https://www.youtube.com");
  expect(url.pathname).toBe("/embed/JI-AyLv68Xs");
  expect(url.searchParams.get("enablejsapi")).toBe("1");
  expect(url.searchParams.has("origin")).toBe(false);
  expect(url.searchParams.has("widget_referrer")).toBe(false);
  expect(iframe.getAttribute("referrerpolicy")).toBe("strict-origin-when-cross-origin");
  dispose();
});
