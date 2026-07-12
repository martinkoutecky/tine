import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { loadSingle, resetStore } from "../store";
import type { PageDto } from "../types";
import { Block } from "./Block";

const TARGET = "64b9c0e2-0000-0000-0000-000000000000";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  resetStore();
  document.body.innerHTML = "";
});

describe("block embed hierarchy", () => {
  it("uses only the embedded root's interactive bullet", async () => {
    const page: PageDto = {
      name: "Embed host",
      kind: "page",
      title: "Embed host",
      pre_block: null,
      blocks: [{ id: "embed-host", raw: `{{embed ((${TARGET}))}}`, collapsed: false, children: [] }],
    };
    loadSingle(page);

    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <Block id="embed-host" />, root);

    try {
      await vi.waitFor(() => {
        expect(root.textContent).toContain("Related Work");
      });
      const host = root.querySelector<HTMLElement>('[data-block-id="embed-host"]');
      expect(host).not.toBeNull();
      expect(host?.querySelectorAll(".bullet-container")).toHaveLength(1);
      expect(host?.classList.contains("block-embed-host")).toBe(true);
      expect(host?.querySelector(".embed-block .bullet-container")).not.toBeNull();
    } finally {
      dispose();
    }
  });
});
