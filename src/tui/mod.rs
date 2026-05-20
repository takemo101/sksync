pub mod app;
pub mod events;
pub mod ui;

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use crossterm::event;
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::application::apply::{apply_link_plan, ApplyOptions};
use crate::application::check::check_lockfile;
use crate::application::list::list_skills;
use crate::application::plan::build_link_plan;
use crate::application::ports::ConfigStore;
use crate::domain::link_plan::LinkPlan;
use crate::domain::lockfile::{LockedFile, LockedSkill, Lockfile};
use crate::infrastructure::builtin_agents::TargetPathResolver;
use crate::infrastructure::fs::FileSystemLinkStore;
use crate::infrastructure::hash::{hash_directory, Sha256SourceHashStore};
use crate::infrastructure::json::{read_lockfile, FileConfigStore, FileLockfileStore};

use app::TuiApp;
use events::TuiCommand;

pub fn run(mut app: TuiApp) -> Result<()> {
    refresh_inventory(&mut app);
    let mut guard = TerminalGuard::default();

    enable_raw_mode()?;
    guard.raw_mode = true;

    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    guard.alternate_screen = true;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let result = run_loop(&mut terminal, &mut app);
    terminal.show_cursor()?;

    result
}

#[derive(Debug, Default)]
struct TerminalGuard {
    raw_mode: bool,
    alternate_screen: bool,
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if self.alternate_screen {
            let _ = execute!(io::stdout(), LeaveAlternateScreen);
        }
        if self.raw_mode {
            let _ = disable_raw_mode();
        }
    }
}

fn run_loop(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut TuiApp) -> Result<()> {
    while !app.should_quit() {
        terminal.draw(|frame| ui::render(frame, app))?;
        if event::poll(Duration::from_millis(250))? {
            let event = event::read()?;
            if let Some(command) = events::handle_event(app, event) {
                handle_command(app, command);
            }
        }
    }

    Ok(())
}

fn handle_command(app: &mut TuiApp, command: TuiCommand) {
    match command {
        TuiCommand::DryRun => app.set_details(run_dry_plan(&app.project_root)),
        TuiCommand::Check => app.set_details(run_check(&app.project_root)),
        TuiCommand::RequestApply => app.request_apply_confirmation(),
        TuiCommand::ConfirmApply => {
            app.clear_apply_confirmation();
            app.set_details(run_apply(&app.project_root));
            app.lockfile_exists = app.project_root.join("sksync-lock.json").exists();
            refresh_inventory(app);
        }
        TuiCommand::CancelApply => {
            app.clear_apply_confirmation();
            app.set_details(vec!["Apply cancelled.".to_owned()]);
        }
    }
}

fn refresh_inventory(app: &mut TuiApp) {
    match load_config_and_resolver(&app.project_root) {
        Ok((config, resolver)) => {
            let lockfile = read_lockfile(app.project_root.join("sksync-lock.json")).ok();
            let report = list_skills(&config, lockfile.as_ref(), &FileSystemLinkStore, &resolver);
            app.set_inventory(
                config.agents.keys().cloned().collect(),
                report
                    .skills
                    .iter()
                    .map(|skill| skill.name.clone())
                    .collect(),
            );
        }
        Err(error) => app.set_details(vec![format!("Failed to load config: {error}")]),
    }
}

fn run_dry_plan(project_root: &Path) -> Vec<String> {
    match load_plan(project_root) {
        Ok((_config, plan)) => plan.display_lines(),
        Err(error) => vec![format!("dry-run failed: {error}")],
    }
}

fn run_check(project_root: &Path) -> Vec<String> {
    match read_lockfile(project_root.join("sksync-lock.json")) {
        Ok(lockfile) => {
            let report = check_lockfile(&lockfile, &Sha256SourceHashStore, &FileSystemLinkStore);
            report.display_lines()
        }
        Err(error) => vec![format!("check failed: {error}")],
    }
}

