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
presets = ["branch=main"]
```

Use placeholders in braces (`{branch}`, `{service}`, etc.). Pantry prompts for values before running/copying.

You can define `presets` as ready-made placeholder assignments. For commands with multiple placeholders, include all values in a single preset string, for example `"service=api env=prod"` or `"branch=main remote=origin"`. At runtime Pantry lets you choose a preset or enter custom values manually.

For multiline commands, prefer TOML multiline strings:

```toml
[[recipe]]
name = "deploy service"
description = "Build and deploy a service"
command = """
docker build -t {service}:{tag} .
docker push {service}:{tag}
kubectl set image deployment/{service} {service}={service}:{tag}
"""
presets = ["service=api tag=latest"]
```

This is usually the most readable format for Pantry recipes, and placeholders still work across newlines.

If you need to use literal `{` or `}` characters in a command, you can escape them by doubling them: `{{` or `}}`.

```toml
[[recipe]]
name = "literal braces example"
command = "echo {{this is literal}} and {this_is_a_placeholder}"
```

## Keybindings

- `/`: enter search mode
- `Esc`: leave search mode
- Type while in search mode to filter recipes
- `up/down`: move selection
- `Enter`: run selected command
- `y`: copy selected command to clipboard
- `Y`: copy selected command to clipboard, then quit
- `r`: reload recipe files
- `q` or `Ctrl+C`: quit
