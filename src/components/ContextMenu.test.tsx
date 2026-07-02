import { describe, expect, it } from "vitest";
import { deletePageMenuLabel, pageMenuAvailability } from "./ContextMenu";

describe("PageMenu page-kind availability", () => {
  it("keeps rename page-only but exposes delete for pages and journals", () => {
    expect(pageMenuAvailability("page")).toEqual({ rename: true, delete: true });
    expect(pageMenuAvailability("journal")).toEqual({ rename: false, delete: true });
  });

  it("labels the delete action by page kind", () => {
    expect(deletePageMenuLabel("page")).toBe("Delete page");
    expect(deletePageMenuLabel("journal")).toBe("Delete journal");
  });
});
