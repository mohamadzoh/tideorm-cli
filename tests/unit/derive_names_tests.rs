use super::{column_name, table_name};

#[test]
fn columns_are_named_as_the_derive_names_them() {
    for (field, column) in [
        ("user_id", "user_id"),
        ("sha256", "sha_256"),
        ("type_", "type_"),
        ("_private", "_private"),
        ("a__b", "a__b"),
        ("createdAt", "created_at"),
        ("HTTPRequest", "http_request"),
    ] {
        assert_eq!(column_name(field), column, "{field}");
    }
}

#[test]
fn tables_are_named_as_the_derive_names_them() {
    for (model, table) in [
        ("User", "users"),
        ("UserSettings", "user_settingses"),
        ("Media", "medias"),
        ("Category", "categories"),
        ("Day", "days"),
        ("Box", "boxes"),
        ("Person", "people"),
        ("S3Object", "s_3_objects"),
        ("XMLParser", "xml_parsers"),
    ] {
        assert_eq!(table_name(model), table, "{model}");
    }
}
