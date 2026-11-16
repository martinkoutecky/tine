import { expect, it } from "vitest";
import { visibleBody } from "./block";

it.each(["Note:: see [[Foo]]", "~a:: b~", "## Org text"])("keeps Org body %s in labels and export", (raw) => {
  expect(visibleBody(raw, "org")).toEqual([raw]);
});

it("removes Org metadata while retaining the body", () => {
  expect(visibleBody("Note:: see [[Foo]]\n:PROPERTIES:\n:id: example\n:END:", "org"))
    .toEqual(["Note:: see [[Foo]]"]);
});
