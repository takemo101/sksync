pub mod app;
pub mod events;
pub mod ui;

use std::io;
use std::time::Duration;

use anyhow::Result;
use crossterm::event;
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use app::TuiApp;

pub fn run(mut app: TuiApp) -> Result<()> {
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
            events::handle_event(app, event);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::events;
    use crate::tui::app::TuiApp;
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
}
