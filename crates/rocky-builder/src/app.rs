//! The builder: the home screen until a project is open, then its editor.

use std::path::{Path, PathBuf};
use std::time::Duration;

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::editor::{self, Editor};
use crate::files::Files;
use crate::home_view::{self, Home};
use crate::recent::Recent;

pub struct App {
    pub quit: bool,
    recent: Recent,
    home: Home,
    editor: Option<Editor>,
}

impl App {
    /// Starts on the home screen, or on the project at `path` if given.
    pub fn new(path: Option<PathBuf>, recent: Recent) -> App {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let mut app = App {
            quit: false,
            recent,
            home: Home::new(&cwd),
            editor: None,
        };
        if let Some(path) = path {
            app.open(&path);
        }
        app
    }

    fn open(&mut self, path: &Path) {
        match Files::open(path) {
            Ok(files) => {
                if let Err(error) = self.recent.record(path) {
                    self.home.message = Some(error);
                }
                self.editor = Some(Editor::new(files));
            }
            Err(error) => self.home.message = Some(error),
        }
    }

    pub fn handle(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        match &mut self.editor {
            Some(editor) => {
                if let editor::Action::Quit = editor.key(key) {
                    self.quit = true;
                }
            }
            None => match self.home.key(key, &mut self.recent) {
                home_view::Action::None => {}
                home_view::Action::Open(path) => self.open(&path),
                home_view::Action::Quit => self.quit = true,
            },
        }
    }

    /// Keeps a run going; returns how long to wait for a key.
    pub fn tick(&mut self) -> Duration {
        self.editor
            .as_mut()
            .map_or(Duration::from_secs(1), Editor::tick)
    }

    pub fn draw(&self, frame: &mut Frame) {
        match &self.editor {
            Some(editor) => editor.draw(frame),
            None => self.home.draw(frame, &self.recent),
        }
    }

    /// A file to open in `$EDITOR`: its name and text.
    pub fn take_external(&mut self) -> Option<(String, String)> {
        self.editor.as_mut()?.take_external()
    }

    pub fn finish_external(&mut self, edited: Result<String, String>) {
        if let Some(editor) = &mut self.editor {
            editor.finish_external(edited);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::*;

    fn examples() -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        std::fs::canonicalize(dir).unwrap()
    }

    /// A copy of `examples/<name>` in a temporary directory.
    fn copy(name: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let from = examples().join(name);
        std::fs::create_dir(dir.path().join("jobs")).unwrap();
        for entry in std::fs::read_dir(&from)
            .unwrap()
            .chain(std::fs::read_dir(from.join("jobs")).unwrap())
        {
            let path = entry.unwrap().path();
            if path.is_file() {
                let relative = path.strip_prefix(&from).unwrap();
                std::fs::copy(&path, dir.path().join(relative)).unwrap();
            }
        }
        dir
    }

    fn screen(app: &App) -> String {
        let mut terminal = Terminal::new(TestBackend::new(140, 40)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    + "\n"
            })
            .collect()
    }

