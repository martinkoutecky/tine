/** The tag table and its availability toggle share one keyed query lifecycle. */
import { createEffect, createResource, type Accessor } from "solid-js";
import { backend } from "./backend";
import { graphOwner, latestOwner, readOwned } from "./owned";
import { referenceKey, referenceRevision } from "./referenceKeys";
import type { RefGroup } from "./types";
import { readOr } from "./resourceRead";

export function createTagTableResource(name: Accessor<string>, enabled: Accessor<boolean> = () => true) {
  const owners = {};
  const dependency = referenceRevision(() => referenceKey("tagTable", name()));
  const resource = createResource(
    () => enabled() ? `${name()}\0${dependency.revision()}` : null,
    async (): Promise<{ groups: RefGroup[]; error?: string } | undefined> => {
      const pageName = name();
      const owner = latestOwner(owners, "tag-table", graphOwner());
      const quoted = `"${pageName.replace(/\\/g, "\\\\").replace(/"/g, "\\\"")}"`;
      try {
        const reading = await readOwned(owner, backend().parseQuery(`(tag ${quoted})`, "macro_query"));
        if (reading.kind === "stale") return undefined;
        const result = await readOwned(owner, backend().queryRun(reading.value.query, reading.value.view));
        if (result.kind === "stale") return undefined;
        const answer = result.value;
        const diagnostic = (answer.diagnostics ?? []).find(item => !item.disabled);
        const groups = diagnostic || answer.anchor !== "block" ? [] : answer.groups;
        return diagnostic ? { groups, error: diagnostic.message } : { groups };
      } catch (error) {
        return { groups: [], error: error instanceof Error ? error.message : String(error) };
      }
    },
  );
  createEffect(() => {
    const answer = readOr(resource[0], undefined, "tag table");
    if (answer) dependency.remember(answer.groups);
  });
  return resource;
}
