import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "./backend";
import * as css from "./customCss";
import { GraphPublish } from "./components/GraphPublish";
import { injectCustomCss } from "./graph";
import { loadGlobalCss, openCssEditor, setCssEditorText, flushCssEditor, retireGraphCssEditor } from "./cssEditor";
import * as published from "./publishedBackend";
import { setToasts, toasts } from "./toasts";

afterEach(() => { document.body.innerHTML = ""; vi.restoreAllMocks(); });

it("keeps an external revision written after the durable save but before its IPC reply", async () => {
  vi.spyOn(backend(), "getAppString").mockResolvedValue("");
  vi.spyOn(backend(), "setAppString").mockResolvedValue();
  vi.spyOn(backend(), "readCssFile").mockResolvedValue({ text: "a { color: black }", revision: "base" });
  await openCssEditor(false);
  setCssEditorText("a { color: red }");
  let finishSave!: (revision: string) => void;
  vi.spyOn(backend(), "saveCssFile").mockReturnValue(new Promise(resolve => { finishSave = resolve; }));
  const saving = flushCssEditor();
  let finishReload!: (css: string) => void;
  vi.spyOn(backend(), "readCustomCss")
    .mockImplementationOnce(() => new Promise(resolve => { finishReload = resolve; }))
    .mockResolvedValue("a { color: blue }");
  const reloading = injectCustomCss(true);
  await vi.waitFor(() => expect(backend().saveCssFile).toHaveBeenCalled());
  finishSave("red-revision");
  await saving;
  finishReload("a { color: blue }");
  await reloading;
  const observed = document.querySelector("#tine-custom-css")?.textContent;
  retireGraphCssEditor();
  expect(observed).toBe("a { color: blue }");
});

it("layers device CSS before graph CSS and disables both in safe mode", () => {
  (css as typeof css & { applyGlobalCss: (text: string) => void }).applyGlobalCss("a { color: blue }");
  css.applyCustomCss("a { color: red }");
  const styles = [...document.head.querySelectorAll("style")];
  const global = styles.find((el) => el.id === "tine-global-css")!;
  const graph = styles.find((el) => el.id === "tine-custom-css")!;
  expect(styles.indexOf(global)).toBeLessThan(styles.indexOf(graph));
  css.setCustomCssDisabled(true);
  expect(global.textContent).toBe("");
  expect(graph.textContent).toBe("");
  css.setCustomCssDisabled(false);
  expect(global.textContent).toContain("blue");
});

it("remembers a picked export directory and offers to reveal the exported site", async () => {
  vi.spyOn(backend(), "getAppString").mockResolvedValue("/previous/export");
  const remember = vi.spyOn(backend(), "setAppString").mockResolvedValue();
  vi.spyOn(backend(), "pickFolder").mockResolvedValue("/chosen/export");
  vi.spyOn(backend(), "publishLive").mockResolvedValue({ path: "/chosen/export/site", pages: 1 } as never);
  const reveal = vi.spyOn(backend(), "revealExport").mockResolvedValue();
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <GraphPublish />, root);
  [...root.querySelectorAll("button")].find((el) => el.textContent?.includes("Export HTML"))!.click();
  await vi.waitFor(() => expect(remember).toHaveBeenCalledWith("html_export_directory", "/chosen/export"));
  await vi.waitFor(() => expect(root.textContent).toContain("Show in folder"));
  expect(backend().pickFolder).toHaveBeenCalledWith(expect.any(String), "/previous/export");
  [...root.querySelectorAll("button")].find((el) => el.textContent === "Show in folder")!.click();
  await vi.waitFor(() => expect(reveal).toHaveBeenCalledWith("/chosen/export/site"));
  dispose();
});

it("a CSS save supersedes an older in-flight graph reload", async () => {
  let finish!: (text: string) => void;
  vi.spyOn(backend(), "readCustomCss").mockReturnValue(new Promise((resolve) => { finish = resolve; }));
  const loading = injectCustomCss(true);
  css.applyCustomCss("a { color: green }");
  finish("a { color: old }");
  await loading;
  expect(document.querySelector("#tine-custom-css")?.textContent).toBe("a { color: green }");
});

it("a CSS save supersedes an older in-flight device reload", async () => {
  let finish!: (text: { text: string; revision: string }) => void;
  vi.spyOn(backend(), "readCssFile").mockReturnValue(new Promise((resolve) => { finish = resolve; }));
  const loading = loadGlobalCss();
  css.applyGlobalCss("a { color: green }");
  finish({ text: "a { color: old }", revision: "old" });
  await loading;
  expect(document.querySelector("#tine-global-css")?.textContent).toBe("a { color: green }");
});

it("opening a published graph does not request device CSS or show a read-only CSS error", async () => {
  vi.spyOn(published, "isPublishedExport").mockReturnValue(true);
  const read = vi.spyOn(backend(), "readCssFile").mockRejectedValue(new Error("read-only export"));
  setToasts([]);
  await loadGlobalCss();
  expect(toasts().some((toast) => toast.kind === "error")).toBe(false);
  expect(read).not.toHaveBeenCalled();
});
