import { describe, it, expect, beforeAll, afterEach, vi } from "vitest";
import { render } from "solid-js/web";
import { InlineText } from "./inline";
import { initParser } from "./parse";
import { backend } from "../backend";
import { setToasts, toasts } from "../toasts";

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

// GH #444: `file:` links written by Logseq (`file://D:\test.txt`) and Obsidian
// (`<file:///D:\test.txt>`) rendered as links but did nothing at all when
// clicked — the backend refused every scheme but http/https/mailto, and the
// frontend discarded the rejection. Both halves are covered here: the link must
// reach `openExternal` with the destination as written, and a refusal must
// become a visible message instead of silence.
describe("local file: links (GH #444)", () => {
  const expectOpensExternally = (raw: string) => {
    const openExternal = vi.spyOn(backend(), "openExternal").mockResolvedValue(undefined);
    const openAsset = vi.spyOn(backend(), "openAsset").mockResolvedValue(undefined);
    const { host, dispose } = mountLink(raw);
    try {
      click(host);
      expect(openAsset).not.toHaveBeenCalled();
      expect(openExternal).toHaveBeenCalledTimes(1);
      return String(openExternal.mock.calls[0][0]);
    } finally {
      dispose();
    }
  };

  it("the Logseq shape the reporter used reaches the opener, backslashes intact", () => {
    expect(expectOpensExternally("[Test](file://D:\\test.txt)")).toBe("file://D:\\test.txt");
  });

  it("the Obsidian angle-bracket shape reaches the opener too", () => {
    expect(expectOpensExternally("[Test](<file:///D:\\test.txt>)")).toBe("file:///D:\\test.txt");
  });

  it("a POSIX path and a directory are the same route", () => {
    expect(expectOpensExternally("[notes](file:///home/user/notes.txt)")).toBe("file:///home/user/notes.txt");
    expect(expectOpensExternally("[folder](file:///home/user/notes/)")).toBe("file:///home/user/notes/");
  });

  it("a percent-escaped filename is handed over untouched, for the backend to decode", () => {
    expect(expectOpensExternally("[spaced](file:///home/user/a%20b.txt)")).toBe("file:///home/user/a%20b.txt");
  });

  it("a file: link is never mistaken for a graph asset", () => {
    expect(expectOpensExternally("[x](file:///graph/assets/a.png)")).toBe("file:///graph/assets/a.png");
  });

  it("says so when the link cannot be opened, instead of doing nothing visible", async () => {
    setToasts([]);
    vi.spyOn(backend(), "openExternal").mockRejectedValue("that file link does not name a local file");
    const { host, dispose } = mountLink("[broken](file://not-a-path)");
    try {
      click(host);
      await vi.waitFor(() => expect(toasts()).toHaveLength(1));
      expect(toasts()[0].kind).toBe("error");
      expect(toasts()[0].message).toContain("file://not-a-path");
    } finally {
      setToasts([]);
      dispose();
    }
  });
});
