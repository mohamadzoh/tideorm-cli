use std::path::Path;

use tideorm::schema::{ColumnInfo, IndexInfo};

use super::{Catalog, Finding, SourceFile, Table, analyze, read_models};

const MODELS: &str = r#"
#[tideorm::model(table = "users", hidden = "password_hash")]
pub struct User {
    #[tideorm(primary_key, auto_increment)]
    pub id: i64,
    #[unique_index]
    pub email: String,
    pub name: String,
    pub active: bool,
    pub password_hash: String,
    pub api_key: String,
    pub balance: f64,
    pub created_at: DateTime<Utc>,
    #[tideorm(has_many = "Post", foreign_key = "user_id")]
    pub posts: HasMany<Post>,
    #[tideorm(has_many = "Comment", foreign_key = "author_id")]
    pub comments: HasMany<Comment>,
}

#[tideorm::model(table = "posts")]
#[index("user_id")]
pub struct Post {
    #[tideorm(primary_key)]
    pub id: i64,
    pub user_id: i64,
    pub title: String,
    #[tideorm(skip)]
    pub preview: String,
    #[tideorm(belongs_to = "User", foreign_key = "user_id")]
    pub user: BelongsTo<User>,
}

#[tideorm::model(table = "comments")]
pub struct Comment {
    #[tideorm(primary_key)]
    pub id: i64,
    pub author_id: i64,
    pub text: String,
}
"#;

fn findings(code: &str, catalog: Option<&Catalog>) -> Vec<Finding> {
    let files = vec![
        SourceFile {
            path: "src/models.rs".into(),
            syntax: syn::parse_file(MODELS).expect("the models parse"),
        },
        SourceFile {
            path: "src/app.rs".into(),
            syntax: syn::parse_file(code).expect("the code parses"),
        },
    ];
    let models = read_models(&files);
    analyze(&files, models, catalog)
}

/// The rules `code` breaks, each with its line, in `src/app.rs` order.
fn rules(code: &str) -> Vec<(&'static str, usize)> {
    let mut found: Vec<_> = findings(code, None)
        .into_iter()
        .filter(|finding| finding.file == Path::new("src/app.rs"))
        .map(|finding| (finding.rule, finding.line))
        .collect();
    found.sort_by_key(|(_, line)| *line);
    found
}

fn names(code: &str) -> Vec<&'static str> {
    rules(code).into_iter().map(|(rule, _)| rule).collect()
}

#[test]
fn models_are_read_as_the_derive_reads_them() {
    let files = vec![SourceFile {
        path: "src/models.rs".into(),
        syntax: syn::parse_file(MODELS).unwrap(),
    }];
    let models = read_models(&files);
    let names: Vec<_> = models
        .iter()
        .map(|model| (model.name.as_str(), model.table.as_str()))
        .collect();
    assert_eq!(
        names,
        [
            ("User", "users"),
            ("Post", "posts"),
            ("Comment", "comments")
        ]
    );
    assert_eq!(models[0].indexes, [vec!["id"], vec!["email"]]);
    assert_eq!(models[1].indexes, [vec!["id"], vec!["user_id"]]);
    assert!(
        models[1].column("preview").is_none(),
        "a skip field has no column"
    );
    let posts = models[0]
        .fields
        .iter()
        .find(|field| field.name == "posts")
        .unwrap();
    let relation = posts.relation.as_ref().expect("a relation");
    assert_eq!(
        (relation.kind, relation.model.as_str()),
        ("has_many", "Post")
    );
}

#[test]
fn models_are_checked_on_their_own() {
    let found: Vec<_> = findings("", None)
        .into_iter()
        .map(|finding| (finding.rule, finding.message))
        .collect();
    let rules: Vec<_> = found.iter().map(|(rule, _)| *rule).collect();
    // By severity, then file and line.
    assert_eq!(
        rules,
        ["sensitive-field", "unindexed-foreign-key", "float-money"],
        "{found:?}"
    );
    assert!(
        found[0].1.contains("User.api_key"),
        "hidden password_hash is fine: {found:?}"
    );
    assert!(
        found[1]
            .1
            .contains("`User.comments` reads `comments` by `author_id`"),
        "{found:?}"
    );
}

