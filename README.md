# Pantry

`pantry` is a terminal command cookbook built with Rust + ratatui.

It keeps reusable shell recipes in TOML and gives you a fast keyboard UI to search, inspect, run, and copy them.

## Install / Run

```bash
cargo run
```

On first launch, Pantry creates a starter recipe file at:

`~/.config/pantry/recipes.toml`

It also loads optional local recipes from:

`./.pantry.toml`

## Recipe format

```toml
[[recipe]]
name = "sync current branch"
tags = ["git", "sync"]
description = "Fetch and rebase current branch onto origin/{branch}."
command = "git fetch origin && git rebase origin/{branch}"
safety = "confirm"
examples = ["branch=main"]
```

Use placeholders in braces (`{branch}`, `{service}`, etc.). Pantry prompts for values before running/copying.

## Keybindings

- `/`: enter search mode
- `Esc`: leave search mode
- Type while in search mode to filter recipes
- `up/down` or `j/k`: move selection
- `Enter`: run selected command
- `Ctrl+Y`: copy selected command to clipboard
- `r`: reload recipe files
- `q` or `Ctrl+C`: quit
