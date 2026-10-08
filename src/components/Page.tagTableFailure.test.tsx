import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { setDoc } from "../document/model";

// GH #490 / #332 family (master c5279d186): a rejected resource read throws into
// render and blanks the region. Page.tsx's tag-table resources now read through
// readOr and show a failure row (an empty table would claim "no tagged blocks").
vi.mock("../owned", async (original) => ({
  ...(await original<typeof import("../owned")>()),
  latestOwner: () => { throw new Error("owner lookup failed"); },
}));

import { TagPageTable } from "./Page";

beforeAll(initParser);
afterEach(() => {
  resetStore();
  document.body.innerHTML = "";
});

function load() {
  setDoc({
    loaded: true, feed: [],
    pages: [{ name: "Tag", kind: "page", title: "Tag", preBlock: null, roots: [], format: "md", readOnly: false, guide: false }],
    byId: {},
  });
}

describe("tag table resources never throw into render", () => {
  it("TagPageTable shows the failure row with a Retry instead of blanking", async () => {
    load();
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <TagPageTable pageName="Tag" />, root);
    await vi.waitFor(() => expect(root.querySelector(".resource-failure")).not.toBeNull());
    expect(root.textContent).toContain("Couldn’t load the tag table.");
    expect(root.querySelector(".resource-failure-retry")).not.toBeNull();
    dispose();
  });
});
