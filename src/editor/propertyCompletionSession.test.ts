import { afterEach, describe, expect, it, vi } from "vitest";
import { backend } from "../backend";
import { invalidateBinding } from "../binding";
import { propertyCompletionSession } from "./propertyCompletionSession";

afterEach(() => vi.restoreAllMocks());

const FACETS: [string, string[]][] = [["alpha", ["one", "two"]], ["beta", ["three"]]];

function serve() {
  return vi.spyOn(backend(), "queryFacets").mockImplementation(async (_autocomplete, scope) => {
    if (scope && "key" in scope) return FACETS.filter(([key]) => key === scope.key);
    if (scope && "keys" in scope) return FACETS.map(([key]) => [key, []]);
    return FACETS;
  });
}

describe("property completion session (cost-model audit 2026-11-02 row 2b)", () => {
  it("asks for the keys once per session and again only after the session ends", async () => {
    const facets = serve();
    const session = propertyCompletionSession();
    for (let keystroke = 0; keystroke < 5; keystroke++) {
      expect(await session.keys()).toEqual(["alpha", "beta"]);
    }
    expect(facets).toHaveBeenCalledTimes(1);
    session.end();
    await session.keys();
    expect(facets.mock.calls).toEqual([[true, { keys: true }], [true, { keys: true }]]);
  });

  it("offers values without a property-name session, sharing one request per key", async () => {
    const facets = serve();
    const session = propertyCompletionSession();
    const [first, second] = await Promise.all([session.load("alpha"), session.load("alpha")]);
    expect([first, second]).toEqual([true, true]);
    expect(session.values("alpha")).toEqual(["one", "two"]);
    expect(await session.load("alpha")).toBe(false);
    expect(facets.mock.calls).toEqual([[true, { key: "alpha" }]]);
  });

  it("drops an answer that lands after its session ended", async () => {
    serve();
    const session = propertyCompletionSession();
    await session.keys();
    const pending = session.load("beta");
    session.end();
    expect(await pending).toBe(false);
    expect(session.values("beta")).toEqual([]);
  });

  it("a failed request caches nothing: the next keystroke asks again", async () => {
    const facets = vi.spyOn(backend(), "queryFacets").mockRejectedValueOnce(new Error("busy"));
    const session = propertyCompletionSession();
    await expect(session.keys()).rejects.toThrow("busy");
    facets.mockResolvedValueOnce([["alpha", []]]);
    expect(await session.keys()).toEqual(["alpha"]);
    facets.mockRejectedValueOnce(new Error("busy"));
    expect(await session.load("alpha")).toBe(false);
    expect(session.values("alpha")).toEqual([]);
    facets.mockResolvedValueOnce([["alpha", ["one"]]]);
    expect(await session.load("alpha")).toBe(true);
    expect(session.values("alpha")).toEqual(["one"]);
  });

  it("never offers values read under another graph binding", async () => {
    const facets = serve();
    const session = propertyCompletionSession();
    await session.keys();
    expect(await session.load("alpha")).toBe(true);
    expect(session.values("alpha")).toEqual(["one", "two"]);
    // Graph switch while the popup is open: the held list is not offered,
    // and the next load reads the new binding's values.
    invalidateBinding();
    expect(session.values("alpha")).toEqual([]);
    expect(await session.load("alpha")).toBe(true);
    expect(session.values("alpha")).toEqual(["one", "two"]);
    expect(facets.mock.calls.filter(([, scope]) => scope && "key" in scope)).toHaveLength(2);
  });
});
