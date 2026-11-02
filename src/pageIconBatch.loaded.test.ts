import { afterEach, expect, it } from "vitest";
import { resetStore } from "./document";
import { setDoc } from "./document/model";
import { pageIconPropertyInUse, readPageIconProperty } from "./pageIconBatch";

afterEach(() => resetStore());
const load = (preBlock: string) => setDoc({ byId: {}, pages: [{ name: "P", kind: "page", title: "P", preBlock,
  roots: [], format: "md", readOnly: false, guide: false }], feed: [], loaded: true });

// L3-05: an open page shows the icon its links show (`page_icons.rs` `pre_block_icon`).
it("takes the first nonblank value of a repeated key, like the native index", () => {
  load("icon::\nicon:: 🏁");
  expect(readPageIconProperty("P")).toBe("🏁");
  expect(pageIconPropertyInUse("P")).toBe("icon");
});
it("keeps the policy's key order through a blank first spelling", () => {
  load("tine.icon::\ntine.icon:: ⭐\nicon:: 🏁");
  expect(readPageIconProperty("P")).toBe("⭐");
  load("tine.icon::\nicon:: 🏁");
  expect(readPageIconProperty("P")).toBe("🏁");
});
