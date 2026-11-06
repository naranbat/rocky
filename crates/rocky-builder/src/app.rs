//! The window: top bar, sidebar, status bar, and the job / project / YAML views.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use eframe::egui::{self, Key, KeyboardShortcut, Modifiers, RichText, Stroke};

use crate::doc::{self, Doc};
use crate::edit::Edit;
use crate::icons::Icon;
use crate::job;
use crate::project;
use crate::recent::Recent;
use crate::run::Run;
use crate::run_view::Shown;
use crate::scheduler::{self, Scheduler};
use crate::scope;
use crate::sidebar::{self, Selection};
use crate::start;
use crate::theme;
use crate::validate::Validation;
use crate::widgets;
use crate::yaml_view;

const SAVE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);

pub struct App {
    doc: Option<Doc>,
    /// The start screen, shown while no project is open.
    start: start::Start,
    recent: Recent,
    selection: Selection,
    validation: Validation,
    show_yaml: bool,
    /// Task file previews, re-read after each save or validate.
    previews: HashMap<PathBuf, String>,
    /// The last save or open message, for the status bar; `Err` in red.
    message: Option<Result<String, String>>,
    title: String,
    /// The latest run, finished or not.
    run: Option<Run>,
    /// The scheduler, while started from here, and the saved text it started with.
    scheduler: Option<(Scheduler, String)>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext, path: Option<PathBuf>) -> App {
        App::with_context(&cc.egui_ctx, path, Recent::load())
    }

    fn with_context(ctx: &egui::Context, path: Option<PathBuf>, recent: Recent) -> App {
        theme::apply(ctx);
        let mut app = App {
            doc: None,
            start: start::Start::default(),
            recent,
            selection: Selection::Project,
            validation: Validation::default(),
            show_yaml: false,
            previews: HashMap::new(),
            message: None,
            title: String::new(),
            run: None,
            scheduler: None,
        };
        if let Some(path) = path {
            app.open(&path);
        }
        app
    }

    fn open(&mut self, path: &Path) {
        match Doc::open(path) {
            Ok(doc) => {
                self.selection = doc
                    .job_names()
                    .first()
                    .map_or(Selection::Project, |name| Selection::Job(name.clone()));
                self.doc = Some(doc);
                self.validation = Validation::default();
                self.previews.clear();
                self.message = None;
                self.run = None;
                self.scheduler = None;
                if let Err(error) = self.recent.record(path) {
                    self.message = Some(Err(error));
                }
            }
            Err(error) => self.message = Some(Err(format!("{error:#}"))),
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.draw(ui);
    }
}

