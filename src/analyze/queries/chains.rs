//! Method chains such as `User::query().where_eq(..).get().await?`, read link
//! by link from the call or variable that starts them.

use syn::Expr;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};

use super::{
    Binding, Checker, Column, Filter, INDEXABLE, Join, Link, SHAPING, UNINDEXABLE, flatten,
    string_literal,
};
use crate::analyze::models::index_help;
use crate::analyze::{Project, Severity, line};

/// Methods whose first argument is SQL run as written.
pub(super) const RAW_SQL: &[&str] = &[
    "where_raw",
    "or_where_raw",
    "and_where_raw",
    "where_raw_with",
    "or_where_raw_with",
    "and_where_raw_with",
    "having_raw",
    "having_raw_with",
    "select_raw",
    "order_by_raw",
    "union_raw",
    "union_raw_with",
    "union_all_raw",
    "union_all_raw_with",
    "exec_raw",
    "exec_raw_with_params",
    "execute_with_params",
    "query_raw",
    "query_raw_with_params",
    "raw_with_params",
    "query_raw_json",
    "query_raw_json_with_params",
    "raw_json_with_params",
    "query_raw_as",
    "query_raw_as_with_params",
    "raw_as_with_params",
];

/// The `Database::` functions that run SQL as written, beside [`RAW_SQL`].
const DATABASE_RAW_SQL: &[&str] = &["execute", "raw", "raw_json", "raw_as"];

/// Methods that hand back what they were called on, or its `Ok` value.
const TRANSPARENT: &[&str] = &[
    "unwrap",
    "expect",
    "unwrap_or_default",
    "map_err",
    "context",
    "with_context",
];

/// Where a chain has got to.
enum State {
    Query(usize),
    /// A call that runs a statement once awaited.
    Pending(Run),
    /// Rows of a model; `fresh` when this chain read them.
    Many {
        model: String,
        fresh: bool,
    },
    One(String),
    /// A relation field of a loaded model, by name.
    Relation(String),
    Nothing,
}

struct Run {
    what: String,
    gives: Gives,
    write: bool,
    /// The relation field a relation load reads.
    relation: Option<String>,
    /// The query builder that runs, by group.
    group: Option<usize>,
    line: usize,
}

enum Gives {
    Many(String),
    One(String),
    Nothing,
}

impl Run {
    fn read(what: String, gives: Gives, line: usize) -> State {
        State::Pending(Self {
            what,
            gives,
            write: false,
            relation: None,
            group: None,
            line,
        })
    }

    fn write(what: String, gives: Gives, line: usize) -> State {
        State::Pending(Self {
            what,
            gives,
            write: true,
            relation: None,
            group: None,
            line,
        })
    }
}

