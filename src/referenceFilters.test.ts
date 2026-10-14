import { expect, it } from "vitest";
import { parseReferenceFilters, writeReferenceFilters } from "./referenceFilters";

it("round-trips OG's string/boolean map including escaped page names", () => {
  const filters = { Germany: "in", 'a "quote" \\ path': "out", 日本: "in" } as const;
  expect(parseReferenceFilters(writeReferenceFilters(filters))).toEqual(filters);
  expect(parseReferenceFilters('{"de" true, "Deutschland" false}')).toEqual({ de: "in", Deutschland: "out" });
  expect(parseReferenceFilters('{ ; OG EDN comment\n "Germany" true }')).toEqual({ Germany: "in" });
  expect(parseReferenceFilters("{}")).toEqual({});
});

it.each(['{"de"', '{"de" true', '{"de" true} garbage', '{"de" "true"}', '{"de" nil}', '{"de" trueish}', '{:de true}', '{"de" true "unterminated}', '[]', '{"bad\\q" true}', '{"bad\\u123" false}'])("rejects incomplete or non-filter EDN: %s", value => {
  expect(parseReferenceFilters(value)).toBeNull();
});
