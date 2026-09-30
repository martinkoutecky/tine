//! SQLite's infix hook keeps flat boolean chains n-ary during construction.
//! All tokens, precedence and leaf syntax still belong to sqlparser. The private
//! operator tags cannot be emitted by its SQLite tokenizer. Parentheses remain
//! Nested nodes, so explicit groups survive lowering. O(tokens), bounded depth.
use sqlparser::{
    ast::{BinaryOperator, Expr, Visit, Visitor},
    dialect::{Dialect, SQLiteDialect},
    keywords::Keyword,
    parser::{Parser, ParserError},
    tokenizer::{Token, TokenWithSpan},
};
use std::{any::TypeId, ops::ControlFlow};

pub(super) const DEPTH_MAX: usize = 64;
const AND: &str = "\0tine.and";
const OR: &str = "\0tine.or";
const REFUSAL: &str = "the query expression is nested too deeply (limit: 64)";

#[derive(Debug)]
pub(super) struct BooleanDialect;
impl Dialect for BooleanDialect {
    fn dialect(&self) -> TypeId {
        TypeId::of::<SQLiteDialect>()
    }
    fn is_identifier_start(&self, ch: char) -> bool {
        SQLiteDialect {}.is_identifier_start(ch)
    }
    fn is_identifier_part(&self, ch: char) -> bool {
        SQLiteDialect {}.is_identifier_part(ch)
    }
    fn is_delimited_identifier_start(&self, ch: char) -> bool {
        SQLiteDialect {}.is_delimited_identifier_start(ch)
    }
    fn identifier_quote_style(&self, name: &str) -> Option<char> {
        SQLiteDialect {}.identifier_quote_style(name)
    }
    fn supports_filter_during_aggregation(&self) -> bool {
        true
    }
    fn supports_start_transaction_modifier(&self) -> bool {
        true
    }
    fn supports_in_empty_list(&self) -> bool {
        true
    }
    fn supports_limit_comma(&self) -> bool {
        true
    }
    fn supports_asc_desc_in_column_definition(&self) -> bool {
        true
    }
    fn supports_dollar_placeholder(&self) -> bool {
        true
    }
    fn supports_notnull_operator(&self) -> bool {
        true
    }
    fn supports_comma_separated_trim(&self) -> bool {
        true
    }

    fn parse_infix(
        &self,
        parser: &mut Parser,
        left: &Expr,
        precedence: u8,
    ) -> Option<Result<Expr, ParserError>> {
        let (keyword, tag) = match &parser.peek_token().token {
            Token::Word(word) if word.keyword == Keyword::AND => (Keyword::AND, AND),
            Token::Word(word) if word.keyword == Keyword::OR => (Keyword::OR, OR),
            _ => return SQLiteDialect {}.parse_infix(parser, left, precedence),
        };
        Some((|| {
            let mut items = vec![left.clone()];
            while parser.parse_keyword(keyword) {
                items.push(parser.parse_subexpr(precedence)?);
            }
            Ok(Expr::BinaryOp {
                left: Box::new(Expr::Tuple(items)),
                op: BinaryOperator::Custom(tag.into()),
                right: Box::new(Expr::Tuple(Vec::new())),
            })
        })())
    }
}

pub(super) fn items(expr: &Expr) -> Option<(bool, &[Expr])> {
    let Expr::BinaryOp {
        left,
        op: BinaryOperator::Custom(tag),
        ..
    } = expr
    else {
        return None;
    };
    let Expr::Tuple(items) = left.as_ref() else {
        return None;
    };
    match tag.as_str() {
        AND => Some((true, items)),
        OR => Some((false, items)),
        _ => None,
    }
}

/// Bound nonboolean construction paths before a recursive AST can exist.
/// Sibling comma lists and flat boolean chains do not consume nesting budget.
/// Literal/comment bodies are already opaque sqlparser tokens.
pub(super) fn admit_tokens(tokens: &[TokenWithSpan]) -> Result<(), String> {
    let mut parents = Vec::new();
    let (mut prefix, mut nodes) = (0usize, 0usize);
    for token in tokens {
        match &token.token {
            Token::Whitespace(_) | Token::EOF => continue,
            Token::Comma => nodes = 0,
            Token::Word(word) if matches!(word.keyword, Keyword::AND | Keyword::OR) => nodes = 0,
            Token::LParen | Token::LBracket | Token::LBrace => {
                nodes += 1;
                parents.push((prefix, nodes));
                prefix += nodes;
                nodes = 0;
            }
            Token::RParen | Token::RBracket | Token::RBrace => {
                if let Some(parent) = parents.pop() {
                    (prefix, nodes) = parent;
                }
            }
            _ => nodes += 1,
        }
        if prefix + nodes > DEPTH_MAX * 8 {
            return Err(REFUSAL.into());
        }
    }
    Ok(())
}

pub(super) fn admit_ast(expr: &Expr) -> Result<(), String> {
    struct Depth(usize);
    impl Visitor for Depth {
        type Break = ();
        fn pre_visit_expr(&mut self, _: &Expr) -> ControlFlow<()> {
            self.0 += 1;
            if self.0 > DEPTH_MAX {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        }
        fn post_visit_expr(&mut self, _: &Expr) -> ControlFlow<()> {
            self.0 -= 1;
            ControlFlow::Continue(())
        }
    }
    match expr.visit(&mut Depth(0)) {
        ControlFlow::Continue(()) => Ok(()),
        ControlFlow::Break(()) => Err(REFUSAL.into()),
    }
}
