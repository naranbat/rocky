//! Paths shown and typed with `~` for the home folder.

use std::path::{Path, PathBuf};

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// `path` with the home folder written as `~`.
pub fn tilde(path: &Path) -> String {
    match home().and_then(|home| path.strip_prefix(home).ok().map(Path::to_path_buf)) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// A typed path, with a leading `~` read as the home folder.
pub fn expand(typed: &str) -> PathBuf {
    let typed = typed.trim();
    let rest = match typed {
        "~" => Some(""),
        _ => typed.strip_prefix("~/"),
    };
    match (rest, home()) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(typed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_the_home_folder() {
        let home = home().unwrap();
        assert_eq!(expand("~/work"), home.join("work"));
        assert_eq!(tilde(&home.join("work")), "~/work");
        assert_eq!(tilde(&home), "~");
        assert_eq!(expand("/tmp/x"), Path::new("/tmp/x"));
    }
}
