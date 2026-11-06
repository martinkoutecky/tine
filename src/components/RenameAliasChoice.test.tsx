import { afterEach, expect, it } from "vitest";
import { render } from "solid-js/web";
import { RenameAliasChoice } from "./RenameAliasChoice";
import { aliasRenameRequest, chooseAliasRename } from "../renameAliasChoice";

afterEach(() => {
  aliasRenameRequest()?.finish(null);
  document.body.innerHTML = "";
});

it("shows the Rust offer's merge-only choices for a file-less source", async () => {
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <RenameAliasChoice />, root);
  try {
    const pending = chooseAliasRename("B", "ML", "Machine Learning", true, false);
    expect(root.querySelector('[data-choice="own-page"]')).toBeNull();
    expect(root.querySelector('[data-choice="merge"]')?.textContent).toContain("Machine Learning");
    expect(root.textContent).toContain("has no file");
    // A stale or forged submission cannot select an unoffered choice.
    aliasRenameRequest()!.finish("own-page");
    expect(aliasRenameRequest()).not.toBeNull();
    (root.querySelector('[data-choice="cancel"]') as HTMLButtonElement).click();
    expect(await pending).toBeNull();
  } finally { dispose(); }
});

it("keeps both answers for a file source, including an empty alias file", async () => {
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <RenameAliasChoice />, root);
  try {
    const pending = chooseAliasRename("ML notes", "ML", "Machine Learning", true, true);
    expect(root.querySelector('[data-choice="merge"]')).not.toBeNull();
    (root.querySelector('[data-choice="own-page"]') as HTMLButtonElement).click();
    expect(await pending).toBe("own-page");
  } finally { dispose(); }
});
