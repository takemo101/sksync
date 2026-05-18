use crossterm::event::{Event, KeyCode, KeyEventKind};

use super::app::TuiApp;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TuiCommand {
    DryRun,
    Check,
    RequestApply,
    ConfirmApply,
    CancelApply,
}

pub fn handle_event(app: &mut TuiApp, event: Event) -> Option<TuiCommand> {
    let Event::Key(key) = event else {
        return None;
    };
    if key.kind != KeyEventKind::Press {
        return None;
    }

    match key.code {
        KeyCode::Char('q') => {
            app.quit();
            None
        }
        KeyCode::Char('d') => Some(TuiCommand::DryRun),
        KeyCode::Char('c') => Some(TuiCommand::Check),
        KeyCode::Char('a') => Some(TuiCommand::RequestApply),
        KeyCode::Char('y') if app.confirm_apply => Some(TuiCommand::ConfirmApply),
        KeyCode::Char('n') if app.confirm_apply => Some(TuiCommand::CancelApply),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{handle_event, TuiCommand};
    use crate::tui::app::TuiApp;
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
    use std::path::PathBuf;

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new_with_kind(
            code,
            KeyModifiers::NONE,
            KeyEventKind::Press,
        ))
    }

    #[test]
    fn q_key_quits() {
        let mut app = TuiApp::new(PathBuf::from("."), false, false);

        handle_event(&mut app, key(KeyCode::Char('q')));

        assert!(app.should_quit());
    }

    #[test]
    fn action_keys_return_commands() {
        let mut app = TuiApp::new(PathBuf::from("."), false, false);
        assert_eq!(
            handle_event(&mut app, key(KeyCode::Char('d'))),
            Some(TuiCommand::DryRun)
        );
        assert_eq!(
            handle_event(&mut app, key(KeyCode::Char('c'))),
            Some(TuiCommand::Check)
        );
        assert_eq!(
            handle_event(&mut app, key(KeyCode::Char('a'))),
            Some(TuiCommand::RequestApply)
        );
    }

    #[test]
    fn apply_confirmation_keys_require_modal() {
        let mut app = TuiApp::new(PathBuf::from("."), false, false);
        assert_eq!(handle_event(&mut app, key(KeyCode::Char('y'))), None);
        app.request_apply_confirmation();
        assert_eq!(
            handle_event(&mut app, key(KeyCode::Char('y'))),
            Some(TuiCommand::ConfirmApply)
        );
        assert_eq!(
            handle_event(&mut app, key(KeyCode::Char('n'))),
            Some(TuiCommand::CancelApply)
        );
    }
}
