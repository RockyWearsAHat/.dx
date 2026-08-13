//! A symbol + reference index, built by the same heuristic this platform already uses for
//! [`crate::search`] and the `dx index` file-tree survey: no parser, no language server —
//! line-shaped patterns and identifier occurrence, extended one level from "this file
//! mentions that file's stem" to real named symbols and edges between them.
//!
//! # What this is not
//! [`trace`] does not build a call graph, does not track scope, and does not know a
//! variable from a function. Two facts follow directly from that, and both are worth
//! saying plainly rather than discovering by surprise:
//!
//! - **A symbol's "body" is never delimited.** There is no brace matching, so a
//!   reference to a symbol's name anywhere in a file counts, whether or not the mention
//!   is inside that symbol's own definition, and whether it names the symbol itself or
//!   an unrelated field or variable that merely shares its name (`.kind` and `fn kind()`
//!   look identical to a scan with no scope). A name that is common English (`new`,
//!   `run`, `get`) will over-match; this is the same tradeoff `doc-cli`'s scaffold
//!   survey already makes for file stems, made once more here for symbol names.
//! - **Names are not scoped.** Two symbols sharing a name — in different files, or a
//!   method and a free function — are indistinguishable to anything that references
//!   them by name alone: a reference to `run` cannot say which `run` it means, so
//!   [`Trace::references_to`] answers for the name, not for one definition of it, and
//!   [`Trace::ranked_by_fan_in`] leaves an ambiguous name out of the ranking entirely
//!   rather than let it drown out names that mean something on a real corpus.
//! - **A full-line comment is not a reference; a trailing one still is.** [`occurrences`]
//!   skips a line that is wholly a `//`- or `#`-style comment (never a Rust attribute,
//!   which also starts with `#`) — a module doc that discusses a symbol at length must
//!   not read as every file it's typed in calling it, and this repository's own prose-
//!   heavy doc-comment style was the proof that mattered before this filter existed. A
//!   trailing comment on a code line (`x(); // note`) is left alone: separating code
//!   from comment mid-line needs knowing where a string literal ends, which a
//!   line-shaped scan cannot answer per language without real parsing.
//! - **A name inside a `"…"` string or `'…'` char literal is not a reference either.**
//!   [`occurrences`] tracks quote state per line ([`code_tokens`]) so a name that only
//!   appears inside a string's text — `&["log", "trace", "diagnostic"]` naming `trace`
//!   nowhere near a call to it — does not count. A `'` is treated as opening a char/rune
//!   literal only when it can immediately close as one ([`char_literal_end`]); otherwise
//!   it is read as a Rust lifetime (`'a`, `'static`) and the identifier after it is
//!   scanned normally, so `&'a Widget` still finds `Widget`. A backtick is deliberately
//!   left alone — see [`code_tokens`] for why a JS template literal and a Markdown
//!   inline-code span cannot be told apart here, and this module keeps counting the
//!   latter.
//!
//! What it does hold, honestly: for a name defined once and referenced by an
//! unambiguous identifier elsewhere, outside a comment line, this finds the edge close
//! to the way a person `grep`-ing for that name would, and does it for every symbol in
//! the corpus at once. It is a starting point for a reader to verify, not a proof.
//!
//! One consequence is worth naming rather than leaving to be discovered by a confusing
//! [`Trace::ranked_by_fan_in`] result: a symbol can be the corpus's *only* definition of
//! its name and still rank high for the wrong reason, when that name is also a common
//! standard-library method (`contains`, `map`, `path`, `join`, `get`) — a scan with no
//! type information cannot tell "calls this project's `contains`" from "calls some
//! unrelated type's `contains`," and every such call still counts. Looking up a
//! *distinctive* name directly ([`Trace::definitions_of`] and [`Trace::references_to`] —
//! this module's own tests use `write_scaffold`, `distinctive_symbol`) is where this
//! index is trustworthy; the ranked top-N view is a lead to check, weighted toward
//! whatever name shape happens to be common, not a verdict.
//!
//! # Language coverage
//! Rust (`fn`, `struct`, `enum`, `trait`), JavaScript/TypeScript (`function`, `class`,
//! `const x = (...) =>`), Python (`def`, `class`), and Go (`func`, including a method's
//! receiver, and `type … struct`/`interface`). A file in any other language contributes
//! no symbols but is still scanned for *references* to symbols other files define — a
//! config file naming a function outside a comment is still a real reference.
//!
//! # Determinism
//! [`trace`] never reads a clock, the filesystem, or an environment variable; every
//! ordering choice (which occurrence line represents a reference, how symbols and
//! references are ranked) breaks ties by name and path, never by hash-map iteration
//! order — the same corpus always produces the same [`Trace`].

use std::collections::{HashMap, HashSet};

