//! The fixed vocabulary of TQL: which identifiers name which attribute on
//! each row (block, page, and a `direct_refs` reference element), the type
//! each compares as, and the §4.2.3 operator × type matrix. A whitelist by
//! construction: anything absent here is a diagnostic in the lowering.

use sqlparser::ast::{BinaryOperator, Expr, UnaryOperator, Value as SqlValue};

use super::Target;
use crate::query::ir::{Attr, CmpOp, ValueType};

pub(super) fn block_attr(name: &str) -> Option<(Attr, ValueType)> {
    Some(match name {
        "content" => (Attr::Content, ValueType::Text),
        "task" => (Attr::Task, ValueType::Text),
        "priority" => (Attr::Priority, ValueType::Text),
        "scheduled" => (Attr::Scheduled, ValueType::Date),
        "deadline" => (Attr::Deadline, ValueType::Date),
        "created_at" => (Attr::CreatedAt, ValueType::Date),
        "last_modified_at" => (Attr::LastModifiedAt, ValueType::Date),
        _ => return None,
    })
}

pub(super) fn page_attr(name: &str) -> Option<(Attr, ValueType)> {
    Some(match name {
        "name" => (Attr::Name, ValueType::Text),
        "journal" => (Attr::Journal, ValueType::Checkbox),
        "day" => (Attr::Day, ValueType::Date),
        "namespace" => (Attr::Namespace, ValueType::Text),
        "used_as_tag" => (Attr::UsedAsTag, ValueType::Checkbox),
        _ => return None,
    })
}

/// The fields of one `direct_refs` element ([`Scope::Ref`]).
pub(super) fn ref_attr(name: &str) -> Option<(Attr, ValueType)> {
    Some(match name {
        "name" => (Attr::Name, ValueType::Text),
        "journal" => (Attr::Journal, ValueType::Checkbox),
        "day" => (Attr::Day, ValueType::Date),
        _ => return None,
    })
}

pub(super) fn attr_label(attr: Attr) -> &'static str {
    match attr {
        Attr::Content => "content",
        Attr::Task => "task",
        Attr::Priority => "priority",
        Attr::Scheduled => "scheduled",
        Attr::Deadline => "deadline",
        Attr::CreatedAt => "created at",
        Attr::LastModifiedAt => "last modified at",
        Attr::Name => "name",
        Attr::Journal => "journal",
        Attr::Day => "day",
        Attr::Namespace => "namespace",
        Attr::UsedAsTag => "used as tag",
        Attr::Key => "key",
        Attr::Value => "value",
        Attr::AtomCount => "atom count",
    }
}

pub(super) fn op_label(op: CmpOp) -> &'static str {
    match op {
        CmpOp::Eq => "=",
        CmpOp::NotEq => "!=",
        CmpOp::Lt => "<",
        CmpOp::Le => "<=",
        CmpOp::Gt => ">",
        CmpOp::Ge => ">=",
        CmpOp::Between => "between",
        CmpOp::In => "in",
        CmpOp::NotIn => "not in",
        CmpOp::Like => "like",
        CmpOp::StartsWith => "like",
        CmpOp::Match => "match",
        CmpOp::Regex => "regexp",
        CmpOp::IsSet => "is not null",
        CmpOp::IsNotSet => "is null",
        CmpOp::IsBlank => "= ''",
    }
}

/// The comparison a SQL binary operator names. `sqlparser` has dozens of
/// operators the query language does not, so this match alone keeps a rest arm.
pub(super) fn binary_cmp(binary: &BinaryOperator) -> Option<CmpOp> {
    Some(match binary {
        BinaryOperator::Eq => CmpOp::Eq,
        BinaryOperator::NotEq => CmpOp::NotEq,
        BinaryOperator::Lt => CmpOp::Lt,
        BinaryOperator::LtEq => CmpOp::Le,
        BinaryOperator::Gt => CmpOp::Gt,
        BinaryOperator::GtEq => CmpOp::Ge,
        BinaryOperator::Match => CmpOp::Match,
        _ => return None,
    })
}

