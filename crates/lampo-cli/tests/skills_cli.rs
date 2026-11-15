//! The Claude Code skills (`claude-plugin/skills/**/*.md`) only teach commands that exist: every
//! `lampo …` invocation in their code blocks and inline code spans must parse with the real clap
//! definition (subcommand path, flags, value enums). Placeholders such as `<name>` are replaced
//! by a dummy value first. An inline span may name a command without its required arguments
//! (`lampo db import` in a sentence); a code block line must be complete. Also checks the
//! skills' frontmatter and that their links to sibling files resolve.

use std::path::{Path, PathBuf};

fn skills_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../claude-plugin/skills")
}

fn markdown_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            markdown_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "md") {
            out.push(path);
        }
    }
}

/// One piece of code of a Markdown file.
struct Fragment {
    line: usize,
    /// An inline code span (else a fenced code block line).
    inline: bool,
    code: String,
}

/// Fenced block lines and inline code spans. The frontmatter is skipped (its description may
/// abbreviate, `up/down`).
fn code_fragments(text: &str) -> Vec<Fragment> {
    let mut fragments = Vec::new();
    let mut fenced = false;
    let mut frontmatter = text.starts_with("---");
    for (index, line) in text.lines().enumerate() {
        let number = index + 1;
        if frontmatter {
            frontmatter = index == 0 || line != "---";
            continue;
        }
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            fragments.push(Fragment { line: number, inline: false, code: line.to_string() });
            continue;
        }
        // Odd segments between backticks are inline code.
        fragments.extend(line.split('`').skip(1).step_by(2).map(|code| Fragment { line: number, inline: true, code: code.to_string() }));
    }
    fragments
}

/// Splits a shell line into commands (`|`, `;`, `&&`, `||`, `$(`...`)`), dropping comments.
fn commands(fragment: &str) -> Vec<String> {
    let without_comment = match fragment.find(" #") {
        Some(at) => &fragment[..at],
        None if fragment.trim_start().starts_with('#') => "",
        None => fragment,
    };
    without_comment
        .replace("&&", "\n")
        .replace("||", "\n")
        .replace("$(", "\n")
        .split(['|', ';', '\n', ')'])
        .map(|s| s.trim().trim_start_matches(['"', '\'', '(']).trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Shell-like tokens: whitespace separated, `"…"` and `'…'` quoted.
fn tokens(command: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    for c in command.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => current.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                started = true;
            }
            (None, c) if c.is_whitespace() => {
                if started || !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            (None, c) => current.push(c),
        }
    }
    if started || !current.is_empty() {
        out.push(current);
    }
    out
}

/// `<name>` (anywhere in a token) → `x1`: a value that every free-text argument accepts.
fn substitute_placeholders(token: &str) -> String {
    let mut out = String::new();
    let mut rest = token;
    while let Some(start) = rest.find('<') {
        let Some(len) = rest[start..].find('>') else { break };
        out.push_str(&rest[..start]);
        out.push_str("x1");
        rest = &rest[start + len + 1..];
    }
    out.push_str(rest);
    out
}

/// One `lampo …` invocation found in a skill.
struct Invocation {
    location: String,
    inline: bool,
    args: Vec<String>,
}

/// Every `lampo <args>` invocation (at least one argument) of the skills, with its location.
fn invocations() -> Vec<Invocation> {
    let mut files = Vec::new();
    markdown_files(&skills_root(), &mut files);
    files.sort();
    let mut found = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap();
        let name = file.strip_prefix(skills_root()).unwrap().display().to_string();
        for fragment in code_fragments(&text) {
            for command in commands(&fragment.code) {
                let words = tokens(&command);
                let Some(first) = words.first() else { continue };
                if first != "lampo" && first != "lampo.exe" || words.len() < 2 {
                    continue;
                }
                let args = std::iter::once("lampo".to_string()).chain(words[1..].iter().map(|w| substitute_placeholders(w))).collect();
                found.push(Invocation { location: format!("{name}:{}: {command}", fragment.line), inline: fragment.inline, args });
            }
        }
    }
    found
}

