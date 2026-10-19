import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

const guide = (page: string) => readFileSync(`crates/tine-core/src/templates/${page}.md`, "utf8");

it("the Guide says where portable Windows Tine keeps its state and how to bring settings along (GH #719, D-20261018-P719)", () => {
  const platforms = guide("platforms-and-mobile");
  for (const detail of [
    "**Running the portable ZIP (Windows)?**",
    "in the `tine-data` folder beside `Tine.exe`, never in AppData",
    "A first portable launch starts with fresh settings",
  ]) expect(platforms).toContain(detail);
});
