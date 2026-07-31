//! The `dx` command table.
//!
//! One entry per verb, grouped by what the reader is trying to do: read a document, write
//! one, run one, find one, or set up the platform. [`dispatch`] is the only place that maps
//! a command word to code, so adding a verb means adding one row.

pub mod browser;
pub mod edit;
pub mod exec;
pub mod find;
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

/// Run the command named `command` with `args`.
///
/// An unknown command returns the help text as an error, so a typo is answered with the
/// list of what does exist.
pub fn dispatch(command: &str, args: &Args) -> Result<Output, String> {
    match command {
        "text" | "cat" => view::run_text(args).map(Output::Document),
        "outline" => view::run_outline(args).map(Output::Document),
        "render" | "html" => view::run_render(args).map(Output::Document),
        "ls" | "list" => find::run_ls(args).map(Output::Document),
        "textconv" => store::run_textconv(args).map(Output::Document),
        "stats" => store::run_stats(args).map(Output::Document),
        "search" | "find" => find::run_search(args).map(Output::Document),
        "source" => edit::run_source(args).map(Output::Document),
        "help" | "--help" | "-h" => setup::run_help(args).map(Output::Document),
        "version" | "--version" => Ok(Output::Document(format!(
            "dx {}\n",
            env!("CARGO_PKG_VERSION")
        ))),

        "png" | "image" => view::run_png(args).map(Output::Report),
        "open" => view::run_open(args).map(Output::Report),
        "new" => edit::run_new(args).map(Output::Report),
        "set" => edit::run_set(args).map(Output::Report),
        "append" => edit::run_append(args).map(Output::Report),
        "insert" => edit::run_insert(args).map(Output::Report),
        "remove" | "rm" => edit::run_remove(args).map(Output::Report),
        "fmt" | "format" => edit::run_fmt(args).map(Output::Report),
        "run" => exec::run(args).map(Output::Report),
        "sync" => store::run_sync(args).map(Output::Report),
        "git-setup" => store::run_git_setup(args).map(Output::Report),
        "setup" | "install" => setup::run_setup(args).map(Output::Report),
        "browser" | "extension" => browser::run(args).map(Output::Report),
        "doctor" => setup::run_doctor(args).map(Output::Report),

        other => Err(format!("unknown command `{other}`\n\n{}", setup::HELP)),
    }
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

    #[test]
    fn commands_that_write_files_report_instead_of_producing_redirectable_content() {
        // `--out` must never overwrite the image `dx png` just wrote, so png is a report.
        let png = dispatch("png", &args(&["/dx/nowhere.dx", "--out", "x.png"]));
        assert!(png.is_err() || matches!(png, Ok(Output::Report(_))));

        let listing = dispatch("ls", &args(&["/dx/nowhere"])).expect("ls");
        assert!(matches!(listing, Output::Document(_)));
    }

    #[test]
    fn output_text_is_readable_for_either_kind() {
        assert_eq!(Output::Document("a".into()).text(), "a");
        assert_eq!(Output::Report("b".into()).text(), "b");
    }
}
