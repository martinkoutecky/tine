import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import { CalcBlock } from "./body";
import { initParser } from "./parse";
import { resetStore } from "../document";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  resetStore();
  document.body.innerHTML = "";
});

// GH #339: a committed ```calc block's AST `code` ends with a newline, and the
// rendered panel used to mint one phantom empty row for it. Only expression
// lines get rows; interior blanks keep theirs.
describe("CalcBlock trailing blank lines (GH #339)", () => {
  it("renders no row for the trailing AST newline", () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <CalcBlock src={"1 + 1\n14 * 3\n"} />, root);
    const inputs = [...root.querySelectorAll<HTMLElement>(".calc-in")].map((el) => el.textContent);
    expect(inputs).toEqual(["1 + 1", "14 * 3"]);
    expect(root.querySelectorAll(".calc-lineno")).toHaveLength(2);
    dispose();
  });

  it("keeps an interior blank row but drops the trailing one", () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <CalcBlock src={"1 + 1\n\n2 + 2\n"} />, root);
    expect(root.querySelectorAll(".calc-lineno")).toHaveLength(3);
    dispose();
  });
});