/// What kind of thing a [`Symbol`] names.
///
/// Deliberately coarse: the set covers every construct the extractors below recognize
/// without needing to know which language produced it, which is what lets
/// [`Trace::ranked_by_fan_in`] rank a Go `struct` against a Rust `struct` on equal terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    /// A function, method, or (JavaScript) arrow function assigned to a `const`.
    Function,
    /// A `struct` (Rust, Go).
    Struct,
    /// An `enum` (Rust).
    Enum,
    /// A `trait` (Rust) or `interface` (Go) — a contract other types implement.
    Trait,
    /// A `class` (JavaScript/TypeScript, Python).
    Class,
    /// A named type alias that is none of the above (Go's `type X = Y` / `type X Y`).
    Type,
    /// Reserved for a future extractor; no current language rule produces this today.
    Const,
}

impl SymbolKind {
    /// The lowercase word used to describe this kind in a report, matching how the
    /// `helpers index lookup` prior art labels a hit (`write_scaffold (function)`).
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            SymbolKind::Function => "function",
            SymbolKind::Struct => "struct",
            SymbolKind::Enum => "enum",
            SymbolKind::Trait => "trait",
            SymbolKind::Class => "class",
            SymbolKind::Type => "type",
            SymbolKind::Const => "const",
        }
    }
}

/// One named definition site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    /// The identifier as written — case preserved, since Rust, JS, and Go names are
    /// case-sensitive and folding them would merge distinct symbols.
    pub name: String,
    /// What kind of construct this is.
    pub kind: SymbolKind,
    /// The path exactly as the caller supplied it in `files`.
    pub file: String,
    /// 1-indexed line the definition starts on.
    pub line: usize,
}

/// One edge: `symbol` is mentioned inside `file`, first at `line`.
///
/// A file appears at most once per symbol name — this is a fan-in edge ("does this file
/// reference that name at all"), not an occurrence count. See the module doc for why the
/// edge is keyed by name rather than by one specific [`Symbol`] when names collide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    /// The referenced symbol's name.
    pub symbol: String,
    /// The file the reference was found in.
    pub file: String,
    /// 1-indexed line of the first occurrence in `file` that is not itself a
    /// definition line for this name.
    pub line: usize,
}

/// The result of [`trace`]: every symbol this corpus defines, and every file→name edge
/// found by scanning the whole corpus for each defined name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Trace {
    /// Every definition found, in no particular order (callers needing a stable order
    /// should sort by `(file, line)`).
    pub symbols: Vec<Symbol>,
    /// Every file→name reference edge found, in no particular order.
    pub references: Vec<Reference>,
}

impl Trace {
    /// Every definition named `name`, the way `helpers index lookup <name>` shows the
    /// definition site(s) — usually one, sometimes more when a name collides.
    #[must_use]
    pub fn definitions_of(&self, name: &str) -> Vec<&Symbol> {
        self.symbols.iter().filter(|s| s.name == name).collect()
    }

    /// Every file that references `name`, each with the line its first non-definition
    /// occurrence was found at — the second half of what `helpers index lookup <name>`
    /// answers.
    #[must_use]
    pub fn references_to(&self, name: &str) -> Vec<&Reference> {
        self.references
            .iter()
            .filter(|r| r.symbol == name)
            .collect()
    }

    /// Every **unambiguously defined** name paired with its fan-in — the count of
    /// distinct files that reference it — ranked highest first, ties broken by name so
    /// the order is deterministic. A name defined but never referenced by anything else
    /// in the corpus does not appear here: this ranks what is load-bearing, not
    /// everything that exists.
    ///
    /// A name with more than one definition site is excluded, not merely deprioritized.
    /// Without scoping (see the module doc), fan-in on a name like `new` or `of` counts
    /// references to every same-named definition at once — high fan-in there measures
    /// how common the name is, not how load-bearing any one symbol is, and it drowns out
    /// the names that actually mean something on a real multi-file corpus. A name
    /// defined exactly once has no such ambiguity: every reference to it can only mean
    /// that one symbol, so its fan-in is real signal — **except** when the name is also
    /// a common single-word accessor/getter/constructor spelling ([`is_generic_name`]):
    /// `path`, `map`, `get`, `id`, `kind`, … A symbol can be the corpus's only real
    /// *definition* of one of these and still see its "fan-in" mostly counting unrelated
    /// types' methods that merely share the name — report `91bfbee3` verified this
    /// against this repository's own `rust/` tree, where the top 25 unfiltered results
    /// were all such names and zero were a domain type. Those names are excluded here
    /// too, for the same over-counting reason ambiguous names already are; unfiltered
    /// lookup by name still works via [`Trace::definitions_of`]/[`Trace::references_to`].
    #[must_use]
    pub fn ranked_by_fan_in(&self) -> Vec<(&str, usize)> {
        let mut definition_counts: HashMap<&str, usize> = HashMap::new();
        for symbol in &self.symbols {
            *definition_counts.entry(symbol.name.as_str()).or_insert(0) += 1;
        }
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for reference in &self.references {
            let name = reference.symbol.as_str();
            if definition_counts.get(name) == Some(&1) && !is_generic_name(name) {
                *counts.entry(name).or_insert(0) += 1;
            }
        }
        let mut ranked: Vec<(&str, usize)> = counts.into_iter().collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
        ranked
    }
}

