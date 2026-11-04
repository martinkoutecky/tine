import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { ExternalLink, openOutboundLink } from "./ExternalLink";
import { LinkSchemePrompt } from "./LinkSchemePrompt";
import { initLinkSchemeSettings, rememberedLinkSchemes, setLinkSchemeRemembered } from "../linkSchemeSettings";
import { Settings } from "./Settings";
import { openSettings, closeSettings } from "../ui";
import { dismissTopTransient } from "../transientLayers";
import { setToasts, toasts } from "../toasts";
import { invalidateBinding } from "../binding";

const events = vi.hoisted(() => ({
  changed: null as null | ((event: { payload: { origin: string } }) => void),
  emit: vi.fn(async () => {}),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: async (name: string, callback: typeof events.changed) => {
    if (name === "link-scheme-consent-changed") events.changed = callback;
    return () => {};
  },
  emit: events.emit,
}));

const tick = () => new Promise(resolve => setTimeout(resolve, 0));
let dispose = () => {};
beforeEach(async () => {
  vi.spyOn(backend(), "getAppString").mockResolvedValue("[]");
  vi.spyOn(backend(), "setAppString").mockResolvedValue();
  vi.spyOn(backend(), "openExternal").mockResolvedValue();
  await initLinkSchemeSettings();
  const host = document.createElement("div"); document.body.append(host);
  dispose = render(() => <><ExternalLink dest="shortcuts://run-shortcut?name=abc">abc</ExternalLink><LinkSchemePrompt /><Settings /></>, host);
});
afterEach(() => { dispose(); closeSettings(); document.body.replaceChildren(); vi.restoreAllMocks(); setToasts([]); });

async function prompt(url = "shortcuts://run-shortcut?name=abc") {
  const pending = openOutboundLink(url);
  await tick();
  return { pending, form: document.querySelector<HTMLFormElement>('.link-scheme-prompt')! };
}

function chooseAlways(form: HTMLFormElement) {
  form.querySelector<HTMLInputElement>('input[type="checkbox"]')!.click();
  form.requestSubmit();
}

it("offers to remember consent for a trusted app scheme", async () => {
  document.querySelector("a")!.click();
  await tick();
  expect(document.querySelector('input[type="checkbox"]')).not.toBeNull();
  expect(document.body.textContent).toContain("Always open shortcuts links");
});

it("remembers only the exact lowercased scheme across hosts and paths, then forgetting in Settings asks again", async () => {
  const { pending, form } = await prompt();
  expect(form.querySelector<HTMLInputElement>('input[type="checkbox"]')!.checked).toBe(false);
  chooseAlways(form); await pending; await tick();
  expect(backend().setAppString).toHaveBeenCalledWith("always_open_link_schemes", '["shortcuts"]');
  await openOutboundLink("Shortcuts://another-host/another-path");
  expect(document.querySelector('.link-scheme-prompt')).toBeNull();
  expect(backend().openExternal).toHaveBeenLastCalledWith("Shortcuts://another-host/another-path", "shortcuts");
  for (const url of ["shortcuts-evil://run", "obsidian://open"]) {
    const next = await prompt(url);
    expect(next.form).not.toBeNull();
    next.form.querySelector<HTMLButtonElement>('button[type="button"]')!.click();
    await next.pending;
  }
  openSettings("editor"); await tick();
  const forget = document.querySelector<HTMLButtonElement>('button[aria-label="Forget shortcuts links"]');
  expect(forget).not.toBeNull(); forget!.click(); await tick();
  expect(backend().setAppString).toHaveBeenLastCalledWith("always_open_link_schemes", "[]");
  closeSettings(); await tick();
  const next = await prompt();
  expect(next.form).not.toBeNull();
});

it("opening once and cancelling a checked choice do not remember consent", async () => {
  const once = await prompt(); once.form.requestSubmit(); await once.pending;
  expect(rememberedLinkSchemes()).toEqual([]);
  const again = await prompt();
  again.form.querySelector<HTMLInputElement>('input')!.click();
  again.form.querySelector<HTMLButtonElement>('button[type="button"]')!.click(); await again.pending;
  expect(backend().openExternal).toHaveBeenCalledTimes(1);
  expect(backend().setAppString).not.toHaveBeenCalled();
});

it("Escape and outside dismissal never save checked consent", async () => {
  const escape = await prompt(); escape.form.querySelector<HTMLInputElement>('input')!.click();
  dismissTopTransient("escape"); await escape.pending;
  const outside = await prompt(); outside.form.querySelector<HTMLInputElement>('input')!.click();
  (outside.form.parentElement as HTMLElement).click(); await outside.pending;
  expect(backend().openExternal).not.toHaveBeenCalled();
  expect(backend().setAppString).not.toHaveBeenCalled();
});

