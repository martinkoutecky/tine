import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { onMount } from "solid-js";
import { InlineText } from "./inline";
import { AstBody } from "./body";
import { Lightbox } from "../components/Toasts";
import { setLightbox } from "../ui";
import { setToasts, toasts } from "../toasts";
import { backend } from "../backend";
import { invalidateBinding } from "../binding";
import { assetNameForBlobUrl, clearAssetBlobCache, refreshAsset, seedAssetBlob } from "../assetCache";

afterEach(() => {
  clearAssetBlobCache();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  document.body.replaceChildren();
  setLightbox(null);
  setToasts([]);
});

for (const renderer of ["InlineText", "AstBody"] as const) {
  it.each([
    { label: "Markdown", text: "![photo](../assets/photo.jpg)", format: "md" as const },
    { label: "Org", text: "[[../assets/photo.jpg]]", format: "org" as const },
    { label: "img macro", text: "{{img ../assets/photo.jpg}}", format: "md" as const },
  ])(`${renderer} $label copies the file after preview refusal`, async ({ text, format }) => {
    vi.spyOn(backend(), "readAsset").mockRejectedValue(new Error("asset-too-large"));
    const copy = vi.spyOn(backend(), "copyAssetFileToClipboard").mockResolvedValue(true);
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(() => renderer === "InlineText"
      ? <InlineText text={text} format={format} blockId="block" />
      : <AstBody raw={text} format={format} blockId="block" />, host);
    try {
      await vi.waitFor(() => expect(host.querySelector(".inline-image-unavailable")).not.toBeNull());
      const button = host.querySelector<HTMLButtonElement>('button[title="Copy image"]');
      expect(button).not.toBeNull();
      button!.click();
      await vi.waitFor(() => expect(copy).toHaveBeenCalledWith("photo.jpg"));
      expect(host.querySelector(".img-resize-grip")).toBeNull();
      if (text.startsWith("{{")) expect(host.querySelector(".asset-action-trash")).toBeNull();
    } finally { dispose(); }
  });
}

it("loaded image macros offer copy without unsupported resize or removal", async () => {
  vi.stubGlobal("URL", { createObjectURL: () => "blob:macro", revokeObjectURL: vi.fn() });
  vi.spyOn(backend(), "readAsset").mockResolvedValue(new Uint8Array([1, 2, 3]));
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <InlineText text="{{img ../assets/photo.jpg}}" blockId="block" />, host);
  try {
    await vi.waitFor(() => expect(host.querySelector("img.inline-image")).not.toBeNull());
    expect(host.querySelector('button[title="Copy image"]')).not.toBeNull();
    expect(host.querySelector(".img-resize-grip")).toBeNull();
    expect(host.querySelector(".asset-action-trash")).toBeNull();
  } finally { dispose(); }
});

it.each([false, true])("GH #745: a remounted loaded image has its source before focus (evicted=%s)", async (evicted) => {
  let next = 0;
  vi.stubGlobal("URL", { createObjectURL: () => next++ === 0 ? "blob:loaded" : `blob:other-${next}`, revokeObjectURL: vi.fn() });
  const read = vi.spyOn(backend(), "readAsset").mockResolvedValue(new Uint8Array([1, 2, 3]));
  const host = document.createElement("div"); document.body.append(host);
  let dispose = render(() => <InlineText text="![photo](../assets/photo.jpg)" />, host);
  try {
    await vi.waitFor(() => expect(host.querySelector("img")?.getAttribute("src")).toBe("blob:loaded"));
    if (evicted) for (let i = 0; i < 128; i++) seedAssetBlob(`other-${i}.jpg`, new Uint8Array([1]));
    dispose();
    let srcAtMount: string | null | undefined;
    const EditorSibling = () => {
      onMount(() => { srcAtMount = host.querySelector("img")?.getAttribute("src"); });
      return <textarea />;
    };
    dispose = render(() => <><InlineText text="![photo](../assets/photo.jpg)" /><EditorSibling /></>, host);
    // No promise/microtask: editor onMount focuses during this same turn.
    expect(host.querySelector("img")?.getAttribute("src")).toBe("blob:loaded");
    expect(srcAtMount).toBe("blob:loaded");
    expect(host.querySelector(".inline-image-missing")).toBeNull();
    expect(read).toHaveBeenCalledTimes(1);
  } finally { dispose(); }
});

