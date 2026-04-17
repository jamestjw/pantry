use std::io;

use clap::{Args, Parser, Subcommand};

use crate::exec;
use crate::model::Recipe;

#[derive(Debug, Parser)]
#[command(name = "pantry", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// List loaded recipes
    List,
    /// Render a recipe command without running it
    Render(RenderArgs),
    /// Run a recipe in headless mode
    Run(RunArgs),
}

#[derive(Debug, Args)]
pub struct RenderArgs {
    /// Recipe name (exact match)
    pub recipe: String,
    /// Placeholder assignment, e.g. --set env=prod
    #[arg(long = "set", value_name = "KEY=VALUE")]
    pub set: Vec<String>,
}

#[derive(Debug, Args)]
pub struct RunArgs {
    /// Recipe name (exact match)
    pub recipe: String,
    /// Placeholder assignment, e.g. --set env=prod
    #[arg(long = "set", value_name = "KEY=VALUE")]
    pub set: Vec<String>,
    /// Allow recipes marked with safety = "confirm"
    #[arg(short = 'y', long)]
    pub yes: bool,
}

pub fn run_command(command: Command, recipes: &[Recipe]) -> io::Result<i32> {
    match command {
        Command::List => {
            print_recipes(recipes);
            Ok(0)
        }
        Command::Render(args) => {
            let recipe = exec::find_recipe(recipes, &args.recipe)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;
            let values = exec::resolve_values(recipe, &args.set)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;
            let rendered = recipe.compiled.render(&values);
            println!("{rendered}");
            Ok(0)
        }
        Command::Run(args) => {
            let recipe = exec::find_recipe(recipes, &args.recipe)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;
            let values = exec::resolve_values(recipe, &args.set)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;
            let rendered = recipe.compiled.render(&values);

            if recipe.safety == "confirm" && !args.yes {
                eprintln!("Recipe '{}' requires confirmation.", recipe.name);
                eprintln!("Rendered command:");
                eprintln!("{rendered}");
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "Re-run with --yes to execute this command in headless mode",
                ));
            }

            let status = exec::run_attached(&rendered)?;
            Ok(status.code().unwrap_or(1))
        }
    }
}

fn print_recipes(recipes: &[Recipe]) {
    if recipes.is_empty() {
        println!("No recipes found");
        return;
    }

    for recipe in recipes {
        if recipe.tags.is_empty() {
            println!("{}", recipe.name);
        } else {
            println!("{} [{}]", recipe.name, recipe.tags.join(", "));
        }
    }
}