#[test]
fn every_lampo_invocation_in_the_skills_parses() {
    use clap::error::ErrorKind as K;
    let found = invocations();
    assert!(found.len() >= 60, "only {} invocations found: is the extraction broken?", found.len());
    let mut failures = Vec::new();
    for Invocation { location, inline, args } in &found {
        match lampo_cli::cli::build_command().try_get_matches_from(args) {
            Ok(_) => {}
            Err(e) if matches!(e.kind(), K::DisplayHelp | K::DisplayVersion) => {}
            // A command named in a sentence: its path and flags are right, its arguments omitted.
            Err(e) if *inline && matches!(e.kind(), K::MissingRequiredArgument | K::MissingSubcommand | K::DisplayHelpOnMissingArgumentOrSubcommand) => {}
            Err(e) => failures.push(format!("{location}\n    {:?}: {}", e.kind(), e.to_string().lines().next().unwrap_or_default())),
        }
    }
    assert!(failures.is_empty(), "{} skill command(s) do not parse:\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn the_extractor_sees_blocks_spans_pipes_and_rejects_unknown_flags() {
    let text = "---\nname: x\ndescription: `lampo up/down`\n---\nRun `lampo status --json` or `lampo:php`.\n```powershell\nlampo env --shell powershell | Out-String | Invoke-Expression\ncd x; lampo db import <file.sql> shop  # comment\n```\n";
    let fragments = code_fragments(text);
    let found: Vec<(bool, Vec<String>)> =
        fragments.iter().flat_map(|f| commands(&f.code).into_iter().map(|c| (f.inline, tokens(&c)))).filter(|(_, t)| t.first().is_some_and(|w| w == "lampo")).collect();
    let expected = [(true, vec!["lampo", "status", "--json"]), (false, vec!["lampo", "env", "--shell", "powershell"]), (false, vec!["lampo", "db", "import", "<file.sql>", "shop"])];
    assert_eq!(found, expected.map(|(i, v)| (i, v.into_iter().map(String::from).collect())));
    assert_eq!(substitute_placeholders("clone:<database>"), "clone:x1");
    assert_eq!(tokens(r#"lampo exec -- php -r "echo 1;""#), ["lampo", "exec", "--", "php", "-r", "echo 1;"]);
    assert!(lampo_cli::cli::build_command().try_get_matches_from(["lampo", "worktree", "up", "--database", "fresh"]).is_err());
    assert!(lampo_cli::cli::build_command().try_get_matches_from(["lampo", "packages", "install", "php"]).is_err());
}

#[test]
fn skills_have_frontmatter_and_their_links_resolve() {
    let root = skills_root();
    let mut files = Vec::new();
    markdown_files(&root, &mut files);
    let mut skills = 0;
    for file in &files {
        let text = std::fs::read_to_string(file).unwrap().replace("\r\n", "\n");
        if file.file_name().is_some_and(|n| n == "SKILL.md") {
            skills += 1;
            let id = file.parent().unwrap().file_name().unwrap().to_string_lossy().into_owned();
            let front = text.strip_prefix("---\n").and_then(|rest| rest.split_once("\n---\n")).map(|(f, _)| f).unwrap_or_else(|| panic!("{}: frontmatter", file.display()));
            assert!(front.lines().any(|l| l == format!("name: {id}")), "{}: name must be `{id}`", file.display());
            let description = front.lines().find_map(|l| l.strip_prefix("description: ")).unwrap_or_else(|| panic!("{}: description", file.display()));
            assert!((100..=1024).contains(&description.len()), "{}: description length {}", file.display(), description.len());
            assert!(text.lines().count() <= 250, "{}: keep SKILL.md under 250 lines", file.display());
        }
        // Relative Markdown links `[text](file.md)` point to existing files.
        for (at, _) in text.match_indices("](") {
            let target = text[at + 2..].split(')').next().unwrap_or_default();
            if target.starts_with("http") || target.starts_with('#') || target.is_empty() {
                continue;
            }
            assert!(file.parent().unwrap().join(target).is_file(), "{}: broken link {target}", file.display());
        }
    }
    assert!(skills >= 8, "expected the 8 skills, found {skills}");
}
