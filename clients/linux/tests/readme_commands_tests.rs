//! Every `dosi-tracker ...` command shown in the README's shell blocks must
//! parse with the real command-line definition (subcommands, flags and
//! values), so the documentation cannot drift from the CLI. The package stage
//! of `ci/check.sh` additionally walks each command with `--help` against the
//! release binary.
#![cfg(target_os = "linux")]

use clap::Parser;
use dosi_tracker_linux::cli::Cli;

const README: &str = include_str!("../README.md");

/// The shell lines of fenced `bash`/`sh`/`shell`/`console` blocks, with
/// backslash continuations joined.
fn shell_lines(markdown: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut in_block = false;
    let mut current = String::new();
    for raw in markdown.lines() {
        let trimmed = raw.trim_start();
        if let Some(info) = trimmed.strip_prefix("```") {
            in_block = !in_block && matches!(info.trim(), "bash" | "sh" | "shell" | "console");
            continue;
        }
        if !in_block {
            continue;
        }
        if let Some(head) = raw.trim_end().strip_suffix('\\') {
            current.push_str(head);
            current.push(' ');
            continue;
        }
        current.push_str(raw);
        lines.push(std::mem::take(&mut current));
    }
    lines
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Word(String),
    /// `|`, `||`, `&&`, `;`, `&`: ends a command.
    Separator,
    /// `>`, `>>`, `<`, `2>`: the rest of the command is a redirection.
    Redirect,
}

/// A small POSIX-shell tokenizer: quotes, backslash escapes, comments,
/// command separators and redirections. Enough for documentation examples.
fn tokenize(line: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut chars = line.chars().peekable();
    let flush = |word: &mut String, in_word: &mut bool, tokens: &mut Vec<Token>| {
        if *in_word {
            tokens.push(Token::Word(std::mem::take(word)));
            *in_word = false;
        }
    };
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                for q in chars.by_ref() {
                    if q == '\'' {
                        break;
                    }
                    word.push(q);
                }
            }
            '"' => {
                in_word = true;
                while let Some(q) = chars.next() {
                    match q {
                        '"' => break,
                        '\\' => {
                            if let Some(&next) = chars.peek() {
                                if matches!(next, '"' | '\\' | '$' | '`') {
                                    word.push(next);
                                    chars.next();
                                    continue;
                                }
                            }
                            word.push('\\');
                        }
                        other => word.push(other),
                    }
                }
            }
            '\\' => {
                in_word = true;
                if let Some(next) = chars.next() {
                    word.push(next);
                }
            }
            '#' if !in_word => break,
            ' ' | '\t' => flush(&mut word, &mut in_word, &mut tokens),
            '|' | ';' | '&' => {
                flush(&mut word, &mut in_word, &mut tokens);
                if matches!(chars.peek(), Some('|') | Some('&')) {
                    chars.next();
                }
                tokens.push(Token::Separator);
            }
            '>' | '<' => {
                // `2>` is a redirection of stderr, not the word "2".
                if word == "2" {
                    word.clear();
                    in_word = false;
                }
                flush(&mut word, &mut in_word, &mut tokens);
                if chars.peek() == Some(&'>') {
                    chars.next();
                }
                tokens.push(Token::Redirect);
            }
            other => {
                in_word = true;
                word.push(other);
            }
        }
    }
    flush(&mut word, &mut in_word, &mut tokens);
    tokens
}

/// The argument vectors of every `dosi-tracker` command in `markdown`.
fn readme_commands(markdown: &str) -> Vec<Vec<String>> {
    let mut commands = Vec::new();
    for line in shell_lines(markdown) {
        let mut segment: Vec<String> = Vec::new();
        let mut redirected = false;
        for token in tokenize(&line).into_iter().chain([Token::Separator]) {
            match token {
                Token::Word(word) if !redirected => segment.push(word),
                Token::Word(_) => {}
                Token::Redirect => redirected = true,
                Token::Separator => {
                    if segment
                        .first()
                        .is_some_and(|w| w.rsplit('/').next() == Some("dosi-tracker"))
                    {
                        commands.push(std::mem::take(&mut segment));
                    }
                    segment.clear();
                    redirected = false;
                }
            }
        }
    }
    commands
}

#[test]
fn the_tokenizer_handles_quotes_comments_separators_and_redirections() {
    assert_eq!(
        tokenize(
            r#"printf '%s\n' "$PW" | dosi-tracker --json login --workspace "Acme Inc" # note"#
        ),
        vec![
            Token::Word("printf".into()),
            Token::Word(r"%s\n".into()),
            Token::Word("$PW".into()),
            Token::Separator,
            Token::Word("dosi-tracker".into()),
            Token::Word("--json".into()),
            Token::Word("login".into()),
            Token::Word("--workspace".into()),
            Token::Word("Acme Inc".into()),
        ]
    );
    assert_eq!(
        tokenize("dosi-tracker completions bash > out 2>/dev/null && dosi-tracker start"),
        vec![
            Token::Word("dosi-tracker".into()),
            Token::Word("completions".into()),
            Token::Word("bash".into()),
            Token::Redirect,
            Token::Word("out".into()),
            Token::Redirect,
            Token::Word("/dev/null".into()),
            Token::Separator,
            Token::Word("dosi-tracker".into()),
            Token::Word("start".into()),
        ]
    );
    let commands = readme_commands(
        "text\n```bash\ndosi-tracker login \\\n  --email a@b.c\nsudo apt remove dosi-tracker\n```\n```toml\ndosi-tracker = 1\n```\n",
    );
    assert_eq!(
        commands,
        vec![vec!["dosi-tracker", "login", "--email", "a@b.c"]]
    );
}

#[test]
fn every_readme_command_parses() {
    let commands = readme_commands(README);
    assert!(
        commands.len() >= 20,
        "expected the README to show the CLI, found {} commands",
        commands.len()
    );
    let mut failures = Vec::new();
    for argv in &commands {
        if let Err(e) = Cli::try_parse_from(argv) {
            use clap::error::ErrorKind;
            if !matches!(e.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion) {
                failures.push(format!("{}: {}", argv.join(" "), e.kind()));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "README commands that do not parse:\n{}",
        failures.join("\n")
    );
}

#[test]
fn the_readme_never_shows_a_password_on_the_command_line() {
    for argv in readme_commands(README) {
        assert!(
            !argv
                .iter()
                .any(|a| a == "-p" || a == "--password" || a.starts_with("--password=")),
            "{}",
            argv.join(" ")
        );
    }
}
