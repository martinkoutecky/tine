// The builder's immutable tree edits (SPEC §7.4), split out of queryBuilder.ts
// to keep that file under the 1500-line ratchet. They need nothing from the
// vocabulary or phrase sections: every function here is over `Filter` alone.
// `queryBuilder.ts` re-exports them, so consumers import from one place.

import type { Filter } from "./queryIr";

// Immutable tree edits.

/** The child list of a boolean node, or `null` for a leaf/raw/true/false. */
export function filterChildren(filter: Filter): Filter[] | null {
  switch (filter.kind) {
    case "and":
    case "or":
      return filter.items;
    case "not":
    case "off":
      return [filter.inner];
    default:
      return null;
  }
}

function withChildren(filter: Filter, children: Filter[]): Filter {
  switch (filter.kind) {
    case "and":
      return { kind: "and", items: children };
    case "or":
      return { kind: "or", items: children };
    case "not":
      return children[0] ? { kind: "not", inner: children[0] } : { kind: "and", items: [] };
    case "off":
      return children[0] ? { kind: "off", inner: children[0] } : { kind: "and", items: [] };
    default:
      return filter;
  }
}

/** The root the bar edits: always an `and`/`or` node, so "add a filter here" has somewhere to add. */
export function builderRoot(filter: Filter): Filter {
  if (filter.kind === "and" || filter.kind === "or") return filter;
  if (filter.kind === "true") return { kind: "and", items: [] };
  return { kind: "and", items: [filter] };
}

function clone(filter: Filter): Filter {
  return structuredClone(filter);
}

/** Resolve `loc` to the node that CONTAINS the addressed child, plus the index within it. */
function locate(root: Filter, loc: number[]): { parent: Filter; children: Filter[]; idx: number } | null {
  if (loc.length === 0) return null;
  let node = root;
  for (let i = 0; i < loc.length - 1; i++) {
    const kids = filterChildren(node);
    if (!kids) return null;
    const next = kids[loc[i]];
    if (!next) return null;
    node = next;
  }
  const children = filterChildren(node);
  if (!children) return null;
  return { parent: node, children, idx: loc[loc.length - 1] };
}

/** Resolve `loc` to the node it addresses (`[]` = root). */
function nodeAt(root: Filter, loc: number[]): Filter | null {
  let node = root;
  for (const i of loc) {
    const kids = filterChildren(node);
    if (!kids) return null;
    const next = kids[i];
    if (!next) return null;
    node = next;
  }
  return node;
}

/** **Put `next` where `loc` points, whatever kind of node holds that place.**
*
*  `locate` above answers a LIST question — it hands back the child array a
*  splice needs — and `filterChildren` synthesizes a fresh one-element array for
*  the unary `not`/`off`. That is right for a splice (you cannot splice two
*  children into a `not`) and silently wrong for an assignment: writing into the
*  synthesized array wrote into a copy, so every edit addressed at a child of a
*  unary wrapper did nothing and STILL returned a new tree, which the sheet
*  saved. The three group actions that address the `and`/`or` inside its
*  wrapper — the all/any header, "None of" and "Ungroup" — were therefore dead
*  clicks that wrote the block and pushed an empty step onto the undo stack, on
*  every `none of` group and (since P6 let a group be switched off) on every
*  disabled one. Assignment goes through here instead. */
function assignAt(draft: Filter, loc: number[], next: Filter): boolean {
  if (loc.length === 0) return false;
  let node = draft;
  for (let i = 0; i < loc.length - 1; i++) {
    const kids = filterChildren(node);
    const child = kids?.[loc[i]];
    if (!child) return false;
    node = child;
  }
  const index = loc[loc.length - 1];
  if (node.kind === "and" || node.kind === "or") {
    if (!node.items[index]) return false;
    node.items[index] = next;
    return true;
  }
  if (node.kind === "not" || node.kind === "off") {
    if (index !== 0) return false;
    node.inner = next;
    return true;
  }
  return false;
}

/** Mutating a node the path does not address is a no-op that returns the input
*  unchanged, so a stale `loc` from a popover that outlived its tree cannot
*  corrupt the query. I-4: only addressed nodes change; authored empty groups
*  elsewhere carry meaning and must survive every operation. */
function edit(root: Filter, apply: (draft: Filter) => boolean): Filter {
  const draft = clone(root);
  return apply(draft) ? draft : root;
}

