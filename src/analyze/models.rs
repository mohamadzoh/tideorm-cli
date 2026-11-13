//! The models a project declares, read from its source as the derive reads
//! them, and the checks a model is held to on its own.

use std::path::{Path, PathBuf};

use syn::ext::IdentExt;
use syn::punctuated::Punctuated;

use super::{Finding, Project, Severity, SourceFile, line};
use crate::commands::models::{TideormOptions, model_structs};
use crate::derive_names::column_name;

pub(crate) struct Model {
    pub(crate) name: String,
    /// `schema.table` when the model names a schema.
    pub(crate) table: String,
    pub(crate) file: PathBuf,
    pub(crate) line: usize,
    pub(crate) fields: Vec<Field>,
    /// The columns of each index the model declares, its primary key first.
    pub(crate) indexes: Vec<Vec<String>>,
}

pub(crate) struct Field {
    pub(crate) name: String,
    pub(crate) column: String,
    /// The type's last path segment, seen through `Option`: `bool`, `f64`, ..
    pub(crate) ty: String,
    pub(crate) optional: bool,
    pub(crate) primary_key: bool,
    /// Hidden from JSON, encrypted, or skipped by a serde derive.
    pub(crate) protected: bool,
    /// `Some` for a relation field, which has no column.
    pub(crate) relation: Option<Relation>,
    pub(crate) line: usize,
}

pub(crate) struct Relation {
    /// The wrapper's kind, snake-cased: `has_one`, `has_many`, `self_ref_many`, ..
    pub(crate) kind: &'static str,
    /// The related model's name; empty for a `MorphTo`.
    pub(crate) model: String,
    pub(crate) foreign_key: Option<String>,
    pub(crate) pivot: Option<String>,
}

impl Model {
    /// The column a field or column name stands for, as the query builder
    /// takes either.
    pub(crate) fn column(&self, name: &str) -> Option<&Field> {
        self.fields
            .iter()
            .filter(|field| field.relation.is_none())
            .find(|field| field.name == name || field.column == name)
    }
}

/// Every model the files declare, inline modules included.
pub(crate) fn read_models(files: &[SourceFile]) -> Vec<Model> {
    files
        .iter()
        .flat_map(|file| {
            model_structs(&file.syntax.items)
                .into_iter()
                .filter_map(|item| read_model(&file.path, item))
        })
        .collect()
}

fn read_model(path: &Path, item: &syn::ItemStruct) -> Option<Model> {
    let syn::Fields::Named(named) = &item.fields else {
        return None;
    };
    let name = item.ident.unraw().to_string();
    let options = TideormOptions::of(&item.attrs);
    let list = |option: &str| -> Vec<String> {
        options
            .value(option)
            .map(|value| {
                value
                    .split(',')
                    .map(|part| part.trim().to_string())
                    .collect()
            })
            .unwrap_or_default()
    };
    let protected = [list("hidden"), list("encrypted")].concat();

    let mut fields = Vec::new();
    let mut field_indexes = Vec::new();
    for field in &named.named {
        let Some(ident) = &field.ident else { continue };
        let field_options = TideormOptions::of(&field.attrs);
        if field_options.has("skip") {
            continue;
        }
        let field_name = ident.unraw().to_string();
        let column = field_options
            .value("column")
            .map_or_else(|| column_name(&field_name), str::to_string);
        if field.attrs.iter().any(|attribute| {
            matches!(attribute.meta, syn::Meta::Path(_))
                && (attribute.path().is_ident("index") || attribute.path().is_ident("unique_index"))
        }) {
            field_indexes.push(vec![column.clone()]);
        }
        let (ty, optional) = type_name(&field.ty);
        fields.push(Field {
            protected: protected.contains(&field_name)
                || protected.contains(&column)
                || serde_skipped(&field.attrs),
            relation: relation(&field.ty, &field_options),
            primary_key: field_options.has("primary_key"),
            name: field_name,
            column,
            ty,
            optional,
            line: line(ident.span()),
        });
    }

    // Columns the attributes name may be field names; an index holds columns.
    let resolve = |name: String| {
        fields
            .iter()
            .find(|field| field.name == name)
            .map_or(name, |field| field.column.clone())
    };
    let primary_key: Vec<String> = fields
        .iter()
        .filter(|field| field.primary_key)
        .map(|field| field.column.clone())
        .collect();
    let mut indexes: Vec<Vec<String>> = (!primary_key.is_empty())
        .then_some(primary_key)
        .into_iter()
        .collect();
    for attribute in &item.attrs {
        if attribute.path().is_ident("index") || attribute.path().is_ident("unique_index") {
            indexes.push(index_columns(attribute).into_iter().map(resolve).collect());
        }
    }
    indexes.extend(field_indexes);

    let table = options.table(&name);
    Some(Model {
        table: match options.value("schema") {
            Some(schema) => format!("{schema}.{table}"),
            None => table,
        },
        file: path.to_path_buf(),
        line: line(item.ident.span()),
        name,
        fields,
        indexes,
    })
}

