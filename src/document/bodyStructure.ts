// DESIGN §1 / GH #891: editor behavior, with no change to the stored outline.
// Keep this boundary below both editors, touch controls, drag and outline paste.
import { doc, pageByName } from "./model";
import { bodyScopeOf, threadRootOf } from "./commentThreads";
import type { OutlineChange, OutlineVerb } from "./kernel";

/** The candidate parent for Tab, omitting annotations between siblings. */
export function previousBodySibling(id: string): string | null {
  const n = doc.byId[id];
  if (!n) return null;
  const siblings = n.parent === null ? pageByName(n.page)?.roots ?? [] : doc.byId[n.parent].children;
  let at = siblings.indexOf(id) - 1;
  while (at >= 0 && bodyScopeOf(siblings[at]) !== bodyScopeOf(id)) at--;
  return at >= 0 ? siblings[at] : null;
}

/** Refuse body gestures inside annotations, or supply the kernel with the
 * body-only decision. Explicit comment insertion/deletion use their existing
 * verbs and still pass through the same undo and guarded save path. */
export function bodyStructureVerb(verb: OutlineVerb): OutlineVerb | null {
  switch (verb.verb) {
    case "split": {
      const owner = bodyScopeOf(verb.key);
      if (owner === undefined) return null;
      const n = doc.byId[verb.key];
      return { ...verb, children_visible: verb.children_visible && !!n?.children.some(id => bodyScopeOf(id) === owner) };
    }
    case "merge":
      return bodyScopeOf(verb.survivor) !== undefined && bodyScopeOf(verb.survivor) === bodyScopeOf(verb.absorbed) ? verb : null;
    case "indent": {
      if (verb.keys.some(id => bodyScopeOf(id) === undefined || bodyScopeOf(id) !== bodyScopeOf(verb.keys[0]))) return null;
      const n = doc.byId[verb.keys[0]];
      if (!n) return verb;
      const siblings = n.parent === null ? pageByName(n.page)?.roots ?? [] : doc.byId[n.parent].children;
      const immediate = siblings[siblings.indexOf(verb.keys[0]) - 1];
      const parent = previousBodySibling(verb.keys[0]);
      if (parent === immediate) return verb;
      if (!parent) return null;
      return { verb: "move_to", keys: verb.keys, to: { at: "last_child", key: parent } };
    }
    case "outdent": {
      const owner = bodyScopeOf(verb.keys[0]);
      if (verb.keys.some(id => bodyScopeOf(id) === undefined || bodyScopeOf(id) !== owner) || doc.byId[verb.keys[0]]?.parent === owner) return null;
      const parent = doc.byId[verb.keys[0]]?.parent;
      const retain = parent ? doc.byId[parent].children.filter(id => bodyScopeOf(id) !== owner) : [];
      return retain.length ? { ...verb, retain } : verb;
    }
    case "move_to": {
      const owner = bodyScopeOf(verb.keys[0]);
      if (owner) {
        const target = verb.to.at === "slot" ? verb.to.parent : verb.to.key;
        return verb.keys.every(id => bodyScopeOf(id) === owner) && target && (bodyScopeOf(target) === owner || target === owner && (verb.to.at === "last_child" || verb.to.at === "slot")) ? verb : null;
      }
      // A selected descendant travels with its selected body ancestor, but a
      // comment captured on its own is never a drag source.
      const roots = new Set(verb.keys);
      if (verb.keys.some(id => {
        if (threadRootOf(id) === null) return false;
        for (let p = doc.byId[id]?.parent; p; p = doc.byId[p]?.parent) if (roots.has(p) && threadRootOf(p) === null) return false;
        return true;
      })) return null;
      const target = verb.to.at === "slot" ? verb.to.parent : verb.to.key;
      if (target && threadRootOf(target) !== null) return null;
      if (verb.inherit_from && threadRootOf(verb.inherit_from) !== null) return null;
      return verb;
    }
    case "move_up_down":
      return verb.keys.some(id => bodyScopeOf(id) === undefined || bodyScopeOf(id) !== bodyScopeOf(verb.keys[0])) ? null : verb;
    case "insert_outline": {
      const target = verb.place.at === "slot" ? verb.place.parent : verb.place.key;
      return target && bodyScopeOf(target) === undefined ? null : verb;
    }
    default:
      return verb;
  }
}

/** Validate the kernel's move-step destination instead of reimplementing its
 * sibling/parent-edge rule. A body nudge cannot enter an annotation either. */
export function bodyStructureChangeAllowed(verb: OutlineVerb, change: OutlineChange): boolean {
  if (verb.verb !== "move_up_down") return true;
  const owner = bodyScopeOf(verb.keys[0]);
  if (owner) return change.pages.every(page => page.upserts.every(block => block.children.every(id => !verb.keys.includes(id) || block.key === owner || bodyScopeOf(block.key) === owner)));
  return change.pages.every(page => page.upserts.every(block => threadRootOf(block.key) === null
    || block.children.every(id => doc.byId[id]?.parent === block.key || threadRootOf(id) !== null)));
}
