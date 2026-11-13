//! The queries a project builds, followed through method chains and the
//! variables that hold them, and what each is checked for: filters no index
//! serves, queries run once per loop iteration, SQL built with `format!`,
//! and rows read only to be counted.

use std::collections::HashMap;

use syn::Expr;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};

use super::{Finding, Project, Severity, SourceFile, line};

mod chains;
mod indexes;
mod syntax;

pub(super) use syntax::{Link, flatten, pattern_name, string_literal};
use syntax::{bound_names, is_builder};

/// Filters an index can serve: the method name after `where_`/`or_where_`.
const INDEXABLE: &[&str] = &[
    "eq",
    "in",
    "in_subquery",
    "gt",
    "gte",
    "lt",
    "lte",
    "between",
    "null",
    "starts_with",
    "like",
];
/// Filters on one column that no plain index serves.
const UNINDEXABLE: &[&str] = &[
    "ne",
    "not",
    "not_in",
    "not_between",
    "not_like",
    "not_null",
    "contains",
    "ends_with",
];
/// Builder methods that shape what a query returns, not which rows it reads.
/// Any other one the checks do not know (a scope, `when`, `has_related`)
/// may filter by what an index serves.
const SHAPING: &[&str] = &[
    "select",
    "select_raw",
    "add_select",
    "select_subquery",
    "select_also_linked",
    "select_with_linked",
    "distinct",
    "with",
    "with_many",
    "with_columns",
    "with_query",
    "with_trashed",
    "group_by",
    "having",
    "having_raw",
    "having_raw_with",
    "order_by_raw",
    "cache",
    "cache_with_key",
    "no_cache",
    "lock_for_update",
    "reorder",
    "reverse",
    "clone",
    "debug",
    // The OR they group is in the `or_where_*`/`and_where_*` calls between.
    "begin_or",
    "end_or",
];

/// What a variable holds, as far as the checks follow it.
#[derive(Clone)]
enum Binding {
    /// A query builder, kept as the group of the function's queries.
    Query(usize),
    /// Rows of a model.
    Many(String),
    /// One model.
    One(String),
    /// Text built with `format!` or `+`.
    Formatted,
}

/// One query: everything done to a builder, across the statements and
/// variables it went through.
struct Group {
    model: String,
    /// The function that started it: `query`, `update_all`, ..
    root: String,
    filters: Vec<Filter>,
    /// The columns it orders by, with their lines.
    orders: Vec<(Column, usize)>,
    joins: usize,
    limited: bool,
    /// Read a page at a time (`page()`, `offset()`, `paginate()`, `chunk()`):
    /// a loop over pages is not N+1.
    paged: bool,
    /// Built for a caller to finish (a `QueryBuilder` parameter or return
    /// value), whose own filters the checks do not see.
    fragment: bool,
}

struct Filter {
    /// `None` when the column is not written out (a variable).
    column: Option<Column>,
    indexable: bool,
    join: Join,
    line: usize,
}

/// How a filter joins the query's others.
#[derive(Clone, Copy, PartialEq)]
enum Join {
    /// `where_*`: ANDed with the query.
    And,
    /// `or_where_*`: starts a term of the query's OR.
    Or,
    /// `and_where_*` (after `begin_or()`): ANDed into the current OR term.
    Branch,
}

#[derive(Clone)]
struct Column {
    table: String,
    name: String,
    /// Written as an unqualified string, which the compiler does not check.
    text: bool,
}

struct Checker<'a> {
    project: &'a Project,
    file: &'a SourceFile,
    findings: Vec<Finding>,
    /// The model an `impl` block is for, which `Self` names.
    self_model: Option<String>,
    loops: usize,
    /// The loops among them that go over items (`for`, `while let`), not
    /// ones that repeat a batch or poll (`loop`, `while`).
    item_loops: usize,
    bindings: HashMap<String, Binding>,
    groups: Vec<Group>,
}

pub(super) fn check(project: &Project, file: &SourceFile) -> Vec<Finding> {
    let mut checker = Checker {
        project,
        file,
        findings: Vec::new(),
        self_model: None,
        loops: 0,
        item_loops: 0,
        bindings: HashMap::new(),
        groups: Vec::new(),
    };
    checker.scoped(|checker| checker.visit_file(&file.syntax));
    checker.findings
}

