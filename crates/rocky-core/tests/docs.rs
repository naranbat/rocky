use std::path::{Path, PathBuf};

fn markdown_files(root: &Path) -> Vec<PathBuf> {
    let mut files = vec![root.join("README.md"), root.join("skills/rocky/SKILL.md")];
    files.extend(
        std::fs::read_dir(root.join("docs"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "md")),
    );
    files
}

#[test]
fn documentation_yaml_blocks_parse() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut blocks = 0;
    for path in markdown_files(&root) {
        let text = std::fs::read_to_string(&path).unwrap();
        let mut yaml = None;
        for line in text.lines() {
            match (&mut yaml, line.trim()) {
                (None, "```yaml") => yaml = Some(String::new()),
                (Some(block), "```") => {
                    serde_yaml::from_str::<serde_yaml::Value>(block)
                        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
                    yaml = None;
                    blocks += 1;
                }
                (Some(block), _) => {
                    block.push_str(line);
                    block.push('\n');
                }
                (None, _) => {}
            }
        }
        assert!(yaml.is_none(), "unclosed YAML block in {}", path.display());
    }
    assert!(blocks > 0);
}

#[test]
fn project_schema_is_valid_json() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schema/rocky.schema.json");
    let text = std::fs::read_to_string(&path).unwrap();
    let schema: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        schema["$schema"],
        "https://json-schema.org/draft/2020-12/schema"
    );
    assert!(schema["$defs"]["task"].is_object());
    assert!(schema["$defs"]["job"].is_object());

    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schema/job.schema.json");
    let job: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(job["$ref"], "rocky.schema.json#/$defs/job");
}
