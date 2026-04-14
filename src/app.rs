use std::collections::HashMap;
use std::io;
use std::process::Command;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};

use crate::clipboard::ClipboardProvider;
use crate::model::{Recipe, reload_recipes};
use crate::template;

pub fn run(recipes: Vec<Recipe>) -> io::Result<()> {
    let mut terminal = ratatui::init();
    let result = run_loop(&mut terminal, App::new(recipes)?);
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
    running_command: Option<RunningCommand>,
    spinner_frame: usize,
    clipboard: ClipboardProvider,
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
    presets: Vec<String>,
    selected_preset: usize,
    stage: PromptStage,
}

enum PromptStage {
    ChoosePreset,
    InputValues,
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

struct RunningCommand {
    receiver: Receiver<RunOutput>,
}

impl App {
    fn new(recipes: Vec<Recipe>) -> io::Result<Self> {
        Ok(Self {
            recipes,
            query: String::new(),
            selected: 0,
            status: String::new(),
            mode: Mode::Normal,
            last_run: None,
            running_command: None,
            spinner_frame: 0,
            clipboard: ClipboardProvider::detect()?,
        })
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
        if matches!(action, Action::Run) && self.running_command.is_some() {
            self.status = "Already running a command".to_string();
            return;
        }

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
            presets: recipe.presets.clone(),
            selected_preset: 0,
            stage: if recipe.presets.is_empty() {
                PromptStage::InputValues
            } else {
                PromptStage::ChoosePreset
            },
        });
        self.status.clear();
    }

    fn execute_action(
        &mut self,
        action: Action,
        recipe_idx: usize,
        values: HashMap<String, String>,
    ) {
        let recipe_name = self.recipes[recipe_idx].name.clone();
        let rendered = template::render(&self.recipes[recipe_idx].command, &values);
        match action {
            Action::Copy => match self.copy_to_clipboard(&rendered) {
                Ok(()) => {
                    self.status = format!("Copied: {}", recipe_name);
                }
                Err(err) => {
                    self.status = format!("Clipboard error: {err}");
                }
            },
            Action::Run => {
                let (sender, receiver) = mpsc::channel();
                thread::spawn(move || {
                    let output = run_command(&rendered);
                    let _ = sender.send(output);
                });
                self.running_command = Some(RunningCommand { receiver });
                self.spinner_frame = 0;
                self.status.clear();
            }
        }
    }

    fn copy_to_clipboard(&mut self, text: &str) -> io::Result<()> {
        self.clipboard.copy(text)
    }

    fn poll_running_command(&mut self) {
        let mut finished_output = None;
        let mut disconnected = false;

        if let Some(running_command) = self.running_command.as_ref() {
            match running_command.receiver.try_recv() {
                Ok(output) => finished_output = Some(output),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => disconnected = true,
            }
        }

        if let Some(output) = finished_output {
            self.status = match output.code {
                Some(0) => "Ran successfully".to_string(),
                Some(code) => format!("Command exited with code {code}"),
                None => "Command terminated by signal".to_string(),
            };
            self.last_run = Some(output);
            self.running_command = None;
            self.spinner_frame = 0;
        } else if disconnected {
            self.status = "Command runner disconnected".to_string();
            self.running_command = None;
            self.spinner_frame = 0;
        }
    }

    fn tick(&mut self) {
        if self.running_command.is_some() {
            self.spinner_frame = (self.spinner_frame + 1) % 4;
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
        app.poll_running_command();
        terminal.draw(|frame| render(frame, &app))?;

        if !event::poll(Duration::from_millis(150))? {
            app.tick();
            continue;
        }
        let Event::Key(key) = event::read()? else {
            app.tick();
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

        app.tick();
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
            app.status.clear();
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
            app.status.clear();
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.mode = Mode::Normal;
            app.status.clear();
            app.move_selection(-1);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.mode = Mode::Normal;
            app.status.clear();
            app.move_selection(1);
        }
        KeyCode::Enter => {
            app.mode = Mode::Normal;
            app.status.clear();
            app.start_action(Action::Run);
        }
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
    let mut status: Option<String> = None;

    if let Mode::Prompt(prompt) = &mut app.mode {
        match prompt.stage {
            PromptStage::ChoosePreset => match key.code {
                KeyCode::Esc => {
                    app.mode = Mode::Normal;
                    app.status = "Cancelled".to_string();
                    return;
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    if prompt.selected_preset > 0 {
                        prompt.selected_preset -= 1;
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if prompt.selected_preset < prompt.presets.len() {
                        prompt.selected_preset += 1;
                    }
                }
                KeyCode::Enter => {
                    if prompt.selected_preset < prompt.presets.len() {
                        let values = template::parse_assignment_values(
                            &prompt.presets[prompt.selected_preset],
                        );
                        if let Some(missing) = prompt
                            .placeholders
                            .iter()
                            .find(|placeholder| !values.contains_key(*placeholder))
                        {
                            status = Some(format!("Preset missing value for {{{missing}}}"));
                        } else {
                            execute = Some((prompt.action, prompt.recipe_idx, values));
                        }
                    } else {
                        prompt.stage = PromptStage::InputValues;
                    }
                }
                _ => {}
            },
            PromptStage::InputValues => match key.code {
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
            },
        }
    }

    if let Some(status) = status {
        app.status = status;
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

    let shortcut_text = match app.mode {
        Mode::Normal => "Normal: / search | Enter run | Ctrl+Y copy | r reload | q / Ctrl+C quit",
        Mode::Search => {
            "Search: type filter | Up/Down move | Enter run | Ctrl+Y copy | Esc stop editing"
        }
        Mode::Prompt(_) => "Prompt: type value | Enter continue | Esc cancel",
    };
    let footer = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(10), Constraint::Length(20)])
        .split(layout[2]);
    let shortcuts = Paragraph::new(Line::from(shortcut_text))
        .block(Block::default().borders(Borders::ALL).title("Shortcuts"))
        .wrap(Wrap { trim: true });
    frame.render_widget(shortcuts, footer[0]);

    let (state_text, state_style) = footer_state(app);
    let state = Paragraph::new(Line::from(state_text).style(state_style))
        .block(Block::default().borders(Borders::ALL).title("State"))
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true });
    frame.render_widget(state, footer[1]);

    if let Mode::Prompt(prompt) = &app.mode {
        render_prompt(frame, prompt);
    }
}

