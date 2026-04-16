use std::collections::HashMap;
use std::io;
use std::process::Command;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use crate::clipboard::ClipboardProvider;
use crate::model::{reload_recipes, Recipe, RunOutput};
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
    choices: HashMap<String, Vec<String>>,
    current: usize,
    values: HashMap<String, String>,
    input: String,
    choice_index: usize,
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
    Copy { quit_after: bool },
}

struct RunningCommand {
    recipe_idx: usize,
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

    fn start_action(&mut self, action: Action) -> bool {
        if matches!(action, Action::Run) && self.running_command.is_some() {
            self.status = "Already running a command".to_string();
            return false;
        }

        let filtered = self.filtered_indices();
        let Some(recipe_idx) = self.selected_recipe_index(&filtered) else {
            self.status = "No recipe selected".to_string();
            return false;
        };
        let recipe = &self.recipes[recipe_idx];
        let placeholders = recipe.compiled.placeholders();
        if placeholders.is_empty() {
            return self.execute_action(action, recipe_idx, HashMap::new());
        }
        self.mode = Mode::Prompt(PromptState {
            action,
            recipe_idx,
            placeholders,
            choices: recipe.choices.clone(),
            current: 0,
            values: HashMap::new(),
            input: String::new(),
            choice_index: 0,
            presets: recipe.presets.clone(),
            selected_preset: 0,
            stage: if recipe.presets.is_empty() {
                PromptStage::InputValues
            } else {
                PromptStage::ChoosePreset
            },
        });
        self.status.clear();
        false
    }

