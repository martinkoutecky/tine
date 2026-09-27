/** Edit intent sent with a page write. Mirrors tine-store::EditKind. */
export type EditKind =
  | "insert-blocks" | "save-block" | "move-blocks" | "delete-blocks"
  | "create-page" | "delete-page" | "rename-page" | "replace-page";

export type EditKinds = readonly [EditKind, ...EditKind[]];

export const EDIT_KIND_VALUES = [
  "insert-blocks", "save-block", "move-blocks", "delete-blocks",
  "create-page", "delete-page", "rename-page", "replace-page",
] as const satisfies readonly EditKind[];

type Assert<T extends true> = T;
export type EditKindCoverage = Assert<Exclude<EditKind, typeof EDIT_KIND_VALUES[number]> extends never ? true : false>;