it("an open lightbox keeps graph file copy after refresh revokes its preview", async () => {
  const revoke = vi.fn();
  vi.stubGlobal("URL", { createObjectURL: () => "blob:lightbox-old", revokeObjectURL: revoke });
  const fetch = vi.fn().mockRejectedValue(new Error("revoked blob URL"));
  vi.stubGlobal("fetch", fetch);
  const read = vi.spyOn(backend(), "readAsset").mockResolvedValue(new Uint8Array([1, 2, 3]));
  const copy = vi.spyOn(backend(), "copyAssetFileToClipboard").mockResolvedValue(true);
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <><InlineText text="![photo](../assets/photo.jpg)" blockId="block" /><Lightbox /></>, host);
  try {
    await vi.waitFor(() => expect(host.querySelector("img.inline-image")).not.toBeNull());
    host.querySelector<HTMLImageElement>("img.inline-image")!.click();
    host.querySelector<HTMLButtonElement>(".lightbox-copy")!.click();
    await vi.waitFor(() => expect(copy).toHaveBeenCalledTimes(1));
    read.mockRejectedValue(new Error("asset-too-large"));
    refreshAsset("photo.jpg");
    await vi.waitFor(() => expect(revoke).toHaveBeenCalledWith("blob:lightbox-old"));
    expect(assetNameForBlobUrl("blob:lightbox-old")).toBeUndefined();
    // Preview refusal has its own asset-read error; isolate the copy outcome.
    setToasts([]);
    host.querySelector<HTMLButtonElement>(".lightbox-copy")!.click();
    await vi.waitFor(() => expect(copy).toHaveBeenCalledTimes(2));
    expect(copy).toHaveBeenLastCalledWith("photo.jpg");
    expect(fetch).not.toHaveBeenCalled();
    expect(toasts().some(t => t.kind === "error")).toBe(false);
    // Retaining a file name must never make an old viewer target another graph.
    invalidateBinding();
    host.querySelector<HTMLButtonElement>(".lightbox-copy")!.click();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(copy).toHaveBeenCalledTimes(2);
    expect(fetch).not.toHaveBeenCalled();
  } finally { dispose(); }
});

it.each([null, "asset-too-large", "io:PermissionDenied", "empty"])("copies the original graph file after preview outcome %s", async (failure) => {
  vi.stubGlobal("URL", { createObjectURL: () => "blob:small", revokeObjectURL: vi.fn() });
  const read = vi.spyOn(backend(), "readAsset").mockImplementation(async (_name, maxBytes) => {
    expect(maxBytes).toBe(32 * 1024 * 1024);
    if (failure === "empty") return new Uint8Array();
    if (failure) throw new Error(failure);
    return new Uint8Array([1, 2, 3]);
  });
  const copy = vi.spyOn(backend(), "copyAssetFileToClipboard").mockResolvedValue(true);
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <InlineText text="![photo](../assets/photo.jpg)" blockId="block" />, host);
  try {
    await vi.waitFor(() => expect(read).toHaveBeenCalled());
    await vi.waitFor(() => expect(host.querySelector('button[title="Copy image"]')).not.toBeNull());
    host.querySelector<HTMLButtonElement>('button[title="Copy image"]')!.click();
    await vi.waitFor(() => expect(copy).toHaveBeenCalledWith("photo.jpg"));
    expect(host.querySelector(".inline-image-missing")).toBeNull();
    if (failure) expect(host.textContent).toContain("Preview unavailable");
  } finally { dispose(); }
});

it("keeps a genuinely missing file's placeholder without copy actions", async () => {
  vi.spyOn(backend(), "readAsset").mockRejectedValue(new Error("not-found"));
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <InlineText text="![absent](../assets/absent.jpg)" blockId="block" />, host);
  try {
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(host.querySelector(".inline-image-missing")?.textContent).toContain("absent");
    expect(host.querySelector('button[title="Copy image"]')).toBeNull();
  } finally { dispose(); }
});

it("refreshes the presentation when an external edit changes preview availability", async () => {
  vi.stubGlobal("URL", { createObjectURL: () => "blob:reloaded", revokeObjectURL: vi.fn() });
  const read = vi.spyOn(backend(), "readAsset").mockRejectedValue(new Error("asset-too-large"));
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <InlineText text="![photo](../assets/edited.jpg)" blockId="block" />, host);
  try {
    await vi.waitFor(() => expect(host.querySelector(".inline-image-unavailable")).not.toBeNull());
    read.mockResolvedValue(new Uint8Array([1, 2, 3]));
    refreshAsset("edited.jpg");
    await vi.waitFor(() => expect(host.querySelector("img")?.getAttribute("src")).toBe("blob:reloaded"));
    expect(host.querySelector(".inline-image-unavailable")).toBeNull();
    read.mockRejectedValue(new Error("not-found"));
    refreshAsset("edited.jpg");
    await vi.waitFor(() => expect(host.querySelector(".inline-image-missing")).not.toBeNull());
    expect(host.querySelector('button[title="Copy image"]')).toBeNull();
  } finally { dispose(); }
});
