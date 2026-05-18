use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuiApp {
    pub project_root: PathBuf,
    pub config_exists: bool,
    pub lockfile_exists: bool,
    should_quit: bool,
}

impl TuiApp {
    pub fn new(project_root: PathBuf, config_exists: bool, lockfile_exists: bool) -> Self {
        Self {
            project_root,
            config_exists,
            lockfile_exists,
            should_quit: false,
        }
    }

    pub fn quit(&mut self) {
        self.should_quit = true;
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
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
}
