//! The indexes the project's migrations create, read from their `Schema`
//! builder calls as `tideorm make migration` writes them, for when the
//! database itself is not read.

use std::collections::HashMap;

use syn::Expr;
use syn::visit::{self, Visit};

use super::SourceFile;
use super::queries::{Link, flatten, pattern_name, string_literal};

/// Table builder methods that index the columns of their last argument.
const TABLE_INDEXES: &[&str] = &[
    "index",
    "index_named",
    "unique_index",
    "unique_index_named",
    "unique",
    "unique_named",
    "primary_key",
    "add_index",
    "add_index_named",
    "add_unique_index",
    "add_unique_index_named",
];

/// `Schema` methods taking `(table, name, columns)`.
const SCHEMA_INDEXES: &[&str] = &[
    "create_index",
    "create_unique_index",
    "create_index_if_not_exists",
    "create_unique_index_if_not_exists",
];

/// The columns of each index the migrations create, by table. Indexes a
/// later migration drops are kept: a missed index is a false warning, a
/// missed drop only a missed one.
pub(super) fn indexes(files: &[SourceFile]) -> HashMap<String, Vec<Vec<String>>> {
    let mut reader = Reader::default();
    for file in files {
        reader.visit_file(&file.syntax);
    }
    reader.indexes
}

#[derive(Default)]
struct Reader {
    indexes: HashMap<String, Vec<Vec<String>>>,
    /// The table a `create_table`/`alter_table` closure builds, and the
    /// name of its builder parameter.
    table: Option<(String, String)>,
}

impl Reader {
    fn add(&mut self, table: &str, columns: Option<Vec<String>>) {
        if let Some(columns) = columns.filter(|columns| !columns.is_empty()) {
            self.indexes
                .entry(table.to_string())
                .or_default()
                .push(columns);
        }
    }

    /// A table builder call: `t.index(&["a", "b"])`, `t.id()`, or a column
    /// made unique or the key, `t.string("email").unique()`.
    fn table_call(&mut self, table: &str, builder: &str, call: &syn::ExprMethodCall) {
        let method = call.method.to_string();
        let on_builder = matches!(&*call.receiver, Expr::Path(path) if path.path.is_ident(builder));
        let columns = if on_builder && TABLE_INDEXES.contains(&method.as_str()) {
            call.args.last().and_then(columns)
        } else if on_builder && method == "id" {
            Some(vec!["id".to_string()])
        } else if on_builder && matches!(method.as_str(), "increments" | "big_increments") {
            call.args
                .first()
                .and_then(string_literal)
                .map(|name| vec![name])
        } else if call.args.is_empty() && matches!(method.as_str(), "unique" | "primary_key") {
            // The column the chain starts from: `t.string("email")`.
            let (root, links) = flatten(&call.receiver);
            match (root, links.first()) {
                (Expr::Path(path), Some(Link::Call { args, .. }))
                    if path.path.is_ident(builder) =>
                {
                    args.first()
                        .and_then(|arg| string_literal(arg))
                        .map(|name| vec![name])
                }
                _ => None,
            }
        } else {
            None
        };
        self.add(table, columns);
    }
}

impl<'ast> Visit<'ast> for Reader {
    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        // `down` undoes `up`; an index it puts back is one `up` dropped.
        if item.sig.ident != "down" {
            visit::visit_impl_item_fn(self, item);
        }
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        let method = call.method.to_string();
        if let Some((table, builder)) = self.table.clone() {
            self.table_call(&table, &builder, call);
        }
        if SCHEMA_INDEXES.contains(&method.as_str())
            && let (Some(table), Some(columns_arg)) =
                (call.args.first().and_then(string_literal), call.args.get(2))
        {
            self.add(&table, columns(columns_arg));
        }
        if matches!(
            method.as_str(),
            "create_table" | "create_table_if_not_exists" | "alter_table"
        ) && let (Some(table), Some(Expr::Closure(closure))) =
            (call.args.first().and_then(string_literal), call.args.get(1))
            && let Some(builder) = closure.inputs.first().and_then(pattern_name)
        {
            visit::visit_expr(self, &call.receiver);
            let outer = self.table.replace((table, builder));
            visit::visit_expr(self, &closure.body);
            self.table = outer;
            return;
        }
        visit::visit_expr_method_call(self, call);
    }
}

/// The column names of `&["a", "b"]`; `None` unless every one is written out.
fn columns(expr: &Expr) -> Option<Vec<String>> {
    match expr {
        Expr::Reference(reference) => columns(&reference.expr),
        Expr::Array(array) => array.elems.iter().map(string_literal).collect(),
        _ => None,
    }
}
