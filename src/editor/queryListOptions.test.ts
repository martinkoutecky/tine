import { beforeAll, expect, it } from "vitest";
import { initParser } from "../render/parse";
import { queryListOptions } from "./queryListOptions";

beforeAll(initParser);
it("reads direct EDN display options through discard-aware UTF-8 spans", () => {
  expect(queryListOptions('{:title "é 😀" :breadcrumb-show? #_ true false :group-by-page? false :table-view? false}'))
    .toEqual({ breadcrumb: false, groupByPage: false, table: false });
  expect(queryListOptions('{:breadcrumb-show? nil :group-by-page? nil}')).toEqual({ breadcrumb: true, groupByPage: false });
});
it.each([
  '{:query [:find :breadcrumb-show? false]}',
  '{:title ":breadcrumb-show? false"}',
  '{:x {:breadcrumb-show? false}}',
  '{#_ :breadcrumb-show? #_ false :query []}',
  '{:query [] ; :breadcrumb-show? false\n}',
  '[:find :breadcrumb-show? false]',
  '{:query []',
])("does not invent options from nested or unreadable text: %s", source => {
  expect(queryListOptions(source)).toEqual({});
});
