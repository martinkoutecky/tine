import { afterEach, describe, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { cycleSelectionTasks, flushAll, moveSelection, redo, resetStore, selectBlock, selectedIds, undo } from "./document";
import { doc } from "./document/model";
import { loadSingle } from "./document/workingSet";
import { bindTestHost, submittedPages } from "./document/host/wiring.test.support";
import { setGraphMeta } from "./graphSession";
import { setWorkflow } from "./ui";
import type { GraphMeta, PageRead } from "./types";

afterEach(() => {
  resetStore();
  setGraphMeta(null);
  setWorkflow("now");
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("selected-block task clocks (UI-TASK-CYCLE-TIMETRACKING)", () => {
  for (const format of ["md", "org"] as const) {
    for (const workflow of ["todo", "now"] as const) {
      for (const enabled of [true, false]) {
        for (const seconds of [true, false]) {
          it(`${format}, ${workflow}, tracking=${enabled}, seconds=${seconds}: raw text, one undo and save`, async () => {
            resetStore();
            setWorkflow(workflow);
            setGraphMeta({ enable_timetracking: enabled, logbook_with_second_support: seconds } as GraphMeta);
            // Freeze only Date: the real host's save/mail promises still run.
            vi.useFakeTimers({ toFake: ["Date"] });
            vi.setSystemTime(new Date(2026, 0, 1, 10, 0, 0));
            const suffix = seconds ? ":00" : "";
            const start = `2026-01-01 Thu 09:00${suffix}`;
            const end = `2026-01-01 Thu 10:00${suffix}`;
            const drawer = `\n:LOGBOOK:\nCLOCK: [${start}]\n:END:`;
            const active = workflow === "todo" ? "DOING" : "NOW";
            const open = workflow === "todo" ? "TODO" : "LATER";
            const before = ["plain", `${open} start`, `${active} finish${drawer}`, "   "];
            const page: PageRead = {
              name: "Clock tasks", title: "Clock tasks", kind: "page", pre_block: null,
              format, id: `pages/clock-tasks.${format}`, rev: "base",
              blocks: before.map((raw, i) => ({ id: `clock-${i}`, raw, collapsed: false, children: [] })),
            };
            loadSingle(page); // Routed page: deliberately absent from doc.feed.
            await bindTestHost();
            const submit = vi.spyOn(backend(), "pageSubmit");
            const ids = page.blocks.map((block) => block.id);
            selectBlock(ids[0]);
            for (let i = 1; i < ids.length; i++) moveSelection(1, true);
            const raws = () => ids.map((id) => doc.byId[id].raw);
            const after = [
              `${open} plain`,
              `${active} start${enabled ? `\n:LOGBOOK:\nCLOCK: [${end}]\n:END:` : ""}`,
              `DONE finish${enabled ? `\n:LOGBOOK:\nCLOCK: [${start}]--[${end}] =>  01:00${suffix}\n:END:` : drawer}`,
              "   ",
            ];

            expect(cycleSelectionTasks()).toBe(true);
            expect(raws()).toEqual(after);
            expect(selectedIds()).toEqual(ids);
            await flushAll();
            expect(submit).toHaveBeenCalledTimes(1);
            expect(submittedPages(submit)[0].blocks.map((block) => block.raw)).toEqual(after);
            undo();
            expect(raws()).toEqual(before);
            redo();
            expect(raws()).toEqual(after);
          });
        }
      }
    }
  }
});