/// Common single-word accessor/getter/constructor names that recur across unrelated
/// types in real multi-file corpora — verified against this repository's own `rust/`
/// tree by report `91bfbee3`, whose unfiltered top 25 by fan-in were exactly these kinds
/// of names and no domain type. Matched case-insensitively so a same-shaped capitalized
/// form (`Path`, `Map`) is caught too. [`Trace::ranked_by_fan_in`] is the only place this
/// applies — a lookup by name ([`Trace::definitions_of`], [`Trace::references_to`]) is
/// never filtered, since a reader who already typed one of these names is not asking for
/// the ranked overview's judgment about how common it is.
const GENERIC_SYMBOL_NAMES: &[&str] = &[
    "at",
    "of",
    "id",
    "get",
    "set",
    "new",
    "len",
    "map",
    "run",
    "add",
    "ok",
    "err",
    "path",
    "join",
    "error",
    "contains",
    "push",
    "index",
    "kind",
    "line",
    "code",
    "value",
    "root",
    "target",
    "as_str",
    "heading",
    "paragraph",
    "bytes",
    "display",
    "name",
    "key",
    "data",
    "type",
    "from",
    "into",
    "default",
    "clone",
    "iter",
    "next",
    "insert",
    "remove",
    "filter",
    "find",
    "parse",
    "read",
    "write",
    "unwrap",
    "format",
    "to_string",
    "as_ref",
    "as_mut",
    "is_empty",
    "first",
    "last",
    "count",
    "sort",
    "entry",
    "keys",
    "values",
    "split",
    "trim",
    "replace",
    "extend",
    "string",
    "vec",
];

/// Whether `name` is one of [`GENERIC_SYMBOL_NAMES`], compared case-insensitively.
fn is_generic_name(name: &str) -> bool {
    GENERIC_SYMBOL_NAMES.contains(&name.to_lowercase().as_str())
}

/// Trace symbols and references across `files` — `(path, text)` pairs, in whatever order
/// the caller found them. Pure and deterministic: no I/O, `wasm32`-friendly, same shape
/// as [`crate::search::SearchIndex::build`] — text in, structured data out.
#[must_use]
pub fn trace(files: &[(String, String)]) -> Trace {
    let mut symbols = Vec::new();
    for (path, text) in files {
        if let Some(language) = Language::detect(path) {
            for (name, kind, line) in language.extract(text) {
                symbols.push(Symbol {
                    name,
                    kind,
                    file: path.clone(),
                    line,
                });
            }
        }
    }

    // Every definition site of every name, so a reference scan can skip a name's own
    // declaration line(s) without mistaking them for a use.
    let mut definition_lines: HashMap<&str, HashSet<(&str, usize)>> = HashMap::new();
    for symbol in &symbols {
        definition_lines
            .entry(symbol.name.as_str())
            .or_default()
            .insert((symbol.file.as_str(), symbol.line));
    }

    let occurrences_by_file: Vec<(&str, HashMap<String, Vec<usize>>)> = files
        .iter()
        .map(|(path, text)| (path.as_str(), occurrences(text)))
        .collect();

    let mut names: Vec<&str> = definition_lines.keys().copied().collect();
    names.sort_unstable();

    let mut references = Vec::new();
    for name in names {
        let definitions = &definition_lines[name];
        for (path, occurrence_map) in &occurrences_by_file {
            let Some(lines) = occurrence_map.get(name) else {
                continue;
            };
            if let Some(&line) = lines
                .iter()
                .find(|&&line| !definitions.contains(&(*path, line)))
            {
                references.push(Reference {
                    symbol: name.to_string(),
                    file: (*path).to_string(),
                    line,
                });
            }
        }
    }

    Trace {
        symbols,
        references,
    }
}

/// A language this module knows how to extract symbols from, by file extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Language {
    Rust,
    JavaScript,
    Python,
    Go,
}

impl Language {
    /// The language `path`'s extension names, or `None` when it names none of the four
    /// covered languages — such a file still contributes reference occurrences, just no
    /// definitions.
    fn detect(path: &str) -> Option<Self> {
        let extension = path.rsplit('.').next().unwrap_or("");
        match extension {
            "rs" => Some(Language::Rust),
            "js" | "mjs" | "cjs" | "jsx" | "ts" | "tsx" => Some(Language::JavaScript),
            "py" => Some(Language::Python),
            "go" => Some(Language::Go),
            _ => None,
        }
    }

    /// `(name, kind, 1-indexed line)` for every definition [`extract_rust`],
    /// [`extract_javascript`], [`extract_python`], or [`extract_go`] recognizes in
    /// `text`.
    fn extract(self, text: &str) -> Vec<(String, SymbolKind, usize)> {
        match self {
            Language::Rust => extract_rust(text),
            Language::JavaScript => extract_javascript(text),
            Language::Python => extract_python(text),
            Language::Go => extract_go(text),
        }
    }
}

