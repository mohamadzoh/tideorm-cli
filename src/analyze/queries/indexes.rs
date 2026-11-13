//! Whether an index serves each query's filters and its order, checked
//! once the function that built the query has ended.

use super::{Checker, Column, Filter, Group, Join, syntax};
use crate::analyze::models::index_help;
use crate::analyze::{Severity, listed};

impl Checker<'_> {
    /// Check one query's filters and ordering once its function has ended.
    pub(super) fn finish(&mut self, group: &Group) {
        let Some(model) = self.project.model(&group.model, &self.file.path) else {
            return;
        };
        for filter in &group.filters {
            if let Some(column) = &filter.column
                && column.text
                && group.joins == 0
                && syntax::is_identifier(&column.name)
                && model.column(&column.name).is_none()
            {
                let columns: Vec<String> = model
                    .fields
                    .iter()
                    .filter(|field| field.relation.is_none())
                    .map(|field| field.column.clone())
                    .collect();
                self.report(
                    Severity::Error,
                    "unknown-column",
                    filter.line,
                    format!("{} has no column `{}`: the query fails when it runs", model.name, column.name),
                    format!("its columns are {}; a typed column (`{}::NAME`) is checked by the compiler", listed(&columns), model.name),
                );
            }
        }
        if group.fragment {
            return;
        }

        // Filters on a joined table, a column held in a variable or one the
        // model does not have may be what an index serves; say nothing about
        // such queries.
        let own: Option<Vec<(&Filter, &Column)>> = group
            .filters
            .iter()
            .map(|filter| {
                let column = filter.column.as_ref()?;
                let known = !column.text || model.column(&column.name).is_some();
                (column.table == model.table && known).then_some((filter, column))
            })
            .collect();
        let Some(own) = own else { return };
        if !self.project.knows_indexes(&model.table) {
            return;
        }
        let leads = |column: &Column| self.project.leads_index(&model.table, &column.name);
        let served =
            |filter: &Filter, column: &Column| filter.indexable && leads(column) == Some(true);
        // The OR's terms, each an `or_where_*` and the `and_where_*` after
        // it: a term is served when one of its filters is.
        let mut terms: Vec<bool> = Vec::new();
        for (filter, column) in &own {
            match filter.join {
                Join::Or => terms.push(served(filter, column)),
                Join::Branch => {
                    if let Some(term) = terms.last_mut() {
                        *term |= served(filter, column);
                    }
                }
                Join::And => {}
            }
        }
        let is_served = own
            .iter()
            .any(|(filter, column)| filter.join == Join::And && served(filter, column))
            || (!terms.is_empty() && terms.iter().all(|served| *served));

        if !is_served {
            let mut candidates: Vec<(String, usize)> = Vec::new();
            for (filter, column) in &own {
                let boolean = model
                    .column(&column.name)
                    .is_some_and(|field| field.ty == "bool");
                if filter.indexable
                    && !boolean
                    && !candidates.iter().any(|(name, _)| *name == column.name)
                {
                    candidates.push((column.name.clone(), filter.line));
                }
            }
            if let Some((first, line)) = candidates.first().cloned() {
                let names: Vec<String> = candidates.into_iter().map(|(name, _)| name).collect();
                self.report(
                    Severity::Warning,
                    "unindexed-filter",
                    line,
                    format!(
                        "{} query filters on {}, which no index of `{}` leads: the database reads every row",
                        model.name,
                        listed(&names),
                        model.table
                    ),
                    format!("index the most selective one: {}", index_help(self.project, &model.table, &first, &self.file.path)),
                );
            }
        }

        if group.filters.is_empty()
            && let Some((column, line)) = group
                .orders
                .iter()
                .find(|(column, _)| column.table == model.table && leads(column) == Some(false))
        {
            self.report(
                Severity::Hint,
                "unindexed-order",
                *line,
                format!(
                    "{} query sorts every row of `{}` by `{}`, which no index leads",
                    model.name, model.table, column.name
                ),
                index_help(self.project, &model.table, &column.name, &self.file.path),
            );
        }
    }
}
