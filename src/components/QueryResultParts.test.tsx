// The Pages answer of a query block, ported in meaning from master's
// src/components/QueryPageResults.test.tsx (q3_* tests). og's engine returns
// `PageRow`s (path, name, kind, own properties) rather than master's search
// hits, so the two hit-only master cases do not apply here:
//  - q3_page_excerpt_is_shown_only_when_it_is_not_the_page_name — a PageRow
//    carries no evidence/excerpt;
//  - q3_page_rows_never_fabricate_properties_for_a_virtual_suggestion — og's
//    `query_run` returns stored pages only, never a virtual name suggestion.
import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { createSignal, type JSX } from "solid-js";
import type { PageRow } from "../editor/queryIr";
import { QueryPageRows, QueryStatisticsSummary, type QueryView } from "./QueryResultParts";
import * as router from "../router";
import * as ui from "../ui";
import { backend } from "../backend";
import { pageByName, resetStore } from "../document";
import type { PageRead } from "../types";

afterEach(() => {
  vi.restoreAllMocks();
  ui.closePageProps();
  resetStore();
  document.body.innerHTML = "";
});

function mount(node: () => JSX.Element): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.appendChild(root);
  return { root, dispose: render(node, root) };
}

function row(name: string, path: string, properties: [string, string][] = []): PageRow {
  return { name, path, kind: "page", properties };
}

const names = (root: HTMLElement) => [...root.querySelectorAll(".query-page-link")].map((el) => el.textContent);

