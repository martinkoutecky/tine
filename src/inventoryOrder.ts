/** Ordered current inventory keys. Buckets live in pageIndex; this tree keeps
 * single-key insertion/removal logarithmic and whole-answer traversal linear. */
interface Branch { key: string; height: number; left?: Branch; right?: Branch }
const height = (node?: Branch): number => node?.height ?? 0;
function update(node: Branch): Branch {
  node.height = 1 + Math.max(height(node.left), height(node.right));
  return node;
}
function left(node: Branch): Branch {
  const next = node.right!;
  node.right = next.left; next.left = update(node);
  return update(next);
}
function right(node: Branch): Branch {
  const next = node.left!;
  node.left = next.right; next.right = update(node);
  return update(next);
}
function balance(node: Branch): Branch {
  update(node);
  if (height(node.left) - height(node.right) > 1) {
    if (height(node.left!.left) < height(node.left!.right)) node.left = left(node.left!);
    return right(node);
  }
  if (height(node.right) - height(node.left) > 1) {
    if (height(node.right!.right) < height(node.right!.left)) node.right = right(node.right!);
    return left(node);
  }
  return node;
}

export class InventoryOrder {
  private root?: Branch;
  constructor(keys: readonly string[], private compare: (a: string, b: string) => number) {
    // Backend keys already arrive in order. Also accept unsorted synthetic or
    // older-host inventories without changing their keyed answers.
    const sorted = keys.some((key, at) => at > 0 && compare(keys[at - 1], key) > 0)
      ? [...keys].sort(compare) : keys;
    const build = (lo: number, hi: number): Branch | undefined => {
      if (lo >= hi) return undefined;
      const middle = (lo + hi) >> 1;
      return update({ key: sorted[middle], height: 1, left: build(lo, middle), right: build(middle + 1, hi) });
    };
    this.root = build(0, sorted.length);
  }
  add(key: string): void {
    const insert = (node?: Branch): Branch => {
      if (!node) return { key, height: 1 };
      const order = this.compare(key, node.key);
      if (order < 0) node.left = insert(node.left);
      else if (order > 0) node.right = insert(node.right);
      return balance(node);
    };
    this.root = insert(this.root);
  }
  delete(key: string): void {
    const remove = (node: Branch | undefined, target: string): Branch | undefined => {
      if (!node) return undefined;
      const order = this.compare(target, node.key);
      if (order < 0) node.left = remove(node.left, target);
      else if (order > 0) node.right = remove(node.right, target);
      else {
        if (!node.left) return node.right;
        if (!node.right) return node.left;
        let next = node.right;
        while (next.left) next = next.left;
        node.key = next.key;
        node.right = remove(node.right, next.key);
      }
      return balance(node);
    };
    this.root = remove(this.root, key);
  }
  *keys(): Generator<string> {
    const stack: Branch[] = [];
    let node = this.root;
    while (node || stack.length) {
      while (node) { stack.push(node); node = node.left; }
      const current = stack.pop()!;
      yield current.key;
      node = current.right;
    }
  }
}