fn run_apply(project_root: &Path) -> Vec<String> {
    let result = (|| -> Result<Vec<String>> {
        let (config, plan) = load_plan(project_root)?;
        let lockfile = build_lockfile_from_plan(&config, &plan, project_root)?;
        apply_link_plan(
            &plan,
            &lockfile,
            &FileSystemLinkStore,
            &FileLockfileStore::new(project_root.join("sksync-lock.json")),
            ApplyOptions { force: false },
        )?;
        let mut lines = plan.display_lines();
        lines.push("Apply complete.".to_owned());
        Ok(lines)
    })();

    match result {
        Ok(lines) => lines,
        Err(error) => vec![format!("apply failed: {error}")],
    }
}

fn load_plan(
    project_root: &Path,
) -> Result<(crate::application::config::ResolvedConfig, LinkPlan)> {
    let (config, resolver) = load_config_and_resolver(project_root)?;
    let plan = build_link_plan(
        &config,
        &FileSystemLinkStore,
        &FileSystemLinkStore,
        &resolver,
    )?;
    Ok((config, plan))
}

fn load_config_and_resolver(
    project_root: &Path,
) -> Result<(
    crate::application::config::ResolvedConfig,
    TargetPathResolver,
)> {
    let config = FileConfigStore::new(project_root.join("sksync.config.json")).load()?;
    let home_dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("~"));
    Ok((config, TargetPathResolver::new(project_root, home_dir)))
}

fn build_lockfile_from_plan(
    config: &crate::application::config::ResolvedConfig,
    _plan: &LinkPlan,
    project_root: &Path,
) -> Result<Lockfile> {
    let mut skills = BTreeMap::new();

    for skill in &config.skills {
        let hash = hash_directory(skill.source.as_path())
            .with_context(|| format!("failed to hash {}", skill.source.as_path().display()))?;
        skills.insert(
            skill.name.clone(),
            LockedSkill {
                source: skill.source.clone(),
                install_source: skill.install_source.clone(),
                hash: hash.hash.clone(),
                files: hash
                    .files
                    .iter()
                    .map(|file| LockedFile {
                        path: file.path.clone(),
                        hash: file.hash.clone(),
                    })
                    .collect(),
                targets: Vec::new(),
            },
        );
    }

    Ok(Lockfile {
        generated_by: format!("sksync@{}", env!("CARGO_PKG_VERSION")),
        generated_at: generated_at(),
        root: project_root.to_path_buf(),
        skills,
    })
}

fn generated_at() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| format!("unix:{}", duration.as_secs()))
        .unwrap_or_else(|_| "unix:0".to_owned())
}

#[cfg(test)]
mod tests {
    use super::{events, handle_command};
    use crate::tui::app::TuiApp;
    use crate::tui::events::TuiCommand;
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
    use std::path::PathBuf;

    #[test]
    fn tui_app_can_quit_from_q_event() {
        let mut app = TuiApp::new(PathBuf::from("."), true, true);
        events::handle_event(
            &mut app,
            Event::Key(KeyEvent::new_with_kind(
                KeyCode::Char('q'),
                KeyModifiers::NONE,
                KeyEventKind::Press,
            )),
        );

        assert!(app.should_quit());
    }

    #[test]
    fn request_apply_command_opens_confirmation() {
        let mut app = TuiApp::new(PathBuf::from("."), true, false);

        handle_command(&mut app, TuiCommand::RequestApply);

        assert!(app.confirm_apply);
        assert!(app.details[0].contains("Apply changes?"));
    }

    #[test]
    fn cancel_apply_command_closes_confirmation() {
        let mut app = TuiApp::new(PathBuf::from("."), true, false);
        app.request_apply_confirmation();

        handle_command(&mut app, TuiCommand::CancelApply);

        assert!(!app.confirm_apply);
        assert_eq!(app.details, vec!["Apply cancelled."]);
    }
}
