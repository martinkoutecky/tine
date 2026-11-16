/** Shared reference dependencies. Registration precedes the initial unlinked
 * read; disposal is ordered after registration and pinned to the old binding. */
import { createMemo, createSignal, onCleanup, type Accessor } from "solid-js";
import { backend } from "./backend";
import { captureBinding } from "./binding";
import { bindingOwner, readOwned, type Owned, type Owner } from "./owned";
import type { Backend } from "./backendInterface";
import { graphEpoch } from "./graphSession";
import { changedAny } from "./changeKeys";
import { pageIdentityKey } from "./pageIdentity";
import type { RefGroup, BlockDto } from "./types";

export function registerUnlinkedReader(name: Accessor<string | undefined>): Accessor<Promise<void>> {
  return createMemo(() => {
    graphEpoch();
    const target = name();
    const host = backend();
    if (!target || !host.registerUnlinkedReference) return Promise.resolve();
    const generation = captureBinding().backendGeneration;
    const owner = bindingOwner();
    const id = crypto.randomUUID();
    const ready = readOwned(owner, host.registerUnlinkedReference(id, target, generation));
    onCleanup(() => { void releaseUnlinkedReader(host, id, generation, owner, ready); });
    return ready.then(result => { if (result.kind === "stale") return; });
  });
}

async function releaseUnlinkedReader(host: Backend, id: string, generation: number, owner: Owner, ready: Promise<Owned<void>>): Promise<void> {
  try {
    const registered = await ready;
    if (registered.kind === "stale") return;
    const released = await readOwned(owner, host.registerUnlinkedReference!(id, null, generation));
    if (released.kind === "stale") return;
  } catch {
    // Active registration failures reach the section's existing load error;
    // cleanup cannot affect membership or disk, but a live-binding failure is
    // diagnostic evidence of excess retained matching work.
    if (owner()) console.warn("reference subscription cleanup failed");
  }
}

/** Observe only answer members, including their displayed descendants. Updating
 * membership installs dependencies even if no link/mention membership changed. */
export function referenceRevision(key: Accessor<string>) {
  const [shown, setShown] = createSignal<string[]>([]);
  const remember = (groups: RefGroup[]) => {
    const dependencies = new Set<string>();
    const visit = (blocks: BlockDto[], page: string) => {
      for (const block of blocks) {
        dependencies.add(`block:${block.id}`);
        // Breadcrumb DTOs carry ancestor text but no ancestor identities.
        // Until that projection names them, retain a source-page dependency
        // for these displayed ancestors; never infer ids from Markdown text.
        if (block.breadcrumb?.length) dependencies.add(`page:${pageIdentityKey(page)}`);
        visit(block.children, page);
      }
    };
    for (const group of groups) visit(group.blocks, group.page);
    const ids = [...dependencies];
    setShown(previous => previous.length === ids.length && previous.every((id, i) => id === ids[i]) ? previous : ids);
  };
  // Newly shown zero-revision members do not require a second initial read.
  const revision = () => changedAny([key(), ...shown()]);
  return { remember, revision };
}

export const referenceKey = (kind: "links" | "text" | "tags" | "tagTable", name: string) => `${kind}:${pageIdentityKey(name)}`;
