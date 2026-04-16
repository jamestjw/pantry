use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::template::Template;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recipe {
    pub name: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub description: String,
    pub command: String,
    #[serde(default)]
    pub presets: Vec<String>,
    #[serde(default)]
    pub choices: HashMap<String, Vec<String>>,
    #[serde(default = "default_safety")]
    pub safety: String,
    #[serde(skip)]
    pub source: String,
    #[serde(skip)]
    pub compiled: Template,
    #[serde(skip)]
    pub last_run: Option<RunOutput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunOutput {
    pub command: String,
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

fn default_safety() -> String {
    "safe".to_string()
}

#[derive(Debug, Clone, Deserialize)]
struct RecipeFile {
    #[serde(default)]
    recipe: Vec<Recipe>,
}

const SAMPLE_RECIPES: &str = r#"[[recipe]]
name = "sync current branch"
tags = ["git", "sync"]
description = "Fetch and rebase current branch onto origin/{branch}."
command = "git fetch origin && git rebase origin/{branch}"
safety = "confirm"
presets = ["branch=main"]
"#;

pub fn load_recipes() -> io::Result<Vec<Recipe>> {
    let global = global_recipe_path()?;
    ensure_global_file(&global)?;

    let mut recipes = parse_recipe_file(&global)?;

    let local = local_recipe_path()?;
    if local.exists() {
        recipes.extend(parse_recipe_file(&local)?);
    }

    Ok(recipes)
}

pub fn reload_recipes() -> io::Result<Vec<Recipe>> {
    load_recipes()
}

fn ensure_global_file(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if !path.exists() {
        fs::write(path, SAMPLE_RECIPES)?;
    }
    Ok(())
}

fn parse_recipe_file(path: &Path) -> io::Result<Vec<Recipe>> {
    let raw = fs::read_to_string(path)?;
    let mut parsed: RecipeFile = toml::from_str(&raw).map_err(|err| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("failed to parse {}: {err}", path.display()),
        )
    })?;
    let source = path.display().to_string();
    for recipe in &mut parsed.recipe {
        recipe.source = source.clone();
        recipe.compiled = Template::parse(&recipe.command);
    }
    Ok(parsed.recipe)
}

fn global_recipe_path() -> io::Result<PathBuf> {
    let base = dirs::config_dir().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "unable to resolve config directory",
        )
    })?;
    Ok(base.join("pantry").join("recipes.toml"))
}

fn local_recipe_path() -> io::Result<PathBuf> {
    Ok(std::env::current_dir()?.join(".pantry.toml"))
}

#[cfg(test)]
mod tests {
    use super::RecipeFile;

    #[test]
    fn deserializes_placeholder_choices() {
        let file: RecipeFile = toml::from_str(
            r#"[[recipe]]
name = "deploy service"
command = "deploy --env {env} --service {service}"
choices = { env = ["dev", "staging", "prod"], service = ["api", "web"] }
"#,
        )
        .expect("recipe file should parse");

        let recipe = &file.recipe[0];
        assert_eq!(
            recipe.choices.get("env"),
            Some(&vec![
                "dev".to_string(),
                "staging".to_string(),
                "prod".to_string()
            ])
        );
        assert_eq!(
            recipe.choices.get("service"),
            Some(&vec!["api".to_string(), "web".to_string()])
        );
    }
}
