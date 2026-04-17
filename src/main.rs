mod app;
mod cli;
mod clipboard;
mod exec;
mod model;
mod template;

use clap::Parser;

fn main() {
    let exit_code = match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("{err}");
            1
        }
    };
    std::process::exit(exit_code);
}

fn run() -> Result<i32, String> {
    let args = cli::Cli::parse();
    let recipes = model::load_recipes().map_err(|err| err.to_string())?;

    match args.command {
        None => {
            app::run(recipes).map_err(|err| err.to_string())?;
            Ok(0)
        }
        Some(command) => cli::run_command(command, &recipes).map_err(|err| err.to_string()),
    }
}
