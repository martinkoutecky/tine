import { describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import { renderBlocks } from "./body";
import { isRenderHiddenProp, propertyKeyNorm } from "./block";

function renderedProperty(key: string, value: string): { key: string | null; value: string | null } {
  const host = document.createElement("div");
  const dispose = render(
    () => renderBlocks([{ kind: "properties", props: [[key, value]] }]),
    host,
  );
  const rendered = {
    key: host.querySelector(".block-property-key")?.textContent ?? null,
    value: host.querySelector(".block-property-val")?.textContent ?? null,
  };
  dispose();
  return rendered;
}

describe("propertyKeyNorm", () => {
  it("folds case, spaces, and underscores to the canonical property key", () => {
    expect(propertyKeyNorm(" Done_At ")).toBe("done-at");
    expect(propertyKeyNorm("Done At")).toBe("done-at");
  });

  it("renders the folded key while leaving the value text unchanged", () => {
    const rendered = renderedProperty("Done_At", "Value_With MIXED Case");
    expect(rendered.key).toBe("done-at");
    expect(rendered.value).toBe("Value_With MIXED Case");
  });

  it("folds user-hidden property names before comparing them", () => {
    expect(isRenderHiddenProp("My_Prop", ["my-prop"])).toBe(true);
  });
});