    fn execute_action(
        &mut self,
        action: Action,
        recipe_idx: usize,
        values: HashMap<String, String>,
    ) -> bool {
        let recipe = &self.recipes[recipe_idx];
        let recipe_name = recipe.name.clone();
        let rendered = recipe.compiled.render(&values);
        match action {
            Action::Copy { quit_after } => match self.copy_to_clipboard(&rendered) {
                Ok(()) => {
                    self.status = format!("Copied: {}", recipe_name);
                    return quit_after;
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
                self.running_command = Some(RunningCommand {
                    recipe_idx,
                    receiver,
                });
                self.spinner_frame = 0;
                self.status.clear();
            }
        }

        false
    }

    fn copy_to_clipboard(&mut self, text: &str) -> io::Result<()> {
        self.clipboard.copy(text)
    }

    fn poll_running_command(&mut self) {
        let mut finished = None;
        let mut disconnected = false;

        if let Some(running_command) = self.running_command.as_ref() {
            match running_command.receiver.try_recv() {
                Ok(output) => {
                    finished = Some((running_command.recipe_idx, output));
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => disconnected = true,
            }
        }

        if let Some((idx, output)) = finished {
            self.status = match output.code {
                Some(0) => "Ran successfully".to_string(),
                Some(code) => format!("Command exited with code {code}"),
                None => "Command terminated by signal".to_string(),
            };
            if idx < self.recipes.len() {
                self.recipes[idx].last_run = Some(output);
            }
            self.running_command = None;
        } else if disconnected {
            self.status = "Command runner disconnected".to_string();
            self.running_command = None;
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

impl PromptState {
    fn current_placeholder(&self) -> &str {
        &self.placeholders[self.current]
    }

    fn current_choices(&self) -> Option<&[String]> {
        self.choices
            .get(self.current_placeholder())
            .filter(|choices| !choices.is_empty())
            .map(Vec::as_slice)
    }

    fn advance(&mut self) -> bool {
        self.input.clear();
        self.choice_index = 0;
        self.current += 1;
        self.current >= self.placeholders.len()
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
                if handle_prompt_key(&mut app, key) {
                    break;
                }
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

    match key.code {
        KeyCode::Char('q') => return true,
        KeyCode::Char('Y') => {
            if app.start_action(Action::Copy { quit_after: true }) {
                return true;
            }
            app.reset_selection_if_needed();
        }
        KeyCode::Char('y') => {
            app.start_action(Action::Copy { quit_after: false });
        }
        KeyCode::Up => app.move_selection(-1),
        KeyCode::Down => app.move_selection(1),
        KeyCode::Enter => {
            app.start_action(Action::Run);
        }
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

    match key.code {
        KeyCode::Esc => {
            app.mode = Mode::Normal;
            app.status.clear();
        }
        KeyCode::Up => {
            app.mode = Mode::Normal;
            app.status.clear();
            app.move_selection(-1);
        }
        KeyCode::Down => {
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

fn handle_prompt_key(app: &mut App, key: KeyEvent) -> bool {
    let mut execute: Option<(Action, usize, HashMap<String, String>)> = None;
    let mut status: Option<String> = None;

    if let Mode::Prompt(prompt) = &mut app.mode {
        match prompt.stage {
            PromptStage::ChoosePreset => match key.code {
                KeyCode::Esc => {
                    app.mode = Mode::Normal;
                    app.status = "Cancelled".to_string();
                    return false;
                }
                KeyCode::Up => {
                    if prompt.selected_preset > 0 {
                        prompt.selected_preset -= 1;
                    }
                }
                KeyCode::Down => {
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
                    return false;
                }
                KeyCode::Up => {
                    if prompt.current_choices().is_some() && prompt.choice_index > 0 {
                        prompt.choice_index -= 1;
                    }
                }
                KeyCode::Down => {
                    if let Some(choices) = prompt.current_choices() {
                        let last_index = choices.len().saturating_sub(1);
                        if prompt.choice_index < last_index {
                            prompt.choice_index += 1;
                        }
                    }
                }
                KeyCode::Backspace => {
                    if prompt.current_choices().is_none() {
                        prompt.input.pop();
                    }
                }
                KeyCode::Enter => {
                    let key_name = prompt.current_placeholder().to_string();
                    let value = if let Some(choices) = prompt.current_choices() {
                        choices[prompt.choice_index].clone()
                    } else {
                        prompt.input.clone()
                    };
                    prompt.values.insert(key_name, value);
                    if prompt.advance() {
                        execute = Some((prompt.action, prompt.recipe_idx, prompt.values.clone()));
                    }
                }
                KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    if prompt.current_choices().is_none() {
                        prompt.input.push(ch);
                    }
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
        return app.execute_action(action, recipe_idx, values);
    }

    false
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
        Mode::Normal => " Search (/ to edit) ",
        Mode::Search => " Search (Esc to stop editing) ",
        Mode::Prompt(_) => " Search ",
    };

    let search_block = match app.mode {
        Mode::Search => Block::default()
            .borders(Borders::ALL)
            .title(search_title)
            .border_style(Style::default().fg(Color::Yellow)),
        _ => Block::default().borders(Borders::ALL).title(search_title),
    };

    let search = Paragraph::new(app.query.clone()).block(search_block);
    frame.render_widget(search, layout[0]);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(layout[1]);

    let items: Vec<ListItem> = filtered
        .iter()
        .map(|idx| {
            let recipe = &app.recipes[*idx];
            let mut spans = vec![Span::raw(recipe.name.clone())];
            for tag in &recipe.tags {
                spans.push(Span::raw(" "));
                spans.push(Span::styled(
                    format!("[{}]", tag),
                    Style::default().fg(Color::Cyan),
                ));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();

    let list_block = match app.mode {
        Mode::Normal => Block::default()
            .borders(Borders::ALL)
            .title(" Recipes ")
            .border_style(Style::default().fg(Color::Blue)),
        _ => Block::default().borders(Borders::ALL).title(" Recipes "),
    };

    let list = List::new(items)
        .block(list_block)
        .highlight_style(Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED))
        .highlight_symbol("> ");
    let mut state = ListState::default();
    if !filtered.is_empty() {
        state.select(Some(app.selected));
    }
    frame.render_stateful_widget(list, body[0], &mut state);

    let detail_text = recipe_details(app, &filtered);
    let details = Paragraph::new(detail_text)
        .block(Block::default().borders(Borders::ALL).title(" Details "))
        .wrap(Wrap { trim: false });
    frame.render_widget(details, body[1]);

    let shortcut_text = match app.mode {
        Mode::Normal => {
            "Normal: / search | Enter run | y copy | Y copy+quit | r reload | q / Ctrl+C quit"
        }
        Mode::Search => "Search: type filter | Up/Down move | Enter run | Esc stop editing",
        Mode::Prompt(_) => "Prompt: type value | Enter continue | Esc cancel",
    };
    let footer = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(10), Constraint::Length(20)])
        .split(layout[2]);
    let shortcuts = Paragraph::new(Line::from(shortcut_text))
        .block(Block::default().borders(Borders::ALL).title(" Shortcuts "))
        .wrap(Wrap { trim: true });
    frame.render_widget(shortcuts, footer[0]);

    let (state_text, state_style) = footer_state(app);
    let state = Paragraph::new(Line::from(state_text).style(state_style))
        .block(Block::default().borders(Borders::ALL).title(" State "))
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

    let header_style = Style::default()
        .fg(Color::Blue)
        .add_modifier(Modifier::BOLD);

    let mut lines = vec![
        Line::from(vec![
            Span::styled("NAME: ", header_style),
            Span::raw(recipe.name.clone()),
        ]),
        Line::from(vec![
            Span::styled("SAFETY: ", header_style),
            Span::styled(
                recipe.safety.clone(),
                if recipe.safety == "safe" {
                    Style::default().fg(Color::Green)
                } else {
                    Style::default().fg(Color::Red)
                },
            ),
        ]),
        Line::from(vec![
            Span::styled("SOURCE: ", header_style),
            Span::styled(recipe.source.clone(), Style::default().fg(Color::Magenta)),
        ]),
        Line::from(String::new()),
        Line::from(vec![Span::styled("DESCRIPTION:", header_style)]),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                recipe.description.clone(),
                Style::default().add_modifier(Modifier::ITALIC),
            ),
        ]),
        Line::from(String::new()),
        Line::from(vec![Span::styled("COMMAND:", header_style)]),
    ];

    for line in recipe.command.lines() {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(line.to_string(), Style::default().fg(Color::LightYellow)),
        ]));
    }

    if !recipe.presets.is_empty() {
        lines.push(Line::from(String::new()));
        lines.push(Line::from(vec![Span::styled("PRESETS:", header_style)]));
        for preset in &recipe.presets {
            lines.push(Line::from(vec![
                Span::raw("  - "),
                Span::styled(preset.clone(), Style::default().fg(Color::Cyan)),
            ]));
        }
    }

    if !recipe.choices.is_empty() {
        lines.push(Line::from(String::new()));
        lines.push(Line::from(vec![Span::styled("CHOICES:", header_style)]));
        let mut choice_names: Vec<_> = recipe.choices.keys().collect();
        choice_names.sort();
        for name in choice_names {
            if let Some(values) = recipe.choices.get(name) {
                lines.push(Line::from(vec![
                    Span::raw("  - "),
                    Span::styled(format!("{{{name}}}"), Style::default().fg(Color::Cyan)),
                    Span::raw(": "),
                    Span::raw(values.join(", ")),
                ]));
            }
        }
    }

    if let Some(run) = &recipe.last_run {
        lines.push(Line::from(String::new()));
        lines.push(Line::from(vec![Span::styled("LAST RUN:", header_style)]));
        lines.push(Line::from(format!("  $ {}", run.command)));
        lines.push(Line::from(vec![
            Span::raw("  exit: "),
            Span::styled(
                format!("{:?}", run.code),
                if run.code == Some(0) {
                    Style::default().fg(Color::Green)
                } else {
                    Style::default().fg(Color::Red)
                },
            ),
        ]));
        if !run.stdout.trim().is_empty() {
            lines.push(Line::from("  stdout:"));
            for line in run.stdout.lines().take(6) {
                lines.push(Line::from(format!("    {}", line)));
            }
        }
        if !run.stderr.trim().is_empty() {
            lines.push(Line::from("  stderr:"));
            for line in run.stderr.lines().take(6) {
                lines.push(Line::from(format!("    {}", line)));
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
                Action::Run => " Choose preset to run ",
                Action::Copy { quit_after: false } => " Choose preset to copy ",
                Action::Copy { quit_after: true } => " Choose preset to copy and quit ",
            };
            let mut lines = Vec::new();
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
            let title = match prompt.action {
                Action::Run => " Fill placeholders to run ",
                Action::Copy { quit_after: false } => " Fill placeholders to copy ",
                Action::Copy { quit_after: true } => " Fill placeholders to copy and quit ",
            };
            let mut lines = Vec::new();
            let current_choices = prompt.current_choices();

            for (idx, placeholder) in prompt.placeholders.iter().enumerate() {
                let value = if idx < prompt.current {
                    prompt.values.get(placeholder).cloned().unwrap_or_default()
                } else if idx == prompt.current {
                    if let Some(choices) = current_choices {
                        choices[prompt.choice_index].clone()
                    } else {
                        prompt.input.clone()
                    }
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

                if idx == prompt.current {
                    if let Some(choices) = current_choices {
                        for (choice_idx, choice) in choices.iter().enumerate() {
                            let marker = if choice_idx == prompt.choice_index {
                                ">"
                            } else {
                                " "
                            };
                            lines.push(Line::from(vec![
                                Span::raw("    "),
                                Span::styled(
                                    format!("{} {}", marker, choice),
                                    if choice_idx == prompt.choice_index {
                                        Style::default().fg(Color::Yellow)
                                    } else {
                                        Style::default().fg(Color::Gray)
                                    },
                                ),
                            ]));
                        }
                    }
                }
            }

            (title, Text::from(lines))
        }
    };

    let footer_text = match prompt.stage {
        PromptStage::ChoosePreset => " Enter select | Esc cancel ",
        PromptStage::InputValues => {
            if prompt.current_choices().is_some() {
                " Up/down choose | Enter accept | Esc cancel "
            } else {
                " Enter accept | Esc cancel "
            }
        }
    };

    let block = Block::default()
        .title(title)
        .title_bottom(
            Line::from(footer_text)
                .alignment(Alignment::Right)
                .style(Style::default().add_modifier(Modifier::DIM)),
        )
        .borders(Borders::ALL);

    let popup = Paragraph::new(text).block(block).wrap(Wrap { trim: false });
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
