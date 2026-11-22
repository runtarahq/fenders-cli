//! `fenders skill`: the agent skill that teaches an AI assistant to use this
//! CLI, built into the binary so it always describes this version's flags.
//!
//! The files are `skills/fenders/` in this repository, in the Agent Skills
//! layout (a `SKILL.md` with its references and templates).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use serde_json::json;

use crate::output::Out;
use crate::ui::{self, To};

/// Every file of the skill: its path inside the skill directory, and its text.
pub const FILES: &[(&str, &str)] = &[
    ("SKILL.md", include_str!("../../skills/fenders/SKILL.md")),
    (
        "reference/sandbox.md",
        include_str!("../../skills/fenders/reference/sandbox.md"),
    ),
    (
        "reference/web-apps.md",
        include_str!("../../skills/fenders/reference/web-apps.md"),
    ),
    (
        "reference/cli.md",
        include_str!("../../skills/fenders/reference/cli.md"),
    ),
    (
        "reference/errors.md",
        include_str!("../../skills/fenders/reference/errors.md"),
    ),
    (
        "templates/http-rust/Cargo.toml",
        include_str!("../../skills/fenders/templates/http-rust/Cargo.toml"),
    ),
    (
        "templates/http-rust/src/lib.rs",
        include_str!("../../skills/fenders/templates/http-rust/src/lib.rs"),
    ),
    (
        "templates/command-rust/Cargo.toml",
        include_str!("../../skills/fenders/templates/command-rust/Cargo.toml"),
    ),
    (
        "templates/command-rust/src/main.rs",
        include_str!("../../skills/fenders/templates/command-rust/src/main.rs"),
    ),
];

#[derive(Subcommand, Debug)]
pub enum SkillCmd {
    /// Install the skill for an AI assistant (default: Claude Code's ~/.claude/skills/fenders)
    #[command(after_help = "\
Examples:
  fenders skill install                         # for Claude Code, all projects
  fenders skill install --dir .claude/skills/fenders   # for this project only
  fenders skill install --force                 # replace an older or changed copy")]
    Install {
        /// Directory to install into (it becomes the skill's directory)
        #[arg(long, value_name = "DIR")]
        dir: Option<PathBuf>,
        /// Replace files that differ from this version's
        #[arg(long)]
        force: bool,
    },
    /// Print the skill's SKILL.md, or one of its files
    Show {
        /// A file of the skill, e.g. reference/sandbox.md
        file: Option<String>,
    },
    /// List the skill's files
    Files,
}

/// `~/.claude/skills/fenders`, where Claude Code looks for personal skills.
fn default_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("cannot find the home directory; pass --dir")?;
    Ok(home.join(".claude").join("skills").join("fenders"))
}

/// What installing into `dir` would replace: files there that differ.
fn differing(dir: &Path) -> Vec<&'static str> {
    FILES
        .iter()
        .filter(|(rel, text)| match std::fs::read_to_string(dir.join(rel)) {
            Ok(existing) => existing != *text,
            Err(_) => false,
        })
        .map(|(rel, _)| *rel)
        .collect()
}

pub fn install(dir: &Path, force: bool) -> Result<usize> {
    let changed = differing(dir);
    if !changed.is_empty() && !force {
        bail!(
            "{} already has a different copy of {} — pass --force to replace it with this version's",
            dir.display(),
            changed.join(", ")
        );
    }
    let mut written = 0;
    for (rel, text) in FILES {
        let path = dir.join(rel);
        if std::fs::read_to_string(&path).is_ok_and(|e| e == *text) {
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("cannot create {}", parent.display()))?;
        }
        std::fs::write(&path, text).with_context(|| format!("cannot write {}", path.display()))?;
        written += 1;
    }
    Ok(written)
}

