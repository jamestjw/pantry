use std::collections::{BTreeSet, HashMap};

pub fn placeholders(command: &str) -> Vec<String> {
    let mut found = BTreeSet::new();
    let chars: Vec<char> = command.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i] == '{' {
            if i + 1 < chars.len() && chars[i + 1] == '{' {
                i += 2;
                continue;
            }
            let mut j = i + 1;
            while j < chars.len() && chars[j] != '}' {
                j += 1;
            }
            if j < chars.len() && j > i + 1 {
                let name: String = chars[i + 1..j].iter().collect();
                if is_valid_name(&name) {
                    found.insert(name);
                }
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
    found.into_iter().collect()
}

pub fn render(command: &str, values: &HashMap<String, String>) -> String {
    let mut out = String::new();
    let chars: Vec<char> = command.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i] == '{' {
            if i + 1 < chars.len() && chars[i + 1] == '{' {
                out.push('{');
                i += 2;
                continue;
            }
            let mut j = i + 1;
            while j < chars.len() && chars[j] != '}' {
                j += 1;
            }
            if j < chars.len() && j > i + 1 {
                let name: String = chars[i + 1..j].iter().collect();
                if let Some(value) = values.get(&name) {
                    out.push_str(value);
                    i = j + 1;
                    continue;
                }
            }
        } else if chars[i] == '}' {
            if i + 1 < chars.len() && chars[i + 1] == '}' {
                out.push('}');
                i += 2;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

pub fn parse_assignment_values(input: &str) -> HashMap<String, String> {
    let mut values = HashMap::new();

    for token in input.split_whitespace() {
        let Some((key, value)) = token.split_once('=') else {
            continue;
        };

        if is_valid_name(key) && !value.is_empty() {
            values.insert(key.to_string(), value.to_string());
        }
    }

    values
}

fn is_valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{parse_assignment_values, placeholders, render};

    #[test]
    fn extracts_unique_placeholders() {
        let got = placeholders("git checkout {branch} && git pull {remote} {branch}");
        assert_eq!(got, vec!["branch".to_string(), "remote".to_string()]);
    }

    #[test]
    fn renders_placeholders() {
        let mut values = HashMap::new();
        values.insert("branch".to_string(), "main".to_string());
        values.insert("remote".to_string(), "origin".to_string());
        let got = render("git pull {remote} {branch}", &values);
        assert_eq!(got, "git pull origin main");
    }

    #[test]
    fn parses_assignment_values() {
        let got = parse_assignment_values("branch=main remote=origin");
        assert_eq!(got.get("branch"), Some(&"main".to_string()));
        assert_eq!(got.get("remote"), Some(&"origin".to_string()));
    }

    #[test]
    fn handles_escaped_braces() {
        let command = "echo {{not_a_placeholder}} {is_placeholder} }}";
        let found = placeholders(command);
        assert_eq!(found, vec!["is_placeholder".to_string()]);

        let mut values = HashMap::new();
        values.insert("is_placeholder".to_string(), "fixed".to_string());
        let rendered = render(command, &values);
        assert_eq!(rendered, "echo {not_a_placeholder} fixed }");
    }
}
