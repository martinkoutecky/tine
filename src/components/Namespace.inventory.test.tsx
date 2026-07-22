import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";

const backendMock = vi.hoisted(() => ({
  listPages: vi.fn(async () => [{
    name: "test", kind: "page", date_key: null, path: "pages/test.md",
  }]),
  referencedPageNames: vi.fn(async () => [
    "test",
    "test/testy test",
    "test/testy tester",
    "test/testy test/another",
  ]),
}));

vi.mock("../backend", () => ({ backend: () => backendMock }));
vi.mock("../warmCache", () => ({ waitForWarmCache: vi.fn(async () => true) }));

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
