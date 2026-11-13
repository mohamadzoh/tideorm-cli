//! Syntax the checks read: method chains link by link, string literals,
//! and the variables a pattern binds.

use syn::Expr;
use syn::visit::{self, Visit};

use crate::analyze::line;

pub(in crate::analyze) enum Link<'a> {
    Call {
        name: String,
        args: Vec<&'a Expr>,
        line: usize,
    },
    Await,
    Try,
}

/// The expression a chain starts from, and its links in the order they run.
pub(in crate::analyze) fn flatten(mut expr: &Expr) -> (&Expr, Vec<Link<'_>>) {
    let mut links = Vec::new();
    loop {
        match expr {
            Expr::MethodCall(call) => {
                links.push(Link::Call {
                    name: call.method.to_string(),
                    args: call.args.iter().collect(),
                    line: line(call.method.span()),
                });
                expr = &call.receiver;
            }
            Expr::Await(awaited) => {
                links.push(Link::Await);
                expr = &awaited.base;
            }
            Expr::Try(tried) => {
                links.push(Link::Try);
                expr = &tried.expr;
            }
            Expr::Paren(paren) => expr = &paren.expr,
            _ => break,
        }
    }
    links.reverse();
    (expr, links)
}

pub(in crate::analyze) fn string_literal(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(text),
            ..
        }) => Some(text.value()),
        Expr::Reference(reference) => string_literal(&reference.expr),
        _ => None,
    }
}

pub(super) fn is_identifier(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The variable a pattern binds: `x`, `mut x`, `x: T`, `&x`, `Some(x)`.
pub(in crate::analyze) fn pattern_name(pattern: &syn::Pat) -> Option<String> {
    match pattern {
        syn::Pat::Ident(ident) => Some(ident.ident.to_string()),
        syn::Pat::Type(typed) => pattern_name(&typed.pat),
        syn::Pat::Reference(reference) => pattern_name(&reference.pat),
        syn::Pat::TupleStruct(tuple) if tuple.elems.len() == 1 => pattern_name(&tuple.elems[0]),
        _ => None,
    }
}

/// Whether `ty` is a `QueryBuilder<..>`.
pub(super) fn is_builder(ty: &syn::Type) -> bool {
    matches!(ty, syn::Type::Path(path)
        if path.path.segments.last().is_some_and(|segment| segment.ident == "QueryBuilder"))
}

/// Every variable a pattern binds: `(a, Some(b))` binds `a` and `b`.
pub(super) fn bound_names(pattern: &syn::Pat) -> Vec<String> {
    struct Names(Vec<String>);
    impl<'ast> Visit<'ast> for Names {
        fn visit_pat_ident(&mut self, ident: &'ast syn::PatIdent) {
            self.0.push(ident.ident.to_string());
            visit::visit_pat_ident(self, ident);
        }
    }
    let mut names = Names(Vec::new());
    names.visit_pat(pattern);
    names.0
}