    fn press(app: &mut App, code: KeyCode) {
        app.handle(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn keys(app: &mut App, text: &str) {
        for c in text.chars() {
            press(app, KeyCode::Char(c));
        }
    }

    fn ctrl(app: &mut App, c: char) {
        app.handle(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
    }

    #[test]
    fn home_shows_each_pane() {
        let mut app = App::new(None, Recent::none());
        let shown = screen(&app);
        assert!(shown.contains("New project"), "{shown}");
        assert!(shown.contains("Will create"));
        press(&mut app, KeyCode::Down);
        assert!(screen(&app).contains("A folder with a rocky.yaml"));
        press(&mut app, KeyCode::Char('q'));
        assert!(app.quit);
    }

    #[test]
    fn creates_a_project_and_opens_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::new(None, Recent::none());
        press(&mut app, KeyCode::Char('n'));
        for _ in 0..200 {
            press(&mut app, KeyCode::Backspace);
        }
        keys(&mut app, &format!("{}/fresh", dir.path().display()));
        press(&mut app, KeyCode::Enter);
        let shown = screen(&app);
        assert!(shown.contains("1 Jobs"), "{shown}");
        assert!(shown.contains("example"));
        assert!(shown.contains("✓ valid"));
        assert!(dir.path().join("fresh/jobs/example.yaml").is_file());
    }

    #[test]
    fn opens_every_example_without_changing_it() {
        for name in ["env", "httpbin", "pipe", "trigger", "values"] {
            let mut app = App::new(Some(examples().join(name)), Recent::none());
            for code in [
                KeyCode::Down,
                KeyCode::Right,
                KeyCode::Enter,
                KeyCode::Tab,
                KeyCode::Char('q'),
            ] {
                press(&mut app, code);
                screen(&app);
            }
            for tab in "234".chars() {
                press(&mut app, KeyCode::Char(tab));
                screen(&app);
            }
            let editor = app.editor.as_ref().unwrap();
            assert_eq!(editor.files.unsaved(), 0, "{name}");
            assert!(editor.problem.is_none(), "{name}: {:?}", editor.problem);
        }
    }

    #[test]
    fn jobs_tab_moves_a_task_and_saves() {
        let dir = copy("httpbin");
        let mut app = App::new(Some(dir.path().to_path_buf()), Recent::none());
        let shown = screen(&app);
        assert!(shown.contains("TASKS · RUN IN THIS ORDER"), "{shown}");
        assert!(shown.contains("conditional"));
        // The first job listed is the first scheduled one: conditional.
        press(&mut app, KeyCode::Right);
        press(&mut app, KeyCode::Char('J'));
        assert!(screen(&app).contains("1 unsaved"));
        ctrl(&mut app, 's');
        let project = rocky_core::project::load(&dir.path().join("rocky.yaml")).unwrap();
        let names: Vec<_> = project.jobs["conditional"]
            .tasks
            .iter()
            .map(|task| task.name.as_str())
            .collect();
        assert_eq!(names, ["big", "stats", "warn"]);
        press(&mut app, KeyCode::Char('q'));
        assert!(app.quit);
    }

    #[test]
    fn yaml_editor_checks_edits_and_undoes_them() {
        let dir = copy("httpbin");
        let mut app = App::new(Some(dir.path().to_path_buf()), Recent::none());
        // conditional's settings: `schedule: hourly: 15`.
        press(&mut app, KeyCode::Enter);
        let shown = screen(&app);
        assert!(shown.contains("Job settings"), "{shown}");
        assert!(shown.contains("✓ schedule ok · hourly :15"), "{shown}");
        press(&mut app, KeyCode::Char('i'));
        // The cursor starts at the end of `  hourly: 15 ... # comment`: go to `15`.
        press(&mut app, KeyCode::Home);
        for _ in 0.."  hourly: 1".len() {
            press(&mut app, KeyCode::Right);
        }
        press(&mut app, KeyCode::Delete);
        keys(&mut app, "70");
        let shown = screen(&app);
        assert!(shown.contains("✗"), "{shown}");
        assert!(shown.contains("● modified"));
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('u'));
        let shown = screen(&app);
        assert!(shown.contains("✓ schedule ok · hourly :15"), "{shown}");
        assert!(!shown.contains("● modified"));

        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Tab);
        let shown = screen(&app);
        assert!(shown.contains("stats highlighted"), "{shown}");
        press(&mut app, KeyCode::Char('q'));
        assert!(screen(&app).contains("1 Jobs"));
    }

    #[test]
    fn adds_a_job() {
        let dir = copy("trigger");
        let mut app = App::new(Some(dir.path().to_path_buf()), Recent::none());
        press(&mut app, KeyCode::Char('+'));
        keys(&mut app, "added");
        press(&mut app, KeyCode::Enter);
        ctrl(&mut app, 's');
        let project = rocky_core::project::load(&dir.path().join("rocky.yaml")).unwrap();
        assert!(project.jobs.contains_key("added"));
    }

    #[test]
    fn debug_runs_a_job() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("jobs")).unwrap();
        std::fs::write(
            dir.path().join("jobs/hello.yaml"),
            "tasks:\n  greet:\n    cmd: [echo, '{\"hi\": 1}']\n  never:\n    when: {this: tasks.greet.stdout.hi, eq: 2}\n    cmd: [echo]\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("rocky.yaml"),
            "jobs:\n  hello: jobs/hello.yaml\n",
        )
        .unwrap();
        let mut app = App::new(Some(dir.path().to_path_buf()), Recent::none());
        press(&mut app, KeyCode::Char('d'));
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !screen(&app).contains("FINISHED") && std::time::Instant::now() < deadline {
            app.tick();
            std::thread::sleep(Duration::from_millis(20));
        }
        let shown = screen(&app);
        assert!(shown.contains("FINISHED"), "{shown}");
        assert!(shown.contains("✓  greet"), "{shown}");
        assert!(shown.contains("skipped"));
        assert!(shown.contains("→ false"));
        assert!(shown.contains("\"hi\": 1"), "{shown}");
        press(&mut app, KeyCode::Esc);
        assert!(screen(&app).contains("1 Jobs"));
    }
}
