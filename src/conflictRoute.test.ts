import { expect, it } from "vitest";
import { parsePersistedSession } from "./session";
import { routeTitle, sameRoute } from "./router";

// og 8c: the Conflicts overview is an ordinary tab route: it is titled,
// compared like Journals (one per kind), and survives a session restore.

it("titles and compares the Conflicts route like Journals", () => {
  expect(routeTitle({ kind: "conflicts" })).toBe("Conflicts");
  expect(sameRoute({ kind: "conflicts" }, { kind: "conflicts" })).toBe(true);
  expect(sameRoute({ kind: "conflicts" }, { kind: "journals" })).toBe(false);
});

it("restores a saved Conflicts tab", () => {
  const raw = JSON.stringify({ tabs: [{ history: [{ kind: "journals" }, { kind: "conflicts" }], pos: 1, pinned: false }], activeIndex: 0 });
  expect(parsePersistedSession(raw)?.snapshots.get("main")?.tabs[0].history).toEqual([{ kind: "journals" }, { kind: "conflicts" }]);
});