impl App {
    fn draw(&mut self, ui: &mut egui::Ui) {
        let Some(doc) = self.doc.as_mut() else {
            let error = match &self.message {
                Some(Err(error)) => Some(error.as_str()),
                _ => None,
            };
            // Projects may have changed, or started a scheduler, while away.
            let focused =
                ui.input(|input| input.events.contains(&egui::Event::WindowFocused(true)));
            if focused {
                self.recent.refresh();
            }
            let picked = self.start.show(ui, &self.recent, error);
            let saved = match picked {
                Some(start::Pick::Open(path)) => {
                    self.open(&path);
                    Ok(())
                }
                Some(start::Pick::Created(path)) => {
                    self.message = Some(Ok(format!("created {}", path.display())));
                    self.recent.record(&path)
                }
                Some(start::Pick::Forget(path)) => self.recent.remove(&path),
                None => Ok(()),
            };
            if let Err(error) = saved {
                self.message = Some(Err(error));
            }
            const WELCOME: &str = "Welcome to Rocky";
            if self.title != WELCOME {
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::Title(WELCOME.to_string()));
                self.title = WELCOME.to_string();
            }
            return;
        };
        let text = match doc.text() {
            Ok(text) => text,
            Err(error) => format!("# cannot write YAML: {error}"),
        };
        let dirty = doc.is_dirty(&text);
        let mut validate_now = false;
        if let Some(run) = &mut self.run {
            run.poll();
            if run.is_running() {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(100));
            }
        }
        if let Some((scheduler, _)) = &mut self.scheduler {
            scheduler.poll();
            if scheduler.is_running() {
                // New log lines, and the next-run countdown.
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(500));
            }
        }
        let running = self.run.as_ref().is_some_and(Run::is_running);
        let can_run = self.validation.is_valid() && !running;
        let mut start: Option<Option<String>> = None;
        let mut save = ui.input_mut(|input| input.consume_shortcut(&SAVE));

        egui::Panel::top("top")
            .frame(
                egui::Frame::new()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin::symmetric(16, 10)),
            )
            .show(ui, |ui| {
                let actions = top_bar(
                    ui,
                    doc,
                    &self.selection,
                    &self.validation,
                    dirty,
                    &mut self.show_yaml,
                    RunState {
                        can_run,
                        running,
                        scheduling: self.scheduler.as_ref().is_some_and(|(s, _)| s.is_running()),
                    },
                );
                save |= actions.save;
                validate_now |= actions.validate;
                if actions.run {
                    start = Some(None);
                }
                if actions.stop
                    && let Some(run) = &mut self.run
                {
                    run.stop();
                }
            });
        if let Some(Err(error)) = &self.validation.result {
            egui::Panel::top("errors")
                .frame(
                    egui::Frame::new()
                        .fill(theme::ERROR_BG)
                        .stroke(Stroke::new(1.0, theme::ERROR.gamma_multiply(0.5)))
                        .inner_margin(egui::Margin::symmetric(16, 8)),
                )
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        widgets::icon(ui, Icon::Warning, 12.0, theme::ERROR);
                        ui.label(RichText::new(&error.message).color(theme::ERROR));
                    });
                });
        }
        egui::Panel::bottom("status")
            .frame(
                egui::Frame::new()
                    .fill(theme::SIDEBAR)
                    .inner_margin(egui::Margin::symmetric(16, 6)),
            )
            .show(ui, |ui| {
                let scheduler = self.scheduler.as_ref().map(|(s, text)| (s, text.as_str()));
                match status_bar(ui, doc, &self.validation, &self.message, scheduler) {
                    Some(SchedulerAction::Start) => {
                        let started = scheduler::find_rocky()
                            .ok_or_else(|| {
                                "can't find the `rocky` binary next to the builder or on PATH; \
                                 install it with `cargo install --path crates/rocky`"
                                    .to_string()
                            })
                            .and_then(|rocky| {
                                Scheduler::start(&rocky, &doc.path)
                                    .map_err(|error| format!("{error:#}"))
                            });
                        match started {
                            Ok(started) => {
                                self.scheduler = Some((started, doc.saved().to_string()));
                                self.message = Some(Ok("scheduler started".to_string()));
                            }
                            Err(error) => self.message = Some(Err(error)),
                        }
                    }
                    Some(SchedulerAction::Stop) => {
                        if let Some((scheduler, _)) = &mut self.scheduler {
                            scheduler.stop();
                        }
                    }
                    None => {}
                }
            });
        let mut changed = false;
        egui::Panel::left("sidebar")
            .default_size(240.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::SIDEBAR)
                    .inner_margin(egui::Margin::symmetric(12, 8)),
            )
            .show(ui, |ui| {
                let scheduler = self.scheduler.as_ref().map(|(s, _)| s);
                changed |= sidebar::show(
                    ui,
                    doc,
                    &mut self.selection,
                    self.validation.problem(),
                    scheduler,
                )
            });

        let jobs = doc.job_names();
        let dir = doc.dir().to_path_buf();
        let job = match &self.selection {
            Selection::Job(name) => name.clone(),
            Selection::Project => String::new(),
        };
        let scope = scope::base(&doc.root, &job, &dir, &mut self.previews);
        let mut ed = Edit {
            changed,
            jobs: &jobs,
            dir: &dir,
            previews: &mut self.previews,
            job: &job,
            run: shown(
                self.run.as_ref(),
                self.scheduler.as_ref().map(|(s, _)| s),
                &job,
            ),
            can_run,
            run_until: None,
            problem: self.validation.problem(),
            scope,
        };
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin::symmetric(28, 20)),
            )
            .show(ui, |ui| {
                if self.show_yaml {
                    yaml_view::show(ui, &text);
                    return;
                }
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        ui.set_max_width(1100.0);
                        main_view(ui, &mut ed, doc, &mut self.selection);
                    });
            });
        if ed.changed {
            ui.ctx().request_repaint();
        }
        if let Some(until) = ed.run_until.take() {
            start = Some(Some(until));
        }
        let scheduler = self
            .scheduler
            .as_ref()
            .map(|(s, _)| s)
            .filter(|s| s.is_running());
        match (start, &self.selection, scheduler) {
            // While the scheduler is up, a whole-job run is queued on it, so runs of a job
            // never overlap.
            (Some(None), Selection::Job(job), Some(scheduler)) => {
                self.message = Some(
                    scheduler::trigger(&scheduler.socket, job)
                        .map(|()| format!("queued {job} on the scheduler")),
                );
            }
            (Some(until), Selection::Job(job), _) => {
                self.run = Some(Run::start(&doc.path, &text, job, until.as_deref()));
                ui.ctx().request_repaint();
            }
            _ => {}
        }

        if save {
            let saved = doc
                .text()
                .and_then(|text| doc.save(text))
                .map(|()| format!("saved {}", doc.path.display()))
                .map_err(|error| format!("{error:#}"));
            self.message = Some(saved);
            self.previews.clear();
            validate_now = true;
        }
        if validate_now {
            self.previews.clear();
        }
        let text = doc.text().unwrap_or(text);
        if let Some(wait) = self.validation.update(&doc.path, &text, validate_now) {
            ui.ctx().request_repaint_after(wait);
        }

        let title = window_title(doc, &self.selection, doc.is_dirty(&text));
        if title != self.title {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }
}

