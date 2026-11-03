// Family 22 / master c7fabcd81 (DUP-2): favorites membership answers "is this
// page a favorite?" with ONE key — kind-scoped, alias-resolved through
// pageIndex.navigationName, folded by pageIdentityKey. Uses only exports that
// predate the port so it runs against the base as the fail-before proof.
import { beforeEach, describe, expect, it } from "vitest";
import { adoptRenamedFavorites } from "./favorites";
import { favorites, isFavorite, removeDeletedPageFromNavigation, renamePageInNavigation, setFavorites, toggleFavorite } from "./ui";

beforeEach(() => setFavorites([]));

describe("favorite membership identity (DUP-2)", () => {
  it("a page starred under one spelling reads as favorited under another case", () => {
    setFavorites([{ name: "Foo", kind: "page" }]);
    expect(isFavorite("foo", "page")).toBe(true);
  });

  it("NFC and NFD spellings are one favorite", () => {
    setFavorites([{ name: "Café", kind: "page" }]);
    expect(isFavorite("Café", "page")).toBe(true);
  });

  it("toggling under another spelling unstars instead of appending a duplicate", () => {
    setFavorites([{ name: "Foo", kind: "page" }]);
    toggleFavorite("foo", "page");
    expect(favorites()).toEqual([]);
  });

  it("a page and a journal with one title are distinct: only the starred kind reads as favorite", () => {
    setFavorites([{ name: "Aug 25th, 2026", kind: "journal" }]);
    expect(isFavorite("Aug 25th, 2026", "journal")).toBe(true);
    expect(isFavorite("Aug 25th, 2026", "page")).toBe(false);
  });

  it("deleting a page does not drop a journal favorite that shares the name", () => {
    setFavorites([{ name: "Aug 25th, 2026", kind: "journal" }]);
    removeDeletedPageFromNavigation("Aug 25th, 2026", "page");
    expect(favorites()).toEqual([{ name: "Aug 25th, 2026", kind: "journal" }]);
  });

  it("deleting a page removes its favorite under another spelling", () => {
    setFavorites([{ name: "foo", kind: "page" }]);
    removeDeletedPageFromNavigation("Foo", "page");
    expect(favorites()).toEqual([]);
  });

  // CM step 2 batch 4 (I-12): config.edn's favorites after a rename have one
  // answerer, the backend's config step, which matches another spelling of the
  // page (scenario f2, cm-favorites/scenarios.json). The window adopts it.
  it("rename leaves favorites to the backend and adopts its re-keyed favorite", () => {
    setFavorites([{ name: "foo", kind: "page" }]);
    renamePageInNavigation("Foo", "Bar");
    expect(favorites()).toEqual([{ name: "foo", kind: "page" }]);
    adoptRenamedFavorites({ names: ["Bar"], page: null });
    expect(favorites()).toEqual([{ name: "Bar", kind: "page" }]);
    expect(isFavorite("bar", "page")).toBe(true);
  });
});
