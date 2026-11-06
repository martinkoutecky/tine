import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { setActiveLocale } from "../i18n";
import { autoEmbedVideoLinks, initPasteSettings, setAutoEmbedVideoLinks } from "../pasteSettings";
import { toasts, setToasts } from "../toasts";
import { closeSettings, openSettings } from "../ui";
import { Settings } from "./Settings";

const tick = () => new Promise(resolve => setTimeout(resolve, 0));
afterEach(async () => {
  closeSettings();
  vi.spyOn(backend(), "setAppBool").mockResolvedValue();
  setAutoEmbedVideoLinks(true); await tick();
  await setActiveLocale("en"); setToasts([]);
  vi.restoreAllMocks(); document.body.replaceChildren();
});

it.each(["en", "zh-CN"])("offers a searchable Editor preference and persists the toggle (%s, GH #861)", async locale => {
  await setActiveLocale(locale);
  const saved = vi.spyOn(backend(), "setAppBool").mockResolvedValue();
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <Settings />, root);
  openSettings("editor"); await tick();
  try {
    const row = root.querySelector('[data-setting-id="settings-auto-embed-video-links"]');
    expect(row).not.toBeNull();
    const toggle = row!.querySelector<HTMLButtonElement>('[role="switch"]')!;
    expect(toggle.getAttribute("aria-checked")).toBe("true");
    toggle.click(); await tick();
    expect(autoEmbedVideoLinks()).toBe(false);
    expect(toggle.getAttribute("aria-checked")).toBe("false");
    expect(saved).toHaveBeenCalledWith("auto_embed_video_links", false);
    const search = root.querySelector<HTMLInputElement>(".settings-search-input")!;
    search.value = "YouTube"; search.dispatchEvent(new InputEvent("input", { bubbles: true }));
    await tick();
    expect([...root.querySelectorAll(".settings-search-result")].some(result => result.textContent?.includes(row!.querySelector(".settings-label")!.textContent!))).toBe(true);
  } finally { dispose(); }
});

it("defaults to embedding, restores the saved choice and rolls back a failed write", async () => {
  const read = vi.spyOn(backend(), "getAppBool").mockImplementation(async (_key, fallback) => fallback);
  await initPasteSettings();
  expect(autoEmbedVideoLinks()).toBe(true);
  expect(read).toHaveBeenCalledWith("auto_embed_video_links", true);
  read.mockResolvedValue(false); await initPasteSettings();
  expect(autoEmbedVideoLinks()).toBe(false);
  vi.spyOn(backend(), "setAppBool").mockRejectedValue(new Error("disk"));
  setAutoEmbedVideoLinks(true);
  expect(autoEmbedVideoLinks()).toBe(true);
  await tick();
  expect(autoEmbedVideoLinks()).toBe(false);
  expect(toasts()).toHaveLength(1);
});

it("does not let a late startup read overwrite the user's new choice", async () => {
  let resolve!: (value: boolean) => void;
  vi.spyOn(backend(), "getAppBool").mockReturnValue(new Promise(done => { resolve = done; }));
  vi.spyOn(backend(), "setAppBool").mockResolvedValue();
  const loaded = initPasteSettings();
  setAutoEmbedVideoLinks(false);
  resolve(true); await loaded; await tick();
  expect(autoEmbedVideoLinks()).toBe(false);
});
