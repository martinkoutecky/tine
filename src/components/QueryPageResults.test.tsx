import { afterEach, expect, it } from "vitest";
import { render } from "solid-js/web";
import { QueryPageResults, type QueryPageHit } from "./QueryPageResults";

afterEach(() => { document.body.innerHTML = ""; });

it("groups adjacent page board rows without changing authored search order", () => {
  const hits = (["A", "B", "A"] as const).map((owner, index): QueryPageHit => ({
    entity: "page",
    page: { name: `Alpha ${index}`, kind: "page", date_key: null, path: `pages/Alpha ${index}.md` },
    row: { name: `Alpha ${index}`, kind: "page",
      path: `pages/Alpha ${index}.md`, properties: [["owner", owner]] },
    display_text: `Alpha ${index}`, evidence: [], score: 0,
  }));
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <QueryPageResults hits={hits} presentation="board"
    view={{ group_by: "prop:owner" }} surfaceId={(hit) => hit.page.name} onOpen={() => {}} />, root);
  try {
    expect([...root.querySelectorAll(".query-board-column h4")].map((node) => node.textContent?.[0])).toEqual(["A", "B", "A"]);
    expect([...root.querySelectorAll(".query-board-card")].map((node) => node.textContent)).toEqual([
      expect.stringContaining("Alpha 0"), expect.stringContaining("Alpha 1"), expect.stringContaining("Alpha 2"),
    ]);
  } finally { dispose(); }
});

// GH #619 item 8 (D-20261018-Q619): the Search tab's page rows for a query-engine answer show the
// page's properties as text under the title, like the block's own Pages rows. A friendly search row
// is hydrated for display columns only, so its caller leaves the strip off.
it.each(["list", "search"] as const)("%s page rows show a query answer's properties only when asked", (presentation) => {
  const hit: QueryPageHit = {
    entity: "page",
    page: { name: "Alpha", kind: "page", date_key: null, path: "pages/Alpha.md" },
    row: { name: "Alpha", kind: "page", path: "pages/Alpha.md", properties: [["status", "open"], ["owner", "Ada"]] },
    display_text: "Alpha", evidence: [], score: 0,
  };
  for (const showProperties of [true, false]) {
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <QueryPageResults hits={[hit]} presentation={presentation} showProperties={showProperties}
      surfaceId={(h) => h.page.name} onOpen={() => {}} />, root);
    try {
      const strip = root.querySelector(".query-page-props");
      if (!showProperties) { expect(strip).toBeNull(); continue; }
      expect(strip?.textContent).toContain("status: open");
      expect(strip?.textContent).toContain("owner: Ada");
      expect(root.querySelector("button")?.contains(strip!)).toBe(false);
    } finally { dispose(); }
  }
});
