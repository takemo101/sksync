use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use super::app::TuiApp;

pub fn render(frame: &mut Frame<'_>, app: &TuiApp) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(3),
        ])
        .split(frame.area());

    let title = Paragraph::new(Line::from(vec![
        Span::styled("sksync", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" TUI MVP"),
    ]))
    .block(Block::default().borders(Borders::ALL));
    frame.render_widget(title, chunks[0]);

    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(24),
            Constraint::Percentage(28),
            Constraint::Percentage(48),
        ])
        .split(chunks[1]);

    let agents = Paragraph::new(lines_or_placeholder(&app.agents, "No agents"))
        .block(Block::default().title("Agents").borders(Borders::ALL));
    frame.render_widget(agents, panes[0]);

    let skills = Paragraph::new(lines_or_placeholder(&app.skills, "No skills"))
        .block(Block::default().title("Skills").borders(Borders::ALL));
    frame.render_widget(skills, panes[1]);

    let details = Paragraph::new(app.details.join("\n"))
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .title(format!(
                    "Details | Config: {} | Lockfile: {}",
                    status(app.config_exists),
                    status(app.lockfile_exists)
                ))
                .borders(Borders::ALL),
        );
    frame.render_widget(details, panes[2]);

    let footer = Paragraph::new("d: dry-run  c: check  a: apply  y/n: confirm/cancel  q: quit")
        .block(Block::default().borders(Borders::ALL));
    frame.render_widget(footer, chunks[2]);
}

fn lines_or_placeholder(values: &[String], placeholder: &str) -> String {
    if values.is_empty() {
        placeholder.to_owned()
    } else {
        values.join("\n")
    }
}

fn status(exists: bool) -> &'static str {
    if exists {
        "found"
    } else {
        "missing"
    }
}
