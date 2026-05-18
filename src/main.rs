mod application;
mod cli;
mod domain;
mod infrastructure;
mod tui;

fn main() -> anyhow::Result<()> {
    cli::run()
}
