/// How a step or state name is checked, worded for error messages.
pub const RULE: &str = "must start with a letter or `_`, contain only letters, digits and `_`, and not be `steps` or `states`";

/// Whether `name` can name a step or state: an identifier (`[A-Za-z_][A-Za-z0-9_]*`) other than
/// `steps` and `states`, which the run sets itself.
pub fn is_valid(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !matches!(name, "steps" | "states")
}

/// How a `vars` or loop `as` name is checked, worded for error messages.
pub const VAR_RULE: &str =
    "must start with an uppercase letter or `_` and contain only uppercase letters, digits and `_`";

/// Whether `name` can name a `vars` entry or a loop variable: `[A-Z_][A-Z0-9_]*`.
pub fn is_valid_var(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_uppercase() || c == '_')
        && chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_identifiers() {
        for name in ["A", "_x", "USER_ID2", "item"] {
            assert!(is_valid(name), "{name}");
        }
    }

    #[test]
    fn rejects_non_identifiers_and_reserved_names() {
        for name in ["", "1A", "a.b", "a-b", "a}", "a b", "é", "steps", "states"] {
            assert!(!is_valid(name), "{name}");
        }
    }

    #[test]
    fn var_names_are_uppercase() {
        for name in ["A", "_X", "USER_ID2", "ITEM"] {
            assert!(is_valid_var(name), "{name}");
        }
        for name in ["", "a", "Item", "1A", "A.B", "A-B", "É", "steps"] {
            assert!(!is_valid_var(name), "{name}");
        }
    }
}
