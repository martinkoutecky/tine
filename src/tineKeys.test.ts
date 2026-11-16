import { describe, expect, it } from "vitest";
import {
  EXPORTS_MARKER, FAVORITES_MARKER, JOURNAL_SECTIONS_MARKER,
  canonicalPropertyKey, hasMarker, keySpellings, markerChangeKeys, markerQuery,
} from "./tineKeys";

// D-20261102-PNS (Discussion #825): Tine writes `tine.<name>`; the page markers
// stable wrote as `tine/<name>` are still read, through the one alias table in
// tine_outline::tine_keys (served to the window by the wasm parser).
describe("tine.* keys and the stable marker spellings", () => {
  it("reads a stable page's slash marker as the dot key", () => {
    expect(canonicalPropertyKey("tine/favorites")).toBe(FAVORITES_MARKER);
    expect(canonicalPropertyKey(" Tine/Journal_Sections ")).toBe(JOURNAL_SECTIONS_MARKER);
    expect(canonicalPropertyKey("tine/exports")).toBe(EXPORTS_MARKER);
    expect(hasMarker([["tine/favorites", "true"]], FAVORITES_MARKER)).toBe(true);
    expect(hasMarker([["tine/journal-sections", " TRUE "]], JOURNAL_SECTIONS_MARKER)).toBe(true);
    expect(hasMarker([["tine.exports", "true"]], EXPORTS_MARKER)).toBe(true);
    expect(hasMarker([["tine/favorites", "false"]], FAVORITES_MARKER)).toBe(false);
    expect(hasMarker([["favorites", "true"]], FAVORITES_MARKER)).toBe(false);
  });

  it("locates and invalidates marker pages under either spelling", () => {
    expect(keySpellings(JOURNAL_SECTIONS_MARKER)).toEqual(["tine.journal-sections", "tine/journal-sections"]);
    expect(markerQuery(FAVORITES_MARKER)).toBe("(or (page-property tine.favorites true) (page-property tine/favorites true))");
    expect(markerChangeKeys(EXPORTS_MARKER)).toEqual(["prop:tine.exports", "prop:tine/exports"]);
  });

  it("keeps no reader for the Beta-only names: a user's quote/author/export stay theirs", () => {
    for (const key of ["quote", "quote-prefix", "quote-suffix", "author", "export"]) {
      expect(canonicalPropertyKey(key)).toBe(key);
      expect(keySpellings(`tine.${key}`)).toEqual([`tine.${key}`]);
    }
  });
});
