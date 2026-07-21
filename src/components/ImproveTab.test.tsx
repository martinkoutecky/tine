import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { clearClipboardSlot, copyBlockOutline, peekClipboardSlot } from "../clipboard";
import { ImproveTab } from "./ImproveTab";

async function flush() {
  for (let i = 0; i < 12; i += 1) await Promise.resolve();
}

describe("Help improve Tine privacy boundary", () => {
  afterEach(() => {
    Reflect.deleteProperty(globalThis, "__tineDiffFixture");
    clearClipboardSlot();
    vi.restoreAllMocks();
    document.body.replaceChildren();
  });

  it("offers only scrubbed reproductions for copy and explicitly omits unsafe findings", async () => {
    Object.assign(globalThis, {
      __tineDiffFixture: {
        tineVersion: "0.5.9",
        lsdocVersion: "v0.5.3",
        stats: { files: 2, totalBytes: 42 },
        lsdocAvailable: true,
        findings: [
          {
            type: "divergence",
            rel: "graph-file-0001.md",
            lineStart: 1,
            lineEnd: 1,
            contextDependent: false,
            anonymized: {
              ok: true,
              tier: "tier 1",
              input: "- Aaaaaa 99",
              lsdocKey: "left",
              mldocKey: "right",
            },
          },
          {
            type: "divergence",
            rel: "graph-file-0002.md",
            lineStart: 2,
            lineEnd: 2,
            contextDependent: true,
            anonymized: { ok: false },
          },
        ],
      },
    });
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText },
    });

    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <ImproveTab />, host);
    try {
      expect(host.textContent).not.toContain("safe to share");
      expect(host.textContent).toContain("omits a finding");
      (host.querySelector(".improve-run button") as HTMLButtonElement).click();
      await vi.waitFor(() => {
        expect(host.querySelectorAll(".improve-findings button")).toHaveLength(2);
      });

      const copyButtons = [...host.querySelectorAll<HTMLButtonElement>(".improve-findings button")];
      expect(copyButtons.map((button) => button.textContent)).toEqual(["Copy all", "Copy"]);
      expect(host.textContent).toContain("Found, but couldn't be anonymized — not shown.");
      copyButtons[1].click();
      await flush();
      expect(writeText).toHaveBeenLastCalledWith(expect.stringContaining("- Aaaaaa 99"));
      expect(writeText).toHaveBeenLastCalledWith(expect.not.stringContaining("graph-file-0002"));

      copyButtons[0].click();
      await flush();
      expect(writeText).toHaveBeenLastCalledWith(expect.stringContaining("Divergence found but not auto-anonymizable — omitted."));
      expect(writeText).toHaveBeenLastCalledWith(expect.not.stringContaining("safe to share"));
    } finally {
      dispose();
    }
  });

  it("shows copy failure while still clearing a stale private block payload", async () => {
    Object.assign(globalThis, {
      __tineDiffFixture: {
        tineVersion: "0.5.9",
        lsdocVersion: "v0.5.3",
        stats: { files: 1, totalBytes: 10 },
        lsdocAvailable: true,
        findings: [{
          type: "divergence",
          rel: "graph-file-0001.md",
          lineStart: 1,
          lineEnd: 1,
          contextDependent: false,
          anonymized: { ok: true, tier: "tier 1", input: "- Aaaaaa", lsdocKey: "left", mldocKey: "right" },
        }],
      },
    });
    vi.spyOn(backend(), "writeRich").mockResolvedValue();
    await copyBlockOutline("cut", "- stale", {
      blocks: [{ raw: "stale", sourceFormat: "md", children: [] }],
      sourcePages: [],
    });
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: vi.fn().mockRejectedValue(new Error("clipboard denied")) },
    });

    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <ImproveTab />, host);
    try {
      (host.querySelector(".improve-run button") as HTMLButtonElement).click();
      await vi.waitFor(() => expect(host.querySelectorAll(".improve-findings button")).toHaveLength(2));
      const copy = host.querySelectorAll<HTMLButtonElement>(".improve-findings button")[1];
      copy.click();
      await vi.waitFor(() => expect(copy.textContent).toBe("Copy failed"));
      expect(peekClipboardSlot()).toBeNull();
    } finally {
      dispose();
    }
  });
});
