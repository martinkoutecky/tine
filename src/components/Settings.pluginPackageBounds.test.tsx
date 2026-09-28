import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { Settings } from "./Settings";
import { closeSettings, openSettings } from "../ui";
import { toasts } from "../toasts";
import { PLUGIN_MANIFEST_MAX_BYTES, PLUGIN_WASM_MAX_BYTES } from "../plugins/manifest";

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

function packageFile(name: string, size: number) {
  const text = vi.fn(async () => "{}");
  const arrayBuffer = vi.fn(async () => new ArrayBuffer(0));
  return { file: { name, size, text, arrayBuffer } as unknown as File, text, arrayBuffer };
}

async function choose(files: File[]) {
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <Settings />, root);
  openSettings("plugins");
  await tick();
  const input = root.querySelector<HTMLInputElement>('input[type="file"][accept*=".wasm"]')!;
  Object.defineProperty(input, "files", { value: files, configurable: true });
  input.dispatchEvent(new Event("change"));
  await tick();
  await tick();
  dispose();
}

afterEach(() => {
  closeSettings();
  document.body.innerHTML = "";
});

describe("local plugin package bounds (I-22)", () => {
  it("refuses an oversized .wasm before reading either file", async () => {
    const manifest = packageFile("manifest.json", 100);
    const wasm = packageFile("plugin.wasm", PLUGIN_WASM_MAX_BYTES + 1);
    await choose([manifest.file, wasm.file]);
    expect(manifest.text, "I-22: size is checked before any read").not.toHaveBeenCalled();
    expect(wasm.arrayBuffer, "I-22: size is checked before any read").not.toHaveBeenCalled();
    expect(toasts().map((toast) => toast.message).join("\n")).toMatch(/too large/);
  });

  it("refuses an oversized manifest before reading either file", async () => {
    const manifest = packageFile("manifest.json", PLUGIN_MANIFEST_MAX_BYTES + 1);
    const wasm = packageFile("plugin.wasm", 100);
    await choose([manifest.file, wasm.file]);
    expect(manifest.text).not.toHaveBeenCalled();
    expect(wasm.arrayBuffer).not.toHaveBeenCalled();
    expect(toasts().map((toast) => toast.message).join("\n")).toMatch(/too large/);
  });

  it("still reads a package at the limits", async () => {
    const manifest = packageFile("manifest.json", PLUGIN_MANIFEST_MAX_BYTES);
    const wasm = packageFile("plugin.wasm", PLUGIN_WASM_MAX_BYTES);
    await choose([manifest.file, wasm.file]);
    expect(manifest.text, "I-4: an in-bounds package is read and validated").toHaveBeenCalled();
  });
});
