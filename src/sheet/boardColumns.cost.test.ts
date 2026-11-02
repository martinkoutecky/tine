import { afterEach, expect, it, vi } from "vitest";
import { buildBoardColumns } from "./boardColumns";

afterEach(() => vi.restoreAllMocks());

// L7s8-04: column order is linear in the distinct group values, not quadratic.
it("orders thousands of distinct groups without rescanning the order per row", () => {
  const rows = Array.from({ length: 4000 }, (_, i) => ({ id: `r${i}` })) as never[];
  const scan = Array.prototype.includes;
  let scanned = 0;
  vi.spyOn(Array.prototype, "includes").mockImplementation(function (this: unknown[], ...args: [unknown, number?]) {
    scanned += this.length;
    return scan.apply(this, args);
  });
  const columns = buildBoardColumns(rows, "custom" as never, [], { workflow: "todo", rowKeys: (row: { id: string }) => [row.id] });
  vi.restoreAllMocks();
  expect(columns.map((column) => column.key).slice(0, 2)).toEqual(["r0", "r1"]);
  expect(columns).toHaveLength(4000);
  expect(scanned).toBeLessThan(100_000);
});
