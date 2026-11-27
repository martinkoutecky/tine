import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { readFileSync } from "node:fs";
import { render } from "solid-js/web";
import { InlineText } from "./inline";
import { initParser } from "./parse";
import { backend } from "../backend";
import { setToasts, toasts } from "../toasts";
import { linkScheme } from "../components/ExternalLink";
import { LinkSchemePrompt } from "../components/LinkSchemePrompt";
import { initLinkSchemeSettings } from "../linkSchemeSettings";
import schemes from "../../src-tauri/external-link-schemes.json";
import { refusedLinkScheme } from "../linkSchemePolicy";

// GH #503 (UI-CUSTOM-SCHEME-LINK-503): a link to another app's scheme
// (`shortcuts://`, `zotero://`, `obsidian://` …) opens in that app after the
// user confirms a prompt naming the scheme and the full target. Before the fix
// the native opener refused every scheme but http(s)/mailto/file, so the click
// only produced "unsupported url scheme" (iOS 0.6.982 included).

beforeAll(initParser);
const disposers: (() => void)[] = [];
beforeEach(async () => {
  vi.spyOn(backend(), "getAppString").mockResolvedValue("[]");
  await initLinkSchemeSettings();
  const host = document.createElement("div"); document.body.append(host);
  disposers.push(render(() => <LinkSchemePrompt />, host));
});
afterEach(() => {
  disposers.splice(0).forEach(dispose => dispose());
  vi.restoreAllMocks();
  document.body.replaceChildren();
  setToasts([]);
});

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

function clickLink(source: string): void {
  const host = document.createElement("div");
  document.body.append(host);
  disposers.push(render(() => <InlineText text={source} />, host));
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
      const open = vi.spyOn(backend(), "openExternal").mockResolvedValue(undefined);
      clickLink(source);
      await flush();
      const prompt = document.querySelector<HTMLFormElement>('[role="dialog"]')!;
      expect(prompt.textContent).toContain(`${scheme}:`);
      expect(prompt.textContent).toContain(dest);
      prompt.requestSubmit();
      await flush();
      expect(open).toHaveBeenCalledExactlyOnceWith(dest, scheme);
      expect(toasts()).toEqual([]);
    });
  }

  it("declining the prompt opens nothing", async () => {
    const open = vi.spyOn(backend(), "openExternal").mockResolvedValue(undefined);
    clickLink("[abc](shortcuts://run-shortcut?name=abc)");
    await flush();
    document.querySelector<HTMLButtonElement>('[role="dialog"] button[type="button"]')!.click();
    await flush();
    expect(open).not.toHaveBeenCalled();
  });

  it("a refusal or failure after the confirm is shown, not swallowed", async () => {
    vi.spyOn(backend(), "openExternal").mockRejectedValue(new Error("no app handles zotero:"));
    clickLink("[item](zotero://select/items/A)");
    await flush();
    document.querySelector<HTMLFormElement>('[role="dialog"]')!.requestSubmit();
    await flush();
    expect(toasts().map((toast) => toast.message).join("\n")).toContain("no app handles zotero:");
  });

  for (const [source, dest] of [
    ["[web](https://example.com/a)", "https://example.com/a"],
    ["[mail](mailto:a@example.com)", "mailto:a@example.com"],
    ["[file](file:///tmp/report.odt)", "file:///tmp/report.odt"],
  ] as const) {
    it(`${dest} keeps today's direct open, with no prompt`, async () => {
      const open = vi.spyOn(backend(), "openExternal").mockResolvedValue(undefined);
      clickLink(source);
      await flush();
      expect(document.querySelector('[role="dialog"]')).toBeNull();
      // tine#985: a file: call carries its purpose and the click's generation;
      // native answers it (here: opened, so no prompt).
      if (dest.startsWith("file:")) expect(open).toHaveBeenCalledExactlyOnceWith(dest, undefined,
        { purpose: "link", bindingGeneration: backend().graphBindingGeneration(), show: undefined });
      else expect(open).toHaveBeenCalledExactlyOnceWith(dest);
    });
  }

  it("javascript: and data: links never reach the prompt or the opener", async () => {
    const open = vi.spyOn(backend(), "openExternal").mockResolvedValue(undefined);
    for (const source of ["[x](javascript:alert(1))", "[x](data:text/html,hi)"]) clickLink(source);
    await flush();
    expect(document.querySelector('[role="dialog"]')).toBeNull();
    expect(open).not.toHaveBeenCalled();
  });

  it("a Windows drive letter is not a scheme", () => {
    expect(linkScheme("C:\\notes\\a.txt")).toBeNull();
    expect(linkScheme("Zotero://x")).toBe("zotero");
  });

  it("the native and browser openers use the same scheme table", () => {
    const ts = readFileSync("src/linkSchemePolicy.ts", "utf8");
    const rs = readFileSync("src-tauri/src/platform.rs", "utf8");
    const build = readFileSync("crates/tine-core/build.rs", "utf8");
    const core = readFileSync("crates/tine-core/src/link_scheme.rs", "utf8");
    expect(ts).toContain('from "../src-tauri/external-link-schemes.json"');
    // Core compiles the list in and answers membership; the native gate asks core.
    expect(build).toContain('"../../src-tauri/external-link-schemes.json"');
    expect(core).toContain('include!(concat!(env!("OUT_DIR"), "/refused_schemes.rs"))');
    expect(rs).not.toMatch(/NEVER_OPENED_SCHEMES: &\[&str\] = &\[/);
    expect(rs).not.toContain("external_link_schemes.rs");
    expect(rs).toContain("tine_core::link_scheme::refused_scheme(&scheme)");
    expect(schemes.refused.length).toBeGreaterThan(0);
    expect(schemes.refused.every(refusedLinkScheme)).toBe(true);
  });
});
