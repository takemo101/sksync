mod application;
mod cli;
mod domain;
mod infrastructure;

fn main() -> anyhow::Result<()> {
    cli::run()
}
