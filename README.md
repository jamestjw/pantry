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

## Clipboard

Pantry copies commands through a system clipboard provider.

- Preferred providers:
  - Wayland: `wl-copy`
  - X11: `xclip` or `xsel`
  - macOS: `pbcopy`
- If one of those is installed, Pantry will use it first.
- If none are available, Pantry falls back to `arboard`.

`arboard` works as a fallback, but on some Linux setups clipboard ownership can stay tied to the running app. That means copied text may not remain available after Pantry exits, or clipboard behavior may be less reliable than with `wl-copy`, `xclip`, or `xsel`.

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

`examples` are shown in the details pane as sample placeholder assignments. For commands with multiple placeholders, include all values in a single example string, for example `"service=api env=prod"` or `"branch=main remote=origin"`.

## Keybindings

- `/`: enter search mode
- `Esc`: leave search mode
- Type while in search mode to filter recipes
- `up/down` or `j/k`: move selection
- `Enter`: run selected command
- `Ctrl+Y`: copy selected command to clipboard
- `r`: reload recipe files
- `q` or `Ctrl+C`: quit
