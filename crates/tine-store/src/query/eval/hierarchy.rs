//! Page-local structural relations (GH #551, I-12/I-25).
//!
//! Build one preorder parent table lazily, O(N) nodes/edges. Each distinct
//! relation leaf evaluates its predicate on N fresh block rows once, then
//! folds N-1 edges: O(N * predicate cost), O(N) retained booleans per leaf.
//! Ancestors/descendants exclude the anchor; no depth cap or result truncation.
//! Fresh-row path refs come from the existing linear path-ref traversal.
//! Nested relations reuse their own folds, so unbounded outlines never induce
//! a repeated subtree traversal or recursive evaluation along outline depth.
//! Tables die with the page's EvalCtx; edits cannot retain stale hierarchy.

use super::{
    eval_block, uses_path_refs, walk_path_refs, DocBlock, EvalCtx, Filter, PathRefCounts, Quant,
    Rel,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

#[cfg(test)]
thread_local! {
    static WORK: std::cell::Cell<(usize, usize, usize)> = const { std::cell::Cell::new((0, 0, 0)) };
}

#[cfg(test)]
fn charge(nodes: usize, predicates: usize, edges: usize) {
    WORK.with(|work| {
        let (n, p, e) = work.get();
        work.set((n + nodes, p + predicates, e + edges));
    });
}

type FoldKey = (usize, u8, u8);

pub(super) struct Hierarchy<'a> {
    blocks: Vec<&'a DocBlock>,
    parents: Vec<Option<usize>>,
    positions: HashMap<usize, usize>,
    folds: RefCell<HashMap<FoldKey, Rc<Vec<bool>>>>,
}

impl<'a> Hierarchy<'a> {
    pub(super) fn new(roots: &'a [DocBlock]) -> Self {
        let mut table = Self {
            blocks: Vec::new(),
            parents: Vec::new(),
            positions: HashMap::new(),
            folds: RefCell::new(HashMap::new()),
        };
        let mut pending: Vec<_> = roots.iter().rev().map(|block| (block, None)).collect();
        while let Some((block, parent)) = pending.pop() {
            let at = table.blocks.len();
            table
                .positions
                .insert(block as *const DocBlock as usize, at);
            table.blocks.push(block);
            table.parents.push(parent);
            pending.extend(block.children.iter().rev().map(|child| (child, Some(at))));
        }
        #[cfg(test)]
        charge(table.blocks.len(), 0, 0);
        table
    }

    pub(super) fn evaluate(
        &self,
        rel: Rel,
        quant: Quant,
        pred: &Filter,
        block: &DocBlock,
        ctx: &EvalCtx,
    ) -> bool {
        let Some(&at) = self.positions.get(&(block as *const DocBlock as usize)) else {
            // The synthetic page-property block is not part of the outline.
            return quant != Quant::Any;
        };
        let key = (pred as *const Filter as usize, rel as u8, quant as u8);
        let cached = self.folds.borrow().get(&key).cloned();
        let values = cached.unwrap_or_else(|| {
            let values = Rc::new(self.fold(rel, quant, pred, ctx));
            self.folds.borrow_mut().insert(key, Rc::clone(&values));
            values
        });
        values[at]
    }

    fn fold(&self, rel: Rel, quant: Quant, pred: &Filter, ctx: &EvalCtx) -> Vec<bool> {
        let mut hits = Vec::with_capacity(self.blocks.len());
        walk_path_refs(
            ctx.page_roots,
            uses_path_refs(pred),
            &mut |block: &DocBlock, refs: &PathRefCounts| {
                #[cfg(test)]
                charge(0, 1, 0);
                hits.push(eval_block(pred, block, refs, ctx));
            },
        );
        let empty = quant != Quant::Any;
        let element = |hit: bool| if quant == Quant::None { !hit } else { hit };
        let combine = |a: bool, b: bool| if quant == Quant::Any { a || b } else { a && b };
        let mut values = vec![empty; hits.len()];
        match rel {
            Rel::Parent | Rel::Ancestors => {
                for at in 0..hits.len() {
                    if let Some(parent) = self.parents[at] {
                        #[cfg(test)]
                        charge(0, 0, 1);
                        values[at] = element(hits[parent]);
                        if rel == Rel::Ancestors {
                            values[at] = combine(values[at], values[parent]);
                        }
                    }
                }
            }
            Rel::Children | Rel::Descendants => {
                for at in (0..hits.len()).rev() {
                    if let Some(parent) = self.parents[at] {
                        #[cfg(test)]
                        charge(0, 0, 1);
                        let mut value = element(hits[at]);
                        if rel == Rel::Descendants {
                            value = combine(value, values[at]);
                        }
                        values[parent] = combine(values[parent], value);
                    }
                }
            }
            _ => unreachable!("only structural block relations use a hierarchy fold"),
        }
        values
    }
}

#[cfg(test)]
#[path = "../../../tests/support/hierarchy_cost.rs"]
mod tests;
