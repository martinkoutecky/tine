import { afterEach, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { toasts, setToasts } from "./toasts";
import { assetNameFormat, setAssetNameFormat, initAssetSettings } from "./assetSettings";
import { launcherRankingEnabled, setLauncherRankingEnabled } from "./launcherRanking";
import { mediaEditorCommand, setMediaEditorCommand } from "./mediaEditorSettings";
import { copyIncludeSubtree, setCopyIncludeSubtree } from "./copySettings";
import { navReuseTabs, setNavReuseTabs } from "./navSettings";
import { allowLocalFileImages, setAllowLocalFileImages } from "./localFileSettings";
import { spellcheckEnabled, setSpellcheckEnabled } from "./spellcheckSettings";
import { linkAutocompletePolicy, setLinkAutocompletePolicy } from "./editor/linkDefault";
import { selectedGalleryTheme, applyTheme } from "./themeGallery";
import { setSmoothScroll, smoothScrollEnabled } from "./smoothScroll";
import { initThemePackages } from "./themes/manager";

const flush = async () => { await Promise.resolve(); await Promise.resolve(); await Promise.resolve(); };

afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); setToasts([]); });

it("restores device preferences and reports a rejected write", async () => {
  vi.spyOn(backend(), "setAppString").mockRejectedValue(new Error("disk full"));
  vi.spyOn(backend(), "setAppBool").mockRejectedValue(new Error("disk full"));
  const beforeAsset = assetNameFormat();
  const beforeRanking = launcherRankingEnabled();
  const beforeMedia = mediaEditorCommand("editor-test");
  const beforeCopy = copyIncludeSubtree();
  setAssetNameFormat("failed.%ext");
  setLauncherRankingEnabled(!beforeRanking);
  setMediaEditorCommand("editor-test", "failed-editor");
  setCopyIncludeSubtree(!beforeCopy);
  await flush();
  expect(assetNameFormat()).toBe(beforeAsset);
  expect(launcherRankingEnabled()).toBe(beforeRanking);
  expect(mediaEditorCommand("editor-test")).toBe(beforeMedia);
  expect(copyIncludeSubtree()).toBe(beforeCopy);
  expect(toasts().filter((toast) => toast.kind === "error")).toHaveLength(4);
});

it("restores editor and navigation settings on a rejected write", async () => {
  vi.spyOn(backend(), "setAppString").mockRejectedValue(new Error("disk full"));
  vi.spyOn(backend(), "setAppBool").mockRejectedValue(new Error("disk full"));
  vi.spyOn(backend(), "applySpellcheck").mockResolvedValue();
  const nav = navReuseTabs();
  const images = allowLocalFileImages();
  const spell = spellcheckEnabled();
  const link = linkAutocompletePolicy();
  setNavReuseTabs(!nav);
  setAllowLocalFileImages(!images);
  setSpellcheckEnabled(!spell);
  setLinkAutocompletePolicy(link === "typed" ? "adaptive" : "typed");
  await flush();
  expect(navReuseTabs()).toBe(nav);
  expect(allowLocalFileImages()).toBe(images);
  expect(spellcheckEnabled()).toBe(spell);
  expect(linkAutocompletePolicy()).toBe(link);
  expect(toasts().filter((toast) => toast.kind === "error")).toHaveLength(4);
});

it("restores the selected gallery theme when persistence fails", async () => {
  vi.spyOn(backend(), "setAppString").mockRejectedValue(new Error("disk full"));
  const before = selectedGalleryTheme();
  applyTheme(before === "nord" ? "solarized" : "nord");
  await flush();
  expect(selectedGalleryTheme()).toBe(before);
  expect(toasts().some((toast) => toast.kind === "error")).toBe(true);
});

it("restores smooth scrolling when persistence fails", async () => {
  vi.stubGlobal("document", { querySelector: () => null });
  vi.spyOn(backend(), "setSmoothScroll").mockRejectedValue(new Error("disk full"));
  setSmoothScroll(true);
  await flush();
  expect(smoothScrollEnabled()).toBe(false);
  expect(toasts().some((toast) => toast.kind === "error")).toBe(true);
  vi.unstubAllGlobals();
});

it("reports a failed preference load", async () => {
  vi.spyOn(backend(), "getAppString").mockRejectedValue(new Error("disk unreadable"));
  await initAssetSettings();
  expect(toasts().some((toast) => toast.kind === "error")).toBe(true);
});

it("reports a failed installed-theme load", async () => {
  vi.spyOn(backend(), "getAppString").mockRejectedValue(new Error("disk unreadable"));
  await initThemePackages();
  expect(toasts().some((toast) => toast.kind === "error")).toBe(true);
});

it("restores the last confirmed value after a later write fails", async () => {
  const first = "confirmed.%ext";
  const previous = assetNameFormat();
  let finish!: () => void;
  vi.spyOn(backend(), "setAppString").mockImplementation((key) => {
    if (key !== "asset_name_format") return Promise.resolve();
    if (assetNameFormat() === first) return new Promise<void>((resolve) => { finish = resolve; });
    return Promise.reject(new Error("disk full"));
  });
  setAssetNameFormat(first);
  await flush();
  setAssetNameFormat("rejected.%ext");
  finish();
  await flush();
  await flush();
  expect(assetNameFormat()).toBe(first);
  expect(toasts().some((toast) => toast.kind === "error")).toBe(true);
  // Restore this test's module state using a successful write.
  vi.spyOn(backend(), "setAppString").mockResolvedValue();
  setAssetNameFormat(previous);
  await flush();
});

it("does not let a late startup read overwrite a manual asset format", async () => {
  let finish!: (value: string) => void;
  vi.spyOn(backend(), "getAppString").mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
  vi.spyOn(backend(), "setAppString").mockResolvedValue();
  const pending = initAssetSettings();
  setAssetNameFormat("manual.%ext");
  finish("older.%ext");
  await pending;
  expect(assetNameFormat()).toBe("manual.%ext");
});