/** Append `filter` to the boolean node addressed by `opLoc` (`[]` = root). */
export function addChild(root: Filter, opLoc: number[], filter: Filter): Filter {
  return edit(root, (draft) => {
    const node = nodeAt(draft, opLoc);
    const children = node ? filterChildren(node) : null;
    if (!node || !children || node.kind === "not" || node.kind === "off") return false;
    children.push(filter);
    return true;
  });
}

export function removeAt(root: Filter, loc: number[]): Filter {
  if (loc.length === 0 || !nodeAt(root, loc)) return root;
  // Prune only ancestors emptied by THIS deletion. Never visit siblings.
  const remove = (node: Filter, depth: number): Filter | null => {
    const children = [...filterChildren(node)!];
    const index = loc[depth];
    const next = depth === loc.length - 1 ? null : remove(children[index], depth + 1);
    if (next) children[index] = next;
    else children.splice(index, 1);
    if (children.length === 0) return depth === 0 ? { kind: node.kind === "or" ? "or" : "and", items: [] } : null;
    return withChildren(node, children);
  };
  return remove(root, 0)!;
}

export function replaceAt(root: Filter, loc: number[], filter: Filter): Filter {
  return edit(root, (draft) => assignAt(draft, loc, filter));
}

/** Wrap the node at `loc` in a new boolean node. */
export function wrapAt(root: Filter, loc: number[], op: "and" | "or" | "not" | "off"): Filter {
  return edit(root, (draft) => {
    const current = nodeAt(draft, loc);
    if (!current) return false;
    return assignAt(
      draft,
      loc,
      op === "not"
        ? { kind: "not", inner: current }
        : op === "off"
          ? { kind: "off", inner: current }
          : { kind: op, items: [current] },
    );
  });
}

// Grouping, reordering and disabling (SPEC §7.4 remainder, P6)

/** The three group headers the sheet offers, in the sheet's own words. */
export type GroupChoice = "all" | "any" | "none";

const groupNode = (choice: GroupChoice, items: Filter[]): Filter =>
  choice === "any"
    ? { kind: "or", items }
    : choice === "none"
      ? { kind: "not", inner: { kind: "or", items } }
      : { kind: "and", items };

/** **Group the selected siblings into one group (§7.4, design §2.5).**
*
*  The ONE grouping operation: multi-select grouping, "group with the row
*  above" and any future gesture all come through here, so the answer to "which
*  rows end up where" is written once.
*
*  Three rules, and each of them is a way a selection can be quietly betrayed:
*
*   - **Selected items keep their original relative order.** Grouping is not a
*     sort, and the order of an `and`/`or` list is the order the author typed
*     (§3.5 keeps child order in the editable form).
*   - **The group is inserted at the FIRST selected position**, and the
*     unselected siblings keep their own order around it. A non-contiguous
*     selection follows the same rule rather than a second one: the group lands
*     where the topmost selected row was.
*   - **Anything that is not a set of siblings is REFUSED**, not repaired. Locs
*     from two different lists, a loc and its own descendant, a duplicate, an
*     index past the end, a stale path from a menu that outlived its tree —
*     each returns the tree unchanged, exactly as every other edit here does.
*     Sibling-ness is what makes ancestor/descendant selection impossible: two
*     locs with the same parent path can never nest.
*
*  Fewer than two locs is a refusal too: "group" of one row is a wrapper the
*  user did not ask for. */
export function groupSelected(root: Filter, locs: number[][], choice: GroupChoice): Filter {
  if (locs.length < 2) return root;
  const parent = locs[0].slice(0, -1);
  const sibling = (loc: number[]) =>
    loc.length === parent.length + 1 && parent.every((step, i) => loc[i] === step);
  if (!locs.every(sibling)) return root;
  const indices = [...new Set(locs.map((loc) => loc[loc.length - 1]))].sort((a, b) => a - b);
  if (indices.length !== locs.length) return root;
  return edit(root, (draft) => {
    const node = nodeAt(draft, parent);
    if (!node || (node.kind !== "and" && node.kind !== "or")) return false;
    const children = node.items;
    if (indices.some((index) => !Number.isInteger(index) || index < 0 || index >= children.length)) {
      return false;
    }
    const picked = indices.map((index) => children[index]);
    const chosen = new Set(indices);
    const kept = children.filter((_, index) => !chosen.has(index));
    // Where the group goes among what is LEFT: as many unselected siblings precede it as preceded the first …
    const before = children.slice(0, indices[0]).filter((_, index) => !chosen.has(index)).length;
    kept.splice(before, 0, groupNode(choice, picked));
    node.items = kept;
    return true;
  });
}