/// The leading identifier characters (ASCII alphanumeric or `_`) of `text`, or `None`
/// when there are none or the run starts with a digit (not a valid identifier start in
/// any of the covered languages).
fn ident_at(text: &str) -> Option<&str> {
    let end = text
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(text.len());
    let name = &text[..end];
    if name.is_empty() || name.as_bytes()[0].is_ascii_digit() {
        None
    } else {
        Some(name)
    }
}

/// `line` with `word` and at least one following whitespace character stripped, or
/// `None` when `line` does not start with `word` as a whole token — so matching `fn`
/// never fires on `fn_helper`.
fn strip_keyword<'a>(line: &'a str, word: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(word)?;
    if rest.starts_with(|c: char| c.is_whitespace()) {
        Some(rest.trim_start())
    } else {
        None
    }
}

/// Definitions in Rust source: `fn`, `struct`, `enum`, `trait`, each with `pub`,
/// `pub(crate)`/`pub(in …)`, `async`, `unsafe`, and `const` modifiers stripped first.
/// `impl … for …` is not itself a definition — the trait and type it names are just
/// identifiers, already caught by the reference scan every line feeds — and a method
/// inside an `impl` block is caught by the same `fn` rule with no brace tracking needed,
/// since this extractor never looks at indentation or nesting.
fn extract_rust(text: &str) -> Vec<(String, SymbolKind, usize)> {
    let mut found = Vec::new();
    for (index, raw_line) in text.lines().enumerate() {
        let line_number = index + 1;
        let line = strip_rust_modifiers(raw_line.trim_start());
        let hit = strip_keyword(line, "fn")
            .map(|rest| (rest, SymbolKind::Function))
            .or_else(|| strip_keyword(line, "struct").map(|rest| (rest, SymbolKind::Struct)))
            .or_else(|| strip_keyword(line, "enum").map(|rest| (rest, SymbolKind::Enum)))
            .or_else(|| strip_keyword(line, "trait").map(|rest| (rest, SymbolKind::Trait)));
        if let Some((rest, kind)) = hit {
            if let Some(name) = ident_at(rest) {
                found.push((name.to_string(), kind, line_number));
            }
        }
    }
    found
}

/// Strip Rust visibility/qualifier modifiers (`pub`, `pub(crate)`, `pub(in path)`,
/// `async`, `unsafe`, `const`, `default`) from the front of a trimmed line, repeatedly,
/// so `pub async fn go()` reaches the `fn` rule the same as `fn go()`.
fn strip_rust_modifiers(mut line: &str) -> &str {
    loop {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("pub(") {
            if let Some(end) = rest.find(')') {
                line = &rest[end + 1..];
                continue;
            }
        }
        let mut stripped = false;
        for keyword in ["pub ", "async ", "unsafe ", "const ", "default "] {
            if let Some(rest) = trimmed.strip_prefix(keyword) {
                line = rest;
                stripped = true;
                break;
            }
        }
        if !stripped {
            return trimmed;
        }
    }
}

/// Definitions in JavaScript/TypeScript source: `function` (plain, `async`, generator
/// `function*`), `class`, and `const x = (…) =>` — each optionally behind `export` or
/// `export default`. A typed const (`const x: Handler = (…) => …`) is not recognized:
/// the arrow rule only fires when `=` follows the name directly, which keeps the rule to
/// exactly the shape it claims and no attempt at parsing a type annotation.
fn extract_javascript(text: &str) -> Vec<(String, SymbolKind, usize)> {
    let mut found = Vec::new();
    for (index, raw_line) in text.lines().enumerate() {
        let line_number = index + 1;
        let mut line = raw_line.trim_start();
        for keyword in ["export default ", "export "] {
            if let Some(rest) = line.strip_prefix(keyword) {
                line = rest;
            }
        }
        line = line.strip_prefix("async ").unwrap_or(line);

        if let Some(rest) = strip_keyword(line, "function") {
            let rest = rest.trim_start_matches('*').trim_start();
            if let Some(name) = ident_at(rest) {
                found.push((name.to_string(), SymbolKind::Function, line_number));
            }
        } else if let Some(rest) = strip_keyword(line, "class") {
            if let Some(name) = ident_at(rest) {
                found.push((name.to_string(), SymbolKind::Class, line_number));
            }
        } else if let Some(rest) = strip_keyword(line, "const") {
            if let Some(name) = ident_at(rest) {
                let after = rest[name.len()..].trim_start();
                if is_assignment(after) && raw_line.contains("=>") {
                    found.push((name.to_string(), SymbolKind::Function, line_number));
                }
            }
        }
    }
    found
}

/// Whether `text` starts with a plain `=` assignment — not `==`, `===`, or `=>`, which
/// all also start with `=` but are not "this name is being assigned".
fn is_assignment(text: &str) -> bool {
    text.starts_with('=') && !text.starts_with("==") && !text.starts_with("=>")
}