impl Checker<'_> {
    fn report(
        &mut self,
        severity: Severity,
        rule: &'static str,
        line: usize,
        message: String,
        help: String,
    ) {
        self.findings.push(Finding {
            severity,
            rule,
            file: self.file.path.clone(),
            line,
            message,
            help,
        });
    }

    /// Run `body` as a function of its own: its variables and queries are
    /// its own, and its queries are checked when it ends.
    fn scoped(&mut self, body: impl FnOnce(&mut Self)) {
        let bindings = std::mem::take(&mut self.bindings);
        let groups = std::mem::take(&mut self.groups);
        let loops = std::mem::take(&mut self.loops);
        let item_loops = std::mem::take(&mut self.item_loops);
        body(self);
        for group in std::mem::replace(&mut self.groups, groups) {
            self.finish(&group);
        }
        self.bindings = bindings;
        self.loops = loops;
        self.item_loops = item_loops;
    }

    /// A function: a query it returns is finished by its caller.
    fn function(&mut self, signature: &syn::Signature, body: impl FnOnce(&mut Self)) {
        let returns_builder =
            matches!(&signature.output, syn::ReturnType::Type(_, ty) if is_builder(ty));
        self.scoped(|checker| {
            for input in &signature.inputs {
                if let syn::FnArg::Typed(typed) = input {
                    checker.bind_builder(&typed.pat, &typed.ty);
                }
            }
            body(checker);
            if returns_builder {
                for group in &mut checker.groups {
                    group.fragment = true;
                }
            }
        });
    }

    /// A parameter typed `QueryBuilder<Model>`, as scopes take them: a
    /// query its caller started.
    fn bind_builder(&mut self, pattern: &syn::Pat, ty: &syn::Type) {
        if let (Some(name), Some(model)) = (pattern_name(pattern), self.builder_model(ty)) {
            let group = self.group(model, "query");
            self.groups[group].fragment = true;
            self.bindings.insert(name, Binding::Query(group));
        }
    }

    /// Run `body` with the variables `patterns` bind hiding any outer ones
    /// of the same names, which come back after it.
    fn shadowing<'p>(
        &mut self,
        patterns: impl IntoIterator<Item = &'p syn::Pat>,
        body: impl FnOnce(&mut Self),
    ) {
        let outer: Vec<_> = patterns
            .into_iter()
            .flat_map(bound_names)
            .map(|name| {
                let binding = self.bindings.remove(&name);
                (name, binding)
            })
            .collect();
        body(self);
        for (name, binding) in outer.into_iter().rev() {
            self.set(name, binding);
        }
    }

    fn builder_model(&self, ty: &syn::Type) -> Option<String> {
        if !is_builder(ty) {
            return None;
        }
        let syn::Type::Path(path) = ty else {
            return None;
        };
        let segment = path.path.segments.last()?;
        let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments else {
            return None;
        };
        let Some(syn::GenericArgument::Type(syn::Type::Path(model))) = arguments.args.first()
        else {
            return None;
        };
        self.model_named(model.path.segments.last()?)
    }

    fn model_named(&self, segment: &syn::PathSegment) -> Option<String> {
        if segment.ident == "Self" {
            return self.self_model.clone();
        }
        let name = segment.ident.to_string();
        self.project.model(&name, &self.file.path).map(|_| name)
    }

    fn group(&mut self, model: String, root: &str) -> usize {
        self.groups.push(Group {
            model,
            root: root.to_string(),
            filters: Vec::new(),
            orders: Vec::new(),
            joins: 0,
            limited: false,
            paged: false,
            fragment: false,
        });
        self.groups.len() - 1
    }

    /// Follow `expr`, checking what it does, and say what it gives.
    fn expr(&mut self, expr: &Expr) -> Option<Binding> {
        match expr {
            Expr::MethodCall(_) | Expr::Await(_) | Expr::Try(_) | Expr::Call(_) => self.chain(expr),
            Expr::Reference(reference) => self.expr(&reference.expr),
            Expr::Paren(paren) => self.expr(&paren.expr),
            Expr::Path(path) => {
                let name = path.path.get_ident()?.to_string();
                self.bindings.get(&name).cloned()
            }
            Expr::Struct(literal) => {
                visit::visit_expr_struct(self, literal);
                let model = self.model_named(literal.path.segments.last()?)?;
                Some(Binding::One(model))
            }
            Expr::Let(binding) => {
                let outcome = self.expr(&binding.expr);
                self.bind(&binding.pat, outcome);
                None
            }
            Expr::Assign(assign) => {
                let outcome = self.expr(&assign.right);
                if let Expr::Path(path) = &*assign.left
                    && let Some(name) = path.path.get_ident()
                {
                    self.set(name.to_string(), outcome);
                }
                None
            }
            Expr::Binary(binary) => {
                self.count_for_existence(binary);
                visit::visit_expr_binary(self, binary);
                None
            }
            Expr::ForLoop(for_loop) => {
                let items = self.expr(&for_loop.expr);
                self.shadowing([&*for_loop.pat], |checker| {
                    if let Some(Binding::Many(model)) = items
                        && let Some(name) = pattern_name(&for_loop.pat)
                    {
                        checker.bindings.insert(name, Binding::One(model));
                    }
                    checker.in_loop(true, |checker| checker.visit_block(&for_loop.body));
                });
                None
            }
            Expr::While(while_loop) => {
                self.visit_expr(&while_loop.cond);
                let items = matches!(&*while_loop.cond, Expr::Let(_));
                self.in_loop(items, |checker| checker.visit_block(&while_loop.body));
                None
            }
            Expr::Loop(body) => {
                self.in_loop(false, |checker| checker.visit_block(&body.body));
                None
            }
            Expr::Closure(closure) => {
                self.shadowing(&closure.inputs, |checker| {
                    for input in &closure.inputs {
                        if let syn::Pat::Type(typed) = input {
                            checker.bind_builder(&typed.pat, &typed.ty);
                        }
                    }
                    checker.visit_expr(&closure.body);
                });
                None
            }
            _ => {
                visit::visit_expr(self, expr);
                None
            }
        }
    }

    fn in_loop(&mut self, items: bool, body: impl FnOnce(&mut Self)) {
        self.loops += 1;
        self.item_loops += usize::from(items);
        body(self);
        self.loops -= 1;
        self.item_loops -= usize::from(items);
    }

    fn bind(&mut self, pattern: &syn::Pat, outcome: Option<Binding>) {
        match pattern_name(pattern) {
            Some(name) => self.set(name, outcome),
            None => {
                for name in bound_names(pattern) {
                    self.bindings.remove(&name);
                }
            }
        }
    }

    fn set(&mut self, name: String, outcome: Option<Binding>) {
        match outcome {
            Some(binding) => self.bindings.insert(name, binding),
            None => self.bindings.remove(&name),
        };
    }

    /// Whether `expr` is text built at run time: `format!`, `+`, or a
    /// variable holding one.
    fn formatted(&self, expr: &Expr) -> bool {
        match expr {
            Expr::Reference(reference) => self.formatted(&reference.expr),
            Expr::Paren(paren) => self.formatted(&paren.expr),
            Expr::MethodCall(call)
                if matches!(
                    call.method.to_string().as_str(),
                    "as_str" | "as_ref" | "to_string" | "to_owned" | "clone" | "into"
                ) =>
            {
                self.formatted(&call.receiver)
            }
            Expr::Macro(mac) => {
                let tokens = mac.mac.tokens.to_string();
                mac.mac.path.is_ident("format") && (tokens.contains(',') || tokens.contains('{'))
            }
            Expr::Binary(binary) => matches!(binary.op, syn::BinOp::Add(_)),
            Expr::Path(path) => path.path.get_ident().is_some_and(|name| {
                matches!(
                    self.bindings.get(&name.to_string()),
                    Some(Binding::Formatted)
                )
            }),
            _ => false,
        }
    }

    /// Report SQL text built at run time handed to `method`, which runs it.
    fn raw_sql(&mut self, method: &str, sql: &Expr, line: usize) {
        if self.formatted(sql) {
            self.report(
                Severity::Warning,
                "sql-injection",
                line,
                format!("`{method}` runs SQL built with `format!`: a value written into it can change the statement"),
                "write `?` markers and pass the values (`where_raw_with`, `having_raw_with`, the `*_with_params` forms); take identifiers from a fixed list"
                    .into(),
            );
        }
    }

    /// The column `arg` names for a query of `model`.
    fn column(&self, model: &str, arg: &Expr) -> Option<Column> {
        let own = self.project.model(model, &self.file.path)?;
        if let Some(text) = string_literal(arg) {
            return Some(match text.rsplit_once('.') {
                Some((table, name)) => Column {
                    table: table.to_string(),
                    name: self
                        .project
                        .model_of_table(table, &self.file.path)
                        .and_then(|model| model.column(name))
                        .map_or_else(|| name.to_string(), |field| field.column.clone()),
                    text: false,
                },
                None => Column {
                    table: own.table.clone(),
                    name: own.column(&text).map_or(text, |field| field.column.clone()),
                    text: true,
                },
            });
        }
        // `User::EMAIL` or `User::columns.email`.
        let (owner, field) = match arg {
            Expr::Path(path) if path.path.segments.len() >= 2 => {
                let segments = &path.path.segments;
                let constant = segments.last()?.ident.to_string();
                (&segments[segments.len() - 2], constant.to_lowercase())
            }
            Expr::Field(access) => {
                let (Expr::Path(path), syn::Member::Named(name)) = (&*access.base, &access.member)
                else {
                    return None;
                };
                let segments = &path.path.segments;
                if segments.len() < 2 || segments.last()?.ident != "columns" {
                    return None;
                }
                (&segments[segments.len() - 2], name.to_string())
            }
            _ => return None,
        };
        let model = self
            .project
            .model(&self.model_named(owner)?, &self.file.path)?;
        let field = model
            .fields
            .iter()
            .find(|candidate| candidate.name == field)?;
        Some(Column {
            table: model.table.clone(),
            name: field.column.clone(),
            text: false,
        })
    }
}

