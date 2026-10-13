import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { readFileSync } from "node:fs";
import { render } from "solid-js/web";
import { InlineText } from "./inline";
import { initParser } from "./parse";
import { backend } from "../backend";
import { setToasts, toasts } from "../toasts";
import { linkScheme } from "../components/ExternalLink";

// GH #503 (UI-CUSTOM-SCHEME-LINK-503): a link to another app's scheme
// (`shortcuts://`, `zotero://`, `obsidian://` …) opens in that app after the
// user confirms a prompt naming the scheme and the full target. Before the fix
// the native opener refused every scheme but http(s)/mailto/file, so the click
// only produced "unsupported url scheme" (iOS 0.6.982 included).

beforeAll(initParser);
afterEach(() => {
  vi.restoreAllMocks();
  document.body.replaceChildren();
  setToasts([]);
});

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

function clickLink(source: string): void {
  const host = document.createElement("div");
  document.body.append(host);
  render(() => <InlineText text={source} />, host);
  const link = host.querySelector("a");
  expect(link, `expected a rendered link for ${source}`).not.toBeNull();
  link!.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
}

describe("custom URL schemes open in their app after a confirm (GH #503)", () => {
  for (const [source, dest, scheme] of [
    ["[abc](shortcuts://run-shortcut?name=abc)", "shortcuts://run-shortcut?name=abc", "shortcuts"],
    ["[item](zotero://select/library/items/ABCD1234)", "zotero://select/library/items/ABCD1234", "zotero"],
    ["[note](obsidian://open?vault=Work&file=Plan)", "obsidian://open?vault=Work&file=Plan", "obsidian"],
  ] as const) {
    it(`asks, naming ${scheme}: and the full target, then hands ${dest} to the OS`, async () => {
      const confirm = vi.spyOn(backend(), "confirm").mockResolvedValue(true);
      const open = vi.spyOn(backend(), "openExternal").mockResolvedValue(undefined);
      clickLink(source);
      await flush();
      expect(confirm).toHaveBeenCalledOnce();
      const [message, title] = confirm.mock.calls[0];
      expect(`${title} ${message}`).toContain(`${scheme}:`);
      expect(message).toContain(dest);
      expect(open).toHaveBeenCalledExactlyOnceWith(dest, scheme);
      expect(toasts()).toEqual([]);
    });
  }

  it("declining the prompt opens nothing", async () => {
    vi.spyOn(backend(), "confirm").mockResolvedValue(false);
    const open = vi.spyOn(backend(), "openExternal").mockResolvedValue(undefined);
    clickLink("[abc](shortcuts://run-shortcut?name=abc)");
    await flush();
    expect(open).not.toHaveBeenCalled();
  });

  it("a refusal or failure after the confirm is shown, not swallowed", async () => {
    vi.spyOn(backend(), "confirm").mockResolvedValue(true);
    vi.spyOn(backend(), "openExternal").mockRejectedValue(new Error("no app handles zotero:"));
    clickLink("[item](zotero://select/items/A)");
    await flush();
    await flush();
    expect(toasts().map((toast) => toast.message).join("\n")).toContain("no app handles zotero:");
  });

  for (const [source, dest] of [
    ["[web](https://example.com/a)", "https://example.com/a"],
    ["[mail](mailto:a@example.com)", "mailto:a@example.com"],
    ["[file](file:///tmp/report.odt)", "file:///tmp/report.odt"],
  ] as const) {
    it(`${dest} keeps today's direct open, with no prompt`, async () => {
      const confirm = vi.spyOn(backend(), "confirm").mockResolvedValue(true);
      const open = vi.spyOn(backend(), "openExternal").mockResolvedValue(undefined);
      clickLink(source);
      await flush();
      expect(confirm).not.toHaveBeenCalled();
      expect(open).toHaveBeenCalledExactlyOnceWith(dest);
    });
  }

  it("javascript: and data: links never reach the prompt or the opener", async () => {
    const confirm = vi.spyOn(backend(), "confirm").mockResolvedValue(true);
    const open = vi.spyOn(backend(), "openExternal").mockResolvedValue(undefined);
    for (const source of ["[x](javascript:alert(1))", "[x](data:text/html,hi)"]) clickLink(source);
    await flush();
    expect(confirm).not.toHaveBeenCalled();
    expect(open).not.toHaveBeenCalled();
  });

  it("a Windows drive letter is not a scheme", () => {
    expect(linkScheme("C:\\notes\\a.txt")).toBeNull();
    expect(linkScheme("Zotero://x")).toBe("zotero");
  });

  it("the never-opened scheme list matches the native opener's (one rule, two layers)", () => {
    const ts = readFileSync("src/components/ExternalLink.tsx", "utf8");
    const rs = readFileSync("src-tauri/src/platform.rs", "utf8");
    const list = (text: RegExpExecArray | null) => [...(text?.[1] ?? "").matchAll(/"([a-z]+)"/g)].map((m) => m[1]).sort();
    const tsList = list(/NEVER_OPENED_SCHEMES = new Set\(\[([^\]]*)\]/.exec(ts));
    const rsList = list(/NEVER_OPENED_SCHEMES: &\[&str\] = &\[([^\]]*)\]/.exec(rs));
    expect(tsList.length).toBeGreaterThan(0);
    expect(tsList).toEqual(rsList);
  });
});
