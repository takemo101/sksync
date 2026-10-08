mod application;
mod cli;
mod domain;
mod infrastructure;
mod tui;

use std::io::Write;
use std::process::ExitCode;

fn main() -> ExitCode {
    finish(cli::run(), &mut std::io::stderr().lock())
}

fn finish(result: anyhow::Result<()>, stderr: &mut impl Write) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // anyhow downcasting also finds a marker wrapped by TUI context.
            if let Some(failure) = error.downcast_ref::<cli::RenderedFailure>() {
                return ExitCode::from(failure.exit_code);
            }
            let _ = writeln!(stderr, "Error: {error:?}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rendered_failure_keeps_exit_code_without_reprinting_even_with_tui_context() {
        let error = anyhow::Error::new(cli::RenderedFailure { exit_code: 7 })
            .context("failed to run sksync check")
            .context("wizard command failed");
        let mut stderr = Vec::new();
        assert_eq!(finish(Err(error), &mut stderr), ExitCode::from(7));
        assert!(stderr.is_empty());
    }

    #[test]
    fn unrendered_failure_prints_once_and_success_prints_nothing() {
        let mut stderr = Vec::new();
        assert_eq!(finish(Ok(()), &mut stderr), ExitCode::SUCCESS);
        assert!(stderr.is_empty());
        assert_eq!(
            finish(Err(anyhow::anyhow!("unrendered")), &mut stderr),
            ExitCode::FAILURE
        );
        assert_eq!(String::from_utf8(stderr).unwrap(), "Error: unrendered\n");
    }
}