/// What Run job can do now: start a run (`can_run`), stop the builder's run (`running`),
/// or queue on the scheduler (`scheduling`).
struct RunState {
    can_run: bool,
    running: bool,
    scheduling: bool,
}

/// What the top bar's buttons asked for.
struct TopActions {
    save: bool,
    validate: bool,
    run: bool,
    stop: bool,
}

fn top_bar(
    ui: &mut egui::Ui,
    doc: &Doc,
    selection: &Selection,
    validation: &Validation,
    dirty: bool,
    show_yaml: &mut bool,
    RunState {
        can_run,
        running,
        scheduling,
    }: RunState,
) -> TopActions {
    let mut actions = TopActions {
        save: false,
        validate: false,
        run: false,
        stop: false,
    };
    ui.horizontal(|ui| {
        theme::logo(ui, 22.0);
        ui.label(RichText::new("rocky").strong().size(15.0));
        let file = file_name(&doc.path);
        let crumb = match selection {
            Selection::Project => format!("{file} / project"),
            Selection::Job(name) => format!("{file} / jobs / {name}"),
        };
        theme::hint(ui, &crumb);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if running {
                if widgets::filled_button(ui, Icon::Stop, "Stop", theme::ERROR)
                    .on_hover_text("Stop the run and kill its commands")
                    .clicked()
                {
                    actions.stop = true;
                }
            } else {
                let enabled = can_run && matches!(selection, Selection::Job(_));
                let run = ui.add_enabled_ui(enabled, |ui| {
                    widgets::filled_button(ui, Icon::Play, "Run job", theme::ACCENT)
                });
                if run
                    .inner
                    .on_hover_text(if scheduling {
                        "Queue a run of this job on the scheduler (its saved version)"
                    } else {
                        "Run this job as shown, unsaved edits included, as `rocky exec` would"
                    })
                    .on_disabled_hover_text("Select a job, and fix any errors first")
                    .clicked()
                {
                    actions.run = true;
                }
            }
            if ui.button("Validate").clicked() {
                actions.validate = true;
            }
            if ui.selectable_label(*show_yaml, "</> YAML").clicked() {
                *show_yaml = !*show_yaml;
            }
            let save = if dirty { "● Save" } else { "Save" };
            if ui.button(save).on_hover_text("Save · Cmd+S").clicked() {
                actions.save = true;
            }
            ui.add_space(4.0);
            match &validation.result {
                Some(Ok(())) => theme::badge(ui, Some(Icon::Check), "VALID", theme::OK),
                Some(Err(_)) => theme::badge(ui, Some(Icon::Close), "INVALID", theme::ERROR),
                None => theme::badge(ui, None, "CHECKING", theme::DIM),
            };
        });
    });
    actions
}

