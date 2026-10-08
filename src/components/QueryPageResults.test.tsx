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
