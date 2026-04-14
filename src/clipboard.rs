use std::env;
use std::io;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

#[derive(Clone, Copy)]
pub struct CommandProvider {
    program: &'static str,
    args: &'static [&'static str],
}

pub enum ClipboardProvider {
    Command(CommandProvider),
    Arboard(arboard::Clipboard),
}

impl ClipboardProvider {
    pub fn detect() -> io::Result<Self> {
        if let Some(provider) = detect_command_provider() {
            return Ok(Self::Command(provider));
        }

        let clipboard = arboard::Clipboard::new()
            .map_err(|err| io::Error::other(format!("failed to open clipboard: {err}")))?;
        Ok(Self::Arboard(clipboard))
    }

    pub fn copy(&mut self, text: &str) -> io::Result<()> {
        match self {
            Self::Command(provider) => {
                if copy_with_command(text, provider.program, provider.args).is_ok() {
                    return Ok(());
                }

                let mut clipboard = arboard::Clipboard::new().map_err(|err| {
                    io::Error::other(format!("failed to open clipboard fallback: {err}"))
                })?;
                clipboard
                    .set_text(text.to_string())
                    .map_err(|err| io::Error::other(format!("failed to copy text: {err}")))?;
                *self = Self::Arboard(clipboard);
                Ok(())
            }
            Self::Arboard(clipboard) => clipboard
                .set_text(text.to_string())
                .map_err(|err| io::Error::other(format!("failed to copy text: {err}"))),
        }
    }
}

fn detect_command_provider() -> Option<CommandProvider> {
    command_candidates()
        .into_iter()
        .find(|provider| command_exists(provider.program))
}

fn command_candidates() -> Vec<CommandProvider> {
    let mut candidates = Vec::new();

    if env::var_os("WAYLAND_DISPLAY").is_some() {
        push_candidate(
            &mut candidates,
            CommandProvider {
                program: "wl-copy",
                args: &[],
            },
        );
    }

    if env::var_os("DISPLAY").is_some() {
        push_candidate(
            &mut candidates,
            CommandProvider {
                program: "xclip",
                args: &["-selection", "clipboard"],
            },
        );
        push_candidate(
            &mut candidates,
            CommandProvider {
                program: "xsel",
                args: &["--clipboard", "--input"],
            },
        );
    }

    push_candidate(
        &mut candidates,
        CommandProvider {
            program: "pbcopy",
            args: &[],
        },
    );
    push_candidate(
        &mut candidates,
        CommandProvider {
            program: "wl-copy",
            args: &[],
        },
    );
    push_candidate(
        &mut candidates,
        CommandProvider {
            program: "xclip",
            args: &["-selection", "clipboard"],
        },
    );
    push_candidate(
        &mut candidates,
        CommandProvider {
            program: "xsel",
            args: &["--clipboard", "--input"],
        },
    );

    candidates
}

fn push_candidate(candidates: &mut Vec<CommandProvider>, provider: CommandProvider) {
    if candidates
        .iter()
        .any(|candidate| candidate.program == provider.program)
    {
        return;
    }

    candidates.push(provider);
}

fn command_exists(program: &str) -> bool {
    let Some(path) = env::var_os("PATH") else {
        return false;
    };

    env::split_paths(&path).any(|dir| executable_path(&dir, program).is_some())
}

fn executable_path(dir: &PathBuf, program: &str) -> Option<PathBuf> {
    let path = dir.join(program);
    if path.is_file() { Some(path) } else { None }
}

fn copy_with_command(text: &str, program: &str, args: &[&str]) -> io::Result<()> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(text.as_bytes())?;
    }

    let status = child.wait()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "{program} exited with status {status}"
        )))
    }
}
