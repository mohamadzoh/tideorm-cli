//! How queries are followed: shadowed variables, fragments, scopes, loops
//! that page or batch, seed code; and the model fields the derive reads.

use super::{MODELS, SourceFile, found_in, read_models, rules};

#[test]
fn a_shadowed_variable_is_not_the_outer_model() {
    let code = r#"
async fn each(names: Vec<String>) -> Result<()> {
    let user = User::find(1).await?;
    for user in names.clone() {
        user.save().await?;
    }
    for id in [1, 2] {
        match lookup(id) {
            Some(user) => user.update().await?,
            None => {}
        }
        join_all(names.iter().map(|user| async move { user.delete().await }));
    }
    for id in [3] {
        user.save().await?;
    }
    Ok(())
}
"#;
    // Only the last loop writes the model `user` names outside it.
    assert_eq!(rules(code), [("write-in-loop", 15)]);
}

#[test]
fn fragments_are_checked_for_columns_but_not_indexes() {
    let code = r#"
fn named(query: QueryBuilder<User>, name: &str) -> QueryBuilder<User> {
    query.where_eq("name", name)
}
fn active() -> QueryBuilder<User> {
    User::query().where_eq("name", "x")
}
async fn listed(query: QueryBuilder<User>) {
    query.get().await;
}
fn typo(query: QueryBuilder<User>) -> QueryBuilder<User> {
    query.where_eq("nmae", 1)
}
async fn applied() {
    User::query().apply(|query: QueryBuilder<User>| query.where_eq("nmae", 1)).first().await;
}
"#;
    assert_eq!(
        rules(code),
        [("unknown-column", 12), ("unknown-column", 15)]
    );
}

#[test]
fn a_scope_or_an_unread_condition_may_be_what_an_index_serves() {
    let code = r#"
async fn scoped(name: &str) {
    User::query().active().where_eq("name", name).get().await;
    User::query().where_has("posts", |q| q).where_eq("name", name).get().await;
    User::query().where_raw("id = ?").where_eq("name", name).get().await;
    User::query().when(true, |q| q.where_eq("id", 1)).where_eq("name", name).get().await;
    User::query().select(&["id"]).with_trashed().where_eq("name", name).get().await;
}
"#;
    assert_eq!(rules(code), [("unindexed-filter", 7)]);
}

#[test]
fn loops_over_pages_or_batches_are_not_n_plus_one() {
    let code = r#"
async fn pages() -> Result<()> {
    for page in 1..=3 {
        User::query().where_eq("email", "a").paginate(page, 10).await?;
        User::paginate(page, 10).await?;
    }
    loop {
        let batch = Post::query().where_eq("user_id", 1).limit(100).get().await?;
        if batch.is_empty() {
            break;
        }
    }
    while let Some(id) = next() {
        Post::query().where_eq("user_id", id).limit(1).get().await?;
    }
    Ok(())
}
"#;
    // A `while let` goes over items like a `for`; a limited batch's length is
    // not a count of the table.
    assert_eq!(rules(code), [("n-plus-one", 14)]);
}

#[test]
fn seeders_may_write_a_row_at_a_time() {
    let seeder = r#"
async fn run() -> Result<()> {
    for id in 1..=3 {
        Comment::create(Comment { id, ..Default::default() }).await?;
        Post::find(id).await?;
    }
    Ok(())
}
"#;
    let found: Vec<_> = found_in(&[
        ("src/models.rs", MODELS),
        ("src/seeders/comment_seeder.rs", seeder),
    ])
    .into_iter()
    .filter(|(_, file, _)| file.contains("seeders"))
    .map(|(rule, _, line)| (rule, line))
    .collect();
    assert_eq!(found, [("n-plus-one", 5)]);
}

#[test]
fn relations_are_read_from_the_field_type() {
    let models = r#"
#[tideorm::model(table = "employees")]
pub struct Employee {
    #[tideorm(primary_key)]
    pub id: i64,
    pub manager_id: Option<i64>,
    pub api_token: String,
    pub secret_count: i32,
    #[serde(skip)]
    pub password: String,
    #[tideorm(foreign_key = "manager_id")]
    pub manager: SelfRef<Employee>,
    #[tideorm(foreign_key = "manager_id")]
    pub reports: SelfRefMany<Employee>,
    #[tideorm(morph_name = "imageable")]
    pub images: Option<MorphMany<Image>>,
}

#[tideorm::model(table = "images")]
pub struct Image {
    #[tideorm(primary_key)]
    pub id: i64,
    #[tideorm(morph_name = "imageable")]
    pub imageable: MorphTo<Imageable>,
}
"#;
    let files = vec![SourceFile {
        path: "src/models.rs".into(),
        syntax: syn::parse_file(models).unwrap(),
    }];
    let read = read_models(&files);
    let kinds: Vec<_> = read
        .iter()
        .flat_map(|model| &model.fields)
        .filter_map(|field| {
            let relation = field.relation.as_ref()?;
            Some((field.name.as_str(), relation.kind, relation.model.as_str()))
        })
        .collect();
    assert_eq!(
        kinds,
        [
            ("manager", "self_ref", "Employee"),
            ("reports", "self_ref_many", "Employee"),
            ("images", "morph_many", "Image"),
            ("imageable", "morph_to", "Imageable"),
        ]
    );
    assert!(
        read[1].column("imageable").is_none(),
        "a relation is no column"
    );

    // An `i32` and a field serde skips are no secrets; children are read by
    // `manager_id`, which no index leads.
    let found = found_in(&[("src/models.rs", models)]);
    assert_eq!(
        found,
        [
            ("sensitive-field", "src/models.rs".to_string(), 7),
            ("unindexed-foreign-key", "src/models.rs".to_string(), 14),
        ]
    );
}

#[test]
fn index_attributes_are_read_in_every_form() {
    let code = r#"
#[tideorm::model(table = "things")]
#[index("a,b")]
#[index("c", "d")]
#[index(e, f)]
#[unique_index(name = "g_h", columns = "g, h")]
pub struct Thing {
    #[tideorm(primary_key)]
    pub id: i64,
}
"#;
    let files = vec![SourceFile {
        path: "src/models.rs".into(),
        syntax: syn::parse_file(code).unwrap(),
    }];
    assert_eq!(
        read_models(&files)[0].indexes,
        [
            vec!["id"],
            vec!["a", "b"],
            vec!["c", "d"],
            vec!["e", "f"],
            vec!["g", "h"]
        ]
    );
}

#[test]
fn an_or_term_is_served_by_any_filter_anded_into_it() {
    let code = r#"
async fn branches(name: &str) {
    User::query().begin_or().or_where_eq("email", name).and_where_eq("name", name).or_where_eq("id", 1).end_or().get().await;
    User::query().begin_or().or_where_eq("name", name).and_where_eq("active", true).or_where_eq("email", name).end_or().get().await;
}
"#;
    // `(email AND name) OR id` reads the email and id indexes; the term
    // `name AND active` has none.
    assert_eq!(rules(code), [("unindexed-filter", 4)]);
}
