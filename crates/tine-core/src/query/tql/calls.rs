//! The shape of a TQL function call on the SQL AST: its name, its
//! positional arguments, and the SQL call forms the query language refuses.

use sqlparser::ast::{Expr, FunctionArg, FunctionArgExpr, FunctionArguments};

pub(super) fn function_name(function: &sqlparser::ast::Function) -> String {
    function
        .name
        .0
        .iter()
        .filter_map(|part| part.as_ident())
        .map(|ident| ident.value.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join(".")
}

/// The SQL function modifiers the query language has no meaning for:
/// `FILTER (WHERE …)`, `OVER …`, `WITHIN GROUP (…)`, `IGNORE/RESPECT NULLS`,
/// a second parameter list, the ODBC `{fn …}` form, `DISTINCT`/`ALL` and the
/// in-list clauses (`ORDER BY`, `LIMIT`, …), and a subquery argument. The
/// SQL grammar accepts them on any call; each is refused as a syntax error,
/// never dropped, so a query never runs as something other than it reads
/// (review r1).
pub(super) fn function_modifier(function: &sqlparser::ast::Function, name: &str) -> Option<String> {
    let modifier = if function.filter.is_some() {
        "FILTER"
    } else if function.over.is_some() {
        "OVER"
    } else if !function.within_group.is_empty() {
        "WITHIN GROUP"
    } else if function.null_treatment.is_some() {
        "IGNORE/RESPECT NULLS"
    } else if !matches!(function.parameters, FunctionArguments::None) {
        "a second argument list"
    } else if function.uses_odbc_syntax {
        "the {fn …} form"
    } else {
        match &function.args {
            FunctionArguments::Subquery(_) => "a subquery",
            FunctionArguments::List(list) if list.duplicate_treatment.is_some() => {
                "DISTINCT or ALL"
            }
            FunctionArguments::List(list) if !list.clauses.is_empty() => {
                "an ORDER BY, LIMIT or other clause inside its arguments"
            }
            _ => return None,
        }
    };
    Some(format!(
        "`{name}` does not take {modifier}: the query language has no SQL function modifiers"
    ))
}

/// A named (`k => v`) or wildcard argument, which no function of the query
/// language takes (`near` refuses its own with the reserved-argument
/// message).
pub(super) fn non_positional_argument(
    function: &sqlparser::ast::Function,
    name: &str,
) -> Option<String> {
    let FunctionArguments::List(list) = &function.args else {
        return None;
    };
    list.args
        .iter()
        .any(|arg| !matches!(arg, FunctionArg::Unnamed(FunctionArgExpr::Expr(_))))
        .then(|| format!("`{name}` takes positional arguments only"))
}

pub(super) fn function_args(function: &sqlparser::ast::Function) -> Vec<&Expr> {
    let FunctionArguments::List(list) = &function.args else {
        return Vec::new();
    };
    list.args
        .iter()
        .filter_map(|arg| match arg {
            FunctionArg::Unnamed(FunctionArgExpr::Expr(expr)) => Some(expr),
            _ => None,
        })
        .collect()
}
