import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc, pageByName } from "../document/model";
import type { BlockDto } from "../types";
import { Block } from "./Block";

beforeAll(() => initParser());
afterEach(() => {
  resetStore();
  document.body.innerHTML = "";
});

function mount(raw: string) {
  const block: BlockDto = {
    id: "marker-click-host",
    raw,
    collapsed: false,
    children: [],
    marker: raw.split(" ", 1)[0],
  };
  loadSingle({
    name: "Marker click",
    kind: "page",
    title: "Marker click",
    pre_block: null,
    blocks: [block],
  });
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(() => (
    <For each={pageByName("Marker click")?.roots ?? []}>{(id) => <Block id={id} />}</For>
  ), root);
  return { root, dispose };
}

describe("task marker label click (GH #259)", () => {
  it("toggles TODO and DOING without entering the keyboard DONE/removal cycle", () => {
    const { root, dispose } = mount("TODO buy milk");
    try {
      let marker = root.querySelector(".block-marker") as HTMLElement;
      expect(marker.classList.contains("marker-clickable")).toBe(true);
      marker.click();
      expect(doc.byId["marker-click-host"].raw.split("\n")[0]).toBe("DOING buy milk");

      marker = root.querySelector(".block-marker") as HTMLElement;
      marker.click();
      expect(doc.byId["marker-click-host"].raw.split("\n")[0]).toBe("TODO buy milk");
    } finally {
      dispose();
    }
  });

  it("leaves DONE intact and does not present it as a clickable marker", () => {
    const { root, dispose } = mount("DONE buy milk");
    try {
      const marker = root.querySelector(".block-marker") as HTMLElement;
      expect(marker.classList.contains("marker-clickable")).toBe(false);
      marker.click();
      expect(doc.byId["marker-click-host"].raw).toBe("DONE buy milk");
    } finally {
      dispose();
    }
  });
});
