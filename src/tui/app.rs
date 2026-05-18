use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuiApp {
    pub project_root: PathBuf,
    pub config_exists: bool,
    pub lockfile_exists: bool,
    pub agents: Vec<String>,
    pub skills: Vec<String>,
    pub details: Vec<String>,
    pub confirm_apply: bool,
    should_quit: bool,
}

impl TuiApp {
    pub fn new(project_root: PathBuf, config_exists: bool, lockfile_exists: bool) -> Self {
        Self {
            project_root,
            config_exists,
            lockfile_exists,
            agents: Vec::new(),
            skills: Vec::new(),
            details: vec!["Press d to dry-run, c to check, a to apply, q to quit.".to_owned()],
            confirm_apply: false,
            should_quit: false,
        }
    }

    pub fn quit(&mut self) {
        self.should_quit = true;
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    pub fn set_inventory(&mut self, agents: Vec<String>, skills: Vec<String>) {
        self.agents = agents;
        self.skills = skills;
    }

    pub fn set_details(&mut self, details: Vec<String>) {
        self.details = if details.is_empty() {
            vec!["No details.".to_owned()]
        } else {
            details
        };
    }

    pub fn request_apply_confirmation(&mut self) {
        self.confirm_apply = true;
        self.set_details(vec![
            "Apply changes? Press y to confirm or n to cancel.".to_owned()
        ]);
    }

    pub fn clear_apply_confirmation(&mut self) {
        self.confirm_apply = false;
    }
}

#[cfg(test)]
mod tests {
    use super::TuiApp;
    use std::path::PathBuf;

    #[test]
    fn quit_marks_app_done() {
        let mut app = TuiApp::new(PathBuf::from("."), true, false);
        assert!(!app.should_quit());

        app.quit();

        assert!(app.should_quit());
    }

    #[test]
    fn apply_confirmation_can_be_toggled() {
        let mut app = TuiApp::new(PathBuf::from("."), true, false);
        app.request_apply_confirmation();
        assert!(app.confirm_apply);

        app.clear_apply_confirmation();
        assert!(!app.confirm_apply);
    }
}
