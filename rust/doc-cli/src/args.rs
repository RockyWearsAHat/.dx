//! Command-line argument parsing.
//!
//! A hand-rolled parser rather than a dependency, because the surface is small and fixed:
//! positional words, `--flag`, `--name value`, and `--name=value`. Anything after a lone
//! `--` is positional, so a file named `--weird` is still reachable.

/// Parsed arguments for one command invocation.
#[derive(Debug, Clone, Default)]
pub struct Args {
    positional: Vec<String>,
    options: Vec<(String, Option<String>)>,
}

impl Args {
    /// Parse raw arguments (excluding the program name and the command word).
    ///
    /// A `--name` immediately followed by a non-flag word takes that word as its value;
    /// otherwise it is a boolean flag. `--name=value` always carries its value.
    #[must_use]
    pub fn parse(raw: &[String]) -> Self {
        let mut args = Self::default();
        let mut index = 0;
        let mut only_positional = false;

        while index < raw.len() {
            let token = &raw[index];
            index += 1;

            if only_positional || !token.starts_with("--") {
                args.positional.push(token.clone());
                continue;
            }
            if token == "--" {
                only_positional = true;
                continue;
            }

            let name = token.trim_start_matches("--");
            if let Some((key, value)) = name.split_once('=') {
                args.options
                    .push((key.to_string(), Some(value.to_string())));
                continue;
            }
            let takes_value = raw
                .get(index)
                .is_some_and(|next| !next.starts_with("--") || next == "-");
            if takes_value {
                args.options
                    .push((name.to_string(), Some(raw[index].clone())));
                index += 1;
            } else {
                args.options.push((name.to_string(), None));
            }
        }

        args
    }

    /// The positional argument at `index`, if present.
    #[must_use]
    pub fn positional(&self, index: usize) -> Option<&str> {
        self.positional.get(index).map(String::as_str)
    }

    /// Every positional argument, in order.
    #[must_use]
    pub fn positionals(&self) -> &[String] {
        &self.positional
    }

    /// The value of `--name`, if it was given one.
    #[must_use]
    pub fn value(&self, name: &str) -> Option<&str> {
        self.options
            .iter()
            .find(|(key, _)| key == name)
            .and_then(|(_, value)| value.as_deref())
    }

    /// Whether `--name` was present at all, with or without a value.
    #[must_use]
    pub fn present(&self, name: &str) -> bool {
        self.options.iter().any(|(key, _)| key == name)
    }

    /// The value of `--name` parsed as a number, when it parses.
    #[must_use]
    pub fn number(&self, name: &str) -> Option<u32> {
        self.value(name).and_then(|value| value.trim().parse().ok())
    }

    /// The names of every option that is not in `allowed`, in the order given.
    ///
    /// This is how a command rejects a flag it would otherwise silently ignore — a
    /// swallowed `--lang` is a caller who believes they authored a runnable block and
    /// did not.
    #[must_use]
    pub fn unknown_options(&self, allowed: &[&str]) -> Vec<String> {
        self.options
            .iter()
            .filter(|(name, _)| !allowed.contains(&name.as_str()))
            .map(|(name, _)| name.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(tokens: &[&str]) -> Args {
        Args::parse(&tokens.iter().map(|t| (*t).to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn separates_positionals_from_options() {
        let args = parse(&["notes.dx", "--section", "intro", "--force"]);
        assert_eq!(args.positional(0), Some("notes.dx"));
        assert_eq!(args.value("section"), Some("intro"));
        assert!(args.present("force"));
        assert_eq!(args.value("force"), None);
    }

    #[test]
    fn accepts_equals_form_and_numbers() {
        let args = parse(&["--width=1400", "--theme=dark"]);
        assert_eq!(args.number("width"), Some(1400));
        assert_eq!(args.value("theme"), Some("dark"));
        assert_eq!(args.number("theme"), None);
    }

    #[test]
    fn a_flag_before_another_flag_stays_boolean() {
        let args = parse(&["--force", "--only", "block"]);
        assert!(args.present("force"));
        assert_eq!(args.value("force"), None);
        assert_eq!(args.value("only"), Some("block"));
    }

    #[test]
    fn a_lone_dash_is_a_value_meaning_stdin() {
        let args = parse(&["--text", "-"]);
        assert_eq!(args.value("text"), Some("-"));
    }

    #[test]
    fn double_dash_forces_the_rest_positional() {
        let args = parse(&["--", "--not-a-flag.dx"]);
        assert_eq!(args.positional(0), Some("--not-a-flag.dx"));
        assert!(!args.present("not-a-flag.dx"));
    }

    #[test]
    fn collects_every_positional_for_multi_file_commands() {
        let args = parse(&["a.dx", "b.dx", "c.dx"]);
        assert_eq!(args.positionals().len(), 3);
    }

    #[test]
    fn names_the_options_a_command_does_not_take() {
        let args = parse(&["doc.dx", "--text", "x", "--lang", "python", "--run"]);
        assert_eq!(
            args.unknown_options(&["text", "lang", "run"]),
            Vec::<String>::new()
        );
        assert_eq!(args.unknown_options(&["text"]), vec!["lang", "run"]);
    }
}