#[test]
fn filters_no_index_serves_are_reported() {
    let code = r#"
async fn queries(name: &str, email: &str) {
    User::query().where_eq("name", name).get().await;
    User::query().where_eq("email", email).first().await;
    User::query().where_eq(User::NAME, name).where_eq(User::columns.email, email).get().await;
    User::query().where_eq("active", true).limit(5).get().await;
    User::query().where_ne("name", name).limit(5).get().await;
    User::query().or_where_eq("email", email).or_where_eq("name", name).first().await;
    User::query().where_eq("users.id", 1).first().await;
}
"#;
    assert_eq!(
        rules(code),
        [("unindexed-filter", 3), ("unindexed-filter", 8)]
    );
}

#[test]
fn a_query_is_followed_through_variables_scopes_and_self() {
    let code = r#"
async fn search(name: Option<&str>) {
    let mut query = User::query();
    if let Some(name) = name {
        query = query.where_eq("name", name);
    }
    query.limit(10).get().await;
}

fn named(query: QueryBuilder<User>, name: &str) -> QueryBuilder<User> {
    query.where_eq("name", name)
}

impl User {
    async fn by_name(name: &str) {
        Self::query().where_eq("name", name).first().await;
    }
}
"#;
    // `named` returns a fragment its callers finish with filters of their own.
    assert_eq!(
        rules(code),
        [("unindexed-filter", 5), ("unindexed-filter", 16)]
    );
}

#[test]
fn a_misspelt_column_is_an_error_unless_the_query_joins() {
    let code = r#"
async fn queries(email: &str) {
    User::query().where_eq("emial", email).first().await;
    Post::query().inner_join("users", "users.id", "posts.user_id").where_eq("emial", email).first().await;
}
"#;
    assert_eq!(names(code), ["unknown-column"]);
}

#[test]
fn queries_and_writes_in_loops_are_reported() {
    let code = r#"
async fn each() -> Result<()> {
    let users = User::query().limit(10).get().await?;
    for user in users {
        let posts = user.posts.load().await?;
        let post = Post::find(user.id).await?;
        user.save().await?;
    }
    for id in [1, 2] {
        Comment::create(Comment { id, ..Default::default() }).await?;
    }
    for page in 1..=3 {
        Post::query().order_asc("id").page(page, 10).get().await?;
    }
    let post = Post::find(1).await?;
    Ok(())
}
"#;
    assert_eq!(
        rules(code),
        [
            ("n-plus-one", 5),
            ("n-plus-one", 6),
            ("write-in-loop", 7),
            ("write-in-loop", 10)
        ]
    );
}

#[test]
fn sql_built_with_format_is_reported() {
    let code = r#"
async fn raw(name: &str) {
    User::query().where_raw(&format!("name = '{name}'")).limit(1).get().await;
    let sql = format!("SELECT * FROM users WHERE name = '{}'", name);
    Database::raw_json(&sql).await;
    db.exec_raw(&("DELETE FROM users WHERE name = ".to_string() + name)).await;
    User::query().where_raw("name = ?").limit(1).get().await;
    User::query().where_raw_with("name = ?", [name]).limit(1).get().await;
    Database::raw_json(&format!("SELECT 1")).await;
}
"#;
    assert_eq!(
        rules(code),
        [
            ("sql-injection", 3),
            ("sql-injection", 5),
            ("sql-injection", 6)
        ]
    );
}

#[test]
fn rows_read_only_to_count_or_test_are_reported() {
    let code = r#"
async fn counts(email: &str, items: Vec<u8>) -> Result<()> {
    if User::query().where_eq("email", email).count().await? > 0 {}
    let taken = User::query().where_eq("email", email).get().await?.len();
    let first = User::query().where_eq("email", email).get().await?.first();
    let some = items.iter().count() > 0;
    let users = User::query().where_eq("email", email).get().await?;
    let n = users.len();
    Ok(())
}
"#;
    assert_eq!(
        rules(code),
        [
            ("count-for-existence", 3),
            ("rows-to-count", 4),
            ("rows-to-first", 5)
        ]
    );
}