/// Definitions in Python source: `def` (including `async def`) and `class`.
fn extract_python(text: &str) -> Vec<(String, SymbolKind, usize)> {
    let mut found = Vec::new();
    for (index, raw_line) in text.lines().enumerate() {
        let line_number = index + 1;
        let line = raw_line.trim_start();
        let line = line.strip_prefix("async ").unwrap_or(line);
        if let Some(rest) = strip_keyword(line, "def") {
            if let Some(name) = ident_at(rest) {
                found.push((name.to_string(), SymbolKind::Function, line_number));
            }
        } else if let Some(rest) = strip_keyword(line, "class") {
            if let Some(name) = ident_at(rest) {
                found.push((name.to_string(), SymbolKind::Class, line_number));
            }
        }
    }
    found
}

/// Definitions in Go source: `func` (plain, or a method with a `(receiver *Type)`
/// clause skipped before the name), and `type X struct|interface|…`.
fn extract_go(text: &str) -> Vec<(String, SymbolKind, usize)> {
    let mut found = Vec::new();
    for (index, raw_line) in text.lines().enumerate() {
        let line_number = index + 1;
        let line = raw_line.trim_start();
        if let Some(rest) = strip_keyword(line, "func") {
            let rest = if rest.starts_with('(') {
                match rest.find(')') {
                    Some(close) => rest[close + 1..].trim_start(),
                    None => continue,
                }
            } else {
                rest
            };
            if let Some(name) = ident_at(rest) {
                found.push((name.to_string(), SymbolKind::Function, line_number));
            }
        } else if let Some(rest) = strip_keyword(line, "type") {
            if let Some(name) = ident_at(rest) {
                let after = rest[name.len()..].trim_start();
                let kind = if after.starts_with("struct") {
                    SymbolKind::Struct
                } else if after.starts_with("interface") {
                    SymbolKind::Trait
                } else {
                    SymbolKind::Type
                };
                found.push((name.to_string(), kind, line_number));
            }
        }
    }
    found
}

/// Every identifier token in `text` (ASCII alphanumeric/`_` runs) that falls outside a
/// string/char literal, mapped to every 1-indexed line it occurs on. Case is preserved
/// — this is the reference-side twin of the extractors above, not [`crate::search`]'s
/// full-text tokenizer, which lowercases and stems for a different purpose.
fn occurrences(text: &str) -> HashMap<String, Vec<usize>> {
    let mut map: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, raw_line) in text.lines().enumerate() {
        if is_full_line_comment(raw_line) {
            continue;
        }
        let line = index + 1;
        for token in code_tokens(raw_line) {
            map.entry(token).or_default().push(line);
        }
    }
    map
}

