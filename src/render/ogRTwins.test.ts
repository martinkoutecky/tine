// Checkpoint-5 packet R (I-12): the app answers ordered-list ownership with the same key fold as
// the static export (Rust `DocBlock::property` → `property_key_norm`, tests/og_r_render.rs).
import { beforeAll, describe, expect, it } from "vitest";
import { initParser } from "./parse";
import { orderListTypeFromRaw, rawWithOrderListType } from "../document/edits/properties";

describe("logseq.order-list-type key fold", () => {
  beforeAll(initParser);
  it("reads the type under the same spelling the export honours (underscores, case)", () => {
    for (const key of ["logseq.order-list-type", "logseq.order_list_type", "Logseq.Order-List-Type"]) {
      expect(orderListTypeFromRaw(`item\n${key}:: number`, "md"), key).toBe("number");
    }
  });
  it("replaces an existing underscore spelling instead of duplicating it", () => {
    const next = rawWithOrderListType("item\nlogseq.order_list_type:: number", "bullet", "md");
    expect(next.match(/order[-_]list[-_]type/g)).toHaveLength(1);
    expect(next).toContain("logseq.order-list-type:: bullet");
  });
});
