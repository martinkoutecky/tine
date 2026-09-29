import { afterEach, expect, it } from "vitest";
import { captureBinding } from "./binding";
import { applyGraphConfigChange } from "./graph";
import { graphMeta, setGraphMeta } from "./graphSession";
import { favorites, seedFavorites } from "./favorites";
import { workflow } from "./ui";
import type { GraphMeta } from "./types";

const meta = {
  root: "/graph", preferred_workflow: "now", favorites: ["A"], favorites_page: null,
  journal_page_title_format: "MMM do, yyyy", default_home: "Home",
} as GraphMeta;

afterEach(() => { setGraphMeta(null); seedFavorites([]); });

it("applies an outside config edit in place and ignores an old binding", () => {
  setGraphMeta(meta);
  seedFavorites(["A"]);
  const binding = captureBinding();
  applyGraphConfigChange({ binding_generation: binding.backendGeneration + 1, meta: { ...meta, favorites: ["B"] } });
  expect(favorites().map((item) => item.name)).toEqual(["A"]);
  applyGraphConfigChange({ binding_generation: binding.backendGeneration, meta: { ...meta, favorites: ["B"], preferred_workflow: "todo" } });
  expect(graphMeta()?.favorites).toEqual(["B"]);
  expect(favorites().map((item) => item.name)).toEqual(["B"]);
  expect(workflow()).toBe("todo");
});
