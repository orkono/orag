//! `docs/engine-interface.md` against the real command line: every flag the
//! document lists must exist, and every one but a deprecated flag must be
//! visible in `--help` (a program may rely on it).

use clap::{Command, CommandFactory};
use orag::cli::Cli;

struct Row {
    command: String,
    flag: String,
    meaning: String,
}

fn document() -> String {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/engine-interface.md");
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("reading {}: {err}", path.display()))
}

fn cells(line: &str) -> Vec<String> {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(|cell| cell.trim().to_string())
        .collect()
}

fn unquote(cell: &str) -> &str {
    cell.trim_matches('`')
}

/// The rows of the table whose header is `Command | Flag | Since | Meaning`.
fn flag_rows(text: &str) -> Vec<Row> {
    let mut lines = text.lines();
    let header = ["Command", "Flag", "Since", "Meaning"];
    lines
        .by_ref()
        .find(|line| cells(line) == header)
        .expect("the flag table (Command | Flag | Since | Meaning)");
    let rows: Vec<Row> = lines
        .skip(1) // the |---| separator
        .take_while(|line| line.trim_start().starts_with('|'))
        .map(|line| {
            let cells = cells(line);
            assert_eq!(cells.len(), 4, "flag table row: {line}");
            Row {
                command: unquote(&cells[0]).to_string(),
                flag: unquote(&cells[1]).to_string(),
                meaning: cells[3].clone(),
            }
        })
        .collect();
    assert!(!rows.is_empty(), "the flag table has no rows");
    rows
}

fn subcommand<'a>(root: &'a Command, path: &str) -> &'a Command {
    path.split_whitespace().fold(root, |command, name| {
        command
            .find_subcommand(name)
            .unwrap_or_else(|| panic!("no subcommand `{name}` in `{path}`"))
    })
}

#[test]
fn every_documented_flag_exists() {
    let root = Cli::command();
    for row in flag_rows(&document()) {
        let name = row
            .flag
            .strip_prefix("--")
            .unwrap_or_else(|| panic!("`{}` is not written as --name", row.flag));
        let command = subcommand(&root, &row.command);
        assert!(
            command
                .get_arguments()
                .any(|arg| arg.get_long() == Some(name)),
            "`orag {}` has no flag --{name}",
            row.command
        );
    }
}

#[test]
fn every_interface_flag_is_visible_except_deprecated() {
    let root = Cli::command();
    for row in flag_rows(&document()) {
        let name = row.flag.trim_start_matches("--");
        let command = subcommand(&root, &row.command);
        if row.meaning.starts_with("Deprecated") {
            continue;
        }
        let arg = command
            .get_arguments()
            .find(|arg| arg.get_long() == Some(name))
            .unwrap_or_else(|| panic!("`orag {}` has no flag --{name}", row.command));
        assert!(
            !arg.is_hide_set(),
            "`orag {} --{name}` is hidden but part of the interface",
            row.command
        );
        // The path to it must be visible too.
        let mut walk = &root;
        for part in row.command.split_whitespace() {
            walk = walk.find_subcommand(part).expect("subcommand");
            assert!(!walk.is_hide_set(), "`orag {}` is hidden", row.command);
        }
    }
}