it("restores consent through the startup settings read and ignores malformed or forbidden grants", async () => {
  vi.mocked(backend().getAppString).mockResolvedValue('["Shortcuts","shortcuts","javascript","data","file","https","tine","shortcuts://host","shortcuts-evil"]');
  await initLinkSchemeSettings();
  expect(rememberedLinkSchemes()).toEqual(["shortcuts", "shortcuts-evil"]);
  await openOutboundLink("shortcuts://new-path");
  expect(document.querySelector('.link-scheme-prompt')).toBeNull();
  expect(backend().openExternal).toHaveBeenCalledExactlyOnceWith("shortcuts://new-path", "shortcuts");
  for (const stored of ["oops", "{}", "null", '[4,null,"javascript","data","file"]']) {
    vi.mocked(backend().getAppString).mockResolvedValue("[]"); await initLinkSchemeSettings();
    vi.mocked(backend().getAppString).mockResolvedValue(stored); await initLinkSchemeSettings();
    expect(rememberedLinkSchemes()).toEqual([]);
  }
});

it("refused and direct schemes cannot be remembered, even from stored settings", async () => {
  for (const scheme of ["javascript", "vbscript", "data", "blob", "about", "file", "http", "https", "mailto", "tine", "shortcuts://x", "a"]) {
    setLinkSchemeRemembered(scheme, true);
  }
  await tick();
  expect(rememberedLinkSchemes()).toEqual([]);
  expect(backend().setAppString).not.toHaveBeenCalled();
  for (const url of ["javascript:alert(1)", "data:text/html,hi", "vbscript:foo", "blob:foo", "about:blank"]) await openOutboundLink(url);
  expect(backend().openExternal).not.toHaveBeenCalled();
  expect(document.querySelector('.link-scheme-prompt')).toBeNull();
});

it("failed persistence restores the prior choice and reports the failure", async () => {
  vi.mocked(backend().setAppString).mockRejectedValue(new Error("disk"));
  const first = await prompt(); chooseAlways(first.form); await first.pending; await tick();
  expect(rememberedLinkSchemes()).toEqual([]);
  expect(toasts().some(toast => toast.kind === "error")).toBe(true);
  const again = await prompt(); expect(again.form).not.toBeNull();
});

it("a click during startup waits for remembered consent instead of asking on the default", async () => {
  let resolve!: (value: string) => void;
  vi.mocked(backend().getAppString).mockReturnValue(new Promise(done => { resolve = done; }));
  const loading = initLinkSchemeSettings();
  const opening = openOutboundLink("shortcuts://startup");
  await tick();
  expect(document.querySelector('.link-scheme-prompt')).toBeNull();
  expect(backend().openExternal).not.toHaveBeenCalled();
  resolve('["shortcuts"]'); await loading; await opening;
  expect(backend().openExternal).toHaveBeenCalledExactlyOnceWith("shortcuts://startup", "shortcuts");
});

it("a stale settings read cannot restore consent after the user forgets it", async () => {
  let resolve!: (value: string) => void;
  vi.mocked(backend().getAppString).mockReturnValue(new Promise(done => { resolve = done; }));
  const loading = initLinkSchemeSettings();
  setLinkSchemeRemembered("shortcuts", false);
  resolve('["shortcuts"]'); await loading; await tick();
  expect(rememberedLinkSchemes()).toEqual([]);
});

it("a graph switch cancels checked consent without persisting or launching", async () => {
  const first = await prompt();
  first.form.querySelector<HTMLInputElement>('input')!.click();
  invalidateBinding(); await first.pending;
  expect(document.querySelector('.link-scheme-prompt')).toBeNull();
  expect(backend().openExternal).not.toHaveBeenCalled();
  expect(backend().setAppString).not.toHaveBeenCalled();
});

it("a failed startup read grants no consent and still asks", async () => {
  vi.mocked(backend().getAppString).mockRejectedValue(new Error("disk"));
  await initLinkSchemeSettings();
  const first = await prompt(); expect(first.form).not.toBeNull();
  expect(backend().openExternal).not.toHaveBeenCalled();
  expect(toasts().some(toast => toast.kind === "error")).toBe(true);
});

it("a second gesture replaces a checked prompt with a fresh unchecked choice for its own target", async () => {
  const first = await prompt(); first.form.querySelector<HTMLInputElement>('input')!.click();
  const second = await prompt("obsidian://different-target"); await first.pending;
  expect(second.form.textContent).toContain("obsidian://different-target");
  expect(second.form.querySelector<HTMLInputElement>('input')!.checked).toBe(false);
  second.form.requestSubmit(); await second.pending;
  expect(backend().openExternal).toHaveBeenCalledExactlyOnceWith("obsidian://different-target", "obsidian");
  expect(backend().setAppString).not.toHaveBeenCalled();
});

it("consent saved or forgotten in another open WebView refreshes the device setting", async () => {
  await tick();
  expect(events.changed).not.toBeNull();
  vi.mocked(backend().getAppString).mockResolvedValue('["shortcuts"]');
  events.changed!({ payload: { origin: "another-webview" } });
  await tick();
  await openOutboundLink("shortcuts://from-another-window");
  expect(backend().openExternal).toHaveBeenCalledExactlyOnceWith("shortcuts://from-another-window", "shortcuts");
  vi.mocked(backend().getAppString).mockResolvedValue("[]");
  events.changed!({ payload: { origin: "another-webview" } });
  await tick();
  const next = await prompt();
  expect(next.form).not.toBeNull();
});