impl<'ast> Visit<'ast> for Checker<'_> {
    fn visit_expr(&mut self, expr: &'ast Expr) {
        self.expr(expr);
    }

    fn visit_local(&mut self, local: &'ast syn::Local) {
        let Some(init) = &local.init else {
            return;
        };
        let outcome = self
            .expr(&init.expr)
            .or_else(|| self.formatted(&init.expr).then_some(Binding::Formatted));
        if let Some((_, diverge)) = &init.diverge {
            self.visit_expr(diverge);
        }
        self.bind(&local.pat, outcome);
    }

    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        self.function(&item.sig, |checker| visit::visit_item_fn(checker, item));
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        self.function(&item.sig, |checker| {
            visit::visit_impl_item_fn(checker, item);
        });
    }

    fn visit_arm(&mut self, arm: &'ast syn::Arm) {
        self.shadowing([&arm.pat], |checker| visit::visit_arm(checker, arm));
    }

    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        let model = match &*item.self_ty {
            syn::Type::Path(path) => path.path.segments.last().and_then(|segment| {
                let name = segment.ident.to_string();
                self.project.model(&name, &self.file.path).map(|_| name)
            }),
            _ => None,
        };
        let previous = std::mem::replace(&mut self.self_model, model);
        visit::visit_item_impl(self, item);
        self.self_model = previous;
    }
}

