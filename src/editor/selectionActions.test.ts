import { describe, expect, it } from "vitest";
import { SELECTION_ACTIONS, essentialSelectionActions, secondarySelectionActions } from "./selectionActions";

describe("selection action registry", () => {
  it("exposes stable unique identities and keeps page links and code essential", () => {
    const ids = SELECTION_ACTIONS.map((action) => action.id);
    expect(new Set(ids).size).toBe(ids.length);
    expect(essentialSelectionActions.map((action) => action.id)).toEqual([
      "bold", "italic", "page-link", "inline-code",
    ]);
    expect(secondarySelectionActions.map((action) => action.id)).toEqual([
      "link", "strikethrough", "highlight",
    ]);
  });

  it("wraps and unwraps page links and inline code without losing the inner selection", () => {
    const page = SELECTION_ACTIONS.find((action) => action.id === "page-link")!;
    const code = SELECTION_ACTIONS.find((action) => action.id === "inline-code")!;
    const wrappedPage = page.apply("alpha beta", 0, 5);
    expect(wrappedPage).toEqual({ text: "[[alpha]] beta", start: 2, end: 7 });
    expect(page.apply(wrappedPage.text, wrappedPage.start, wrappedPage.end)).toEqual({
      text: "alpha beta", start: 0, end: 5,
    });
    expect(code.apply("alpha beta", 0, 5)).toEqual({
      text: "`alpha` beta", start: 1, end: 6,
    });
  });

  it.each([
    ["md", "bold", "**"],
    ["md", "italic", "*"],
    ["md", "strikethrough", "~~"],
    ["md", "highlight", "=="],
    ["org", "bold", "*"],
    ["org", "italic", "/"],
    ["org", "strikethrough", "+"],
    ["org", "highlight", "^^"],
  ] as const)(
    "uses %s %s syntax and leaves browser-selected outer whitespace outside the delimiters",
    (format, id, delimiter) => {
      const action = SELECTION_ACTIONS.find((candidate) => candidate.id === id)!;
      const edit = action.apply("before \tselected \r\nafter", 6, 18, format, "backward");
      expect(edit).toEqual({
        text: `before \t${delimiter}selected${delimiter} \r\nafter`,
        start: 8 + delimiter.length,
        end: 16 + delimiter.length,
        direction: "backward",
      });
    },
  );

  it("does not change the neighboring page-link and inline-code whitespace semantics", () => {
    const page = SELECTION_ACTIONS.find((action) => action.id === "page-link")!;
    const code = SELECTION_ACTIONS.find((action) => action.id === "inline-code")!;
    expect(page.apply("alpha ", 0, 6, "org").text).toBe("[[alpha ]]");
    expect(code.apply("alpha ", 0, 6, "org").text).toBe("`alpha `");
  });
});
