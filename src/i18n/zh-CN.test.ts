import { expect, it } from "vitest";
// @ts-expect-error: shared ESM catalog checker has no declaration file.
import { checkCatalog } from "../../scripts/lib/i18n-catalog.mjs";
import { msg, render, renderEnglish, setActiveLocale, t } from "./index";

it("ships a complete, reviewed Simplified Chinese interface catalog", () => {
  const { errors, report } = checkCatalog();
  expect(errors).toEqual([]);
  expect(report["zh-CN"].missing).toEqual([]);
  expect(report["zh-CN"].stale).toEqual([]);
});

it("shows Chinese query and recovery controls while authored names and reports stay literal", async () => {
  const page = "研发 [[TODO]] <strong>原文</strong>";
  const notice = msg("conflict-bar-live-notice", { name: page });
  try {
    await setActiveLocale("zh-CN");
    expect(t("query-query-sheet-parts-task-marker")).toBe("任务标记");
    expect(t("context-menu-ui-copy-block-embed")).toBe("复制块嵌入");
    expect(t("query-preview-count", { kind: "page", count: 2 })).toBe("2 个页面");
    expect(render(notice)).toContain(`你编辑“${page}”时`);
    expect(renderEnglish(notice)).toContain(`“${page}” changed on disk`);
    expect(t("settings-ui-todo-doing")).toBe("TODO / DOING");
    expect(t("settings-ui-template-inserted-into-a-new-day-s-journal-saved"))
      .toContain("<code>:default-templates {:journals …}</code>");
    await setActiveLocale("en");
    expect(t("query-query-sheet-parts-task-marker")).toBe("Task marker");
  } finally {
    await setActiveLocale("en");
  }
});