fn footer_state(app: &App) -> (String, Style) {
    if app.running_command.is_some() {
        let frames = ["|", "/", "-", "\\"];
        let spinner = frames[app.spinner_frame % frames.len()];
        return (format!("RUN {spinner}"), Style::default().fg(Color::Cyan));
    }

    if app.status.starts_with("Clipboard error:") {
        return ("COPY ERROR".to_string(), Style::default().fg(Color::Red));
    }
    if app.status.starts_with("Reload failed:") {
        return ("RELOAD ERROR".to_string(), Style::default().fg(Color::Red));
    }
    if app.status == "Command terminated by signal" {
        return ("SIGNAL".to_string(), Style::default().fg(Color::Red));
    }
    if app.status.starts_with("Command exited with code") {
        return ("RUN FAILED".to_string(), Style::default().fg(Color::Red));
    }
    if app.status == "Command runner disconnected" {
        return ("RUN ERROR".to_string(), Style::default().fg(Color::Red));
    }
    if app.status.starts_with("Copied:") {
        return (
            "COPIED!".to_string(),
            Style::default().fg(Color::LightGreen),
        );
    }
    if app.status == "Reloaded recipes" {
        return ("RELOADED".to_string(), Style::default().fg(Color::Cyan));
    }
    if app.status == "Ran successfully" {
        return ("RAN".to_string(), Style::default().fg(Color::LightGreen));
    }
    if app.status == "No recipe selected" {
        return ("NO RECIPE".to_string(), Style::default().fg(Color::Yellow));
    }
    if app.status == "Cancelled" {
        return ("CANCELLED".to_string(), Style::default().fg(Color::Yellow));
    }
    if app.status == "Already running a command" {
        return ("RUNNING".to_string(), Style::default().fg(Color::Yellow));
    }

    match app.mode {
        Mode::Normal => ("NORMAL".to_string(), Style::default().fg(Color::Blue)),
        Mode::Search => ("SEARCH".to_string(), Style::default().fg(Color::Yellow)),
        Mode::Prompt(_) => ("PROMPT".to_string(), Style::default().fg(Color::Magenta)),
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
    ];

    for line in recipe.command.lines() {
        lines.push(Line::from(format!("  {line}")));
    }

    if !recipe.presets.is_empty() {
        lines.push(Line::from(String::new()));
        lines.push(Line::from("Presets:"));
        for preset in &recipe.presets {
            lines.push(Line::from(format!("- {}", preset)));
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
    let area = match prompt.stage {
        PromptStage::ChoosePreset => centered_rect(60, 20, frame.area()),
        PromptStage::InputValues => centered_rect(70, 40, frame.area()),
    };
    frame.render_widget(Clear, area);

    let (title, text) = match prompt.stage {
        PromptStage::ChoosePreset => {
            let title = match prompt.action {
                Action::Run => "Choose preset to run",
                Action::Copy => "Choose preset to copy",
            };
            let mut lines = Vec::new();
            lines.push(
                "Use Up/Down to choose a preset, Enter to continue, Esc to cancel".to_string(),
            );
            lines.push(String::new());
            for (idx, preset) in prompt.presets.iter().enumerate() {
                let marker = if idx == prompt.selected_preset {
                    ">"
                } else {
                    " "
                };
                lines.push(format!("{} {}", marker, preset));
            }
            let custom_marker = if prompt.selected_preset == prompt.presets.len() {
                ">"
            } else {
                " "
            };
            lines.push(format!("{} Custom values", custom_marker));
            (title, Text::from(lines.join("\n")))
        }
        PromptStage::InputValues => {
            let field = &prompt.placeholders[prompt.current];
            let title = match prompt.action {
                Action::Run => "Fill placeholders to run",
                Action::Copy => "Fill placeholders to copy",
            };
            let mut lines = vec![
                Line::from(format!(
                    "Field {}/{} is active",
                    prompt.current + 1,
                    prompt.placeholders.len()
                )),
                Line::from(String::new()),
            ];

            for (idx, placeholder) in prompt.placeholders.iter().enumerate() {
                let value = if idx < prompt.current {
                    prompt.values.get(placeholder).cloned().unwrap_or_default()
                } else if idx == prompt.current {
                    prompt.input.clone()
                } else {
                    String::new()
                };

                let prefix = if idx == prompt.current { ">" } else { " " };
                let line = if idx == prompt.current {
                    Line::from(vec![
                        Span::styled(
                            format!("{} {{{}}}: ", prefix, placeholder),
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(value, Style::default().fg(Color::Yellow)),
                    ])
                } else {
                    Line::from(format!("{} {{{}}}: {}", prefix, placeholder, value))
                };
                lines.push(line);
            }

            lines.push(Line::from(String::new()));
            lines.push(Line::from("Enter to continue, Esc to cancel"));
            (title, Text::from(lines))
        }
    };

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