/// What the status bar asked of the scheduler.
enum SchedulerAction {
    Start,
    Stop,
}

/// The scheduler's state and Start / Stop, the next job due, and where the scheduler
/// listens and logs; then the file and whether it is valid.
fn status_bar(
    ui: &mut egui::Ui,
    doc: &Doc,
    validation: &Validation,
    message: &Option<Result<String, String>>,
    scheduler: Option<(&Scheduler, &str)>,
) -> Option<SchedulerAction> {
    let mut action = None;
    let running = scheduler.filter(|(scheduler, _)| scheduler.is_running());
    ui.horizontal(|ui| {
        match running {
            Some((scheduler, started_with)) => {
                widgets::dot(ui, theme::OK, true);
                ui.label(RichText::new("Scheduler running").color(theme::OK).size(12.0));
                if ui
                    .small_button("Stop")
                    .on_hover_text("Stop `rocky run`; running commands are killed")
                    .clicked()
                {
                    action = Some(SchedulerAction::Stop);
                }
                theme::hint(ui, &format!("| {} scheduled jobs", scheduler.scheduled()));
                let now = jiff::Zoned::now();
                if let Some((job, at)) = scheduler.next_due(&now) {
                    let wait = Duration::try_from(now.duration_until(&at)).unwrap_or_default();
                    theme::hint(ui, &format!("| next: {job} in {}", countdown(wait)));
                }
                if doc.saved() != started_with {
                    ui.label(
                        RichText::new(
                            "| saved since start: tasks apply on the next run, schedules after a restart",
                        )
                        .color(theme::ACCENT)
                        .size(12.0),
                    );
                }
            }
            None => {
                widgets::dot(ui, theme::DIM, false);
                ui.label(RichText::new("Scheduler stopped").color(theme::DIM).size(12.0));
                if ui
                    .small_button("Start")
                    .on_hover_text("Run `rocky run` on the saved project, with a socket, until stopped")
                    .clicked()
                {
                    action = Some(SchedulerAction::Start);
                }
                if let Some(exited) = scheduler.and_then(|(scheduler, _)| scheduler.exited.as_deref())
                    && exited != "stopped"
                {
                    ui.label(RichText::new(format!("| {exited}")).color(theme::ERROR).size(12.0));
                }
            }
        }
        match message {
            Some(Ok(text)) => theme::hint(ui, &format!("· {text}")),
            Some(Err(text)) => {
                ui.label(RichText::new(format!("· {text}")).color(theme::ERROR).size(12.0));
            }
            None => {}
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let file = file_name(&doc.path);
            let (text, color) = if validation.is_valid() {
                (format!("{file} valid"), theme::OK)
            } else {
                (format!("{file} invalid"), theme::ERROR)
            };
            ui.label(RichText::new(text).color(color).size(12.0));
            if let Some((scheduler, _)) = running {
                theme::hint(ui, &format!("{} |", scheduler.log));
                theme::hint(ui, &format!("socket {} |", scheduler.socket.display()));
            }
        });
    });
    action
}

/// `2m 41s`, `1h 5m`, `12s`.
fn countdown(wait: Duration) -> String {
    let secs = wait.as_secs();
    match (secs / 3600, secs % 3600 / 60, secs % 60) {
        (0, 0, s) => format!("{s}s"),
        (0, m, s) => format!("{m}m {s}s"),
        (h, m, _) => format!("{h}h {m}m"),
    }
}