/** **"Group with the row above" (§7.4, design §2.5).**
*
*  The addressed row and the sibling immediately before it, in place. It is
*  {@link groupSelected} with the selection the menu implies rather than a
*  second implementation of the same question — which is why it offers the same
*  all/any/none choices and lands in the same place. The first row of a list has
*  nothing above it, so it is a no-op, as is a stale `loc`. */
export function groupWithPrevious(root: Filter, loc: number[], choice: GroupChoice = "all"): Filter {
  if (loc.length === 0) return root;
  const previous = [...loc.slice(0, -1), loc[loc.length - 1] - 1];
  return groupSelected(root, [previous, loc], choice);
}

/** **Move a row or group among its own siblings (§7.4, P6).**
*
*  `to` is the index the node ends up at in the SAME list — the drop position a
*  drag reports and the one step "move up"/"move down" ask for, which is why
*  keyboard and pointer produce the same tree.
*
*  **Atomic against the original tree.** Removing the node and inserting it
*  again are one edit over one draft, so the destination is computed against the
*  list the user was looking at. A remove-then-insert built out of `removeAt`
*  and `addChild` would normalize in between: `removeAt` PRUNES an `and`/`or`
*  its last child just left, so moving the only row out of a group would delete
*  the group and shift every path after it — including the one holding the
*  destination. Boundary destinations are refused rather than clamped: a clamp
*  turns "I pressed up on the first row" into a silent save of an unchanged
*  tree. */
export function moveSibling(root: Filter, loc: number[], to: number): Filter {
  return edit(root, (draft) => {
    const at = locate(draft, loc);
    if (!at) return false;
    const node = at.children[at.idx];
    if (!node) return false;
    if (!Number.isInteger(to) || to < 0 || to >= at.children.length || to === at.idx) return false;
    at.children.splice(at.idx, 1);
    at.children.splice(to, 0, node);
    return true;
  });
}

/** **Move a condition or group into ANOTHER list, or to another place in its own (GH #619 item 6).**
*
*  `from` is the node's loc (its outermost `not`/`off` wrapper, as the sheet
*  addresses it); `toParent` is the loc of the `and`/`or` whose list receives it;
*  `toIndex` is the slot in that list's ORIGINAL children, `0..length`, the way a
*  drop line reads ("before the row that is there now"). Everything is resolved to
*  object references on the one clone BEFORE anything moves, so removing the node
*  cannot shift the path the destination or the source group is found by.
*
*  Refused (tree handed back unchanged): a stale path, a destination that is not an
*  `and`/`or`, a destination inside the moved node (a group into itself or its
*  descendants), and a same-list move that lands where it already is.
*
*  The group the node LEFT is tidied, and only that one (I-4: authored empty groups
*  elsewhere carry meaning). Left with one child, it dissolves into its parent: the
*  child takes the group's place, inside the group's own `not`/`off` wrapper when it
*  has one (`none of [A, B]` minus B is `not A`, which is the same query). A wrapper
*  that would end up holding another `not`/`off` keeps its one-child group instead
*  of stacking wrappers the sheet draws as one row. Left with none, it is pruned the
*  way `removeAt` prunes. The root never dissolves or goes. Moving within one list
*  leaves every list's size alone, so nothing dissolves. */
export function moveAcross(root: Filter, from: number[], toParent: number[], toIndex: number): Filter {
  if (from.length === 0 || !Number.isInteger(toIndex)) return root;
  // A node cannot move into itself or anything below it.
  if (from.length <= toParent.length && from.every((part, i) => toParent[i] === part)) return root;
  return edit(root, (draft) => {
    const at = locate(draft, from);
    const dest = nodeAt(draft, toParent);
    if (!at || !dest || (dest.kind !== "and" && dest.kind !== "or")) return false;
    if (at.parent.kind !== "and" && at.parent.kind !== "or") return false;
    const node = at.children[at.idx];
    if (!node || toIndex < 0 || toIndex > dest.items.length) return false;

    const sourceList = at.children;
    const sameList = sourceList === dest.items;
    if (sameList && (toIndex === at.idx || toIndex === at.idx + 1)) return false;

    // The ancestry of the group being left, as references, before anything moves.
    const chain: Filter[] = [draft];
    for (const index of from.slice(0, -1)) chain.push(filterChildren(chain[chain.length - 1])![index]);

    sourceList.splice(at.idx, 1);
    dest.items.splice(sameList && at.idx < toIndex ? toIndex - 1 : toIndex, 0, node);
    if (sameList) return true;

    // The last link is the `and`/`or` that lost a child.
    const owner = chain[chain.length - 1] as Filter & { kind: "and" | "or" };
    const container = chain.length > 1 ? chain[chain.length - 2] : null;
    if (!container) return true; // the root never dissolves
    const only = owner.items.length === 1 ? owner.items[0] : null;
    if (only) {
      if (container.kind === "and" || container.kind === "or") {
        container.items[container.items.indexOf(owner)] = only;
      } else if ((container.kind === "not" || container.kind === "off") && only.kind !== "not" && only.kind !== "off") {
        container.inner = only;
      }
    } else if (owner.items.length === 0) {
      // Prune upward through whatever the removal empties, never past the root.
      for (let level = chain.length - 1; level >= 1; level--) {
        const gone = chain[level];
        const holder = chain[level - 1];
        if (holder.kind === "and" || holder.kind === "or") {
          holder.items.splice(holder.items.indexOf(gone), 1);
          if (holder.items.length > 0 || level === 1) break;
        }
        // A unary wrapper that lost its child goes too; keep climbing.
      }
    }
    return true;
  });
}

