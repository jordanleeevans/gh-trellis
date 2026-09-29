//! An entry-point TUI panel listing locally tracked `gh stack` stacks.

mod config;
mod doctor;
mod git;
mod shell;
mod stack;
#[cfg(test)]
mod test_fixtures;
mod theme;
mod tui;

use anyhow::Result;
use std::env;
use std::io::IsTerminal;

#[tokio::main]
async fn main() -> Result<()> {
    let config = match config::Config::load() {
        Ok(config) => config,
        Err(err) => {
            eprintln!("error: invalid config: {err}");
            std::process::exit(1);
        }
    };
    match tui::keymap::Keymap::default_keymap().with_overrides(&config.keybindings) {
        Ok(keymap) => tui::keymap::install(keymap),
        Err(err) => {
            let path = config::config_path().map(|p| p.display().to_string());
            eprintln!("error: invalid config {}: {err}", path.unwrap_or_default());
            std::process::exit(1);
        }
    }
    theme::glyphs::set_nerd_fonts(config.nerd_fonts);
    config::install(config);

    let cwd = env::current_dir()?;
    let shell = shell::ProcessShell;

    // A plain report on stderr, before the alternate screen, so the user can
    // select and copy the commands. A healthy machine prints nothing.
    let diagnosis = doctor::diagnose(&shell, cwd.as_path()).await;
    let color = doctor::use_color(
        std::io::stderr().is_terminal(),
        env::var_os("NO_COLOR").as_deref(),
        env::var_os("TERM").as_deref(),
    );
    if let Some(report) = doctor::render(&diagnosis, doctor::Platform::current(), color) {
        eprint!("{report}");
        std::process::exit(1);
    }

    tui::run(std::sync::Arc::new(shell), cwd.as_path()).await
}
