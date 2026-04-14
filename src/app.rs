use std::collections::HashMap;
use std::io;
use std::process::Command;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Text};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};

use crate::model::{Recipe, reload_recipes};
use crate::template;

pub fn run(recipes: Vec<Recipe>) -> io::Result<()> {
    let mut terminal = ratatui::init();
    let result = run_loop(&mut terminal, App::new(recipes));
    ratatui::restore();
    result
}

struct App {
    recipes: Vec<Recipe>,
    query: String,
    selected: usize,
    status: String,
    mode: Mode,
    last_run: Option<RunOutput>,
}

enum Mode {
    Normal,
    Search,
    Prompt(PromptState),
}

struct PromptState {
    action: Action,
    recipe_idx: usize,
    placeholders: Vec<String>,
    current: usize,
    values: HashMap<String, String>,
    input: String,
}

#[derive(Copy, Clone)]
enum Action {
    Run,
    Copy,
}

struct RunOutput {
    command: String,
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

impl App {
    fn new(recipes: Vec<Recipe>) -> Self {
        Self {
            recipes,
            query: String::new(),
            selected: 0,
            status: String::from("Press / to search recipes"),
            mode: Mode::Normal,
            last_run: None,
        }
    }

    fn filtered_indices(&self) -> Vec<usize> {
        if self.query.is_empty() {
            return (0..self.recipes.len()).collect();
        }

        let matcher = SkimMatcherV2::default();
        let mut scored = Vec::new();
        for (idx, recipe) in self.recipes.iter().enumerate() {
            let haystack = format!(
                "{} {} {} {}",
                recipe.name,
                recipe.tags.join(" "),
                recipe.description,
                recipe.command
            );
            if let Some(score) = matcher.fuzzy_match(&haystack, &self.query) {
                scored.push((idx, score));
            }
        }
        scored.sort_by(|a, b| b.1.cmp(&a.1));
        scored.into_iter().map(|(idx, _)| idx).collect()
    }

    fn selected_recipe_index(&self, filtered: &[usize]) -> Option<usize> {
        filtered.get(self.selected).copied()
    }

    fn move_selection(&mut self, delta: isize) {
        let filtered_len = self.filtered_indices().len();
        if filtered_len == 0 {
            self.selected = 0;
            return;
        }
        let next = self.selected as isize + delta;
        self.selected = next.clamp(0, (filtered_len - 1) as isize) as usize;
    }

    fn reset_selection_if_needed(&mut self) {
        let filtered_len = self.filtered_indices().len();
        if filtered_len == 0 {
            self.selected = 0;
        } else if self.selected >= filtered_len {
            self.selected = filtered_len - 1;
        }
    }

    fn start_action(&mut self, action: Action) {
        let filtered = self.filtered_indices();
        let Some(recipe_idx) = self.selected_recipe_index(&filtered) else {
            self.status = "No recipe selected".to_string();
            return;
        };
        let recipe = &self.recipes[recipe_idx];
        let placeholders = template::placeholders(&recipe.command);
        if placeholders.is_empty() {
            self.execute_action(action, recipe_idx, HashMap::new());
            return;
        }
        self.mode = Mode::Prompt(PromptState {
            action,
            recipe_idx,
            placeholders,
            current: 0,
            values: HashMap::new(),
            input: String::new(),
        });
    }

    fn execute_action(
        &mut self,
        action: Action,
        recipe_idx: usize,
        values: HashMap<String, String>,
    ) {
        let recipe = &self.recipes[recipe_idx];
        let rendered = template::render(&recipe.command, &values);
        match action {
            Action::Copy => match copy_to_clipboard(&rendered) {
                Ok(()) => {
                    self.status = format!("Copied: {}", recipe.name);
                }
                Err(err) => {
                    self.status = format!("Clipboard error: {err}");
                }
            },
            Action::Run => {
                let output = run_command(&rendered);
                self.status = match output.code {
                    Some(0) => format!("Ran successfully: {}", recipe.name),
                    Some(code) => format!("Command exited with code {code}"),
                    None => "Command terminated by signal".to_string(),
                };
                self.last_run = Some(output);
            }
        }
    }

