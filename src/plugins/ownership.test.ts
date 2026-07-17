import { afterEach, describe, expect, it } from "vitest";
import { bumpGraphEpoch, graphEpoch, setGraphMeta, setGraphTransitioning } from "../ui";
import type { GraphMeta } from "../types";
import { bindPluginBlockSnapshot, capturePluginGraphOwner, isPluginGraphOwnerCurrent } from "./ownership";

function meta(root: string): GraphMeta {
  return {
    root,
    journals_dir: "journals",
    pages_dir: "pages",
    preferred_workflow: "now",
    shortcuts: {},
    start_of_week: 6,
    block_hidden_properties: [],
    default_journal_template: null,
    favorites: [],
    journal_page_title_format: "MMM do, yyyy",
    journal_file_name_format: "yyyy_MM_dd",
    preferred_format: "md",
    macros: {},
    enable_timetracking: true,
    logbook_with_second_support: true,
    logbook_enabled_in_timestamped_blocks: false,
    logbook_enabled_in_all_blocks: false,
    guide_announced: true,
  };
}

afterEach(() => {
  setGraphTransitioning(false);
  setGraphMeta(null);
});

describe("plugin graph ownership", () => {
  it("captures a frozen host-only owner and rejects root, epoch, and transition changes", () => {
    setGraphMeta(meta("/graph-a"));
    const owner = capturePluginGraphOwner();
    expect(owner).toEqual({ graphRoot: "/graph-a", generation: graphEpoch() });
    expect(Object.isFrozen(owner)).toBe(true);
    expect(isPluginGraphOwnerCurrent(owner!)).toBe(true);

    setGraphTransitioning(true);
    expect(isPluginGraphOwnerCurrent(owner!)).toBe(false);
    setGraphTransitioning(false);
    bumpGraphEpoch();
    expect(isPluginGraphOwnerCurrent(owner!)).toBe(false);
    setGraphMeta(meta("/graph-b"));
    expect(isPluginGraphOwnerCurrent(owner!)).toBe(false);
  });

  it("binds a copied block snapshot only while a graph is live", () => {
    setGraphMeta(meta("/graph-a"));
    const owned = bindPluginBlockSnapshot({ id: "shared-id", raw: "same raw", parentId: null, depth: 0, format: "md" });
    expect(owned?.block).toMatchObject({ id: "shared-id", raw: "same raw" });
    expect(Object.isFrozen(owned?.block)).toBe(true);
    setGraphTransitioning(true);
    expect(bindPluginBlockSnapshot({ id: "shared-id", raw: "same raw", parentId: null, depth: 0 })).toBeNull();
  });
});