#[test]
fn reads_without_bounds_wildcards_orders_and_joins_are_hinted() {
    let code = r#"
async fn reads() {
    User::all().await;
    User::query().get().await;
    User::query().limit(10).get().await;
    User::query().where_like("email", "%@example.com").first().await;
    User::query().where_like("email", "ann%").first().await;
    User::query().order_desc("name").limit(10).get().await;
    User::query().latest().limit(10).get().await;
    Post::query().inner_join("comments", "comments.author_id", "posts.user_id").limit(5).get().await;
}
"#;
    assert_eq!(
        rules(code),
        [
            ("unbounded-read", 3),
            ("unbounded-read", 4),
            ("leading-wildcard", 6),
            ("unindexed-order", 8),
            ("unindexed-order", 9),
            ("unindexed-join", 10),
        ]
    );
}

fn column(name: &str, nullable: bool, primary_key: bool) -> ColumnInfo {
    ColumnInfo {
        name: name.into(),
        data_type: "text".into(),
        nullable,
        default: None,
        primary_key,
        unique: false,
        foreign_key: false,
        extra: None,
    }
}

fn index(name: &str, columns: &[&str]) -> IndexInfo {
    IndexInfo {
        name: name.into(),
        columns: columns.iter().map(|column| column.to_string()).collect(),
        unique: false,
        primary: false,
        whole_columns: true,
    }
}

#[test]
fn the_database_supplies_the_indexes_and_is_checked_against_the_models() {
    let mut catalog = Catalog::new();
    let users = [
        "id",
        "email",
        "name",
        "active",
        "password_hash",
        "balance",
        "created_at",
    ];
    catalog.insert(
        "users".into(),
        Table {
            columns: users
                .iter()
                .map(|name| column(name, *name == "name", *name == "id"))
                .collect(),
            indexes: vec![index("users_name", &["name"])],
            view: false,
        },
    );
    catalog.insert(
        "posts".into(),
        Table {
            columns: ["id", "user_id", "title"]
                .iter()
                .map(|name| column(name, false, *name == "id"))
                .collect(),
            // Only the primary key makes another index redundant: the catalog
            // does not say whether `posts_user_title` is partial.
            indexes: vec![
                index("posts_user", &["user_id"]),
                index("posts_user_title", &["user_id", "title"]),
                index("posts_by_id", &["id"]),
            ],
            view: false,
        },
    );
    let code = r#"
async fn by_name(name: &str) {
    User::query().where_eq("name", name).first().await;
    User::query().where_eq("email", name).first().await;
}
"#;
    let found: Vec<_> = findings(code, Some(&catalog))
        .into_iter()
        .map(|finding| (finding.rule, finding.message))
        .collect();
    let rules: Vec<_> = found.iter().map(|(rule, _)| *rule).collect();
    assert_eq!(
        rules,
        [
            "missing-column",
            "unindexed-filter",
            "nullable-mismatch",
            "sensitive-field",
            "missing-table",
            "float-money",
            "redundant-index",
        ],
        "{found:?}"
    );
    assert!(found[0].1.contains("`User.api_key`"), "{found:?}");
    assert!(
        found[1].1.contains("`email`"),
        "the database has no index on email: {found:?}"
    );
    assert!(found[2].1.contains("`users.name` may be NULL"), "{found:?}");
    assert!(found[4].1.contains("`comments`"), "{found:?}");
    assert!(
        found[6].1.contains("`posts_by_id`") && found[6].1.contains("primary key (id)"),
        "{found:?}"
    );

    // A view's columns all read as nullable: no NULL check holds there.
    catalog.insert(
        "comments".into(),
        Table {
            columns: ["id", "author_id", "text"]
                .iter()
                .map(|name| column(name, true, false))
                .collect(),
            indexes: Vec::new(),
            view: true,
        },
    );
    let comments: Vec<_> = findings(code, Some(&catalog))
        .into_iter()
        .filter(|finding| {
            finding.message.contains("comments") || finding.message.contains("Comment")
        })
        .map(|finding| finding.rule)
        .collect();
    assert_eq!(comments, ["unindexed-foreign-key"], "{comments:?}");
}

