use std::collections::{BTreeSet, HashMap};

pub fn placeholders(command: &str) -> Vec<String> {
    let mut found = BTreeSet::new();
    let chars: Vec<char> = command.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i] == '{' {
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
        }
        out.push(chars[i]);
        i += 1;
    }
    out
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

    use super::{placeholders, render};

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
}
