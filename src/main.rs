mod app;
mod clipboard;
mod model;
mod template;

use std::io;

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn run() -> io::Result<()> {
    let recipes = model::load_recipes()?;
    app::run(recipes)
}
