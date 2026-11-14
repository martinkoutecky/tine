// Repeating outputs store only an explicit exclusion, on their ordinary
// definition block. Call only for an explicit choice; normal edits do no work.
import { blockProperty, blockWritable, isDirty, node, setBlockProperty, flushPage } from "./document";
import { bindingOwner, type WriteOwner } from "./owned";
import { t } from "./i18n";

export const COMMENTS_PROPERTY = "tine.comments";

export function stageExportComments(id: string, include: boolean, owner: WriteOwner = bindingOwner(), expectedRaw?: string): string | null {
  if (!owner()) return null;
  const source = node(id);
  if (!source) throw new Error(t("export-definition-cannot-edit"));
  const value = include ? null : "false";
  const page = source.page;
  const changed = blockProperty(id, COMMENTS_PROPERTY) !== value;
  if (!changed && !isDirty(page)) return null;
  if (changed && expectedRaw !== undefined && source.raw !== expectedRaw) throw new Error(t("export-definition-changed"));
  if (!blockWritable(id)) throw new Error(t("export-definition-cannot-edit"));
  if (changed) setBlockProperty(id, COMMENTS_PROPERTY, value);
  return page;
}

export async function saveExportComments(id: string, include: boolean, owner: WriteOwner = bindingOwner()): Promise<void> {
  const page = stageExportComments(id, include, owner);
  if (page && !(await flushPage(page))) throw new Error(t("export-definition-save-failed", { page }));
}
