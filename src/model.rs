use std::collections::HashMap;
use std::fmt;
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
    pub interactive: bool,
    #[serde(default)]
    pub presets: Vec<String>,
    #[serde(default)]
    pub choices: HashMap<String, Vec<String>>,
    #[serde(default = "default_safety")]
    pub safety: Safety,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Safety {
    Safe,
    Confirm,
}

impl fmt::Display for Safety {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Safe => f.write_str("safe"),
            Self::Confirm => f.write_str("confirm"),
        }
    }
}

fn default_safety() -> Safety {
    Safety::Safe
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

    validate_unique_names(&recipes)?;

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
        validate_recipe(recipe)?;
    }
    Ok(parsed.recipe)
}

fn validate_recipe(recipe: &Recipe) -> io::Result<()> {
    if recipe.name.trim().is_empty() {
        return invalid_recipe(recipe, "recipe name cannot be empty");
    }

    if recipe.command.trim().is_empty() {
        return invalid_recipe(recipe, "recipe command cannot be empty");
    }

    let placeholders = recipe.compiled.placeholders();

    let mut unknown_choices: Vec<_> = recipe
        .choices
        .keys()
        .filter(|key| !placeholders.contains(key))
        .cloned()
        .collect();
    unknown_choices.sort();
    if !unknown_choices.is_empty() {
        return invalid_recipe(
            recipe,
            &format!(
                "choices reference unknown placeholders: {}",
                unknown_choices.join(", ")
            ),
        );
    }

    for preset in &recipe.presets {
        let values = crate::template::parse_assignment_values(preset);
        let mut missing: Vec<_> = placeholders
            .iter()
            .filter(|placeholder| !values.contains_key(*placeholder))
            .cloned()
            .collect();
        missing.sort();
        if !missing.is_empty() {
            return invalid_recipe(
                recipe,
                &format!(
                    "preset '{preset}' is missing values for: {}",
                    missing.join(", ")
                ),
            );
        }

        let mut unknown: Vec<_> = values
            .keys()
            .filter(|key| !placeholders.contains(key))
            .cloned()
            .collect();
        unknown.sort();
        if !unknown.is_empty() {
            return invalid_recipe(
                recipe,
                &format!(
                    "preset '{preset}' assigns unknown placeholders: {}",
                    unknown.join(", ")
                ),
            );
        }
    }

    Ok(())
}

fn validate_unique_names(recipes: &[Recipe]) -> io::Result<()> {
    let mut seen: HashMap<&str, &str> = HashMap::new();
    for recipe in recipes {
        if let Some(first_source) = seen.insert(recipe.name.as_str(), recipe.source.as_str()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "duplicate recipe name '{}' found in {} and {}",
                    recipe.name, first_source, recipe.source
                ),
            ));
        }
    }
    Ok(())
}

fn invalid_recipe<T>(recipe: &Recipe, message: &str) -> io::Result<T> {
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "invalid recipe '{}' in {}: {message}",
            recipe.name, recipe.source
        ),
    ))
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
    use super::{RecipeFile, Safety, parse_recipe_file};
    use std::fs;

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

    #[test]
    fn deserializes_safety_as_enum() {
        let file: RecipeFile = toml::from_str(
            r#"[[recipe]]
name = "deploy service"
command = "deploy"
safety = "confirm"
"#,
        )
        .expect("recipe file should parse");

        assert_eq!(file.recipe[0].safety, Safety::Confirm);
    }

    #[test]
    fn rejects_choices_for_unknown_placeholders() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("pantry-test-{}.toml", std::process::id()));
        fs::write(
            &path,
            r#"[[recipe]]
name = "deploy service"
command = "deploy {service}"
choices = { env = ["prod"] }
"#,
        )
        .expect("test file should be written");

        let err = parse_recipe_file(&path).expect_err("unknown choice should fail");
        fs::remove_file(&path).ok();

        assert!(
            err.to_string()
                .contains("choices reference unknown placeholders")
        );
    }
}
