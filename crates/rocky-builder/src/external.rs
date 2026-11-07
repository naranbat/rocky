//! Editing a file's text in `$VISUAL` or `$EDITOR` (else `vi`), while the builder waits.

use std::process::Command;

/// The editor to run: `$VISUAL`, `$EDITOR`, or `vi`.
pub fn editor() -> String {
    std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string())
}

/// Opens `text` in `editor`, in a temporary file named like `name`, and returns the text it
/// was saved with.
pub fn edit(editor: &str, name: &str, text: &str) -> Result<String, String> {
    let path = std::env::temp_dir().join(format!("rocky-{}-{name}", std::process::id()));
    std::fs::write(&path, text)
        .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    // Through `sh`, so an editor with arguments (`code --wait`) works.
    let status = Command::new("sh")
        .arg("-c")
        .arg(format!("{editor} \"$1\""))
        .arg("sh")
        .arg(&path)
        .status();
    let edited = std::fs::read_to_string(&path);
    let _ = std::fs::remove_file(&path);
    match status {
        Ok(status) if status.success() => edited.map_err(|error| error.to_string()),
        Ok(status) => Err(format!("{editor} exited with {status}")),
        Err(error) => Err(format!("cannot run {editor}: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_what_the_editor_saved() {
        assert_eq!(edit("true", "a.yaml", "x: 1\n"), Ok("x: 1\n".to_string()));
        let append = "sh -c 'echo y: 2 >> \"$0\"'";
        assert_eq!(
            edit(append, "b.yaml", "x: 1\n"),
            Ok("x: 1\ny: 2\n".to_string())
        );
        assert!(edit("false", "c.yaml", "").is_err());
    }
}
