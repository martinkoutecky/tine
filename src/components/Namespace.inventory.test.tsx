import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";

const backendMock = vi.hoisted(() => {
  const referenced = (name: string) => ({
    key: name.toLowerCase(),
    name,
    is_journal: false,
    day: null,
    target: { kind: "absent" as const, id: `pages/${name.replaceAll("/", "___")}.md` },
  });
  return {
    graphBindingGeneration: () => 0,
    pageInventory: vi.fn(async () => ({
      rev: "1",
      entries: [
        {
          key: "test",
          name: "test",
          is_journal: false,
          day: null,
          target: { kind: "existing" as const, id: "pages/test.md", others: [] },
        },
        referenced("test/testy test"),
        referenced("test/testy test/another"),
        referenced("test/testy tester"),
      ],
    })),
  };
});

vi.mock("../backend", () => ({ backend: () => backendMock }));

import { NamespaceHierarchy, NamespaceMacro } from "./Namespace";
import { createSignal } from "solid-js";
import { resetReferenceSectionState } from "../referenceSectionState";

afterEach(() => {
  resetReferenceSectionState();
  document.body.innerHTML = "";
});

// L7s6-04: OG's header is the namespace the macro names, and the tree is its descendants.
describe("{{namespace a/b}}", () => {
  it("heads with the named namespace and lists only what lies below it", async () => {
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <NamespaceMacro root="test/testy test" />, root);
    try {
      await vi.waitFor(() => expect(root.querySelector(".ns-macro-head .page-ref")?.textContent).toBe("test/testy test"));
      expect([...root.querySelectorAll(".ns-macro-tree .page-ref")].map((a) => a.textContent)).toEqual(["another"]);
    } finally { dispose(); }
  });
});

describe("GH #229 reference-only namespace descendants", () => {
  it("GH #738: folds by title, retains the page choice across navigation/remount and resets with the graph", async () => {
    const root = document.createElement("div");
    document.body.append(root);
    const [name, setName] = createSignal("test");
    let dispose = render(() => <NamespaceHierarchy name={name()} />, root);
    const title = () => root.querySelector<HTMLButtonElement>('button[aria-expanded]')!;
    try {
      await vi.waitFor(() => expect(root.querySelectorAll(".ns-hier-row")).toHaveLength(3));
      expect(title()?.textContent).toContain("Hierarchy");
      expect(title()?.getAttribute("aria-expanded")).toBe("true");
      title().focus();
      expect(document.activeElement).toBe(title());
      // Native button activation supports Enter/Space without a custom key handler.
      title().click();
      expect(title().getAttribute("aria-expanded")).toBe("false");
      expect(root.querySelector(".ns-hier-row")).toBeNull();
      setName("test/testy test");
      expect(title().getAttribute("aria-expanded")).toBe("true");
      setName("test");
      expect(title().getAttribute("aria-expanded")).toBe("false");
      dispose();
      dispose = render(() => <NamespaceHierarchy name="test" />, root);
      expect(title().getAttribute("aria-expanded")).toBe("false");
      title().click();
      expect(root.querySelectorAll(".ns-hier-row")).toHaveLength(3);
      title().click();
      dispose();
      resetReferenceSectionState();
      dispose = render(() => <NamespaceHierarchy name="test" />, root);
      expect(title().getAttribute("aria-expanded")).toBe("true");
    } finally { dispose(); }
  });
  it("shows the synthesized reference-only descendants in the real Hierarchy component", async () => {
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <NamespaceHierarchy name="test" />, root);

    try {
      await vi.waitFor(() => {
        const rows = [...root.querySelectorAll(".ns-hier-row")].map((row) =>
          [...row.querySelectorAll(".page-ref")]
            .map((link) => link.textContent?.replaceAll("[[", "").replaceAll("]]", ""))
            .join("/")
        );
        expect(rows).toEqual([
          "test/testy test",
          "test/testy test/another",
          "test/testy tester",
        ]);
      });
    } finally {
      dispose();
    }
  });
});
