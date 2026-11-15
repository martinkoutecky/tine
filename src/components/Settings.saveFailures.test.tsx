import { afterEach, expect, it, vi } from "vitest";

afterEach(() => { vi.restoreAllMocks(); document.body.replaceChildren(); });

it("renders graph and device save failures in Chinese and follows a live language change", async () => {
  vi.resetModules();
  const { render } = await import("solid-js/web");
  const { backend } = await import("../backend");
  const { setActiveLocale } = await import("../i18n");
  const { setGraphMeta } = await import("../graphSession");
  const { changeShowBrackets } = await import("../ui");
  const { setAssetNameFormat } = await import("../assetSettings");
  const { setNavReuseTabs, navReuseTabs } = await import("../navSettings");
  const { toasts, setToasts } = await import("../toasts");
  const { Toasts } = await import("./Toasts");
  const loaded = await backend().loadGraph("");
  if (loaded.kind === "focused_existing") throw new Error("missing graph");
  setGraphMeta({ ...loaded.meta, root: "/synthetic", show_brackets: true });
  vi.spyOn(backend(), "setShowBrackets").mockRejectedValue(new Error("disk full"));
  vi.spyOn(backend(), "setAppString").mockRejectedValue(new Error("disk full"));
  vi.spyOn(backend(), "setAppBool").mockRejectedValue(new Error("disk full"));
  await setActiveLocale("zh-CN");
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <Toasts />, root);
  try {
    changeShowBrackets(false);
    setAssetNameFormat("failure.%ext");
    setNavReuseTabs(!navReuseTabs());
    await vi.waitFor(() => expect(root.querySelectorAll(".toast-error")).toHaveLength(3));
    const chinese = [...root.querySelectorAll(".toast-msg")].map(node => node.textContent);
    expect(chinese).toEqual(expect.arrayContaining([
      "无法保存显示括号。", "无法保存新附件文件名。", "无法保存导航时切换到已打开的标签页。",
    ]));
    await setActiveLocale("en");
    const english = [...root.querySelectorAll(".toast-msg")].map(node => node.textContent);
    expect(english).toEqual(toasts().map(toast => toast.message));
    expect(english.every(text => text?.startsWith("Could not save"))).toBe(true);
  } finally { dispose(); setGraphMeta(null); setToasts([]); await setActiveLocale("en"); }
});
