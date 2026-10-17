/// How a variable name is checked, worded for error messages.
pub const RULE: &str = "must start with a letter or `_`, contain only letters, digits and `_`, and not be `steps` or `states`";

/// Whether `name` can name a variable: an identifier (`[A-Za-z_][A-Za-z0-9_]*`) other than
/// `steps` and `states`, which the run sets itself.
pub fn is_valid(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !matches!(name, "steps" | "states")
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
}