/// The identifier tokens on one line of code, skipping any that fall inside a `"…"`
/// string literal, or a `'…'` char/rune literal — but not a Rust lifetime (`'a`,
/// `'static`), which must keep scanning as ordinary code. See the module doc for why
/// this matters and what it does not catch.
///
/// A backtick is deliberately **not** treated as a string delimiter here, even though
/// it opens a JS/TS template literal: this scanner has no language tag to tell that
/// case apart from a Markdown/prose file's inline-code span (`` `helper()` ``), and the
/// latter is a real reference this module has always counted — [`crate::search`]'s own
/// prior art and this module's own test corpus both depend on it. Skipping template
/// literal *text* (not `${…}` interpolation, which this line-shaped scan cannot isolate
/// either) is the false negative already accepted for `` `…` ``; making backtick-code-
/// span references disappear everywhere else would trade one blind spot for a bigger
/// one.
fn code_tokens(line: &str) -> Vec<String> {
    let chars: Vec<char> = line.chars().collect();
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '"' {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            i = skip_delimited(&chars, i, c);
            continue;
        }
        if c == '\'' {
            if let Some(close) = char_literal_end(&chars, i) {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
                i = close + 1;
                continue;
            }
            // Not a char literal — a Rust lifetime or a stray apostrophe. Treat it as
            // an ordinary delimiter so the identifier that follows still scans.
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            i += 1;
            continue;
        }
        if c.is_ascii_alphanumeric() || c == '_' {
            current.push(c);
            i += 1;
            continue;
        }
        if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
        i += 1;
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// The index just past the closing `quote` for a `"…"` string literal opening at
/// `chars[start]`, honoring `\`-escapes. Runs to the end of the line when unterminated
/// (an unterminated literal cannot hide a real reference past the line's own end).
fn skip_delimited(chars: &[char], start: usize, quote: char) -> usize {
    let mut i = start + 1;
    while i < chars.len() {
        if chars[i] == '\\' {
            i += 2;
            continue;
        }
        if chars[i] == quote {
            return i + 1;
        }
        i += 1;
    }
    chars.len()
}

/// The index of the closing `'` of a `'…'` char/rune literal opening at `chars[start]`,
/// when what follows reads as one: a single character (`'a'`), a `\`-escape (`'\n'`,
/// `'\''`), or a `\u{…}` unicode escape (`'\u{1F600}'`), each immediately closed.
/// `None` when it does not — most importantly a Rust lifetime (`'a`, `'static`), which
/// never closes with a second `'` at all, so this never mistakes one for a literal and
/// swallows the rest of the line as string content.
fn char_literal_end(chars: &[char], start: usize) -> Option<usize> {
    if chars.get(start + 1) == Some(&'\\') {
        if chars.get(start + 2) == Some(&'u') && chars.get(start + 3) == Some(&'{') {
            let mut i = start + 4;
            while i < chars.len() && chars[i] != '}' {
                i += 1;
            }
            let close = i + 1;
            return (chars.get(close) == Some(&'\'')).then_some(close);
        }
        let close = start + 3;
        return (chars.get(close) == Some(&'\'')).then_some(close);
    }
    let close = start + 2;
    (chars.get(close) == Some(&'\'')).then_some(close)
}

/// Whether `line`, trimmed, is wholly a comment and therefore prose rather than code:
/// `//`-style (Rust/JS/TS/Go, including the doc-comment forms `///` and `//!`) or
/// `#`-style (Python, shell, config) — but never a Rust attribute (`#[...]`/`#![...]`),
/// which also starts with `#` and names real symbols (`#[derive(Trace)]`).
///
/// A name mentioned only in a module's own prose about itself is not evidence that
/// prose's file uses that symbol, and this repository's own module docs are exactly the
/// kind of dense, identifier-naming prose that would otherwise dominate every ranking —
/// this file's fan-in noise on itself, before this filter, was the proof. A **trailing**
/// comment on a code line (`x(); // note`) is deliberately left alone: separating code
/// from comment mid-line needs knowing where a string literal ends, which a line-shaped
/// scan cannot answer per language, and the conservative choice is to keep counting it
/// rather than risk dropping real code inside what looks like a string.
fn is_full_line_comment(line: &str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.starts_with("//") {
        return true;
    }
    trimmed.starts_with('#') && !trimmed.starts_with("#[") && !trimmed.starts_with("#!")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(path, text)| ((*path).to_string(), (*text).to_string()))
            .collect()
    }

    #[test]
    fn an_empty_file_traces_to_nothing() {
        let traced = trace(&files(&[("empty.rs", "")]));
        assert!(traced.symbols.is_empty());
        assert!(traced.references.is_empty());
    }

    #[test]
    fn no_files_traces_to_nothing() {
        let traced = trace(&[]);
        assert!(traced.symbols.is_empty());
        assert!(traced.references.is_empty());
    }

    #[test]
    fn rust_defines_and_a_sibling_file_references_it() {
        let traced = trace(&files(&[
            (
                "src/lib.rs",
                "pub fn scaffold() -> bool {\n    true\n}\n\npub(crate) struct Config {}\n\npub trait Runner {}\n\nenum Mode { A, B }\n",
            ),
            (
                "src/main.rs",
                "fn main() {\n    lib::scaffold();\n}\n",
            ),
        ]));

        let scaffold = traced
            .symbols
            .iter()
            .find(|s| s.name == "scaffold")
            .expect("scaffold defined");
        assert_eq!(scaffold.kind, SymbolKind::Function);
        assert_eq!(scaffold.file, "src/lib.rs");
        assert_eq!(scaffold.line, 1);

        assert!(traced
            .symbols
            .iter()
            .any(|s| s.name == "Config" && s.kind == SymbolKind::Struct));
        assert!(traced
            .symbols
            .iter()
            .any(|s| s.name == "Runner" && s.kind == SymbolKind::Trait));
        assert!(traced
            .symbols
            .iter()
            .any(|s| s.name == "Mode" && s.kind == SymbolKind::Enum));

        let references = traced.references_to("scaffold");
        assert_eq!(references.len(), 1);
        assert_eq!(references[0].file, "src/main.rs");
        assert_eq!(references[0].line, 2);
    }

    #[test]
    fn javascript_covers_function_class_and_arrow_const() {
        let text = "export function fetchData() {}\n\nexport default class Widget {}\n\nconst handleClick = (event) => {\n  doThing();\n};\n\nconst PLAIN = 1;\n";
        let found = extract_javascript(text);
        assert!(found.contains(&("fetchData".to_string(), SymbolKind::Function, 1)));
        assert!(found.contains(&("Widget".to_string(), SymbolKind::Class, 3)));
        assert!(found.contains(&("handleClick".to_string(), SymbolKind::Function, 5)));
        // A const with no `=>` on the same line is not an arrow function.
        assert!(!found.iter().any(|(name, ..)| name == "PLAIN"));
    }

    #[test]
    fn a_javascript_reference_crosses_files() {
        let traced = trace(&files(&[
            (
                "src/widget.js",
                "export function render() {\n  return 1;\n}\n",
            ),
            (
                "src/app.js",
                "import { render } from './widget';\nrender();\n",
            ),
        ]));
        let references = traced.references_to("render");
        assert_eq!(references.len(), 1);
        assert_eq!(references[0].file, "src/app.js");
    }

    #[test]
    fn python_covers_def_and_class() {
        let text =
            "class Loader:\n    def load(self):\n        pass\n\nasync def fetch():\n    pass\n";
        let found = extract_python(text);
        assert!(found.contains(&("Loader".to_string(), SymbolKind::Class, 1)));
        assert!(found.contains(&("load".to_string(), SymbolKind::Function, 2)));
        assert!(found.contains(&("fetch".to_string(), SymbolKind::Function, 5)));
    }

    #[test]
    fn a_python_reference_crosses_files() {
        let traced = trace(&files(&[
            ("loader.py", "def load():\n    pass\n"),
            ("main.py", "from loader import load\nload()\n"),
        ]));
        let references = traced.references_to("load");
        assert_eq!(references.len(), 1);
        assert_eq!(references[0].file, "main.py");
        assert_eq!(references[0].line, 1); // the import line names it first
    }

    #[test]
    fn go_covers_func_receiver_methods_and_typed_struct() {
        let text = "package widgets\n\nfunc New() *Widget {\n\treturn &Widget{}\n}\n\nfunc (w *Widget) Render() string {\n\treturn \"\"\n}\n\ntype Widget struct {\n\tName string\n}\n\ntype Renderer interface {\n\tRender() string\n}\n";
        let found = extract_go(text);
        assert!(found.contains(&("New".to_string(), SymbolKind::Function, 3)));
        assert!(found.contains(&("Render".to_string(), SymbolKind::Function, 7)));
        assert!(found.contains(&("Widget".to_string(), SymbolKind::Struct, 11)));
        assert!(found.contains(&("Renderer".to_string(), SymbolKind::Trait, 15)));
    }

    #[test]
    fn a_go_reference_crosses_files() {
        let traced = trace(&files(&[
            (
                "widget.go",
                "package widgets\n\nfunc New() int {\n\treturn 1\n}\n",
            ),
            (
                "main.go",
                "package main\n\nfunc main() {\n\twidgets.New()\n}\n",
            ),
        ]));
        let references = traced.references_to("New");
        assert_eq!(references.len(), 1);
        assert_eq!(references[0].file, "main.go");
    }

    #[test]
    fn a_symbol_can_reference_itself() {
        let traced = trace(&files(&[(
            "fib.rs",
            "fn fib(n: u32) -> u32 {\n    if n < 2 { n } else { fib(n - 1) + fib(n - 2) }\n}\n",
        )]));
        let references = traced.references_to("fib");
        assert_eq!(references.len(), 1);
        assert_eq!(references[0].file, "fib.rs");
        assert_eq!(references[0].line, 2);
    }

    #[test]
    fn two_same_named_symbols_in_different_files_both_appear_as_definitions() {
        let traced = trace(&files(&[
            ("a.rs", "pub fn run() {}\n"),
            ("b.rs", "pub fn run() {}\n"),
            ("c.rs", "fn main() {\n    run();\n}\n"),
        ]));
        let definitions = traced.definitions_of("run");
        assert_eq!(definitions.len(), 2);
        let files_defining: HashSet<&str> = definitions.iter().map(|s| s.file.as_str()).collect();
        assert!(files_defining.contains("a.rs"));
        assert!(files_defining.contains("b.rs"));

        // The name is not scoped, so the third file's one reference is not duplicated
        // per colliding definition.
        let references = traced.references_to("run");
        assert_eq!(references.len(), 1);
        assert_eq!(references[0].file, "c.rs");
    }

    #[test]
    fn a_non_covered_language_still_contributes_references_but_no_definitions() {
        let traced = trace(&files(&[
            ("lib.rs", "pub fn helper() {}\n"),
            ("notes.md", "Call `helper()` to do the thing.\n"),
        ]));
        assert_eq!(traced.symbols.len(), 1);
        let references = traced.references_to("helper");
        assert_eq!(references.len(), 1);
        assert_eq!(references[0].file, "notes.md");
    }

    /// A comment-only line naming a symbol is prose, not a use of it — a module doc that
    /// talks about `helper` at length must not read as several files calling it. A Rust
    /// attribute still counts: `#[...]` starts with `#` too, and is real code.
    #[test]
    fn a_full_line_comment_is_not_a_reference_but_an_attribute_line_still_is() {
        let traced = trace(&files(&[
            ("lib.rs", "pub fn helper() {}\n"),
            (
                "caller.rs",
                "// helper is mentioned here in prose\n\
                 # helper (python-style comment mentions it too)\n\
                 #[helper]\n\
                 fn go() {}\n",
            ),
        ]));
        let references = traced.references_to("helper");
        assert_eq!(
            references.len(),
            1,
            "only the attribute line's mention should count: {references:?}"
        );
        assert_eq!(references[0].file, "caller.rs");
        assert_eq!(references[0].line, 3);
    }

    #[test]
    fn ranked_by_fan_in_orders_by_distinct_referencing_files_then_name() {
        let traced = trace(&files(&[
            (
                "core.rs",
                "pub fn widely_used() {}\npub fn rarely_used() {}\n",
            ),
            ("a.rs", "fn a() { widely_used(); }\n"),
            ("b.rs", "fn b() { widely_used(); rarely_used(); }\n"),
        ]));
        let ranked = traced.ranked_by_fan_in();
        assert_eq!(ranked[0].0, "widely_used");
        assert_eq!(ranked[0].1, 2);
        assert_eq!(ranked[1].0, "rarely_used");
        assert_eq!(ranked[1].1, 1);
    }

    /// A name defined in many files (the idiomatic-short-name case: `new`, `of`, `from`)
    /// is exactly what real multi-crate corpora are full of, and its raw fan-in is noise
    /// — a reference to `new` could mean any of a dozen constructors. Ranking must leave
    /// it out entirely rather than let it crowd out the one symbol whose name is
    /// actually unique and whose fan-in is therefore real signal.
    #[test]
    fn ranked_by_fan_in_excludes_names_with_more_than_one_definition() {
        let traced = trace(&files(&[
            ("a.rs", "pub fn new() -> A { A }\n"),
            ("b.rs", "pub fn new() -> B { B }\n"),
            ("unique.rs", "pub fn distinctive_symbol() {}\n"),
            (
                "caller.rs",
                "fn go() { new(); new(); distinctive_symbol(); }\n",
            ),
        ]));
        let ranked = traced.ranked_by_fan_in();
        assert!(
            ranked.iter().all(|(name, _)| *name != "new"),
            "an ambiguously-defined name must not be ranked: {ranked:?}"
        );
        assert_eq!(ranked, vec![("distinctive_symbol", 1)]);
    }

    /// report `ee2c5053`: a symbol's name appearing only inside an unrelated string
    /// literal must not count as a real reference — the exact repro from the report.
    #[test]
    fn a_name_inside_a_string_literal_is_not_a_reference() {
        let traced = trace(&files(&[
            ("a.rs", "pub fn trace() {}\n"),
            (
                "b.rs",
                "const KINDS: &[&str] = &[\"log\", \"trace\", \"diagnostic\"];\n",
            ),
        ]));
        assert!(
            traced.references_to("trace").is_empty(),
            "the string literal must not count: {:?}",
            traced.references_to("trace")
        );
    }

    /// A real call to a symbol on the same line as an unrelated string still counts —
    /// the fix must not over-correct into skipping the whole line.
    #[test]
    fn a_real_call_beside_a_string_literal_still_counts() {
        let traced = trace(&files(&[
            ("a.rs", "pub fn trace() {}\n"),
            ("b.rs", "fn go() { log(\"not trace\"); trace(); }\n"),
        ]));
        let references = traced.references_to("trace");
        assert_eq!(references.len(), 1);
        assert_eq!(references[0].file, "b.rs");
    }

    /// A Rust lifetime (`'a`) must never be mistaken for an opening char literal — that
    /// would swallow the rest of the line as "string content" and lose a real reference
    /// sitting right after it.
    #[test]
    fn a_rust_lifetime_does_not_suppress_a_reference_on_the_same_line() {
        let traced = trace(&files(&[
            ("widget.rs", "pub struct Widget;\n"),
            (
                "borrow.rs",
                "fn borrow<'a>(w: &'a Widget) -> &'a Widget {\n    w\n}\n",
            ),
        ]));
        let references = traced.references_to("Widget");
        assert_eq!(references.len(), 1);
        assert_eq!(references[0].file, "borrow.rs");
        assert_eq!(references[0].line, 1);
    }

    /// A genuine char literal is still skipped as literal content, same as a string.
    #[test]
    fn a_char_literal_is_not_a_reference() {
        let traced = trace(&files(&[
            ("a.rs", "pub fn x() {}\n"),
            ("b.rs", "fn go(c: char) -> bool {\n    c == 'x'\n}\n"),
        ]));
        assert!(traced.references_to("x").is_empty());
    }

    /// report `91bfbee3`: a common single-word accessor name must not rank even when it
    /// is, technically, defined exactly once in the corpus — its raw fan-in is noise,
    /// verified against this repository's own tree. A distinctive name alongside it
    /// still ranks normally.
    #[test]
    fn ranked_by_fan_in_excludes_generic_accessor_names() {
        let traced = trace(&files(&[
            (
                "core.rs",
                "pub fn path() -> String { String::new() }\npub fn distinctive_widget() {}\n",
            ),
            ("a.rs", "fn a() { path(); path(); distinctive_widget(); }\n"),
            ("b.rs", "fn b() { path(); }\n"),
        ]));
        let ranked = traced.ranked_by_fan_in();
        assert!(
            ranked.iter().all(|(name, _)| *name != "path"),
            "a generic accessor name must not be ranked: {ranked:?}"
        );
        assert_eq!(ranked, vec![("distinctive_widget", 1)]);
    }

    #[test]
    fn symbol_kind_label_covers_every_variant() {
        for kind in [
            SymbolKind::Function,
            SymbolKind::Struct,
            SymbolKind::Enum,
            SymbolKind::Trait,
            SymbolKind::Class,
            SymbolKind::Type,
            SymbolKind::Const,
        ] {
            assert!(!kind.label().is_empty());
        }
    }
}