impl Checker<'_> {
    /// `query.count().await? > 0`: counting to learn whether a row exists.
    fn count_for_existence(&mut self, binary: &syn::ExprBinary) {
        use syn::BinOp::{Eq, Ge, Gt, Le, Lt, Ne};
        let value = |expr: &Expr| match expr {
            Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Int(int),
                ..
            }) => int.base10_parse::<u64>().ok(),
            _ => None,
        };
        let (count_left, literal) = match (value(&binary.left), value(&binary.right)) {
            (None, Some(literal)) if self.is_query_count(&binary.left) => (true, literal),
            (Some(literal), None) if self.is_query_count(&binary.right) => (false, literal),
            _ => return,
        };
        // Any comparison with 0, and `>= 1`/`< 1` (or `1 <=`/`1 >`).
        let existence = matches!(
            (literal, &binary.op, count_left),
            (0, Eq(_) | Ne(_) | Gt(_) | Lt(_) | Ge(_) | Le(_), _)
                | (1, Ge(_) | Lt(_), true)
                | (1, Le(_) | Gt(_), false)
        );
        if existence {
            self.report(
                Severity::Hint,
                "count-for-existence",
                line(binary.span()),
                "counts every matching row only to learn whether one exists".into(),
                "use `.exists()`, which stops at the first row".into(),
            );
        }
    }

    /// Whether `expr` awaits a model query's `count()`.
    fn is_query_count(&self, expr: &Expr) -> bool {
        let (root, links) = flatten(expr);
        let Some(position) = links
            .iter()
            .position(|link| matches!(link, Link::Call { name, .. } if name == "count"))
        else {
            return false;
        };
        let awaited = links[position..]
            .iter()
            .any(|link| matches!(link, Link::Await));
        let of_a_model = match root {
            Expr::Call(call) => matches!(&*call.func, Expr::Path(path)
                if path.path.segments.len() >= 2
                    && self.model_named(&path.path.segments[path.path.segments.len() - 2]).is_some()),
            Expr::Path(path) => path.path.get_ident().is_some_and(|name| {
                matches!(
                    self.bindings.get(&name.to_string()),
                    Some(Binding::Query(_))
                )
            }),
            Expr::Field(_) => chains::relation_field(self.project, root).is_some(),
            _ => false,
        };
        awaited && of_a_model
    }
}