/// The columns of `#[index("a,b")]`, `#[index("a", "b")]`, `#[index(a, b)]`
/// or `#[index(name = "..", columns = "a,b")]`.
fn index_columns(attribute: &syn::Attribute) -> Vec<String> {
    let split = |text: String| -> Vec<String> {
        text.split(',')
            .map(|part| part.trim().to_string())
            .filter(|part| !part.is_empty())
            .collect()
    };
    if let Ok(literals) =
        attribute.parse_args_with(Punctuated::<syn::LitStr, syn::Token![,]>::parse_terminated)
    {
        return literals
            .iter()
            .flat_map(|literal| split(literal.value()))
            .collect();
    }
    let Ok(entries) =
        attribute.parse_args_with(Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated)
    else {
        return Vec::new();
    };
    let mut columns = Vec::new();
    for entry in entries {
        match entry {
            syn::Meta::Path(path) => {
                columns.extend(path.get_ident().map(|i| i.unraw().to_string()))
            }
            syn::Meta::NameValue(pair) if pair.path.is_ident("columns") => {
                if let syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(literal),
                    ..
                }) = pair.value
                {
                    columns.extend(split(literal.value()));
                }
            }
            _ => {}
        }
    }
    columns
}

/// The relation a field's type declares, as the derive reads it: the kind
/// from the wrapper (seen through `Option`, `Box`, `Rc`, `Arc`), the related
/// model from its first type argument, else from `has_many = ".."` and the like.
fn relation(ty: &syn::Type, options: &TideormOptions) -> Option<Relation> {
    let syn::Type::Path(path) = ty else {
        return None;
    };
    let segment = path.path.segments.last()?;
    let argument = match &segment.arguments {
        syn::PathArguments::AngleBracketed(arguments) => {
            arguments.args.iter().find_map(|argument| match argument {
                syn::GenericArgument::Type(inner) => Some(inner),
                _ => None,
            })
        }
        _ => None,
    };
    let kind = match segment.ident.to_string().as_str() {
        "Option" | "Box" | "Rc" | "Arc" => return relation(argument?, options),
        "HasOne" => "has_one",
        "HasMany" => "has_many",
        "BelongsTo" => "belongs_to",
        "HasManyThrough" => "has_many_through",
        "MorphOne" => "morph_one",
        "MorphMany" => "morph_many",
        "MorphTo" => "morph_to",
        "SelfRef" => "self_ref",
        "SelfRefMany" => "self_ref_many",
        _ => return None,
    };
    let model = argument
        .map(|inner| type_name(inner).0)
        .filter(|name| !name.is_empty())
        .or_else(|| {
            let model = options.value(kind)?;
            Some(model.rsplit("::").next().unwrap_or(model).to_string())
        })
        .unwrap_or_default();
    Some(Relation {
        kind,
        model,
        foreign_key: options.value("foreign_key").map(str::to_string),
        pivot: options.value("pivot").map(str::to_string),
    })
}

/// Whether `#[serde(skip)]` or `#[serde(skip_serializing)]` keeps the field
/// out of a user's own serde derive.
fn serde_skipped(attributes: &[syn::Attribute]) -> bool {
    attributes
        .iter()
        .filter(|attribute| attribute.path().is_ident("serde"))
        .any(|attribute| {
            let mut skipped = false;
            let _ = attribute.parse_nested_meta(|meta| {
                skipped |= meta.path.is_ident("skip") || meta.path.is_ident("skip_serializing");
                if meta.input.peek(syn::Token![=]) {
                    let _: syn::Expr = meta.value()?.parse()?;
                }
                Ok(())
            });
            skipped
        })
}

