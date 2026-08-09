//! The `dx` command table.
//!
//! One entry per verb, grouped by what the reader is trying to do: read a document, write
//! one, run one, find one, or set up the platform. [`COMMANDS`] is the only place that maps
//! a command word to code, so adding a verb means adding one row — its names, the flags it
//! takes, and what runs.
//!
//! The flags column is load-bearing: a flag a command does not take is *rejected*, never
//! ignored. A swallowed flag is a caller who believes something happened — `--lang` on a
//! command that dropped it authored a block that cannot run — and `--help` on any verb asks
//! what it does, which must never *do* it.

pub mod browser;
pub mod edit;
pub mod exec;
pub mod find;
pub mod index;
pub mod play;
pub mod setup;
pub mod store;
pub mod view;

use crate::args::Args;

/// What a command produced.
///
/// The distinction matters for `--out`: a [`Document`](Output::Document) is the thing the
/// reader asked for and can be redirected to a file, while a [`Report`](Output::Report) is
/// the command telling you what it already did. Sending a report to `--out` would overwrite
/// the very file the command just wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Output {
    /// Content the reader asked for; honors `--out`.
    Document(String),
    /// A status report about work already done; always goes to standard output.
    Report(String),
}

impl Output {
    /// The text of this output, whichever kind it is.
    #[must_use]
    pub fn text(&self) -> &str {
        match self {
            Self::Document(body) | Self::Report(body) => body,
        }
    }
}

/// One verb: the words that reach it, the flags it reads, and what runs.
struct Command {
    /// Every spelling of this verb, the canonical one first.
    names: &'static [&'static str],
    /// Every `--flag` the verb reads. Anything else is rejected rather than ignored.
    flags: &'static [&'static str],
    /// The command itself.
    run: fn(&Args) -> Result<Output, String>,
}

/// The command table — the single map from a command word to code.
const COMMANDS: &[Command] = &[
    // Reading.
    Command {
        names: &["text", "cat"],
        flags: &["section", "ids", "hidden", "all", "out"],
        run: |args| view::run_text(args).map(Output::Document),
    },
    Command {
        names: &["outline"],
        flags: &["section", "out"],
        run: |args| view::run_outline(args).map(Output::Document),
    },
    Command {
        names: &["render", "html"],
        flags: &[
            "section",
            "theme",
            "hidden",
            "fragment",
            "show-code",
            "title",
            "block",
            "body",
            "field",
            "out",
            "all",
        ],
        // `--all` writes one page per document itself and answers with a report of the
        // files it wrote — content that must not be redirected into `--out`, which names
        // the export *directory* in that form.
        run: |args| {
            view::run_render(args).map(if args.present("all") {
                Output::Report
            } else {
                Output::Document
            })
        },
    },
    Command {
        names: &["ls", "list"],
        flags: &["out"],
        run: |args| find::run_ls(args).map(Output::Document),
    },
    Command {
        names: &["textconv"],
        flags: &["root", "out"],
        run: |args| store::run_textconv(args).map(Output::Document),
    },
    Command {
        names: &["stats"],
        flags: &["out"],
        run: |args| store::run_stats(args).map(Output::Document),
    },
    Command {
        names: &["search", "find"],
        flags: &["limit", "out"],
        run: |args| find::run_search(args).map(Output::Document),
    },
    Command {
        names: &["source"],
        flags: &["block", "header", "out"],
        run: |args| edit::run_source(args).map(Output::Document),
    },
    Command {
        names: &["help", "--help", "-h"],
        flags: &[],
        run: |args| setup::run_help(args).map(Output::Document),
    },
    Command {
        names: &["version", "--version"],
        flags: &[],
        run: |_| {
            Ok(Output::Document(format!(
                "dx {}\n",
                env!("CARGO_PKG_VERSION")
            )))
        },
    },
    // Rendering to files and windows.
    Command {
        names: &["png", "image"],
        flags: &[
            "section",
            "theme",
            "width",
            "pages",
            "page-height",
            "scale",
            "block",
            "against",
            "out",
        ],
        run: |args| view::run_png(args).map(Output::Report),
    },
    Command {
        names: &["play"],
        flags: &[
            "script", "node", "fps", "width", "height", "theme", "section", "out",
        ],
        run: |args| play::run(args).map(Output::Report),
    },
    Command {
        names: &["open"],
        flags: &[
            "section",
            "theme",
            "hidden",
            "fragment",
            "show-code",
            "title",
        ],
        run: |args| view::run_open(args).map(Output::Report),
    },
    // Writing.
    Command {
        names: &["new"],
        flags: &["title", "force"],
        run: |args| edit::run_new(args).map(Output::Report),
    },
    Command {
        names: &["index"],
        flags: &["force"],
        run: |args| index::run(args).map(Output::Report),
    },
    Command {
        names: &["set"],
        flags: &["text", "from", "header", "replace", "with", "all"],
        run: |args| edit::run_set(args).map(Output::Report),
    },
    Command {
        names: &["append"],
        flags: &["type", "id", "lang", "level", "run", "deps", "text", "from"],
        run: |args| edit::run_append(args).map(Output::Report),
    },
    Command {
        names: &["insert"],
        flags: &[
            "after", "type", "id", "level", "text", "lang", "run", "deps",
        ],
        run: |args| edit::run_insert(args).map(Output::Report),
    },
    Command {
        names: &["remove", "rm"],
        flags: &["block"],
        run: |args| edit::run_remove(args).map(Output::Report),
    },
    Command {
        names: &["check"],
        flags: &["block", "item"],
        run: |args| edit::run_check(args).map(Output::Report),
    },
    Command {
        names: &["board"],
        flags: &[
            "place",
            "add",
            "arrange",
            "detach",
            "link",
            "unlink",
            "to",
            "from-side",
            "to-side",
            "x",
            "y",
            "w",
            "h",
        ],
        run: |args| edit::run_board(args).map(Output::Report),
    },
    Command {
        names: &["fmt", "format"],
        flags: &["check"],
        run: |args| edit::run_fmt(args).map(Output::Report),
    },
    // Running.
    Command {
        names: &["run"],
        flags: &[
            "only",
            "force",
            "dry",
            "timeout",
            "review",
            "approve",
            "follow-edges",
        ],
        run: |args| exec::run(args).map(Output::Report),
    },
    // The store.
    Command {
        names: &["sync"],
        flags: &[],
        run: |args| store::run_sync(args).map(Output::Report),
    },
    Command {
        names: &["git-setup"],
        flags: &[],
        run: |args| store::run_git_setup(args).map(Output::Report),
    },
    // The platform.
    Command {
        names: &["setup", "install"],
        flags: &["all", "bin-dir", "uninstall", "print", "no-path"],
        run: |args| setup::run_setup(args).map(Output::Report),
    },
    Command {
        names: &["browser", "extension"],
        flags: &["browser", "target", "from", "dir", "open"],
        run: |args| browser::run(args).map(Output::Report),
    },
    Command {
        names: &["doctor"],
        flags: &[],
        run: |args| setup::run_doctor(args).map(Output::Report),
    },
];

