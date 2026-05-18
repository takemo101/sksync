use crossterm::event::{Event, KeyCode, KeyEventKind};

use super::app::TuiApp;

pub fn handle_event(app: &mut TuiApp, event: Event) {
    if let Event::Key(key) = event {
        if key.kind == KeyEventKind::Press && matches!(key.code, KeyCode::Char('q')) {
            app.quit();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::handle_event;
    use crate::tui::app::TuiApp;
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
    use std::path::PathBuf;

    #[test]
    fn q_key_quits() {
        let mut app = TuiApp::new(PathBuf::from("."), false, false);
        let event = Event::Key(KeyEvent::new_with_kind(
            KeyCode::Char('q'),
            KeyModifiers::NONE,
            KeyEventKind::Press,
        ));

        handle_event(&mut app, event);

        assert!(app.should_quit());
    }
}
