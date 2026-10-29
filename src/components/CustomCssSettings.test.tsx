import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { CustomCssSettings } from "./CustomCssSettings";
import { backend } from "../backend";
import * as platform from "../platform";
import { setToasts, toasts } from "../toasts";
import { paletteCommands } from "../keybindings";

const mobile = vi.hoisted(() => ({ value: false }));
vi.mock("../nativeChrome", async importOriginal => ({
  ...await importOriginal<typeof import("../nativeChrome")>(),
  get isMobilePlatform() { return mobile.value; },
}));

let dispose = () => {};
afterEach(() => { dispose(); vi.restoreAllMocks(); mobile.value = false; document.body.innerHTML = ""; setToasts([]); });
function mount() {
  const root = document.createElement("div");
  document.body.append(root);
  dispose = render(() => <CustomCssSettings />, root);
  return root;
}

it("keeps both in-app editors and opens graph and global CSS in the default app with visible failures", async () => {
  vi.spyOn(platform, "platformKind").mockResolvedValue("desktop");
  const graph = vi.spyOn(backend(), "editCustomCss").mockRejectedValue(new Error("No default app for .css"));
  const root = mount();
  await vi.waitFor(() => expect([...root.querySelectorAll("button")].filter(b => b.textContent === "Open in default app")).toHaveLength(2));
  const global = vi.spyOn(backend(), "editGlobalCss").mockRejectedValue(new Error("Global CSS opener failed"));
  expect([...root.querySelectorAll("button")].map(b => b.textContent)).toEqual(expect.arrayContaining(["Edit custom.css", "Edit global CSS"]));
  const buttons = [...root.querySelectorAll("button")].filter(b => b.textContent === "Open in default app");
  buttons[0].click();
  await vi.waitFor(() => expect(graph).toHaveBeenCalledOnce());
  await vi.waitFor(() => expect(toasts().some(t => t.kind === "error" && String(t.message).includes("No default app for .css"))).toBe(true));
  buttons[1].click();
  await vi.waitFor(() => expect(global).toHaveBeenCalledOnce());
  await vi.waitFor(() => expect(toasts().some(t => t.kind === "error" && t.message.includes("Global CSS opener failed"))).toBe(true));
});

it("offers both desktop palette actions alongside the in-app editors and uses the same handoff", async () => {
  const graph = vi.spyOn(backend(), "editCustomCss").mockResolvedValue();
  const global = vi.spyOn(backend(), "editGlobalCss").mockResolvedValue();
  const commands = paletteCommands().filter(c => c.id.startsWith("appearance/") && c.id.includes("css"));
  expect(commands.map(c => c.label)).toEqual([
    "Edit custom.css", "Open custom.css in default app",
    "Edit global CSS (global.css)", "Open global CSS (global.css) in default app",
  ]);
  commands[1].run();
  commands[3].run();
  await vi.waitFor(() => { expect(graph).toHaveBeenCalledOnce(); expect(global).toHaveBeenCalledOnce(); });
  mobile.value = true;
  expect(paletteCommands().some(c => c.id.includes("css-default-app"))).toBe(false);
});

it.each(["android", "ios"] as const)("hides external CSS actions on %s", async kind => {
  vi.spyOn(platform, "platformKind").mockResolvedValue(kind);
  const root = mount();
  await Promise.resolve();
  expect(root.textContent).not.toContain("Open in default app");
  expect(root.textContent).toContain("Edit custom.css");
  expect(root.textContent).toContain("Edit global CSS");
});