/// Run the command named `command` with `args`.
///
/// An unknown command returns the help text as an error, so a typo is answered with the
/// list of what does exist. `--help` on any command prints that same text and runs
/// nothing, and a flag the command does not read is refused by name.
pub fn dispatch(command: &str, args: &Args) -> Result<Output, String> {
    let Some(entry) = COMMANDS.iter().find(|entry| entry.names.contains(&command)) else {
        return Err(format!("unknown command `{command}`\n\n{}", setup::HELP));
    };

    if args.present("help") {
        return setup::run_help(args).map(Output::Document);
    }
    if let Some(refused) = refuse_unknown_flags(command, args, entry.flags) {
        return Err(refused);
    }

    (entry.run)(args)
}

/// The sentence refusing flags `command` does not read, or `None` when every flag is known.
///
/// One formatter for every surface that rejects a flag — the dispatch table above, and the
/// long-running `dx serve` and `dx mcp`, which cannot go through it — so a refused flag
/// reads the same wherever it was typed, and is never silently swallowed.
#[must_use]
pub fn refuse_unknown_flags(command: &str, args: &Args, flags: &[&str]) -> Option<String> {
    let unknown = args.unknown_options(flags);
    if unknown.is_empty() {
        return None;
    }
    let rejected = unknown
        .iter()
        .map(|name| format!("--{name}"))
        .collect::<Vec<_>>()
        .join(", ");
    let accepted = if flags.is_empty() {
        "no flags".to_string()
    } else {
        flags
            .iter()
            .map(|name| format!("--{name}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    Some(format!(
        "`dx {command}` does not take {rejected}. It takes {accepted} — see `dx help`."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(tokens: &[&str]) -> Args {
        Args::parse(&tokens.iter().map(|t| (*t).to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn an_unknown_command_answers_with_the_command_list() {
        let error = dispatch("frobnicate", &args(&[])).expect_err("should fail");
        assert!(error.contains("unknown command `frobnicate`"));
        assert!(error.contains("dx render"));
    }

    #[test]
    fn version_reports_the_build_version() {
        let out = dispatch("version", &args(&[])).expect("version");
        assert!(out.text().starts_with("dx "));
    }

    #[test]
    fn familiar_aliases_reach_the_same_command() {
        assert!(dispatch("cat", &args(&[])).is_err()); // needs a file, but resolved the verb
        assert!(dispatch("help", &args(&[])).is_ok());
        assert_eq!(
            dispatch("list", &args(&["/dx/nowhere"])).expect("ls"),
            dispatch("ls", &args(&["/dx/nowhere"])).expect("ls")
        );
    }

    /// `--help` asks what a command does; it must never *do* it. This used to run a real
    /// insert, writing a stray block into the document that was named.
    #[test]
    fn help_on_any_command_answers_without_running_it() {
        let answered = dispatch("insert", &args(&["/dx/nowhere.dx", "--help"])).expect("help");
        assert!(matches!(answered, Output::Document(_)));
        assert!(answered.text().contains("dx insert"));
    }

    /// A flag a command does not read is refused by name, not swallowed: a caller whose
    /// `--lang` vanished believes they authored a runnable block.
    #[test]
    fn a_flag_a_command_does_not_take_is_refused_by_name() {
        let error =
            dispatch("set", &args(&["doc.dx", "b", "--lang", "python"])).expect_err("should fail");
        assert!(error.contains("--lang"), "{error}");
        assert!(error.contains("--text"), "{error}");

        let bare = dispatch("doctor", &args(&["--verbose"])).expect_err("should fail");
        assert!(bare.contains("no flags"), "{bare}");
    }

    /// Every flag the help text documents is one its command actually accepts.
    #[test]
    fn the_documented_flags_are_all_accepted() {
        for (command, flags) in [
            ("text", vec!["section", "ids"]),
            (
                "render",
                vec![
                    "section",
                    "theme",
                    "hidden",
                    "show-code",
                    "block",
                    "body",
                    "field",
                    "out",
                    "all",
                ],
            ),
            ("png", vec!["section", "out", "pages", "block", "against"]),
            ("search", vec!["limit"]),
            ("source", vec!["block", "header"]),
            (
                "set",
                vec!["text", "from", "header", "replace", "with", "all"],
            ),
            (
                "insert",
                vec![
                    "after", "type", "id", "level", "lang", "run", "deps", "text",
                ],
            ),
            (
                "append",
                vec!["type", "id", "level", "lang", "run", "deps", "text"],
            ),
            (
                "board",
                vec![
                    "place",
                    "add",
                    "arrange",
                    "detach",
                    "link",
                    "unlink",
                    "to",
                    "from-side",
                    "to-side",
                    "x",
                    "y",
                    "w",
                    "h",
                ],
            ),
            ("check", vec!["block", "item"]),
            (
                "run",
                vec![
                    "only",
                    "force",
                    "dry",
                    "timeout",
                    "review",
                    "approve",
                    "follow-edges",
                ],
            ),
            ("new", vec!["title", "force"]),
            ("index", vec!["force"]),
            ("fmt", vec!["check"]),
            ("setup", vec!["all", "bin-dir", "uninstall"]),
        ] {
            let entry = COMMANDS
                .iter()
                .find(|entry| entry.names.contains(&command))
                .unwrap_or_else(|| panic!("no `{command}` row"));
            for flag in flags {
                assert!(
                    entry.flags.contains(&flag),
                    "`dx {command}` rejects --{flag}"
                );
            }
        }
    }

    /// `dx search` reads `--limit`, so the table has to accept it. It did not, and the flag
    /// was refused by name — a documented command that cannot be run as documented.
    #[test]
    fn search_accepts_the_limit_it_reads() {
        let out = dispatch(
            "search",
            &args(&["kubernetes", "/dx/nowhere", "--limit", "3"]),
        )
        .expect("search");
        assert!(out.text().contains("no documents match"), "{}", out.text());
    }

    #[test]
    fn commands_that_write_files_report_instead_of_producing_redirectable_content() {
        // `--out` must never overwrite the image `dx png` just wrote, so png is a report.
        let png = dispatch("png", &args(&["/dx/nowhere.dx", "--out", "x.png"]));
        assert!(png.is_err() || matches!(png, Ok(Output::Report(_))));

        let listing = dispatch("ls", &args(&["/dx/nowhere"])).expect("ls");
        assert!(matches!(listing, Output::Document(_)));
    }

    /// The one refusal formatter serves the servers too: `dx serve` names its one flag,
    /// and `dx mcp` — which reads none — says so instead of silently swallowing them.
    #[test]
    fn the_servers_refuse_unknown_flags_through_the_same_formatter() {
        let refused =
            refuse_unknown_flags("serve", &args(&["--verbose"]), &["port"]).expect("should refuse");
        assert!(
            refused.contains("`dx serve` does not take --verbose"),
            "{refused}"
        );
        assert!(refused.contains("--port"), "{refused}");

        let bare =
            refuse_unknown_flags("mcp", &args(&["--root", "/x"]), &[]).expect("should refuse");
        assert!(bare.contains("`dx mcp` does not take --root"), "{bare}");
        assert!(bare.contains("no flags"), "{bare}");

        assert_eq!(
            refuse_unknown_flags("serve", &args(&["--port", "8080"]), &["port"]),
            None
        );
        assert_eq!(refuse_unknown_flags("mcp", &args(&[]), &[]), None);
    }

    #[test]
    fn output_text_is_readable_for_either_kind() {
        assert_eq!(Output::Document("a".into()).text(), "a");
        assert_eq!(Output::Report("b".into()).text(), "b");
    }
}
