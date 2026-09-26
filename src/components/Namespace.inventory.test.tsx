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

import { NamespaceHierarchy } from "./Namespace";

afterEach(() => {
  document.body.innerHTML = "";
});

describe("GH #229 reference-only namespace descendants", () => {
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