/// The run whose results the cards of `job` show: the newer of the builder's latest run and
/// the scheduler's latest run of `job`.
fn shown<'a>(
    run: Option<&'a Run>,
    scheduler: Option<&'a Scheduler>,
    job: &str,
) -> Option<Shown<'a>> {
    let local = run.map(Run::shown).filter(|run| run.covers(job));
    let scheduled = scheduler.and_then(|scheduler| scheduler.shown(job));
    match (local, scheduled) {
        (Some(local), Some(scheduled)) => Some(if local.started >= scheduled.started {
            local
        } else {
            scheduled
        }),
        (local, scheduled) => local.or(scheduled),
    }
}

/// The project panel or the selected job's editor.
fn main_view(ui: &mut egui::Ui, ed: &mut Edit, doc: &mut Doc, selection: &mut Selection) {
    let name = match selection {
        Selection::Project => {
            project::editor(ui, ed, doc::as_map(&mut doc.root));
            return;
        }
        Selection::Job(name) => name.clone(),
    };
    let jobs = doc.jobs_mut();
    let Some(job_value) = jobs.get_mut(name.as_str()) else {
        *selection = Selection::Project;
        return;
    };
    match job::editor(ui, ed, &name, job_value) {
        Some(job::Action::Rename(new)) => {
            if doc::rename(jobs, &name, &new) {
                *selection = Selection::Job(new);
                ed.changed = true;
            }
        }
        Some(job::Action::Delete) => {
            doc::set(jobs, &name, None);
            *selection = doc::keys(jobs)
                .first()
                .map_or(Selection::Project, |first| Selection::Job(first.clone()));
            ed.changed = true;
        }
        None => {}
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn window_title(doc: &Doc, selection: &Selection, dirty: bool) -> String {
    let file = file_name(&doc.path);
    let mark = if dirty { " ●" } else { "" };
    match selection {
        Selection::Project => format!("{file}{mark}"),
        Selection::Job(name) => format!("{name} — {file}{mark}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Draws every view (the project, each job, and the YAML) of the project at `path`, and
    /// checks that just looking at it leaves the document unchanged and valid.
    fn view_all(path: &Path) {
        let ctx = egui::Context::default();
        let mut app = App::with_context(&ctx, Some(path.to_path_buf()), Recent::none());
        let doc = app.doc.as_ref().unwrap();
        let before = doc.text().unwrap();
        let views: Vec<Selection> = std::iter::once(Selection::Project)
            .chain(doc.job_names().into_iter().map(Selection::Job))
            .collect();
        for view in views {
            for yaml in [false, true] {
                app.selection = view.clone();
                app.show_yaml = yaml;
                for _ in 0..2 {
                    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| app.draw(ui));
                    output.textures_delta.clear();
                }
            }
        }
        let after = app.doc.as_ref().unwrap().text().unwrap();
        assert_eq!(before, after, "{}", path.display());
        assert!(
            app.validation.is_valid(),
            "{}: {:?}",
            path.display(),
            app.validation.result
        );
    }

    #[test]
    fn viewing_every_example_changes_nothing() {
        let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        let mut viewed = 0;
        for entry in std::fs::read_dir(examples).unwrap() {
            let path = entry.unwrap().path();
            if path
                .extension()
                .is_some_and(|extension| extension == "yaml")
            {
                view_all(&path);
                viewed += 1;
            }
        }
        assert!(viewed > 5);
    }

    /// Without a project, the start screen draws, and a typed path that doesn't exist
    /// shows why it can't be opened.
    #[test]
    fn start_screen_reports_a_bad_path() {
        let ctx = egui::Context::default();
        let mut app = App::with_context(
            &ctx,
            Some(PathBuf::from("/no/such/project.yaml")),
            Recent::none(),
        );
        for _ in 0..2 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| app.draw(ui));
            output.textures_delta.clear();
        }
        assert!(app.doc.is_none());
        assert!(matches!(&app.message, Some(Err(error)) if error.contains("no/such")));
    }

    /// An old-syntax task is invalid, flagged on its card, and drawn with its fix; viewing
    /// it changes nothing.
    #[test]
    fn draws_old_syntax_with_its_problem() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("project.yaml");
        let yaml = "jobs:\n  j:\n    tasks:\n      route: {if: {this: A, eq: 1}, then: {b: {cmd: [echo]}}}\n";
        std::fs::write(&path, yaml).unwrap();
        let ctx = egui::Context::default();
        let mut app = App::with_context(&ctx, Some(path), Recent::none());
        for _ in 0..2 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| app.draw(ui));
            output.textures_delta.clear();
        }
        let problem = app.validation.problem().unwrap();
        assert!(problem.is_on("j", "route"));
        let doc = app.doc.as_ref().unwrap();
        assert!(!doc.is_dirty(&doc.text().unwrap()));
    }

    /// Draws a job while it runs and after, with results on every card.
    #[test]
    fn draws_a_run_to_the_end() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("project.yaml");
        let yaml = "\
jobs:
  main:
    tasks:
      list: {cmd: [echo, '[1, 2]']}
      each:
        for_each: tasks.list.stdout
        do:
          show: {cmd: [echo, '${ITEM}']}
      fail: {cmd: [sh, -c, 'echo oops >&2; exit 3'], allow_failure: true}
      never: {cmd: [echo], when: {this: tasks.fail.ok, eq: true}}
      page: {trigger: other}
  other:
    tasks:
      only: {cmd: [echo, done]}
