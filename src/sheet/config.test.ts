import { describe, expect, it } from "vitest";
import { sheetConfig } from "./config";

describe("sheetConfig", () => {
  it("reads grid view, header, and positional column widths", () => {
    const cfg = sheetConfig([
      ["tine.view", "grid"],
      ["tine.header", "true"],
      ["tine.col-widths", "0=120;2=200"],
    ]);

    expect(cfg.view).toBe("grid");
    expect(cfg.header).toBe(true);
    expect(Array.from(cfg.colWidths.entries())).toEqual([
      [0, 120],
      [2, 200],
    ]);
  });

  it("matches keys and boolean values case-insensitively", () => {
    const cfg = sheetConfig([
      ["Tine.View", "BOARD"],
      ["TINE.HEADER", "TRUE"],
    ]);

    expect(cfg.view).toBe("board");
    expect(cfg.header).toBe(true);
  });

  it("degrades unknown view values to plain outline", () => {
    expect(sheetConfig([["tine.view", "calendar"]]).view).toBe(null);
    expect(sheetConfig([["tine.view", "list"]]).view).toBe(null);
  });

  it("skips malformed width entries without throwing", () => {
    const cfg = sheetConfig([
      ["tine.col-widths", "0=120; nope; 2 = 88 ; -1=10; 3=wide; 4=0"],
    ]);

    expect(Array.from(cfg.colWidths.entries())).toEqual([
      [0, 120],
      [2, 88],
      [4, 0],
    ]);
  });

  it("ignores non-tine properties", () => {
    const cfg = sheetConfig([
      ["title", "Project"],
      ["query-table", "true"],
    ]);

    expect(cfg.view).toBe(null);
    expect(cfg.header).toBe(false);
    expect(cfg.colWidths.size).toBe(0);
  });
});
