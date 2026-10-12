import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { activeLocale, setActiveLocale } from "../i18n";
import { startUiLanguage } from "../i18n/preference";
import { LanguageSetting } from "./LanguageSetting";

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

afterEach(async () => {
  await setActiveLocale("en");
  document.body.innerHTML = "";
  vi.restoreAllMocks();
});

// tine#107: the language picker switches the UI live, persists the device
// preference, and hands native code the resolved tag (never the raw choice).
describe("the language setting", () => {
  it("switches the UI live, persists the choice and reports the resolved locale", async () => {
    const saved = vi.spyOn(backend(), "setAppString").mockResolvedValue();
    const native = vi.spyOn(backend(), "setUiLocale").mockResolvedValue();
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <LanguageSetting />, root);
    expect(root.querySelector(".settings-label")!.textContent).toBe("Language");

    const select = root.querySelector("select")!;
    select.value = "zh-CN";
    select.dispatchEvent(new Event("change", { bubbles: true }));
    await vi.waitFor(() => expect(activeLocale()).toBe("zh-CN"));
    expect(root.querySelector(".settings-label")!.textContent).toBe("语言");
    expect(document.documentElement.lang).toBe("zh-CN");
    await settle();
    expect(saved).toHaveBeenCalledWith("ui_language", "zh-CN");
    expect(native).toHaveBeenLastCalledWith("zh-CN");
    // The search join key does not follow the language.
    expect(root.querySelector("[data-setting-label]")!.getAttribute("data-setting-label")).toBe("Language");
    dispose();
  });

  it("starts in the stored language before anything mounts, and falls back to the system's on a slow read", async () => {
    vi.spyOn(backend(), "setUiLocale").mockResolvedValue();
    vi.spyOn(backend(), "getAppString").mockResolvedValue("zh-CN");
    await startUiLanguage();
    expect(activeLocale()).toBe("zh-CN");

    vi.useFakeTimers();
    try {
      vi.spyOn(backend(), "getAppString").mockReturnValue(new Promise(() => {}));
      const started = startUiLanguage();
      await vi.advanceTimersByTimeAsync(500);
      await started;
      // jsdom's navigator is en-US.
      expect(activeLocale()).toBe("en");
    } finally {
      vi.useRealTimers();
    }
  });

  it("applies a slow stored choice when it lands, and keeps the system's language on a failed read", async () => {
    vi.spyOn(backend(), "setUiLocale").mockResolvedValue();
    let land!: (value: string) => void;
    vi.spyOn(backend(), "getAppString").mockReturnValue(new Promise((resolve) => { land = resolve; }));
    vi.useFakeTimers();
    try {
      const started = startUiLanguage();
      await vi.advanceTimersByTimeAsync(500);
      await started;
      expect(activeLocale()).toBe("en");
      land("zh-CN");
      await vi.waitFor(() => expect(activeLocale()).toBe("zh-CN"));

      const toasts = await import("../toasts");
      vi.spyOn(backend(), "getAppString").mockRejectedValue(new Error("settings unreadable"));
      await startUiLanguage();
      expect(activeLocale()).toBe("en");
      expect(toasts.toasts().some((toast) => toast.key.startsWith("toast-language-load-failed:"))).toBe(true);
    } finally {
      vi.useRealTimers();
    }
  });
});
