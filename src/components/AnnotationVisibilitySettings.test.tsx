import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { AnnotationVisibilitySettings } from "./AnnotationVisibilitySettings";
import { authorMarksHidden, setAuthorMarksHidden } from "../authorMarksPref";
import { commentMarginHidden, forceCommentMarginShown, commentsOnClick, changeCommentsOnClick } from "../commentMarginPref";
import { setActiveLocale } from "../i18n";
import { setToasts, toasts } from "../toasts";
import { backend } from "../backend";

afterEach(async () => { vi.restoreAllMocks(); changeCommentsOnClick(false); setToasts([]); setAuthorMarksHidden(false); forceCommentMarginShown(); await setActiveLocale("en"); document.body.innerHTML = ""; });
it.each(["en", "zh-CN"])("%s Settings groups two independently labelled switches", async locale => {
  await setActiveLocale(locale); setAuthorMarksHidden(false); forceCommentMarginShown();
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <AnnotationVisibilitySettings />, root);
  try {
    const marks = root.querySelector<HTMLButtonElement>('[data-setting-id="settings-author-marks"] [role="switch"]')!;
    const comments = root.querySelector<HTMLButtonElement>('[data-setting-id="settings-ui-comment-margin"] [role="switch"]')!;
    expect(marks.closest('[role="group"]')?.contains(comments)).toBe(true);
    expect(root.textContent).toContain(locale === "en" ? "Show/Hide author marks" : "显示/隐藏作者标记");
    marks.click(); expect(authorMarksHidden()).toBe(true); expect(commentMarginHidden()).toBe(false);
    comments.click(); expect(authorMarksHidden()).toBe(true); expect(commentMarginHidden()).toBe(true);
    marks.click(); expect(authorMarksHidden()).toBe(false); expect(commentMarginHidden()).toBe(true);
  } finally { dispose(); }
});
it.each(["en", "zh-CN"])("%s offers the default margin choice and persists On click with one local write", async locale => {
  await setActiveLocale(locale);
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <AnnotationVisibilitySettings />, root);
  try {
    const select = root.querySelector<HTMLSelectElement>('select[aria-label="' + (locale === "en" ? "Comments" : "评论") + '"]')!;
    expect(select.value).toBe("margin"); expect(commentsOnClick()).toBe(false);
    expect([...select.options].map(option => option.textContent)).toEqual(locale === "en" ? ["In the margin", "On click"] : ["在页边显示", "点击时显示"]);
    const write = vi.spyOn(Storage.prototype, "setItem");
    const remove = vi.spyOn(Storage.prototype, "removeItem");
    const open = vi.spyOn(backend(), "pageOpen"), submit = vi.spyOn(backend(), "pageSubmit");
    select.value = "click"; select.dispatchEvent(new Event("change", { bubbles: true }));
    expect(commentsOnClick()).toBe(true); expect(write).toHaveBeenCalledExactlyOnceWith("logseq-claude.comments-on-click", "1");
    expect(remove).not.toHaveBeenCalled();
    select.value = "margin"; select.dispatchEvent(new Event("change", { bubbles: true }));
    expect(commentsOnClick()).toBe(false); expect(remove).toHaveBeenCalledExactlyOnceWith("logseq-claude.comments-on-click");
    expect(write).toHaveBeenCalledTimes(1);
    expect(open).not.toHaveBeenCalled(); expect(submit).not.toHaveBeenCalled();
  } finally { dispose(); }
});
it.each([false, true])("a refused display write keeps On click=%s in Settings and reports the failure", onClick => {
  changeCommentsOnClick(onClick);
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <AnnotationVisibilitySettings />, root);
  try {
    vi.spyOn(Storage.prototype, onClick ? "removeItem" : "setItem").mockImplementation(() => { throw new Error("quota"); });
    const select = root.querySelector<HTMLSelectElement>('[data-setting-id="settings-comments-display"] select')!;
    select.value = onClick ? "margin" : "click"; select.dispatchEvent(new Event("change", { bubbles: true }));
    expect(commentsOnClick()).toBe(onClick); expect(select.value).toBe(onClick ? "click" : "margin");
    expect(toasts().some(toast => toast.kind === "error")).toBe(true);
  } finally { dispose(); }
});