impl Checker<'_> {
    /// Check a chain, and say what it gives.
    pub(super) fn chain(&mut self, expr: &Expr) -> Option<Binding> {
        let (root, links) = flatten(expr);
        visit::visit_expr(self, root);
        for link in &links {
            if let Link::Call { args, .. } = link {
                for arg in args {
                    self.visit_expr(arg);
                }
            }
        }

        let mut state = self.start(root);
        for link in &links {
            state = match link {
                Link::Await => self.awaited(state),
                Link::Try => state,
                Link::Call { name, args, line } => {
                    let sql = match name.as_str() {
                        "set_trusted_raw" => args.get(1),
                        name if RAW_SQL.contains(&name) => args.first(),
                        _ => None,
                    };
                    if let Some(sql) = sql {
                        self.raw_sql(name, sql, *line);
                    }
                    self.step(state, name, args, *line)
                }
            };
        }
        match state {
            State::Query(group) => Some(Binding::Query(group)),
            State::Many { model, .. } => Some(Binding::Many(model)),
            State::One(model) => Some(Binding::One(model)),
            _ => None,
        }
    }

    fn start(&mut self, root: &Expr) -> State {
        match root {
            Expr::Call(call) => {
                let Expr::Path(path) = &*call.func else {
                    return State::Nothing;
                };
                let segments = &path.path.segments;
                if segments.len() < 2 {
                    return State::Nothing;
                }
                let function = segments[segments.len() - 1].ident.to_string();
                let owner = &segments[segments.len() - 2];
                let line = line(call.span());
                if owner.ident == "Database" {
                    if (DATABASE_RAW_SQL.contains(&function.as_str())
                        || RAW_SQL.contains(&function.as_str()))
                        && let Some(sql) = call.args.first()
                    {
                        self.raw_sql(&format!("Database::{function}"), sql, line);
                    }
                    return State::Nothing;
                }
                let Some(model) = self.model_named(owner) else {
                    return State::Nothing;
                };
                let what = format!("{model}::{function}()");
                match function.as_str() {
                    "query" | "query_with" | "update_all" => {
                        State::Query(self.group(model, &function))
                    }
                    "all" => {
                        self.unbounded(&model, line);
                        Run::read(what, Gives::Many(model), line)
                    }
                    "find" | "find_or_fail" | "find_with" | "first" | "last" => {
                        Run::read(what, Gives::One(model), line)
                    }
                    "count" | "exists" | "exists_any" => Run::read(what, Gives::Nothing, line),
                    "paginate" => {
                        let group = self.group(model, "paginate");
                        self.groups[group].paged = true;
                        State::Pending(Run {
                            what,
                            gives: Gives::Nothing,
                            write: false,
                            relation: None,
                            group: Some(group),
                            line,
                        })
                    }
                    "create" | "insert_or_update" => Run::write(what, Gives::One(model), line),
                    "insert_all" | "destroy" | "delete_all" => {
                        Run::write(what, Gives::Nothing, line)
                    }
                    _ => State::Nothing,
                }
            }
            Expr::Path(path) => {
                let binding = path
                    .path
                    .get_ident()
                    .and_then(|name| self.bindings.get(&name.to_string()));
                match binding {
                    Some(Binding::Query(group)) => State::Query(*group),
                    Some(Binding::Many(model)) => State::Many {
                        model: model.clone(),
                        fresh: false,
                    },
                    Some(Binding::One(model)) => State::One(model.clone()),
                    _ => State::Nothing,
                }
            }
            Expr::Struct(literal) => literal
                .path
                .segments
                .last()
                .and_then(|segment| self.model_named(segment))
                .map_or(State::Nothing, State::One),
            Expr::Field(_) => {
                relation_field(self.project, root).map_or(State::Nothing, State::Relation)
            }
            _ => State::Nothing,
        }
    }

    fn step(&mut self, state: State, name: &str, args: &[&Expr], line: usize) -> State {
        match state {
            State::Query(group) => self.query_step(group, name, args, line),
            State::Many { model, fresh } => match name {
                "len" | "is_empty" if fresh => {
                    self.report(
                        Severity::Warning,
                        "rows-to-count",
                        line,
                        format!("reads every {model} row only to {}", if name == "len" { "count them" } else { "see whether there is one" }),
                        "use `.count()` or `.exists()` on the query: the database counts without sending the rows".into(),
                    );
                    State::Nothing
                }
                "first" if fresh => {
                    self.report(
                        Severity::Hint,
                        "rows-to-first",
                        line,
                        format!("reads every {model} row to keep the first"),
                        "call `.first()` on the query, which reads one row".into(),
                    );
                    State::Nothing
                }
                "iter" | "into_iter" | "iter_mut" => State::Many {
                    model,
                    fresh: false,
                },
                name if TRANSPARENT.contains(&name) => State::Many { model, fresh },
                _ => State::Nothing,
            },
            State::One(model) => {
                let what = format!("{model}.{name}()");
                match name {
                    "save" | "update" => Run::write(what, Gives::One(model), line),
                    "delete" | "force_delete" => Run::write(what, Gives::Nothing, line),
                    "reload" => Run::read(what, Gives::One(model), line),
                    name if TRANSPARENT.contains(&name) => State::One(model),
                    _ => State::Nothing,
                }
            }
            State::Relation(field) => match name {
                "load" | "load_with" | "count" | "exists" => State::Pending(Run {
                    what: format!(".{field}.{name}()"),
                    gives: Gives::Nothing,
                    write: false,
                    relation: Some(field),
                    group: None,
                    line,
                }),
                _ => State::Nothing,
            },
            State::Pending(_) | State::Nothing => State::Nothing,
        }
    }

    fn query_step(&mut self, group: usize, name: &str, args: &[&Expr], line: usize) -> State {
        if self.filter(group, name, args, line) {
            return State::Query(group);
        }
        let model = self.groups[group].model.clone();
        let what = format!("{model}::{}()...{name}()", self.groups[group].root);
        let run = |gives, write| {
            State::Pending(Run {
                what: what.clone(),
                gives,
                write,
                relation: None,
                group: Some(group),
                line,
            })
        };
        match name {
            "order_by" | "order_asc" | "order_desc" | "latest_by" | "oldest_by" => {
                if let Some(column) = args.first().and_then(|arg| self.column(&model, arg)) {
                    self.groups[group].orders.push((column, line));
                }
            }
            "latest" | "oldest" => {
                let column = self.column(&model, &syn::parse_quote!("created_at"));
                self.groups[group]
                    .orders
                    .extend(column.map(|column| (column, line)));
            }
            "inner_join" | "left_join" | "right_join" | "inner_join_as" | "left_join_as"
            | "right_join_as" => {
                self.groups[group].joins += 1;
                self.join(&model, args, line);
            }
            "limit" => self.groups[group].limited = true,
            "page" | "offset" => {
                self.groups[group].limited |= name == "page";
                self.groups[group].paged = true;
            }
            "get" | "get_json" | "get_as" | "pluck" => {
                let query = &self.groups[group];
                if query.filters.is_empty() && !query.limited && query.joins == 0 && !query.fragment
                {
                    self.unbounded(&model, line);
                }
                let gives = if name == "get" {
                    Gives::Many(model.clone())
                } else {
                    Gives::Nothing
                };
                return run(gives, false);
            }
            "first" | "first_or_fail" | "find" | "find_or_fail" | "last" => {
                return run(Gives::One(model.clone()), false);
            }
            "count" | "count_distinct" | "exists" | "value" | "sum" | "avg" | "min" | "max"
            | "aggregates" => return run(Gives::Nothing, false),
            "paginate" | "cursor_paginate" | "chunk" => {
                self.groups[group].paged = true;
                return run(Gives::Nothing, false);
            }
            "delete" | "delete_all" | "execute" | "execute_returning" | "soft_delete"
            | "restore" | "force_delete" => return run(Gives::Nothing, true),
            // A scope or a condition the checks cannot read.
            name if !SHAPING.contains(&name) => self.opaque(group, Join::And, line),
            _ => {}
        }
        State::Query(group)
    }

    /// Record a `where_*`/`or_where_*`/`and_where_*` filter on one column;
    /// false for any other method.
    fn filter(&mut self, group: usize, name: &str, args: &[&Expr], line: usize) -> bool {
        let (join, operator) = if let Some(operator) = name.strip_prefix("where_") {
            (Join::And, operator)
        } else if let Some(operator) = name.strip_prefix("or_where_") {
            (Join::Or, operator)
        } else if let Some(operator) = name.strip_prefix("and_where_") {
            (Join::Branch, operator)
        } else {
            return false;
        };
        let indexable = INDEXABLE.contains(&operator);
        if !indexable && !UNINDEXABLE.contains(&operator) {
            // `where_raw`, `where_has`, `where_json_contains`, ..
            self.opaque(group, join, line);
            return true;
        }
        let model = self.groups[group].model.clone();
        let column = args.first().and_then(|arg| self.column(&model, arg));
        let leading_wildcard = matches!(operator, "contains" | "ends_with")
            || (operator == "like"
                && args
                    .get(1)
                    .and_then(|arg| string_literal(arg))
                    .is_some_and(|pattern| pattern.starts_with('%')));
        if leading_wildcard {
            let target = column.as_ref().map_or_else(
                || "its column".to_string(),
                |column| format!("`{}`", column.name),
            );
            self.report(
                Severity::Hint,
                "leading-wildcard",
                line,
                format!("`{name}` searches inside {target}, which no index can serve"),
                "on a large table, use full-text search (`search()` with a `FullTextIndex`)".into(),
            );
        }
        self.groups[group].filters.push(Filter {
            column,
            indexable: indexable && !leading_wildcard,
            join,
            line,
        });
        true
    }

    /// A filter whose column the checks cannot tell, which may be one an
    /// index serves: the query's filters are not judged.
    fn opaque(&mut self, group: usize, join: Join, line: usize) {
        self.groups[group].filters.push(Filter {
            column: None,
            indexable: false,
            join,
            line,
        });
    }

    /// The join's column on the joined table should lead one of its indexes.
    fn join(&mut self, model: &str, args: &[&Expr], line: usize) {
        let Some(table) = args.first().and_then(|arg| string_literal(arg)) else {
            return;
        };
        let columns: Vec<Column> = args[1..]
            .iter()
            .filter_map(|arg| self.column(model, arg))
            .collect();
        for column in columns {
            if column.table == table
                && self.project.leads_index(&table, &column.name) == Some(false)
            {
                self.report(
                    Severity::Hint,
                    "unindexed-join",
                    line,
                    format!(
                        "joins `{table}` on `{}`, which no index of it leads",
                        column.name
                    ),
                    index_help(self.project, &table, &column.name, &self.file.path),
                );
            }
        }
    }

    fn unbounded(&mut self, model: &str, line: usize) {
        let table = self
            .project
            .model(model, &self.file.path)
            .map_or_else(String::new, |model| model.table.clone());
        self.report(
            Severity::Hint,
            "unbounded-read",
            line,
            format!("reads every row of `{table}` at once"),
            "filter it, `limit()` it, or page through it with `paginate()` or `chunk()`".into(),
        );
    }

    fn awaited(&mut self, state: State) -> State {
        let State::Pending(run) = state else {
            return State::Nothing;
        };
        let query = run.group.map(|group| &self.groups[group]);
        // A loop over pages, or a `loop`/`while` taking a batch each time.
        let batched =
            query.is_some_and(|query| query.paged || (query.limited && self.item_loops == 0));
        let fresh = query.is_none_or(|query| !query.limited);
        if self.loops > 0 && !batched {
            if run.write {
                if !seed_code(&self.file.path) {
                    self.report(
                        Severity::Warning,
                        "write-in-loop",
                        run.line,
                        format!(
                            "`{}` runs once per loop iteration: one statement for each row",
                            run.what
                        ),
                        "write the rows together: `insert_all(..)` for new rows (it runs no model \
                         callbacks), `update_all()` or a filtered `delete()` for existing ones"
                            .into(),
                    );
                }
            } else {
                let help = match &run.relation {
                    Some(field) => format!("eager-load it with `.with(\"{field}\")` on the query that read these rows"),
                    None => "read the rows the loop needs in one query before it (`where_in` on their keys) \
                             and look them up in memory"
                        .into(),
                };
                self.report(
                    Severity::Warning,
                    "n-plus-one",
                    run.line,
                    format!("`{}` runs once per loop iteration (N+1 queries)", run.what),
                    help,
                );
            }
        }
        match run.gives {
            Gives::Many(model) => State::Many { model, fresh },
            Gives::One(model) => State::One(model),
            Gives::Nothing => State::Nothing,
        }
    }
}

/// Seeders and factories, which fill a database once: a write per row there
/// is not a hot path, and often wants each row's callbacks.
fn seed_code(path: &std::path::Path) -> bool {
    path.components().any(|part| {
        matches!(
            part.as_os_str().to_str(),
            Some("seeders" | "seeds" | "factories")
        )
    })
}

/// The name of `expr`'s field when it is `value.field` and some model has a
/// relation of that name.
pub(super) fn relation_field(project: &Project, expr: &Expr) -> Option<String> {
    let Expr::Field(access) = expr else {
        return None;
    };
    let syn::Member::Named(name) = &access.member else {
        return None;
    };
    let name = name.to_string();
    project
        .models
        .iter()
        .flat_map(|model| &model.fields)
        .any(|field| field.relation.is_some() && field.name == name)
        .then_some(name)
}