/// SPEC §4.2.3 operator × type matrix (K9). Anything absent here is a `Syntax`
/// diagnostic naming the operator and the type.
pub(super) fn op_applies(target: &Target, op: CmpOp, ty: ValueType) -> bool {
    let on_property = matches!(target, Target::Prop { .. } | Target::Atom);
    // A reference element answers only what `eval_direct_ref` evaluates: a
    // name by equality, `journal` by (in)equality, `day` by the date matrix
    // plus presence (a reference to a non-journal page has no day).
    if let Target::Ref { attr, .. } = target {
        return match attr {
            Attr::Name => op == CmpOp::Eq,
            Attr::Journal => matches!(op, CmpOp::Eq | CmpOp::NotEq),
            Attr::Day => matches!(
                op,
                CmpOp::Eq
                    | CmpOp::NotEq
                    | CmpOp::Lt
                    | CmpOp::Le
                    | CmpOp::Gt
                    | CmpOp::Ge
                    | CmpOp::Between
                    | CmpOp::IsSet
                    | CmpOp::IsNotSet
            ),
            _ => false,
        };
    }
    match op {
        CmpOp::Eq | CmpOp::NotEq => true,
        CmpOp::Lt | CmpOp::Le | CmpOp::Gt | CmpOp::Ge | CmpOp::Between => {
            matches!(ty, ValueType::Number | ValueType::Date)
        }
        CmpOp::In | CmpOp::NotIn => !matches!(ty, ValueType::Date | ValueType::Checkbox),
        CmpOp::Like | CmpOp::StartsWith => matches!(ty, ValueType::Text | ValueType::Ref),
        CmpOp::Match => matches!(
            target,
            Target::Attr {
                attr: Attr::Content,
                ..
            }
        ),
        // `regexp` is permitted only on `content` with a text pattern, and is
        // forbidden on every other row and type in the §4.2.3 matrix.
        CmpOp::Regex => {
            ty == ValueType::Text
                && matches!(
                    target,
                    Target::Attr {
                        attr: Attr::Content,
                        ..
                    }
                )
        }
        CmpOp::IsSet | CmpOp::IsNotSet => {
            on_property
                || matches!(
                    target,
                    Target::Attr {
                        attr: Attr::Task
                            | Attr::Priority
                            | Attr::Scheduled
                            | Attr::Deadline
                            | Attr::CreatedAt
                            | Attr::LastModifiedAt,
                        ..
                    }
                )
        }
        CmpOp::IsBlank => on_property,
    }
}

/// The type a literal spells. It decides how the IR stores the VALUE; the atom
/// it is compared against is typed by the registry at evaluation (§6.3).
pub(super) fn literal_type(expr: &Expr) -> ValueType {
    match expr {
        Expr::Nested(inner) => literal_type(inner),
        Expr::UnaryOp {
            op: UnaryOperator::Minus,
            expr,
        } => literal_type(expr),
        Expr::Identifier(ident) if ident.value.eq_ignore_ascii_case("today") => ValueType::Date,
        Expr::Value(value) => match &value.value {
            SqlValue::Number(text, _) => {
                if is_day_ordinal(text) {
                    ValueType::Date
                } else {
                    ValueType::Number
                }
            }
            SqlValue::Boolean(_) => ValueType::Checkbox,
            SqlValue::SingleQuotedString(text) | SqlValue::DoubleQuotedString(text) => {
                if is_date_literal(text) {
                    ValueType::Date
                } else {
                    ValueType::Text
                }
            }
            _ => ValueType::Text,
        },
        _ => ValueType::Text,
    }
}

pub(super) fn is_day_ordinal(text: &str) -> bool {
    text.len() == 8 && text.bytes().all(|byte| byte.is_ascii_digit())
}

/// `'2026-09-04'` or a SIGNED relative `'-7d'` / `'+2w'` / `'-1m'` / `'-1y'`:
/// the subset of the [`DateToken`](crate::query::DateToken) grammar that types
/// a quoted TQL literal as a date (Rule 3: one grammar, no local recognizer).
/// An out-of-range offset still types as a date, so it is diagnosed.
pub(super) fn is_date_literal(text: &str) -> bool {
    use crate::query::DateToken;
    if text != text.trim() {
        return false;
    }
    match DateToken::parse(text) {
        Some(DateToken::Relative { .. } | DateToken::OutOfRange) => {
            matches!(text.as_bytes().first(), Some(b'+' | b'-'))
        }
        Some(DateToken::Stem(_)) => text.len() == 10 && text.as_bytes()[4] == b'-',
        _ => false,
    }
}
