// Semantic extension of the #868 outline journey: real source and embed, guarded
// autosave bytes, guide and arrow folds, Enter, and indent/outdent visibility.
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";

export async function embedFoldJourney({ browser, scope, expected, sourceOutline, capture, clickContent, pause, run }) {
  const candidates = expected.page.blocks.flatMap(block => block.children
    .filter(child => child.children.length && !child.raw.includes("{{"))
    .map(child => ({ guide: block, branch: child })));
  assert(candidates.length, "fixture has a source root with a foldable child branch");
  const { guide, branch } = candidates[0];
  const rowSelector = (container, id) => `${container} [data-block-id="${id}"]`;
  const guideSelector = `${rowSelector(scope, guide.id)} > .block-children-container > .block-children-left-border`;
  const arrowSelector = (container, id) => `${rowSelector(container, id)} > .block-main .collapse-toggle`;
  const sourceView = process.env.E2E_SOURCE_VIEW || "sidebar";
  const actionSelector = `${scope} > .embed-actions .embed-action-source`;
  const action = await browser.$(actionSelector);
  // Focus keeps the toolbar exposed while its button is above a tall embed's
  // top edge; scrolling the button alone can otherwise move off hover first.
  await browser.execute(selector => {
    const button = document.querySelector(selector);
    button.scrollIntoView({ block: "center" }); button.focus();
  }, actionSelector);
  await action.waitForDisplayed({ timeout: 10000 });
  if (sourceView !== "main") await browser.performActions([{ type: "key", id: "keyboard", actions: [{ type: "keyDown", value: sourceView === "split" ? "\uE00A" : "\uE008" }] }]);
  await action.click(); await browser.releaseActions();
  let peerKey;
  await browser.waitUntil(async () => !!(peerKey = await browser.execute(args => {
    if (args.pane) return [...document.querySelectorAll("[data-pane-id]")]
      .find(el => el.querySelector("h1.page-title")?.textContent.trim() === args.title)?.dataset.paneId;
    return [...document.querySelectorAll(".right-sidebar .rs-item")]
      .find(el => el.querySelector(".rs-item-title")?.textContent.trim() === args.name)?.dataset.sidebarSurface;
  }, { pane: sourceView !== "sidebar", name: expected.page.name, title: expected.page.title })), { timeout: 20000 });
  const peer = sourceView !== "sidebar" ? `[data-pane-id=${JSON.stringify(peerKey)}]`
    : `.right-sidebar [data-sidebar-surface=${JSON.stringify(peerKey)}]`;
  await browser.$(rowSelector(peer, guide.id)).waitForExist({ timeout: 20000 });
  const folded = async (container, id) => browser.execute(selector => document.querySelector(selector)?.classList.contains("collapsed"), rowSelector(container, id));
  // The supplied corpus starts its deep branches folded. Open the selected
  // parent and child from the source first so both views start from the same
  // expanded source state before reproducing the indent-guide click.
  for (const id of [guide.id, branch.id]) {
    await browser.$(rowSelector(peer, id)).waitForExist({ timeout: 10000 });
    if (await folded(peer, id)) {
      await browser.$(arrowSelector(peer, id)).click(); await pause();
    }
  }
  await browser.$(guideSelector).waitForExist({ timeout: 10000 });
  await capture("09-source-and-embed-prepared");
  const failures = [];
  const check = (condition, message) => { if (!condition) failures.push(message); };
  // Check the target's own property lines, stopping before its first child or
  // next sibling. Raw text is read from the disposable fixture, never committed.
  const sourceFoldProperty = () => {
    const lines = fs.readFileSync(expected.file, "utf8").split("\n");
    const title = branch.raw.split("\n")[0].trim();
    const index = lines.findIndex(line => line.trim() === `- ${title}`);
    assert(index >= 0, "branch bullet found in saved Markdown source");
    const properties = [];
    for (let i = index + 1; i < lines.length && !/^\s*- /.test(lines[i]); i++) properties.push(lines[i].trim());
    return properties.includes("collapsed:: true");
  };
  await browser.waitUntil(() => !sourceFoldProperty(), { timeout: 10000, interval: 100 });
  const fileBefore = fs.readFileSync(expected.file, "utf8");
  fs.writeFileSync(path.join(run, "source-before-guide.md"), fileBefore);
  // Any folded descendant makes the first guide click expand all. Start the
  // collapse reproduction with a completely open subtree, as on the source.
  if ((await browser.$(guideSelector).getAttribute("aria-expanded")) === "false") {
    await browser.$(guideSelector).click(); await pause();
  }
  await browser.$(guideSelector).click(); await pause();
  await capture("10-guide-folded-two-views");
  check(await folded(scope, branch.id), "guide folded the branch in the embed");
  check(await folded(peer, branch.id), "source view must fold with the embed guide");
  try { await browser.waitUntil(sourceFoldProperty, { timeout: 4000, interval: 100 }); }
  catch { check(false, "guide fold must save collapsed:: true on the source branch"); }
  fs.writeFileSync(path.join(run, "source-after-guide.md"), fs.readFileSync(expected.file));

  await clickContent(branch.id);
  await browser.keys(["Control", "End"]); await browser.keys("Enter"); await pause();
  const added = await browser.execute(selector => {
    const editor = document.activeElement;
    if (!(editor instanceof HTMLTextAreaElement) || !editor.matches("textarea.block-editor") || !document.querySelector(selector)?.contains(editor)) return null;
    return editor.closest(".ls-block")?.dataset.blockId;
  }, scope);
  check(!!added && added !== branch.id, "Enter must leave a visible new block editor inside the embed");
  const entered = await capture("11-enter-after-folded-branch");
  if (added) check(entered.find(row => row.id === added)?.depth === entered.find(row => row.id === branch.id)?.depth,
    "Enter at the end of a folded branch inserts a sibling at the same visible depth");
  if (added && added !== branch.id) {
    const sentinel = "GH868 visible sibling";
    await browser.keys(sentinel); await pause();
    await browser.keys("Tab"); await pause();
    const indented = await capture("12-indented-new-block");
    check(indented.some(row => row.id === added && row.text === sentinel), "indenting keeps the new block visible in its embed");
    await browser.keys(["Shift", "Tab"]); await pause();
    const outdented = await capture("13-outdented-new-block");
    const branchRow = outdented.find(row => row.id === branch.id);
    check(outdented.some(row => row.id === added && row.depth === branchRow?.depth), "outdent returns the new block to a visible sibling of the folded branch");
    await browser.keys("Escape"); await pause();
    await browser.waitUntil(() => fs.readFileSync(expected.file, "utf8").includes(sentinel), { timeout: 10000, interval: 100 });
    const saved = await sourceOutline();
    let parent;
    const findParent = (block, parentId) => {
      if (block.id === added) parent = parentId;
      block.children.forEach(child => findParent(child, block.id));
    };
    saved.page.blocks.forEach(block => findParent(block, null));
    check(parent === guide.id, "Enter/new-block placement agrees with the source page's folded-branch sibling rule");
    // Arrow folding shares the same source state; expanding from the source
    // exercises the reverse direction as well.
    if (!(await folded(scope, branch.id))) {
      await browser.$(arrowSelector(scope, branch.id)).click(); await pause();
    }
    await browser.waitUntil(sourceFoldProperty, { timeout: 10000, interval: 100 });
    check(await folded(peer, branch.id), "source view follows the embedded bullet arrow");
    await browser.$(arrowSelector(peer, branch.id)).click(); await pause();
    await browser.waitUntil(() => !sourceFoldProperty(), { timeout: 10000, interval: 100 });
    check(!(await folded(scope, branch.id)), "embed follows source arrow expansion");
    await capture("14-source-arrow-expanded");
    fs.writeFileSync(path.join(run, "source-after-insertion.md"), fs.readFileSync(expected.file));
  }
  fs.writeFileSync(path.join(run, "fold-evidence.json"), JSON.stringify({ sourceView, guide: guide.id, branch: branch.id, added,
    sourceBytesChanged: fileBefore !== fs.readFileSync(expected.file, "utf8"), failures }, null, 2));
  assert.deepEqual(failures, [], "source folds and new-block visibility must agree in both views");
}
