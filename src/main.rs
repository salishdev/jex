mod app;
mod tree;
mod ui;

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

    let mut app = App::new(value, source_name, cli.expand_depth);
    app::run(&mut app)?;
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