/// The last path segment of `ty` and whether it is an `Option`.
fn type_name(ty: &syn::Type) -> (String, bool) {
    let syn::Type::Path(path) = ty else {
        return (String::new(), false);
    };
    let Some(segment) = path.path.segments.last() else {
        return (String::new(), false);
    };
    if segment.ident == "Option"
        && let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments
        && let Some(syn::GenericArgument::Type(inner)) = arguments.args.first()
    {
        return (type_name(inner).0, true);
    }
    (segment.ident.to_string(), false)
}

/// The checks a model is held to on its own: relation keys without an
/// index, secrets that serialize, money in floats.
pub(crate) fn check(project: &Project) -> Vec<Finding> {
    let mut findings = Vec::new();
    for model in &project.models {
        let finding = |field: &Field, severity, rule, message: String, help: String| Finding {
            severity,
            rule,
            file: model.file.clone(),
            line: field.line,
            message,
            help,
        };
        for field in &model.fields {
            if let Some(relation) = &field.relation {
                if let Some((table, key)) = relation_key(project, relation, &model.file)
                    && project.leads_index(&table, &key) == Some(false)
                {
                    findings.push(finding(
                        field,
                        Severity::Warning,
                        "unindexed-foreign-key",
                        format!(
                            "`{}.{}` reads `{table}` by `{key}`, which no index leads: \
                             each load scans the whole table",
                            model.name, field.name
                        ),
                        index_help(project, &table, &key, &model.file),
                    ));
                }
                continue;
            }
            let words: Vec<&str> = field.column.split('_').collect();
            // A secret is text: `password_changed_at` is a timestamp.
            let text = matches!(
                field.ty.as_str(),
                "String" | "str" | "Uuid" | "Vec" | "Value"
            );
            if !field.protected && text && looks_secret(&words) {
                findings.push(finding(
                    field,
                    Severity::Warning,
                    "sensitive-field",
                    format!(
                        "`{}.{}` looks secret but is neither hidden nor encrypted: \
                         `to_json()` and API responses include it",
                        model.name, field.name
                    ),
                    format!(
                        "add `hidden = \"{}\"` (or `encrypted = \"..\"`) to #[tideorm::model] of {}",
                        field.name, model.name
                    ),
                ));
            }
            if matches!(field.ty.as_str(), "f32" | "f64") && looks_like_money(&words) {
                findings.push(finding(
                    field,
                    Severity::Hint,
                    "float-money",
                    format!(
                        "`{}.{}` holds money in a float, which rounds (0.1 + 0.2 != 0.3)",
                        model.name, field.name
                    ),
                    "use `rust_decimal::Decimal`, or an integer number of cents".into(),
                ));
            }
        }
    }
    findings
}

/// The table and column a relation reads its rows by, when a key is not
/// the related model's primary key: `has_one`/`has_many`/`self_ref_many`
/// read the related table by the foreign key, `has_many_through` the pivot.
fn relation_key(project: &Project, relation: &Relation, file: &Path) -> Option<(String, String)> {
    let key = relation.foreign_key.clone()?;
    let table = match relation.kind {
        "has_one" | "has_many" | "self_ref_many" => {
            project.model(&relation.model, file)?.table.clone()
        }
        "has_many_through" => relation.pivot.clone()?,
        _ => return None,
    };
    let column = project
        .model_of_table(&table, file)
        .and_then(|model| model.column(&key))
        .map_or(key, |field| field.column.clone());
    Some((table, column))
}

/// How to index `table` by `column`, said to code in `file`: on its model
/// when one reads the table.
pub(super) fn index_help(project: &Project, table: &str, column: &str, file: &Path) -> String {
    match project.model_of_table(table, file) {
        Some(model) => format!(
            "add #[index(\"{column}\")] to {} (or create the index in a migration)",
            model.name
        ),
        None => format!("create an index on `{table}` ({column}) in a migration"),
    }
}

fn looks_secret(words: &[&str]) -> bool {
    words
        .iter()
        .any(|word| matches!(*word, "password" | "passwd" | "secret"))
        || words.last() == Some(&"token")
        || words
            .windows(2)
            .any(|pair| matches!(pair, ["api" | "private" | "secret", "key"]))
}

fn looks_like_money(words: &[&str]) -> bool {
    words.iter().any(|word| {
        matches!(
            *word,
            "price"
                | "amount"
                | "total"
                | "subtotal"
                | "cost"
                | "balance"
                | "fee"
                | "salary"
                | "tax"
                | "discount"
        )
    })
}
