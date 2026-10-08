import { beforeAll, describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import { renderBlocks } from "./body";
import { initParser } from "./parse";
import { parseBody } from "./facets";
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

beforeAll(initParser);

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

it("direct AST rendering uses the same earliest-group and last-value rule as live blocks", () => {
  const raw = "status:: first\nSTATUS:: last\nid:: hidden\nbody\nstatus:: trailing";
  const host = document.createElement("div");
  const dispose = render(() => renderBlocks(parseBody(raw, "md")), host);
  try {
    expect(host.querySelectorAll(".prop")).toHaveLength(1);
    expect(host.querySelector(".prop-value")?.textContent).toBe("last");
    expect(host.querySelector(".prop-key .page-ref")?.textContent).toBe("status");
  } finally { dispose(); }
});
