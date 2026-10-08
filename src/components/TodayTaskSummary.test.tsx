import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { setDoc, type FeedPage } from "../document/model";
import { TodayTaskSummary, summarizePageTasks } from "./TodayTaskSummary";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  resetStore();
  document.body.innerHTML = "";
});

describe("today task summary", () => {
  it("counts open and in-progress markers across the loaded page tree", () => {
    const page: FeedPage = {
      name: "August 28th, 2026",
      title: "August 28th, 2026",
      kind: "journal",
      preBlock: null,
      roots: ["todo", "done", "plain"],
      format: "md",
      readOnly: false,
      guide: false,
    };
    setDoc({
      byId: {
        todo: { id: "todo", raw: "TODO Parent", collapsed: false, parent: null, page: page.name, children: ["doing"] },
        doing: { id: "doing", raw: "DOING Child", collapsed: false, parent: "todo", page: page.name, children: [] },
        done: { id: "done", raw: "DONE Finished", collapsed: false, parent: null, page: page.name, children: [] },
        plain: { id: "plain", raw: "Notes", collapsed: false, parent: null, page: page.name, children: [] },
      },
      pages: [page],
      feed: [page.name],
      loaded: true,
    });

    expect(summarizePageTasks(page)).toEqual({ open: 2, inProgress: 1 });

    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <TodayTaskSummary page={page} />, host);
    expect(host.textContent).toBe("2 tasks today, 1 in progress");
    expect(host.querySelector(".today-task-summary")?.getAttribute("aria-label"))
      .toBe("2 tasks today, 1 in progress");
    dispose();
  });
});
