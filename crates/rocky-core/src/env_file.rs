use std::collections::BTreeMap;
use std::path::Path;

use crate::error::Error;

/// Parses a `.env` file into a map without touching the process environment.
pub fn load(path: &Path) -> Result<BTreeMap<String, String>, Error> {
    let to_error = |error| Error::EnvFile {
        path: path.to_path_buf(),
        // An I/O error's message is the I/O error's own, so use it directly to avoid
        // printing the same cause twice.
        source: match error {
            dotenvy::Error::Io(io) => Box::new(io),
            other => Box::new(other),
        },
    };
    dotenvy::from_path_iter(path)
        .map_err(to_error)?
        .map(|item| item.map_err(to_error))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quotes_comments_and_export() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env");
        std::fs::write(
            &path,
            "# comment\nA=plain\nB=\"quoted value\"\nexport C='single'\n",
        )
        .unwrap();
        let vars = load(&path).unwrap();
        assert_eq!(vars["A"], "plain");
        assert_eq!(vars["B"], "quoted value");
        assert_eq!(vars["C"], "single");
        assert_eq!(vars.len(), 3);
    }

    #[test]
    fn missing_file_errors() {
        let err = load(Path::new("/nonexistent/.env")).unwrap_err();
        assert!(matches!(err, Error::EnvFile { .. }));
    }

    #[test]
    fn malformed_line_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env");
        std::fs::write(&path, "A=\"unclosed\n").unwrap();
        let err = load(&path).unwrap_err();
        assert!(matches!(err, Error::EnvFile { .. }), "{err}");
    }
}
