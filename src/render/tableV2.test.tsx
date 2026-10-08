import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import { TableV2, tableColumnCount } from "./tableV2";
import { AstBody } from "./body";
import { clearSeededFacets } from "./facets";
import { initParser } from "./parse";
import { Block } from "../components/Block";
import { resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import type { PageDto } from "../types";
import { setGraphMeta } from "../graphSession";

const TABLE = "| Fruit | Count |\n| --- | ---: |\n| apple | 2 |";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  resetStore();
  setGraphMeta(null);
  document.body.innerHTML = "";
});

function page(raw: string): PageDto {
  return {
    name: "Table v2", kind: "page", title: "Table v2", pre_block: null,
    blocks: [{ id: "table-v2", raw, collapsed: false, children: [] }],
  };
}

function tableWith(...properties: string[]): string {
  // Block properties precede the table in the parsed body, as they do in the
  // replay receipt. Keeping them contiguous exercises the parser's property AST.
  return [...properties, TABLE].join("\n");
}

async function mountBlock(raw: string): Promise<{ root: HTMLElement; dispose: () => void }> {
  loadSingle(page(raw));
  // Hand-built test DTOs do not carry the backend's facets projection. Exercise
  // the same raw-property derivation that the live renderer uses for this block.
  clearSeededFacets();
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <Block id="table-v2" />, root);
  await Promise.resolve();
  return { root, dispose };
}

async function mountAstBody(raw: string): Promise<{ root: HTMLElement; dispose: () => void }> {
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <AstBody raw={raw} />, root);
  await Promise.resolve();
  return { root, dispose };
}

describe("property-configured table v2", () => {
  it("selects the v2 container for a block property instead of md-table", async () => {
    const mounted = await mountBlock(tableWith("logseq.table.version:: 2"));
    try {
      expect(mounted.root.querySelector('[data-testid="v2-table-container"]')).not.toBeNull();
      expect(mounted.root.querySelector(".md-table")).toBeNull();
    } finally {
      mounted.dispose();
    }
  });

  it("applies compact spacing and normalized header modes", async () => {
    const mounted = await mountBlock(tableWith(
      "logseq.table.version:: 2",
      "logseq.table.compact:: true",
      "logseq.table.headers:: UPPERCASE",
    ));
    try {
      const header = mounted.root.querySelector<HTMLElement>(".v2-table-header");
      const cell = mounted.root.querySelector<HTMLElement>(".v2-table-cell");
      expect(header).not.toBeNull();
      expect(header?.classList.contains("v2-table-headers-uppercase")).toBe(true);
      expect(header?.style.textTransform).toBe("uppercase");
      expect(cell?.classList.contains("v2-table-compact")).toBe(true);
    } finally {
      mounted.dispose();
    }
  });

  it("hides logseq.table.* properties from the rendered property chrome", async () => {
    const mounted = await mountBlock(tableWith(
      "logseq.table.version:: 2",
      "logseq.table.compact:: true",
      "visible:: shown",
    ));
    try {
      const propertyChrome = mounted.root.querySelector(".block-properties");
      expect(propertyChrome).not.toBeNull();
      expect(propertyChrome?.textContent).toContain("visible");
      expect(propertyChrome?.textContent).not.toContain("logseq.table.version");
      expect(propertyChrome?.textContent).not.toContain("logseq.table.compact");
    } finally {
      mounted.dispose();
    }
  });

  it.each(["", "logseq.table.version:: 1"])("keeps the existing md-table for version 1/default (%s)", async (property) => {
    const mounted = await mountBlock(property ? tableWith(property) : TABLE);
    try {
      expect(mounted.root.querySelector(".md-table")).not.toBeNull();
      expect(mounted.root.querySelector('[data-testid="v2-table-container"]')).toBeNull();
    } finally {
      mounted.dispose();
    }
  });

  it("AstBody, also used by SheetGrid cells, inherits the v2 presentation", async () => {
    const mounted = await mountAstBody(tableWith("logseq.table.version:: 2"));
    try {
      expect(mounted.root.querySelector('[data-testid="v2-table-container"]')).not.toBeNull();
      expect(mounted.root.querySelector(".md-table")).toBeNull();
    } finally {
      mounted.dispose();
    }
  });
});


it("renders a broad table without an argument spread (OG-B-FRONT)", () => {
  const root = document.createElement("div");
  const dispose = render(() => <TableV2
    table={{kind: "table", header: [[{k: "plain", text: "Header"}]], aligns: [], rows: Array.from({length: 150000}, () => [])}}
    options={{version: 2, compact: false, headers: "none"}}
    renderCell={cell => cell[0].k === "plain" ? cell[0].text : ""}
  />, root);
  try {
    expect(root.querySelector('[role="columnheader"]')?.textContent).toBe("Header");
    expect(root.querySelector<HTMLElement>('[role="grid"]')?.style.gridTemplateColumns).toContain("repeat(1,");
  } finally { dispose(); }
});


it.each([2000, 150000])("answers width for %i valid populated rows without an argument spread", count => {
  const table = {kind: "table" as const, header: null, aligns: [], rows: Array.from({length: count}, () => [[{k: "plain" as const, text: "cell"}]])};
  expect(tableColumnCount(table)).toBe(1);
  table.rows[count - 1].push([{k: "plain", text: "second column"}]);
  expect(tableColumnCount(table)).toBe(2);
});
