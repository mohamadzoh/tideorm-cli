//! The table and column names the model derive makes up, by its own rules
//! (`tideorm-macros`' `case.rs` and `pluralize`), which the CLI has to match
//! to find a model's table and columns. Not the generators' naming, which
//! writes the names it picks into the code.

use unicode_segmentation::UnicodeSegmentation;

/// The column of a field without `column = ".."`: `type_` is `type_`,
/// `sha256` is `sha_256`.
pub(crate) fn column_name(field: &str) -> String {
    snake_case(field)
}

/// The table of a model without `table = ".."`: `UserSettings` is
/// `user_settingses`, `S3Object` is `s_3_objects`.
pub(crate) fn table_name(model: &str) -> String {
    pluralize(&snake_case(model))
}

fn snake_case(ident: &str) -> String {
    words(ident)
        .into_iter()
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join("_")
}

#[derive(Clone, Copy)]
enum Class {
    Lower,
    Upper,
    Digit,
    Other,
}

fn classify(grapheme: &str) -> Class {
    if grapheme.chars().all(|c| c.is_ascii_digit()) {
        return Class::Digit;
    }
    let (upper, lower) = (grapheme.to_uppercase(), grapheme.to_lowercase());
    if upper == lower {
        Class::Other
    } else if grapheme == upper {
        Class::Upper
    } else if grapheme == lower {
        Class::Lower
    } else {
        Class::Other
    }
}

/// Splits at `_` (dropped, empty words kept), between a lowercase and an
/// uppercase letter, between a letter and a digit, and before the last
/// capital of an acronym, over grapheme clusters.
fn words(ident: &str) -> Vec<&str> {
    use Class::{Digit, Lower, Upper};

    let graphemes: Vec<(usize, &str)> = ident.grapheme_indices(true).collect();
    let classes: Vec<Class> = graphemes.iter().map(|&(_, g)| classify(g)).collect();
    let mut words = Vec::new();
    let mut start = 0;
    for (i, &(offset, grapheme)) in graphemes.iter().enumerate() {
        if grapheme == "_" {
            words.push(&ident[start..offset]);
            start = offset + 1;
            continue;
        }
        let boundary_follows = matches!(
            (classes[i], classes.get(i + 1), classes.get(i + 2)),
            (Lower, Some(Upper | Digit), _)
                | (Upper, Some(Digit), _)
                | (Digit, Some(Lower | Upper), _)
                | (Upper, Some(Upper), Some(Lower))
        );
        if boundary_follows {
            let end = graphemes[i + 1].0;
            words.push(&ident[start..end]);
            start = end;
        }
    }
    words.push(&ident[start..]);
    words
}

fn pluralize(word: &str) -> String {
    let irregular = match word {
        "person" => "people",
        "man" => "men",
        "woman" => "women",
        "child" => "children",
        "tooth" => "teeth",
        "foot" => "feet",
        "mouse" => "mice",
        "goose" => "geese",
        "leaf" => "leaves",
        "knife" => "knives",
        "life" => "lives",
        "wife" => "wives",
        "wolf" => "wolves",
        "calf" => "calves",
        "half" => "halves",
        "loaf" => "loaves",
        "self" => "selves",
        "shelf" => "shelves",
        "thief" => "thieves",
        "quiz" => "quizzes",
        "fez" => "fezzes",
        _ => "",
    };
    if !irregular.is_empty() {
        irregular.to_string()
    } else if ["s", "x", "z", "ch", "sh"]
        .iter()
        .any(|ending| word.ends_with(ending))
    {
        format!("{word}es")
    } else if word.ends_with('y') && !["ay", "ey", "oy", "uy"].iter().any(|e| word.ends_with(e)) {
        format!("{}ies", &word[..word.len() - 1])
    } else {
        format!("{word}s")
    }
}

#[cfg(test)]
#[path = "../tests/unit/derive_names_tests.rs"]
mod tests;
