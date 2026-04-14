use nom::{
    branch::alt,
    bytes::complete::{tag, take_while1},
    character::complete::{multispace0, multispace1},
    combinator::map,
    multi::{many0, separated_list0},
    sequence::{delimited, separated_pair},
    IResult, Parser,
};
use std::collections::{BTreeSet, HashMap};

#[derive(Debug, PartialEq)]
enum Part {
    Literal(String),
    Placeholder(String),
    EscapedOpen,
    EscapedClose,
}

fn parse_placeholder(input: &str) -> IResult<&str, Part> {
    map(
        delimited(tag("{"), take_while1(is_valid_name_char), tag("}")),
        |name: &str| Part::Placeholder(name.to_string()),
    )
    .parse(input)
}

fn parse_escaped_open(input: &str) -> IResult<&str, Part> {
    map(tag("{{"), |_| Part::EscapedOpen).parse(input)
}

fn parse_escaped_close(input: &str) -> IResult<&str, Part> {
    map(tag("}}"), |_| Part::EscapedClose).parse(input)
}

fn parse_literal(input: &str) -> IResult<&str, Part> {
    map(take_while1(|c| c != '{' && c != '}'), |s: &str| {
        Part::Literal(s.to_string())
    })
    .parse(input)
}

fn parse_literal_single_brace(input: &str) -> IResult<&str, Part> {
    map(alt((tag("{"), tag("}"))), |s: &str| {
        Part::Literal(s.to_string())
    })
    .parse(input)
}

fn parse_template(input: &str) -> IResult<&str, Vec<Part>> {
    many0(alt((
        parse_escaped_open,
        parse_escaped_close,
        parse_placeholder,
        parse_literal,
        parse_literal_single_brace,
    )))
    .parse(input)
}

fn is_valid_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

pub fn placeholders(command: &str) -> Vec<String> {
    let mut found = BTreeSet::new();
    if let Ok((_, parts)) = parse_template(command) {
        for part in parts {
            if let Part::Placeholder(name) = part {
                found.insert(name);
            }
        }
    }
    found.into_iter().collect()
}

pub fn render(command: &str, values: &HashMap<String, String>) -> String {
    let mut out = String::new();
    if let Ok((_, parts)) = parse_template(command) {
        for part in parts {
            match part {
                Part::Literal(s) => out.push_str(&s),
                Part::Placeholder(name) => {
                    if let Some(value) = values.get(&name) {
                        out.push_str(value);
                    } else {
                        out.push('{');
                        out.push_str(&name);
                        out.push('}');
                    }
                }
                Part::EscapedOpen => out.push('{'),
                Part::EscapedClose => out.push('}'),
            }
        }
    }
    out
}

fn parse_assignment(input: &str) -> IResult<&str, (String, String)> {
    map(
        separated_pair(
            take_while1(is_valid_name_char),
            tag("="),
            take_while1(|c: char| !c.is_whitespace()),
        ),
        |(k, v): (&str, &str)| (k.to_string(), v.to_string()),
    )
    .parse(input)
}

pub fn parse_assignment_values(input: &str) -> HashMap<String, String> {
    let mut parser = delimited(
        multispace0,
        separated_list0(multispace1, parse_assignment),
        multispace0,
    );
    if let Ok((_, assignments)) = parser.parse(input) {
        assignments.into_iter().collect()
    } else {
        HashMap::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

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

    #[test]
    fn handles_complex_template() {
        let command = "ls {dir} {{literal}} {file}";
        let found = placeholders(command);
        assert_eq!(found, vec!["dir".to_string(), "file".to_string()]);

        let mut values = HashMap::new();
        values.insert("dir".to_string(), "/tmp".to_string());
        values.insert("file".to_string(), "test.txt".to_string());
        let rendered = render(command, &values);
        assert_eq!(rendered, "ls /tmp {literal} test.txt");
    }

    #[test]
    fn handles_multiline_commands() {
        let command = "git commit -m \"{message}\"\ngit push {remote} {branch}";
        let found = placeholders(command);
        assert_eq!(
            found,
            vec![
                "branch".to_string(),
                "message".to_string(),
                "remote".to_string()
            ]
        );

        let mut values = HashMap::new();
        values.insert("message".to_string(), "feat: add tests".to_string());
        values.insert("remote".to_string(), "origin".to_string());
        values.insert("branch".to_string(), "main".to_string());

        let rendered = render(command, &values);
        assert_eq!(
            rendered,
            "git commit -m \"feat: add tests\"\ngit push origin main"
        );
    }

    #[test]
    fn parses_multiline_assignments() {
        let got = parse_assignment_values("branch=main\nremote=origin");
        assert_eq!(got.get("branch"), Some(&"main".to_string()));
        assert_eq!(got.get("remote"), Some(&"origin".to_string()));
    }

    #[test]
    fn handles_newlines_at_edges() {
        let command = "\n{cmd}\n";
        let found = placeholders(command);
        assert_eq!(found, vec!["cmd".to_string()]);

        let mut values = HashMap::new();
        values.insert("cmd".to_string(), "ls".to_string());
        let rendered = render(command, &values);
        assert_eq!(rendered, "\nls\n");
    }
}
