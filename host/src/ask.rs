//! What `tooob` asks a person at the keyboard. Only ever in a terminal: scripts and Agents say
//! everything with flags, and are never asked anything.
use anyhow::{Result, bail};
use dialoguer::{Confirm, Input, Select, theme::ColorfulTheme};
use std::path::{Path, PathBuf};
use tooob::hosts::NO_RELAY;

fn theme() -> ColorfulTheme {
    ColorfulTheme::default()
}

/// "What do you want to share?", until the answer is a folder or file that exists.
pub fn what_to_share() -> Result<PathBuf> {
    let answer: String = Input::with_theme(&theme())
        .with_prompt("What do you want to share? (a folder or a file)")
        .validate_with(|p: &String| match expand(p).exists() {
            true => Ok(()),
            false => Err("no such folder or file"),
        })
        .interact_text()?;
    Ok(expand(&answer).canonicalize()?)
}

fn expand(path: &str) -> PathBuf {
    match (path.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => PathBuf::from(path.trim()),
    }
}

/// Say what's in `path` before it's shared, and make the Owner confirm when it holds things they
/// probably don't mean to hand out: a repository's history, secrets, or their whole home folder.
pub fn confirm_contents(path: &Path) -> Result<()> {
    let seen = Contents::of(path);
    println!("  {}: {}", path.display(), seen.summary());
    let mut worries = Vec::new();
    if std::env::var_os("HOME").is_some_and(|home| Path::new(&home) == path) {
        worries.push("it's your whole home folder".to_string());
    }
    for found in &seen.sensitive {
        worries.push(format!("it includes {found}"));
    }
    if worries.is_empty() {
        return Ok(());
    }
    for worry in &worries {
        println!("  ! {worry}");
    }
    let anyway = Confirm::with_theme(&theme())
        .with_prompt("Everyone you invite can see all of it. Share it anyway?")
        .default(false)
        .interact()?;
    if !anyway {
        bail!("nothing shared");
    }
    Ok(())
}

/// Folders, files and bytes under a path, counted up to a limit (a whole disk would take ages).
struct Contents {
    folders: usize,
    files: usize,
    bytes: u64,
    more: bool,
    /// Things worth a second thought: `.git`, `.env`, `.ssh`.
    sensitive: Vec<String>,
}

const COUNT_LIMIT: usize = 20_000;

impl Contents {
    fn of(path: &Path) -> Contents {
        let mut seen = Contents {
            folders: 0,
            files: 0,
            bytes: 0,
            more: false,
            sensitive: Vec::new(),
        };
        if path.is_file() {
            seen.files = 1;
            seen.bytes = path.metadata().map_or(0, |m| m.len());
            return seen;
        }
        let mut todo = vec![path.to_path_buf()];
        while let Some(dir) = todo.pop() {
            for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                if seen.folders + seen.files >= COUNT_LIMIT {
                    seen.more = true;
                    return seen;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                if [".git", ".env", ".ssh"].contains(&name.as_str()) && seen.sensitive.len() < 5 {
                    let relative = entry.path();
                    let relative = relative.strip_prefix(path).unwrap_or(&relative);
                    seen.sensitive.push(relative.display().to_string());
                }
                match entry.file_type() {
                    Ok(t) if t.is_dir() => {
                        seen.folders += 1;
                        todo.push(entry.path());
                    }
                    Ok(_) => {
                        seen.files += 1;
                        seen.bytes += entry.metadata().map_or(0, |m| m.len());
                    }
                    Err(_) => {}
                }
            }
        }
        seen
    }

    fn summary(&self) -> String {
        let plural = |n: usize, what: &str| format!("{n} {what}{}", if n == 1 { "" } else { "s" });
        let more = if self.more { "more than " } else { "" };
        format!(
            "{more}{}, {}, {}",
            plural(self.folders, "folder"),
            plural(self.files, "file"),
            size(self.bytes)
        )
    }
}

/// A size the way Finder shows it: 14 B, 1.5 MB.
fn size(bytes: u64) -> String {
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    for unit in ["KB", "MB", "GB", "TB"] {
        value /= 1000.0;
        if value < 1000.0 {
            return format!("{value:.1} {unit}");
        }
    }
    format!("{value:.1} PB")
}

/// One person to invite, as the Owner answered: their Label, how long the link works, and
/// whether their device waits for approval. None when the Owner is done (an empty answer).
pub struct Invitee {
    pub label: String,
    pub expires: String,
    pub approve: bool,
}

pub fn invitee(first: bool) -> Result<Option<Invitee>> {
    let prompt = if first {
        "Who's it for? (Enter to skip)"
    } else {
        "Who else? (Enter when done)"
    };
    let label: String = Input::with_theme(&theme())
        .with_prompt(prompt)
        .allow_empty(true)
        .interact_text()?;
    let label = label.trim().to_string();
    if label.is_empty() {
        return Ok(None);
    }
    let expires: String = Input::with_theme(&theme())
        .with_prompt("How long should the link work? (e.g. 1h, 7d)")
        .default("7d".into())
        .validate_with(|e: &String| crate::seconds(e).map(drop).map_err(|e| e.to_string()))
        .interact_text()?;
    let approve = Confirm::with_theme(&theme())
        .with_prompt("Approve their device before they get in?")
        .default(false)
        .interact()?;
    Ok(Some(Invitee {
        label,
        expires,
        approve,
    }))
}

/// A new Host's `relay` setting: a Relay's URL, or `none` for this network only.
pub fn relay() -> Result<String> {
    let relay: String = Input::with_theme(&theme())
        .with_prompt(
            format!(
                "Which Relay do Visitors reach it through? (its URL, or {NO_RELAY} for this network only)"
            ),
        )
        .validate_with(|r: &String| {
            tooob::hosts::check_relay(r.trim()).map_err(|e| e.to_string())
        })
        .interact_text()?;
    Ok(relay.trim().to_string())
}

/// The Label for the device a Pairing Invite is for.
pub fn label() -> Result<String> {
    let label: String = Input::with_theme(&theme())
        .with_prompt("Which device is it for? (e.g. my phone)")
        .validate_with(|l: &String| match l.trim().is_empty() {
            true => Err("say which device"),
            false => Ok(()),
        })
        .interact_text()?;
    Ok(label.trim().to_string())
}

/// Pick one of `items` with the arrow keys; None if the Owner backs out (Esc or q). The first is
/// highlighted, but nothing is chosen until the Owner presses Enter.
pub fn pick(prompt: &str, items: &[String]) -> Result<Option<usize>> {
    Ok(Select::with_theme(&theme())
        .with_prompt(prompt)
        .items(items)
        .default(0)
        .interact_opt()?)
}
