import { expect, it } from "vitest";
import { embedBlockTarget, embedPageTarget } from "./embedTarget";

// L3-09: the embed target is what OG and the native static render take: all of `[[ ... ]]`.
it("takes a page name holding brackets, like OG and the published render", () => {
  expect(embedPageTarget("[[a]b]]")).toBe("a]b");
  expect(embedPageTarget("[[a [[b]]]]")).toBe("a [[b]]");
  expect(embedPageTarget(" [[plain]] ")).toBe("plain");
});
it("is not fooled by anything else", () => {
  expect(embedPageTarget("[[]]")).toBeNull();
  expect(embedPageTarget("[[a]")).toBeNull();
  expect(embedPageTarget("x [[a]]")).toBeNull();
  expect(embedBlockTarget("((11111111-1111-4111-8111-111111111111))")).toBe("11111111-1111-4111-8111-111111111111");
  expect(embedBlockTarget("(())")).toBeNull();
});