    fn reload(&mut self) {
        match reload_recipes() {
            Ok(recipes) => {
                self.recipes = recipes;
                self.status = "Reloaded recipes".to_string();
                self.reset_selection_if_needed();
            }
            Err(err) => {
                self.status = format!("Reload failed: {err}");
            }
        }
    }
}

fn run_loop(terminal: &mut ratatui::DefaultTerminal, mut app: App) -> io::Result<()> {
    loop {
        terminal.draw(|frame| render(frame, &app))?;

        if !event::poll(Duration::from_millis(150))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };

        match &mut app.mode {
            Mode::Normal => {
                if handle_normal_key(&mut app, key) {
                    break;
                }
            }
            Mode::Search => {
                if handle_search_key(&mut app, key) {
                    break;
                }
            }
            Mode::Prompt(_) => {
                handle_prompt_key(&mut app, key);
            }
        }
    }

    Ok(())
}

fn handle_normal_key(app: &mut App, key: KeyEvent) -> bool {
    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c')) {
        return true;
    }

    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('y')) {
        app.start_action(Action::Copy);
        app.reset_selection_if_needed();
        return false;
    }

    match key.code {
        KeyCode::Char('q') => return true,
        KeyCode::Up | KeyCode::Char('k') => app.move_selection(-1),
        KeyCode::Down | KeyCode::Char('j') => app.move_selection(1),
        KeyCode::Enter => app.start_action(Action::Run),
        KeyCode::Char('r') => app.reload(),
        KeyCode::Char('/') => {
            app.mode = Mode::Search;
            app.status = "Search mode".to_string();
        }
        _ => {}
    }

    app.reset_selection_if_needed();
    false
}

fn handle_search_key(app: &mut App, key: KeyEvent) -> bool {
    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c')) {
        return true;
    }

    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('y')) {
        app.start_action(Action::Copy);
        app.reset_selection_if_needed();
        return false;
    }

    match key.code {
        KeyCode::Esc => {
            app.mode = Mode::Normal;
            app.status = "Navigation mode".to_string();
        }
        KeyCode::Up | KeyCode::Char('k') => app.move_selection(-1),
        KeyCode::Down | KeyCode::Char('j') => app.move_selection(1),
        KeyCode::Enter => app.start_action(Action::Run),
        KeyCode::Backspace => {
            app.query.pop();
            app.selected = 0;
        }
        KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.query.push(ch);
            app.selected = 0;
        }
        _ => {}
    }

    app.reset_selection_if_needed();
    false
}

fn handle_prompt_key(app: &mut App, key: KeyEvent) {
    let mut execute: Option<(Action, usize, HashMap<String, String>)> = None;

    if let Mode::Prompt(prompt) = &mut app.mode {
        match key.code {
            KeyCode::Esc => {
                app.mode = Mode::Normal;
                app.status = "Cancelled".to_string();
                return;
            }
            KeyCode::Backspace => {
                prompt.input.pop();
            }
            KeyCode::Enter => {
                let key_name = prompt.placeholders[prompt.current].clone();
                prompt.values.insert(key_name, prompt.input.clone());
                prompt.input.clear();
                prompt.current += 1;
                if prompt.current >= prompt.placeholders.len() {
                    execute = Some((prompt.action, prompt.recipe_idx, prompt.values.clone()));
                }
            }
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                prompt.input.push(ch);
            }
            _ => {}
        }
    }

    if let Some((action, recipe_idx, values)) = execute {
        app.mode = Mode::Normal;
        app.execute_action(action, recipe_idx, values);
    }
}