/** The path of the node's OWN `off` wrapper, relative to the node, or `null` when it has none. */
function ownOffPath(node: Filter): number[] | null {
  if (node.kind === "off") return [];
  if (node.kind === "not" && node.inner.kind === "off") return [0];
  return null;
}

/** Whether the node at `loc` carries its OWN `off` wrapper. */
export function isDisabledAt(root: Filter, loc: number[]): boolean {
  const node = nodeAt(root, loc);
  return !!node && ownOffPath(node) !== null;
}

/** **The enabled control: add or remove this node's own `Off` (§3.5, §7.4).**
*
*  Disabling WRAPS the addressed node whole, so the `not` that spells the row's
*  negative operator, the `raw` payload of a condition the parser could not
*  read, an opaque advanced subtree past the rendering cap, and any `off` a
*  DESCENDANT carries all travel inside it untouched. Enabling removes exactly
*  the one wrapper the row draws as its greyed state and nothing else, so a
*  disabled group full of individually disabled rows comes back as it went in.
*
*  Nothing is deleted, coerced or re-read: `Off` is structural omission, and a
*  re-enabled `Raw` is the same bytes with the same diagnostic it always had
*  (§4.3.2). */
export function toggleDisabledAt(root: Filter, loc: number[]): Filter {
  if (loc.length === 0) return root;
  const node = nodeAt(root, loc);
  if (!node) return root;
  const off = ownOffPath(node);
  if (off === null) return wrapAt(root, loc, "off");
  // Enabling REBUILDS the row's whole node rather than addressing the `off` inside it.
  const inner = off.length === 0
    ? (node as Filter & { kind: "off" }).inner
    : { kind: "not" as const, inner: ((node as Filter & { kind: "not" }).inner as Filter & { kind: "off" }).inner };
  return replaceAt(root, loc, clone(inner));
}

/** Replace the boolean node at `loc` with its children spliced into the parent. */
export function unwrapAt(root: Filter, loc: number[]): Filter {
  const current = nodeAt(root, loc);
  const kids = current ? filterChildren(current) : null;
  if (!current || !kids || kids.length === 0) return root;
  const parent = loc.length > 1 ? nodeAt(root, loc.slice(0, -1)) : root;
  if (parent && (parent.kind === "not" || parent.kind === "off")) {
    return kids.length === 1 ? replaceAt(root, loc, clone(kids[0])) : root;
  }
  return edit(root, (draft) => {
    const at = locate(draft, loc);
    if (!at || !at.children[at.idx]) return false;
    at.children.splice(at.idx, 1, ...(filterChildren(at.children[at.idx]) ?? []));
    return true;
  });
}

/** Change `and` ↔ `or` on the node addressed by `loc` (`[]` = root). */
export function setOp(root: Filter, loc: number[], op: "and" | "or"): Filter {
  if (loc.length === 0) {
    if (root.kind !== "and" && root.kind !== "or") return root;
    return { kind: op, items: structuredClone(filterChildren(root) ?? []) };
  }
  return edit(root, (draft) => {
    const current = nodeAt(draft, loc);
    if (!current || (current.kind !== "and" && current.kind !== "or")) return false;
    return assignAt(draft, loc, { kind: op, items: current.items });
  });
}
