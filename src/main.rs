mod app;
mod filter;
mod tree;
mod ui;
mod ui_state;

use std::{
    fs,
    io::{self, IsTerminal, Read},
    path::PathBuf,
};

use anyhow::{Context, Result, bail};
use clap::Parser;
use serde_json::Value;

use crate::app::App;

/// Explore JSON without losing your place.
#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    /// JSON file to open. Omit it to read JSON from stdin.
    #[arg(value_name = "FILE")]
    file: Option<PathBuf>,

    /// Expand containers this many levels on startup.
    #[arg(short = 'd', long, default_value_t = 1, value_name = "N")]
    expand_depth: usize,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let (source_name, input) = read_input(cli.file)?;
    let value: Value = serde_json::from_str(&input).with_context(|| {
        if source_name == "stdin" {
            "could not parse JSON from stdin".to_string()
        } else {
            format!("could not parse JSON in {source_name}")
        }
    })?;

    let state_path = ui_state::state_path();
    let saved_state = state_path
        .as_deref()
        .map(ui_state::load_or_default)
        .unwrap_or_default();
    let mut app = App::with_pane_split_percent(
        value,
        source_name,
        cli.expand_depth,
        saved_state.tree_pane_percent,
    );
    app::run(&mut app)?;

    if app.pane_split_changed()
        && let Some(path) = state_path
        && let Err(error) = ui_state::save(&path, app.pane_split_percent())
    {
        eprintln!(
            "warning: could not save UI state to {}: {error}",
            path.display()
        );
    }

    if let Some(output) = app.output {
        println!("{output}");
    }
    Ok(())
}

fn read_input(file: Option<PathBuf>) -> Result<(String, String)> {
    match file {
        Some(path) => {
            let input = fs::read_to_string(&path)
                .with_context(|| format!("could not read {}", path.display()))?;
            Ok((path.display().to_string(), input))
        }
        None if !io::stdin().is_terminal() => {
            let mut input = String::new();
            io::stdin()
                .read_to_string(&mut input)
                .context("could not read JSON from stdin")?;
            Ok(("stdin".into(), input))
        }
        None => bail!("pass a JSON file or pipe JSON into jex"),
    }
}