describe("QueryPageRows", () => {
  // master q3_page_rows_key_by_path_and_kind_not_display_name
  it("keeps two same-named pages at two paths as two distinct links", () => {
    const { root, dispose } = mount(() => (
      <QueryPageRows rows={[row("Research", "pages/a/Research.md"), row("Research", "pages/b/Research.md")]} view="list" />
    ));
    try {
      const paths = [...root.querySelectorAll<HTMLElement>(".query-page-link")].map((el) => el.dataset.pagePath);
      expect(paths).toEqual(["pages/a/Research.md", "pages/b/Research.md"]);
    } finally {
      dispose();
    }
  });

  // master q3_page_presentations_render_their_own_dom_contract
  it("renders list, table (own page properties as columns) and board", () => {
    const rows = [row("Alpha", "pages/Alpha.md", [["status", "open"]]), row("Beta", "pages/Beta.md", [["status", "done"]])];
    const [view, setView] = createSignal<QueryView>("list");
    const { root, dispose } = mount(() => <QueryPageRows rows={rows} view={view()} groupBy="prop:status" />);
    try {
      expect(root.querySelector(".query-results-list")?.getAttribute("aria-label")).toBe("Page results");
      expect(root.querySelectorAll(".query-results-list > li")).toHaveLength(2);

      setView("table");
      const table = root.querySelector("table.query-page-table")!;
      expect([...table.querySelectorAll("thead th")].map((th) => th.textContent)).toEqual(["Page", "status"]);
      expect(table.querySelectorAll("tbody tr")).toHaveLength(2);
      expect(table.querySelectorAll("tbody tr")[0].textContent).toContain("open");

      setView("board");
      const columns = [...root.querySelectorAll<HTMLElement>(".query-board-column")];
      expect(columns.map((column) => column.getAttribute("aria-label"))).toEqual(["open", "done"]);
      expect(columns[0].querySelectorAll(".query-page-link")).toHaveLength(1);
    } finally {
      dispose();
    }
  });

  // GH #619 item 8: OG shows a page result with its page properties; the default list
  // view showed only the name, so properties were visible only after switching to Table.
  it("list view shows each page's own properties as selectable text beside its link", () => {
    const rows = [
      row("Alpha", "pages/Alpha.md", [["status", "open"], ["owner", "Ada"]]),
      row("Beta", "pages/Beta.md"),
    ];
    const { root, dispose } = mount(() => <QueryPageRows rows={rows} view="list" />);
    try {
      const items = [...root.querySelectorAll(".query-results-list > li")];
      expect(items).toHaveLength(2);
      const props = [...items[0].querySelectorAll(".query-page-prop")].map((el) => el.textContent?.replace(/\s+/g, " ").trim());
      expect(props).toEqual(["status: open", "owner: Ada"]);
      // A page with no properties gets no empty property strip.
      expect(items[1].querySelector(".query-page-props")).toBeNull();
      // The values are plain, selectable content (no buttons inside the strip); the only
      // buttons are the page link and the edit pencil (follow-up B: edit from the row).
      expect(items[0].querySelector(".query-page-props")!.querySelectorAll("button")).toHaveLength(0);
      expect([...items[0].querySelectorAll("button")].map((b) => b.className)).toEqual(["query-page-link", "query-page-props-edit"]);
      expect(items[0].querySelector(".query-page-props")?.getAttribute("data-selectable")).toBe("text");
    } finally {
      dispose();
    }
  });

  // GH #619 item 8 / follow-up B: the pencil LOADS the page (a read) and opens the existing
  // properties panel; the panel's write is setPageProperty (covered by the real-app journey).
  it("loads the page and opens the properties panel from the row's pencil", async () => {
    const page = { id: "p1", name: "Alpha", kind: "page", title: "Alpha", pre_block: "status:: open", blocks: [], rev: "r1", format: "md" } as PageRead;
    const getPage = vi.spyOn(backend(), "getPage").mockResolvedValue(page);
    const { root, dispose } = mount(() => <QueryPageRows rows={[row("Alpha", "pages/Alpha.md", [["status", "open"]])]} view="list" />);
    try {
      expect(pageByName("Alpha")).toBeUndefined();
      root.querySelector<HTMLButtonElement>(".query-page-props-edit")!.click();
      await vi.waitFor(() => expect(ui.pagePropsPanel()?.scope).toEqual({ kind: "page", name: "Alpha" }));
      expect(getPage).toHaveBeenCalledWith("Alpha", "page");
      expect(pageByName("Alpha")?.name).toBe("Alpha");
    } finally {
      dispose();
    }
  });

  it("opens no panel when the page is gone, and says so", async () => {
    vi.spyOn(backend(), "getPage").mockResolvedValue(null as unknown as PageRead);
    const { root, dispose } = mount(() => <QueryPageRows rows={[row("Gone", "pages/Gone.md")]} view="list" />);
    try {
      root.querySelector<HTMLButtonElement>(".query-page-props-edit")!.click();
      await new Promise((resolve) => setTimeout(resolve, 20));
      expect(ui.pagePropsPanel()).toBeNull();
      expect(pageByName("Gone")).toBeUndefined();
    } finally {
      dispose();
    }
  });

  // C3X X6 (L13): two components answered "what does this page column show"; only the
  // search-hit one accepted `journal_day`, so a Table column of that name was blank here.
  it("shows the same journal_day / journal-day / day value in every answerer", () => {
    const journal: PageRow = { name: "Sep 29th, 2026", path: "journals/2026_09_29.md", kind: "journal", journal_day: 20260929, properties: [] };
    for (const field of ["journal_day", "journal-day", "day", "prop:journal_day"]) {
      const { root, dispose } = mount(() => <QueryPageRows rows={[journal]} view="table" columns={[field]} />);
      try {
        expect(root.querySelector("tbody tr")?.textContent).toContain("20260929");
      } finally {
        dispose();
      }
    }
  });

  it("uses the view's chosen columns when it names them", () => {
    const rows = [row("Alpha", "pages/Alpha.md", [["status", "open"], ["owner", "ann"]])];
    const { root, dispose } = mount(() => <QueryPageRows rows={rows} view="table" columns={["prop:owner"]} />);
    try {
      expect([...root.querySelectorAll("thead th")].map((th) => th.textContent)).toEqual(["Page", "owner"]);
      expect(root.querySelector("tbody tr")?.textContent).toContain("ann");
    } finally {
      dispose();
    }
  });

  // master q3_page_board_groups_by_adjacency_and_never_reorders_the_backends_answer
  it("groups a board by adjacency and never reorders the engine's answer", () => {
    const rows = [
      row("Alpha", "pages/Alpha.md", [["status", "open"]]),
      row("Beta", "pages/Beta.md", [["status", "done"]]),
      row("Gamma", "pages/Gamma.md", [["status", "open"]]),
    ];
    const { root, dispose } = mount(() => <QueryPageRows rows={rows} view="board" groupBy="prop:status" />);
    try {
      expect([...root.querySelectorAll(".query-board-column")].map((c) => c.getAttribute("aria-label")))
        .toEqual(["open", "done", "open"]);
      expect(names(root)).toEqual(["Alpha", "Beta", "Gamma"]);
    } finally {
      dispose();
    }
  });

  // master q3_page_rows_never_resort_or_truncate_what_the_backend_returned
  it("never re-sorts or truncates the rows the engine returned", () => {
    const rows = ["Zulu", "Alpha", "Mike"].map((name) => row(name, `pages/${name}.md`));
    const { root, dispose } = mount(() => <QueryPageRows rows={rows} view="list" />);
    try {
      expect(names(root)).toEqual(["Zulu", "Alpha", "Mike"]);
    } finally {
      dispose();
    }
  });

  // master q3_page_row_navigation_carries_the_hosts_link_gestures
  it("opens the page by path, and in the sidebar with Shift", () => {
    const opened = vi.spyOn(router, "openPageTarget").mockImplementation(() => {});
    const sidebar = vi.spyOn(ui, "openPageInSidebar").mockImplementation(() => {});
    const { root, dispose } = mount(() => <QueryPageRows rows={[row("Alpha", "pages/Alpha.md")]} view="list" />);
    try {
      const link = root.querySelector<HTMLButtonElement>(".query-page-link")!;
      expect(link.dataset.pageKind).toBe("page");
      link.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      expect(opened).toHaveBeenCalledWith({ name: "Alpha", pageKind: "page", path: "pages/Alpha.md" });
      link.dispatchEvent(new MouseEvent("click", { bubbles: true, shiftKey: true }));
      expect(sidebar).toHaveBeenCalledWith({ name: "Alpha", pageKind: "page", path: "pages/Alpha.md" });
    } finally {
      dispose();
    }
  });
});

describe("QueryStatisticsSummary", () => {
  it("shows the engine's overall aggregates, including a marker instead of a number", () => {
    const { root, dispose } = mount(() => (
      <QueryStatisticsSummary
        statistics={{
          count: 3,
          aggregates: [["prop:points", "sum"], ["prop:points", "avg"]],
          group_by: null,
          overall: [{ kind: "number", value: 12, skipped: 0 }, { kind: "marker", reason: "non_numeric", skipped: 1 }],
          groups: null,
          grouping_status: "none",
        }}
      />
    ));
    try {
      const text = root.textContent ?? "";
      expect(text).toContain("12");
      expect(text).toContain("Unavailable (non numeric)");
      expect(text).not.toContain("NaN");
    } finally {
      dispose();
    }
  });
});
