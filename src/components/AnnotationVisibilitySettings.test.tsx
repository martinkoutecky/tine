import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { AnnotationVisibilitySettings } from "./AnnotationVisibilitySettings";
import { authorMarksHidden, setAuthorMarksHidden } from "../authorMarksPref";
import { commentMarginHidden, forceCommentMarginShown, commentsInPopup, changeCommentsInPopup } from "../commentMarginPref";
import { setActiveLocale } from "../i18n";
import { setToasts, toasts } from "../toasts";
import { backend } from "../backend";

afterEach(async () => { vi.restoreAllMocks(); changeCommentsInPopup(false); setToasts([]); setAuthorMarksHidden(false); forceCommentMarginShown(); await setActiveLocale("en"); document.body.innerHTML = ""; });
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
it.each(["en", "zh-CN"])("%s offers the default margin choice and persists In a popup with one local write", async locale => {
  await setActiveLocale(locale);
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <AnnotationVisibilitySettings />, root);
  try {
    const select = root.querySelector<HTMLSelectElement>('select[aria-label="' + (locale === "en" ? "Comments" : "评论") + '"]')!;
    expect(select.value).toBe("margin"); expect(commentsInPopup()).toBe(false);
    expect([...select.options].map(option => option.textContent)).toEqual(locale === "en" ? ["In the margin", "In a popup"] : ["在页边显示", "在弹窗中显示"]);
    const write = vi.spyOn(Storage.prototype, "setItem");
    const remove = vi.spyOn(Storage.prototype, "removeItem");
    const open = vi.spyOn(backend(), "pageOpen"), submit = vi.spyOn(backend(), "pageSubmit");
    select.value = "popup"; select.dispatchEvent(new Event("change", { bubbles: true }));
    expect(commentsInPopup()).toBe(true); expect(write).toHaveBeenCalledExactlyOnceWith("logseq-claude.comments-in-popup", "1");
    expect(remove).not.toHaveBeenCalled();
    select.value = "margin"; select.dispatchEvent(new Event("change", { bubbles: true }));
    expect(commentsInPopup()).toBe(false); expect(remove).toHaveBeenCalledExactlyOnceWith("logseq-claude.comments-in-popup");
    expect(write).toHaveBeenCalledTimes(1);
    expect(open).not.toHaveBeenCalled(); expect(submit).not.toHaveBeenCalled();
  } finally { dispose(); }
});
it.each([false, true])("a refused display write keeps In a popup=%s in Settings and reports the failure", onClick => {
  changeCommentsInPopup(onClick);
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <AnnotationVisibilitySettings />, root);
  try {
    vi.spyOn(Storage.prototype, onClick ? "removeItem" : "setItem").mockImplementation(() => { throw new Error("quota"); });
    const select = root.querySelector<HTMLSelectElement>('[data-setting-id="settings-comments-display"] select')!;
    select.value = onClick ? "margin" : "popup"; select.dispatchEvent(new Event("change", { bubbles: true }));
    expect(commentsInPopup()).toBe(onClick); expect(select.value).toBe(onClick ? "popup" : "margin");
    expect(toasts().some(toast => toast.kind === "error")).toBe(true);
  } finally { dispose(); }
});
