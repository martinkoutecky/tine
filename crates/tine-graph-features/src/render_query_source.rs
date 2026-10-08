//! Restore query transport from authored bytes before lsdoc emits data-args.
//! The AST recognizes macros; the shared raw reader supplies their arguments.

use std::collections::HashMap;
use tine_core::lsdoc::ast::{Block, Inline, ListItem, Span};
use tine_core::query::macro_text::{query_macro_extents, MacroExtent};

pub(super) fn restore(blocks: &mut [Block], raw: &str) {
    if !tine_core::query::query_source_within_limit(raw) {
        return;
    }
    let extents: HashMap<_, _> = query_macro_extents(raw)
        .into_iter()
        .map(|extent| (extent.start, extent))
        .collect();
    if extents.is_empty() {
        return;
    }
    // parse_block prepends two bytes to raw.trim_start() in both formats.
    let lead = raw.len() - raw.trim_start().len();
    enum Node<'a> {
        Block(&'a mut Block),
        Item(&'a mut ListItem),
        Inlines(&'a mut [Inline]),
    }
    let mut todo: Vec<_> = blocks.iter_mut().map(Node::Block).collect();
    while let Some(node) = todo.pop() {
        match node {
            Node::Block(block) => match block {
                Block::Paragraph { inline, .. }
                | Block::Heading { inline, .. }
                | Block::Bullet { inline, .. }
                | Block::FootnoteDef { inline, .. } => todo.push(Node::Inlines(inline)),
                Block::Quote { children, .. } | Block::Custom { children, .. } => {
                    todo.extend(children.iter_mut().map(Node::Block));
                }
                Block::List { items, .. } => todo.extend(items.iter_mut().map(Node::Item)),
                Block::Table { header, rows, .. } => {
                    for row in header.iter_mut().chain(rows) {
                        todo.extend(row.iter_mut().map(|cell| Node::Inlines(cell)));
                    }
                }
                _ => {}
            },
            Node::Item(item) => {
                todo.push(Node::Inlines(&mut item.name));
                todo.extend(item.content.iter_mut().map(Node::Block));
                todo.extend(item.items.iter_mut().map(Node::Item));
            }
            Node::Inlines(inlines) => {
                restore_arguments(inlines, &extents, lead);
                for inline in inlines {
                    match inline {
                        Inline::Emphasis { children, .. }
                        | Inline::Subscript { children, .. }
                        | Inline::Superscript { children, .. }
                        | Inline::Tag { children, .. } => todo.push(Node::Inlines(children)),
                        Inline::Link { label, .. } => todo.push(Node::Inlines(label)),
                        _ => {}
                    }
                }
            }
        }
    }
}

fn restore_arguments(inlines: &mut [Inline], extents: &HashMap<usize, MacroExtent>, lead: usize) {
    let mut closing_brace = None;
    for inline in inlines {
        // lsdoc closes a map-bearing macro one byte early: its final `}` is
        // emitted as the next Plain node. Consume that byte with the argument.
        if let Inline::Plain {
            text,
            span: Some(Span(start, _)),
            ..
        } = inline
        {
            if closing_brace == Some(*start) && text.starts_with('}') {
                text.remove(0);
                *start += 1;
            }
        }
        closing_brace = None;
        let Inline::Macro {
            name,
            args,
            span: Some(Span(start, end)),
        } = inline
        else {
            continue;
        };
        let Some(raw_start) = start.checked_sub(2).and_then(|at| at.checked_add(lead)) else {
            continue;
        };
        let Some(extent) = extents
            .get(&raw_start)
            .filter(|extent| extent.name.eq_ignore_ascii_case(name))
        else {
            continue;
        };
        let raw_end = end.saturating_sub(2) + lead;
        // Repair the parser's known final-map-brace loss, or comma splitting
        // within an otherwise complete span. Do not consume unrelated markup
        // after a parser that stopped earlier inside a nested/quoted payload.
        if extent.end != raw_end && extent.end != raw_end + 1 {
            continue;
        }
        if extent.end == raw_end + 1 {
            closing_brace = Some(*end);
            *end += 1;
        }
        *args = vec![extent.argument.clone()];
    }
}