pub fn run(out: &Out, cmd: SkillCmd) -> Result<()> {
    match cmd {
        SkillCmd::Install { dir, force } => {
            let dir = match dir {
                Some(d) => d,
                None => default_dir()?,
            };
            let written = install(&dir, force)?;
            if out.json {
                out.print_json(&json!({
                    "dir": dir.display().to_string(),
                    "files": FILES.iter().map(|(r, _)| *r).collect::<Vec<_>>(),
                    "written": written,
                    "version": env!("CARGO_PKG_VERSION"),
                }));
                return Ok(());
            }
            if written == 0 {
                ui::success(&format!(
                    "The skill in {} is already this version's",
                    ui::bold(&dir.display().to_string(), To::Err)
                ));
            } else {
                ui::success(&format!(
                    "Installed the fenders skill in {} ({written} of {} files written)",
                    ui::bold(&dir.display().to_string(), To::Err),
                    FILES.len()
                ));
            }
            ui::hint(
                "Claude Code loads it in new sessions; ask it to deploy a .wasm app or run code in a sandbox.",
            );
            ui::hint(
                "Give it a tenant profile, not the operator's: fenders keys issue TENANT --save-profile NAME",
            );
            Ok(())
        }
        SkillCmd::Show { file } => {
            let want = file.unwrap_or_else(|| "SKILL.md".into());
            let want = want.trim_start_matches("./");
            match FILES.iter().find(|(rel, _)| *rel == want) {
                Some((_, text)) => {
                    print!("{text}");
                    Ok(())
                }
                None => bail!("the skill has no file {want:?} (see `fenders skill files`)"),
            }
        }
        SkillCmd::Files => {
            if out.json {
                out.print_json(&json!(FILES.iter().map(|(r, _)| *r).collect::<Vec<_>>()));
            } else {
                for (rel, _) in FILES {
                    println!("{rel}");
                }
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The files compiled in are exactly the skill directory's: a file added
    /// to `skills/fenders/` must be added to FILES too.
    #[test]
    fn every_file_of_the_skill_is_built_in() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("skills/fenders");
        let mut on_disk = Vec::new();
        let mut stack = vec![root.clone()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    if p.file_name().is_some_and(|n| n == "target") {
                        continue;
                    }
                    stack.push(p);
                } else {
                    on_disk.push(
                        p.strip_prefix(&root)
                            .unwrap()
                            .to_string_lossy()
                            .replace('\\', "/"),
                    );
                }
            }
        }
        on_disk.sort();
        let mut built: Vec<String> = FILES.iter().map(|(r, _)| r.to_string()).collect();
        built.sort();
        assert_eq!(on_disk, built);
    }

    #[test]
    fn skill_md_has_the_frontmatter_agents_read() {
        let text = FILES[0].1;
        let front = text
            .strip_prefix("---\n")
            .and_then(|t| t.split_once("\n---\n"))
            .map(|(f, _)| f)
            .expect("SKILL.md starts with --- frontmatter ---");
        let field = |k: &str| {
            front
                .lines()
                .find_map(|l| l.strip_prefix(&format!("{k}: ")))
                .unwrap_or_else(|| panic!("frontmatter has {k}"))
        };
        assert_eq!(field("name"), "fenders");
        let d = field("description");
        assert!(
            (50..=1024).contains(&d.len()),
            "description is {} chars",
            d.len()
        );
    }

    /// Every link from one file of the skill to another points at a file it has.
    #[test]
    fn links_between_the_skills_files_resolve() {
        for (rel, text) in FILES.iter().filter(|(r, _)| r.ends_with(".md")) {
            let base = Path::new(rel).parent().unwrap();
            for (i, _) in text.match_indices("](") {
                let target = &text[i + 2..];
                let target = &target[..target.find(')').unwrap()];
                if target.starts_with("http") || target.starts_with('#') {
                    continue;
                }
                let joined = base.join(target);
                let norm = joined.to_string_lossy().replace('\\', "/");
                let exists = FILES
                    .iter()
                    .any(|(r, _)| *r == norm || r.starts_with(&format!("{norm}/")));
                assert!(
                    exists,
                    "{rel} links to {target}, which the skill does not have"
                );
            }
        }
    }

    /// Split a shell line into words, honouring single and double quotes.
    fn words(s: &str) -> Vec<String> {
        let (mut out, mut cur, mut q, mut any) = (Vec::new(), String::new(), None, false);
        for c in s.chars() {
            match (q, c) {
                (None, '\'' | '"') => {
                    q = Some(c);
                    any = true;
                }
                (Some(open), c) if c == open => q = None,
                (None, c) if c.is_whitespace() => {
                    if any {
                        out.push(std::mem::take(&mut cur));
                        any = false;
                    }
                }
                (_, c) => {
                    cur.push(c);
                    any = true;
                }
            }
        }
        if any {
            out.push(cur);
        }
        out
    }

    /// Every `fenders …` command the skill tells an agent to run is one this
    /// binary accepts: in code blocks and in `inline code`, after pipes.
    #[test]
    fn every_command_in_the_skill_parses() {
        use clap::Parser;
        let mut checked = 0;
        for (rel, text) in FILES.iter().filter(|(r, _)| r.ends_with(".md")) {
            // (text, inline): inline code in prose may name a command
            // without its arguments ("`fenders deploy` it").
            let mut candidates: Vec<(String, bool)> = Vec::new();
            let mut in_block = false;
            for line in text.lines() {
                if line.trim_start().starts_with("```") {
                    in_block = !in_block;
                    continue;
                }
                if in_block {
                    candidates.push((line.to_string(), false));
                } else {
                    for (n, span) in line.split('`').enumerate() {
                        if n % 2 == 1 {
                            candidates.push((span.to_string(), true));
                        }
                    }
                }
            }
            for (c, inline) in candidates {
                let c = c.replace("\\|", "|");
                for seg in c
                    .split(['|', ';'])
                    .flat_map(|s| s.split("&&"))
                    .flat_map(|s| s.split("$("))
                {
                    let seg = seg.trim().trim_end_matches(')');
                    let seg = seg.split(" > ").next().unwrap();
                    let seg = seg.split(" # ").next().unwrap();
                    if !seg.starts_with("fenders ") {
                        continue;
                    }
                    let argv = words(seg);
                    if let Err(e) = crate::Cli::try_parse_from(&argv) {
                        use clap::error::ErrorKind::*;
                        let named_only = inline
                            && matches!(
                                e.kind(),
                                MissingRequiredArgument | DisplayHelpOnMissingArgumentOrSubcommand
                            )
                            && argv
                                .iter()
                                .all(|w| w.chars().all(|c| c.is_ascii_lowercase() || c == '-'));
                        if !named_only {
                            panic!("{rel}: `{seg}` is not a valid command: {e}");
                        }
                    }
                    checked += 1;
                }
            }
        }
        assert!(checked > 40, "only {checked} commands found");
    }

    #[test]
    fn install_writes_every_file_and_keeps_local_changes_unless_forced() {
        let dir = std::env::temp_dir().join(format!("fenders-skill-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(install(&dir, false).unwrap(), FILES.len());
        for (rel, text) in FILES {
            assert_eq!(std::fs::read_to_string(dir.join(rel)).unwrap(), *text);
        }
        assert_eq!(
            install(&dir, false).unwrap(),
            0,
            "a second install writes nothing"
        );
        std::fs::write(dir.join("SKILL.md"), "edited").unwrap();
        let err = install(&dir, false).unwrap_err().to_string();
        assert!(err.contains("--force") && err.contains("SKILL.md"), "{err}");
        assert_eq!(install(&dir, true).unwrap(), 1);
        assert_eq!(
            std::fs::read_to_string(dir.join("SKILL.md")).unwrap(),
            FILES[0].1
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