/// The `(rule, file, line)` of each finding for `files`, offline.
fn found_in(files: &[(&str, &str)]) -> Vec<(&'static str, String, usize)> {
    let files: Vec<SourceFile> = files
        .iter()
        .map(|(path, code)| SourceFile {
            path: path.into(),
            syntax: syn::parse_file(code).expect("the code parses"),
        })
        .collect();
    let models = read_models(&files);
    analyze(&files, models, None)
        .into_iter()
        .map(|finding| {
            let file = finding.file.to_string_lossy().replace('\\', "/");
            (finding.rule, file, finding.line)
        })
        .collect()
}

#[test]
fn each_bin_program_reads_its_own_models() {
    let first = r#"
#[tideorm::model(table = "items")]
pub struct Item {
    #[tideorm(primary_key)]
    pub id: i64,
    pub name: String,
}

async fn named(name: &str) {
    Item::query().where_eq("name", name).get().await;
    Shared::query().where_eq("code", name).first().await;
}
"#;
    let second = r#"
#[tideorm::model(table = "items")]
#[index("title")]
pub struct Item {
    #[tideorm(primary_key)]
    pub id: i64,
    pub title: String,
}

async fn titled(title: &str) {
    Item::query().where_eq("title", title).first().await;
}
"#;
    let library = r#"
#[tideorm::model(table = "shared")]
pub struct Shared {
    #[tideorm(primary_key)]
    pub id: i64,
    #[index]
    pub code: String,
}

mod admin {
    #[tideorm::model(table = "admin_accounts")]
    pub struct Account { #[tideorm(primary_key)] pub id: i64 }
}
mod api {
    #[tideorm::model(table = "api_accounts")]
    pub struct Account { #[tideorm(primary_key)] pub id: i64 }
}
"#;
    let app = r#"
async fn ambiguous() {
    Account::query().where_eq("nope", 1).get().await;
    Item::query().where_eq("name", "x").get().await;
}
"#;
    let found = found_in(&[
        ("src/bin/first.rs", first),
        ("src/bin/second/main.rs", second),
        ("src/models.rs", library),
        ("src/app.rs", app),
    ]);
    // `name` is first.rs's own column, which no index of `items` leads
    // (second.rs's `title` index is the table's); `Shared` is the library's;
    // two library `Account`s are not guessed between; `src/app.rs` sees no
    // program's `Item`.
    assert_eq!(
        found,
        [("unindexed-filter", "src/bin/first.rs".to_string(), 10)]
    );
}

#[test]
fn offline_indexes_include_the_ones_migrations_create() {
    let models = r#"
#[tideorm::model(table = "customers")]
pub struct Customer {
    #[tideorm(primary_key)]
    pub id: i64,
    pub email: String,
    pub status: String,
    pub region: String,
    pub code: String,
    pub tier: String,
}
"#;
    let migration = r#"
#[async_trait]
impl Migration for CreateCustomers {
    async fn up(&self, schema: &mut Schema) -> tideorm::Result<()> {
        schema
            .create_table_if_not_exists("customers", |t| {
                t.big_increments("id");
                t.string("email").not_null().unique();
                t.string("status");
                t.index(&["status", "region"]);
            })
            .await?;
        schema.create_index("customers", "idx_code", &["code"]).await?;
        Ok(())
    }

    async fn down(&self, schema: &mut Schema) -> tideorm::Result<()> {
        schema.alter_table("customers", |t| { t.add_index(&["tier"]); }).await?;
        Ok(())
    }
}
"#;
    let app = r#"
async fn served(value: &str) {
    Customer::query().where_eq("email", value).first().await;
    Customer::query().where_eq("status", value).get().await;
    Customer::query().where_eq("code", value).first().await;
}
async fn region(value: &str) {
    Customer::query().where_eq("region", value).get().await;
}
async fn tier(value: &str) {
    Customer::query().where_eq("tier", value).get().await;
}
"#;
    let found = found_in(&[
        ("src/models.rs", models),
        ("src/migrations/m_0001.rs", migration),
        ("src/app.rs", app),
    ]);
    // `region` is second in its index; `down` puts back no index `up` made.
    assert_eq!(
        found,
        [
            ("unindexed-filter", "src/app.rs".to_string(), 8),
            ("unindexed-filter", "src/app.rs".to_string(), 11),
        ]
    );
}

#[path = "analyze_flow_tests.rs"]
mod flow;