";
        std::fs::write(&path, yaml).unwrap();
        let ctx = egui::Context::default();
        let mut app = App::with_context(&ctx, Some(path.clone()), Recent::none());
        app.selection = Selection::Job("main".to_string());
        app.run = Some(Run::start(&path, yaml, "main", None));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.run.as_ref().is_some_and(Run::is_running) && std::time::Instant::now() < deadline
        {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| app.draw(ui));
            output.textures_delta.clear();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        for _ in 0..2 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| app.draw(ui));
            output.textures_delta.clear();
        }
        let results = &app.run.as_ref().unwrap().results;
        assert!(matches!(
            results.end,
            Some(crate::results::End::Done { failed: None })
        ));
        assert_eq!(results.task("main", "show").unwrap().runs, 2);
        assert_eq!(results.task("main", "fail").unwrap().failures, 1);
        assert!(results.task("main", "never").unwrap().skipped);
        assert!(results.task("other", "only").is_some());
        assert_eq!(
            app.doc.as_ref().unwrap().text().unwrap(),
            serde_yaml::to_string(&serde_yaml::from_str::<serde_yaml::Value>(yaml).unwrap())
                .unwrap()
        );
    }

    /// Covers what the examples don't: while, any, exists, weekly and at schedules,
    /// defaults, capture_stdout, and task files with `with` and `when`.
    #[test]
    fn viewing_every_field_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("tasks")).unwrap();
        std::fs::write(
            dir.path().join("tasks/greet.yaml"),
            "cmd: [echo, 'hello ${WHO}']\n",
        )
        .unwrap();
        let path = dir.path().join("project.yaml");
        std::fs::write(
            &path,
            "\
values: {BASE: 'https://example.com'}
defaults: {timeout: 30s, env: {A: b}, retry: {attempts: 2, delay: 1s}}
jobs:
  weekly:
    schedule: {weekly: {days: [mon, fri], time: '09:30'}, run_on_start: true}
    defaults: {cwd: .}
    tasks:
      check: {cmd: [echo, '200'], capture_stdout: true}
      wait:
        while: {any: [{this: '${tasks.check.stdout}', ne: 200}, {not: {this: X, exists: true}}]}
        attempts: 3
        on_failure: next
        do:
          pause: {cmd: [sleep, '0']}
      alice:
        file: tasks/greet.yaml
        with: {WHO: alice}
        when: [{this: '${tasks.check.ok}', eq: true}, {this: '${tasks.check.stdout}', gt: 1, lt: 300}]
      bob: tasks/greet.yaml
  once:
    schedule: {at: '2030-01-01T09:00:00'}
    tasks:
      go: {trigger: weekly}
",
        )
        .unwrap();
        view_all(&path);
    }
}
