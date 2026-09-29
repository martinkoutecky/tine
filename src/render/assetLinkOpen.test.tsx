import { describe, it, expect, beforeAll, afterEach, vi } from "vitest";
import { render } from "solid-js/web";
import { InlineText } from "./inline";
import { initParser } from "./parse";
import { backend } from "../backend";

// GH #367: a labeled link into `assets/` (`[image](./assets/quick-capture.png)`)
// is not an external URL — clicking it must reach the OS opener for that asset
// (file: system viewer; directory/empty path: file manager), while a genuine
// http(s) link must keep routing to the URL opener. og's opener also carries
// the graph binding generation so a click cannot reach a since-switched graph.

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  vi.restoreAllMocks();
});

function mountLink(raw: string, format: "md" | "org" = "md"): { host: HTMLElement; dispose: () => void } {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const dispose = render(() => <InlineText text={raw} format={format} />, host);
  return { host, dispose: () => { dispose(); host.remove(); } };
}

function click(host: HTMLElement): void {
  const a = host.querySelector("a.external-link");
  expect(a, "expected a rendered fallback link").not.toBeNull();
  a!.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
}

describe("asset link opens through the OS opener", () => {
  it("a labeled link to an asset file opens the asset, not an external URL", () => {
    const openAsset = vi.spyOn(backend(), "openAsset").mockResolvedValue(undefined);
    const openExternal = vi.spyOn(backend(), "openExternal").mockResolvedValue(undefined);
    const { host, dispose } = mountLink("[image](./assets/quick-capture.png)");
    try {
      click(host);
      expect(openAsset).toHaveBeenCalledWith("quick-capture.png", backend().graphBindingGeneration());
      expect(openExternal).not.toHaveBeenCalled();
    } finally {
      dispose();
    }
  });

  it("the empty asset path `[path](./assets/)` opens the assets directory", () => {
    const openAsset = vi.spyOn(backend(), "openAsset").mockResolvedValue(undefined);
    const { host, dispose } = mountLink("[path](./assets/)");
    try {
      click(host);
      expect(openAsset).toHaveBeenCalledWith("", backend().graphBindingGeneration());
    } finally {
      dispose();
    }
  });

  it("the bare `./assets` root (no trailing slash) also opens the directory", () => {
    const openAsset = vi.spyOn(backend(), "openAsset").mockResolvedValue(undefined);
    const { host, dispose } = mountLink("[path](./assets)");
    try {
      click(host);
      expect(openAsset).toHaveBeenCalledWith("", backend().graphBindingGeneration());
    } finally {
      dispose();
    }
  });

  it("a directory link resolves the nested directory, decoded", () => {
    const openAsset = vi.spyOn(backend(), "openAsset").mockResolvedValue(undefined);
    const { host, dispose } = mountLink("[folder](./assets/some%20dir)");
    try {
      click(host);
      expect(openAsset).toHaveBeenCalledWith("some dir", backend().graphBindingGeneration());
    } finally {
      dispose();
    }
  });

  it("a directory link with a trailing slash opens that directory", () => {
    const openAsset = vi.spyOn(backend(), "openAsset").mockResolvedValue(undefined);
    const { host, dispose } = mountLink("[folder](./assets/some%20dir/)");
    try {
      click(host);
      expect(openAsset).toHaveBeenCalledWith("some dir", backend().graphBindingGeneration());
    } finally {
      dispose();
    }
  });

  it("nested, spaced and Unicode paths are decoded before opening", () => {
    const openAsset = vi.spyOn(backend(), "openAsset").mockResolvedValue(undefined);
    const { host, dispose } = mountLink("[notes](./assets/some%20dir/%E6%8A%A5%E8%A1%A8.md)");
    try {
      click(host);
      expect(openAsset).toHaveBeenCalledWith("some dir/报表.md", backend().graphBindingGeneration());
    } finally {
      dispose();
    }
  });

  it("an Org link into assets follows the same opener route", () => {
    const openAsset = vi.spyOn(backend(), "openAsset").mockResolvedValue(undefined);
    const { host, dispose } = mountLink("[[../assets/quick-capture.png][image]]", "org");
    try {
      click(host);
      expect(openAsset).toHaveBeenCalledWith("quick-capture.png", backend().graphBindingGeneration());
    } finally {
      dispose();
    }
  });

  it("an https URL that merely contains assets/ stays an external URL", () => {
    const openAsset = vi.spyOn(backend(), "openAsset").mockResolvedValue(undefined);
    const openExternal = vi.spyOn(backend(), "openExternal").mockResolvedValue(undefined);
    const { host, dispose } = mountLink("[doc](https://example.com/assets/a.docx)");
    try {
      click(host);
      expect(openExternal).toHaveBeenCalledWith("https://example.com/assets/a.docx");
      expect(openAsset).not.toHaveBeenCalled();
    } finally {
      dispose();
    }
  });

  it("a non-asset relative link is untouched by the asset route", () => {
    const openAsset = vi.spyOn(backend(), "openAsset").mockResolvedValue(undefined);
    const { host, dispose } = mountLink("[x](../journals/2026_08_23.md)");
    try {
      click(host);
      expect(openAsset).not.toHaveBeenCalled();
    } finally {
      dispose();
    }
  });
});
