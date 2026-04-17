use std::collections::{BTreeSet, HashMap};
use std::io;
use std::process::{Command, ExitStatus};

use crate::model::{Recipe, RunOutput};

pub fn find_recipe<'a>(recipes: &'a [Recipe], name: &str) -> Result<&'a Recipe, String> {
    let matches: Vec<&Recipe> = recipes
        .iter()
        .filter(|recipe| recipe.name == name)
        .collect();

    match matches.len() {
        0 => Err(format!("Recipe '{name}' not found")),
        1 => Ok(matches[0]),
        _ => {
            let sources = matches
                .iter()
                .map(|recipe| recipe.source.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            Err(format!(
                "Multiple recipes named '{name}' found in: {sources}"
            ))
        }
    }
}

pub fn resolve_values(
    recipe: &Recipe,
    set_args: &[String],
) -> Result<HashMap<String, String>, String> {
    let mut values = HashMap::new();

    for set in set_args {
        let Some((key, value)) = set.split_once('=') else {
            return Err(format!(
                "Invalid --set value '{set}'. Expected format key=value"
            ));
        };

        if key.trim().is_empty() {
            return Err(format!("Invalid --set value '{set}'. Key cannot be empty"));
        }

        values.insert(key.to_string(), value.to_string());
    }

    let placeholders = recipe.compiled.placeholders();
    let placeholder_set: BTreeSet<&str> = placeholders.iter().map(String::as_str).collect();

    let mut unknown_keys: Vec<String> = values
        .keys()
        .filter(|key| !placeholder_set.contains(key.as_str()))
        .cloned()
        .collect();
    unknown_keys.sort();
    unknown_keys.dedup();
    if !unknown_keys.is_empty() {
        return Err(format!(
            "Unknown placeholder assignments: {}",
            unknown_keys.join(", ")
        ));
    }

    let missing: Vec<String> = placeholders
        .iter()
        .filter(|placeholder| !values.contains_key(*placeholder))
        .cloned()
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "Missing values for placeholders: {}",
            missing.join(", ")
        ));
    }

    for placeholder in placeholders {
        let Some(allowed) = recipe.choices.get(&placeholder) else {
            continue;
        };
        if allowed.is_empty() {
            continue;
        }

        let value = values
            .get(&placeholder)
            .expect("placeholder value must exist after missing check");
        if !allowed.iter().any(|candidate| candidate == value) {
            return Err(format!(
                "Invalid value '{value}' for {{{placeholder}}}. Allowed: {}",
                allowed.join(", ")
            ));
        }
    }

    Ok(values)
}

pub fn run_attached(command: &str) -> io::Result<ExitStatus> {
    Command::new("sh").arg("-lc").arg(command).status()
}

pub fn run_captured(command: &str) -> RunOutput {
    match Command::new("sh").arg("-lc").arg(command).output() {
        Ok(output) => RunOutput {
            command: command.to_string(),
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        },
        Err(err) => RunOutput {
            command: command.to_string(),
            code: Some(1),
            stdout: String::new(),
            stderr: err.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::model::Recipe;
    use crate::template::Template;

    use super::resolve_values;

    fn recipe(command: &str) -> Recipe {
        Recipe {
            name: "test".to_string(),
            tags: Vec::new(),
            description: String::new(),
            command: command.to_string(),
            interactive: false,
            presets: Vec::new(),
            choices: HashMap::new(),
            safety: "safe".to_string(),
            source: "test".to_string(),
            compiled: Template::parse(command),
            last_run: None,
        }
    }

    #[test]
    fn resolves_set_values() {
        let recipe = recipe("deploy {service} {env}");
        let values = resolve_values(
            &recipe,
            &["service=api".to_string(), "env=prod".to_string()],
        )
        .expect("values should resolve");

        assert_eq!(values.get("service"), Some(&"api".to_string()));
        assert_eq!(values.get("env"), Some(&"prod".to_string()));
    }

    #[test]
    fn errors_for_missing_placeholder() {
        let recipe = recipe("deploy {service} {env}");
        let err = resolve_values(&recipe, &["service=api".to_string()])
            .expect_err("missing env should error");
        assert!(err.contains("Missing values for placeholders"));
        assert!(err.contains("env"));
    }

    #[test]
    fn errors_for_unknown_placeholder_assignment() {
        let recipe = recipe("deploy {service}");
        let err = resolve_values(
            &recipe,
            &["service=api".to_string(), "env=prod".to_string()],
        )
        .expect_err("unknown key should error");
        assert!(err.contains("Unknown placeholder assignments"));
        assert!(err.contains("env"));
    }

    #[test]
    fn validates_choices() {
        let mut recipe = recipe("deploy {env}");
        recipe.choices.insert(
            "env".to_string(),
            vec!["dev".to_string(), "staging".to_string(), "prod".to_string()],
        );

        let err = resolve_values(&recipe, &["env=qa".to_string()])
            .expect_err("invalid choice should error");
        assert!(err.contains("Invalid value 'qa'"));
        assert!(err.contains("dev, staging, prod"));
    }
}
