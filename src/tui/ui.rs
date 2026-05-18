use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use super::app::TuiApp;

pub fn render(frame: &mut Frame<'_>, app: &TuiApp) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(3),
        ])
        .split(frame.area());

    let title = Paragraph::new(Line::from(vec![
        Span::styled("sksync", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" TUI MVP"),
    ]))
    .block(Block::default().borders(Borders::ALL));
    frame.render_widget(title, chunks[0]);

    let body = Paragraph::new(vec![
        Line::from(format!("Project: {}", app.project_root.display())),
        Line::from(format!("Config: {}", status(app.config_exists))),
        Line::from(format!("Lockfile: {}", status(app.lockfile_exists))),
        Line::from(""),
        Line::from("Use CLI commands for apply/check/list until TUI actions are wired."),
    ])
    .block(Block::default().title("Status").borders(Borders::ALL));
    frame.render_widget(body, chunks[1]);

    let footer = Paragraph::new("Press q to quit").block(Block::default().borders(Borders::ALL));
    frame.render_widget(footer, chunks[2]);
}

fn status(exists: bool) -> &'static str {
    if exists {
        "found"
    } else {
        "missing"
    }
}