fn render(frame: &mut Frame, app: &App) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(3),
            Constraint::Length(3),
        ])
        .split(frame.area());

    let filtered = app.filtered_indices();

    let search_title = match app.mode {
        Mode::Normal => "Search (/ to edit)",
        Mode::Search => "Search (Esc to stop editing)",
        Mode::Prompt(_) => "Search",
    };
    let search = Paragraph::new(app.query.clone())
        .block(Block::default().borders(Borders::ALL).title(search_title));
    frame.render_widget(search, layout[0]);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(layout[1]);

    let items: Vec<ListItem> = filtered
        .iter()
        .map(|idx| {
            let recipe = &app.recipes[*idx];
            let subtitle = if recipe.tags.is_empty() {
                String::new()
            } else {
                format!(" [{}]", recipe.tags.join(", "))
            };
            ListItem::new(Line::from(format!("{}{}", recipe.name, subtitle)))
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Recipes"))
        .highlight_style(Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED))
        .highlight_symbol("> ");
    let mut state = ListState::default();
    if !filtered.is_empty() {
        state.select(Some(app.selected));
    }
    frame.render_stateful_widget(list, body[0], &mut state);

    let detail_text = recipe_details(app, &filtered);
    let details = Paragraph::new(detail_text)
        .block(Block::default().borders(Borders::ALL).title("Details"))
        .wrap(Wrap { trim: false });
    frame.render_widget(details, body[1]);

    let status = Paragraph::new(app.status.clone())
        .block(Block::default().borders(Borders::ALL).title("Status"));
    frame.render_widget(status, layout[2]);

    let shortcut_text = match app.mode {
        Mode::Normal => "Normal: / search | Enter run | Ctrl+Y copy | r reload | q / Ctrl+C quit",
        Mode::Search => {
            "Search: type filter | Up/Down move | Enter run | Ctrl+Y copy | Esc stop editing"
        }
        Mode::Prompt(_) => "Prompt: type value | Enter continue | Esc cancel",
    };
    let shortcuts = Paragraph::new(Line::from(shortcut_text))
        .block(Block::default().borders(Borders::ALL).title("Shortcuts"))
        .wrap(Wrap { trim: true });
    frame.render_widget(shortcuts, layout[3]);

    if let Mode::Prompt(prompt) = &app.mode {
        render_prompt(frame, prompt);
    }
}

fn recipe_details(app: &App, filtered: &[usize]) -> Text<'static> {
    let Some(recipe_idx) = app.selected_recipe_index(filtered) else {
        return Text::from("No recipes found. Add some in ~/.config/pantry/recipes.toml");
    };
    let recipe = &app.recipes[recipe_idx];

    let mut lines = vec![
        Line::from(format!("Name: {}", recipe.name)),
        Line::from(format!("Safety: {}", recipe.safety)),
        Line::from(format!("Source: {}", recipe.source)),
        Line::from(String::new()),
        Line::from("Description:"),
        Line::from(recipe.description.clone()),
        Line::from(String::new()),
        Line::from("Command:"),
        Line::from(recipe.command.clone()),
    ];

    if !recipe.examples.is_empty() {
        lines.push(Line::from(String::new()));
        lines.push(Line::from("Examples:"));
        for example in &recipe.examples {
            lines.push(Line::from(format!("- {}", example)));
        }
    }

    if let Some(run) = &app.last_run {
        lines.push(Line::from(String::new()));
        lines.push(Line::from("Last run:"));
        lines.push(Line::from(format!("$ {}", run.command)));
        lines.push(Line::from(format!("exit: {:?}", run.code)));
        if !run.stdout.trim().is_empty() {
            lines.push(Line::from("stdout:"));
            for line in run.stdout.lines().take(6) {
                lines.push(Line::from(line.to_string()));
            }
        }
        if !run.stderr.trim().is_empty() {
            lines.push(Line::from("stderr:"));
            for line in run.stderr.lines().take(6) {
                lines.push(Line::from(line.to_string()));
            }
        }
    }

    Text::from(lines)
}

fn render_prompt(frame: &mut Frame, prompt: &PromptState) {
    let area = centered_rect(60, 20, frame.area());
    frame.render_widget(Clear, area);

    let field = &prompt.placeholders[prompt.current];
    let title = match prompt.action {
        Action::Run => "Fill placeholders to run",
        Action::Copy => "Fill placeholders to copy",
    };
    let text = format!(
        "Value for {{{}}} ({}/{}):\n{}\n\nEnter to continue, Esc to cancel",
        field,
        prompt.current + 1,
        prompt.placeholders.len(),
        prompt.input
    );

    let popup = Paragraph::new(text)
        .block(Block::default().title(title).borders(Borders::ALL))
        .wrap(Wrap { trim: false });
    frame.render_widget(popup, area);
}

fn centered_rect(percent_x: u16, percent_y: u16, rect: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(rect);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

fn copy_to_clipboard(text: &str) -> io::Result<()> {
    let mut clipboard = arboard::Clipboard::new()
        .map_err(|err| io::Error::other(format!("failed to open clipboard: {err}")))?;
    clipboard
        .set_text(text.to_string())
        .map_err(|err| io::Error::other(format!("failed to copy text: {err}")))
}

fn run_command(command: &str) -> RunOutput {
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
