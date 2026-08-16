//! The `actions` shorthand for a `lang=capture` block's body: `wait`, `click`, `hover`,
//! `type`, `key`, `scroll`, and the `eval` escape hatch, instead of hand-written
//! JavaScript for the common cases.
//!
//! This adds no capability `live.rs` did not already have — every verb here compiles to
//! exactly the JavaScript [`crate::live::capture`] would already have evaluated, so it
//! carries none of its own security reasoning; [`crate::live`]'s module doc is still the
//! authority on what a capture may reach. What this buys is a lower barrier: driving a
//! live page into a state (fill a form, click a button, wait for something to settle)
//! without writing raw DOM JavaScript for it — a target here is a **CSS selector**, not a
//! `.dx` block id, because the page a capture opens is not this document's own page.
//!
//! Compiled output is one JavaScript statement per action, joined with `;\n` — the same
//! text [`crate::live::capture`] already wraps in an async IIFE and evaluates, so `wait`
//! (the only asynchronous verb) reads naturally as `await`.

/// Compile an `actions`-shorthand script into the JavaScript body [`crate::live::capture`]
/// evaluates. Each `;`-separated statement is one action; a `;`, `'`, or `"` inside a
/// quoted argument or inside `(`/`[`/`{` nesting does not end the statement.
///
/// # Errors
/// Returns a sentence naming the statement and what was wrong with it.
pub(crate) fn compile(script: &str) -> Result<String, String> {
    let mut lines = Vec::new();
    for statement in split_statements(script) {
        lines.push(compile_statement(&statement)?);
    }
    if lines.is_empty() {
        return Err(
            "an `actions` block needs at least one statement — wait, click, hover, type, \
             key, scroll, or eval"
                .to_string(),
        );
    }
    Ok(lines.join(";\n"))
}

/// One action statement, compiled to the JavaScript that performs it.
fn compile_statement(statement: &str) -> Result<String, String> {
    let (verb, rest) = statement
        .split_once(char::is_whitespace)
        .unwrap_or((statement, ""));
    let rest = rest.trim();
    match verb {
        "wait" => {
            let ms = parse_duration(rest)?;
            Ok(format!("await new Promise(r => setTimeout(r, {ms}))"))
        }
        "click" => {
            let selector = js_string(require_arg("click", rest)?);
            Ok(format!(
                "await __dxAction({selector}, el => {{ \
                 el.dispatchEvent(new MouseEvent('mousedown', {{bubbles:true,cancelable:true}})); \
                 el.dispatchEvent(new MouseEvent('mouseup', {{bubbles:true,cancelable:true}})); \
                 el.click(); \
                 }})"
            ))
        }
        "hover" => {
            let selector = js_string(require_arg("hover", rest)?);
            Ok(format!(
                "await __dxAction({selector}, el => {{ \
                 el.dispatchEvent(new MouseEvent('mouseover', {{bubbles:true}})); \
                 el.dispatchEvent(new MouseEvent('mouseenter', {{bubbles:true}})); \
                 }})"
            ))
        }
        "type" => {
            let (selector, text) = split_selector_and_quoted(rest, "type")?;
            let selector = js_string(&selector);
            let text = js_string(&text);
            Ok(format!(
                "await __dxAction({selector}, el => {{ \
                 const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value') \
                 || Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, 'value'); \
                 el.focus(); \
                 if (setter && setter.set) {{ setter.set.call(el, {text}); }} else {{ el.value = {text}; }} \
                 el.dispatchEvent(new Event('input', {{bubbles:true}})); \
                 el.dispatchEvent(new Event('change', {{bubbles:true}})); \
                 }})"
            ))
        }
        "key" => {
            let name = js_string(require_arg("key", rest)?);
            Ok(format!(
                "(() => {{ \
                 const el = document.activeElement || document.body; \
                 const opts = {{key: {name}, bubbles:true, cancelable:true}}; \
                 el.dispatchEvent(new KeyboardEvent('keydown', opts)); \
                 el.dispatchEvent(new KeyboardEvent('keyup', opts)); \
                 }})()"
            ))
        }
        "scroll" => compile_scroll(rest),
        "eval" => {
            if rest.is_empty() {
                return Err("`eval` needs a JavaScript expression after it".to_string());
            }
            Ok(rest.to_string())
        }
        other => Err(format!(
            "`{other}` is not an action — wait, click, hover, type, key, scroll, or eval"
        )),
    }
}

/// `scroll 200` (the window) or `scroll <selector> 200` (one element's own overflow).
fn compile_scroll(rest: &str) -> Result<String, String> {
    let words: Vec<&str> = rest.split_whitespace().collect();
    match words.as_slice() {
        [amount] => {
            let amount: i64 = amount
                .parse()
                .map_err(|_| format!("`scroll {amount}` is not a pixel amount"))?;
            Ok(format!("window.scrollBy(0, {amount})"))
        }
        [selector, amount] => {
            let amount: i64 = amount
                .parse()
                .map_err(|_| format!("`scroll {selector} {amount}` is not a pixel amount"))?;
            let selector = js_string(selector);
            Ok(format!(
                "await __dxAction({selector}, el => el.scrollBy(0, {amount}))"
            ))
        }
        _ => Err(format!(
            "`scroll {rest}` — write `scroll <pixels>` or `scroll <selector> <pixels>`"
        )),
    }
}

/// The one non-empty argument a verb needs, or a sentence naming what was missing.
fn require_arg<'a>(verb: &str, rest: &'a str) -> Result<&'a str, String> {
    if rest.is_empty() {
        Err(format!("`{verb}` needs a CSS selector after it"))
    } else {
        Ok(rest)
    }
}

/// `<selector> "quoted text"` — the selector is everything up to the opening quote.
fn split_selector_and_quoted(rest: &str, verb: &str) -> Result<(String, String), String> {
    let Some(quote_at) = rest.find(['"', '\'']) else {
        return Err(format!(
            "`{verb}` needs a selector and quoted text: `{verb} #field \"hello\"`"
        ));
    };
    let selector = rest[..quote_at].trim();
    if selector.is_empty() {
        return Err(format!(
            "`{verb}` needs a CSS selector before the quoted text"
        ));
    }
    let quote = rest.as_bytes()[quote_at] as char;
    let body = &rest[quote_at + 1..];
    let Some(close_at) = find_unescaped(body, quote) else {
        return Err(format!("`{verb} {rest}` — the quoted text is never closed"));
    };
    let text = unescape(&body[..close_at]);
    Ok((selector.to_string(), text))
}

/// Byte offset of the first unescaped `quote` in `text`.
fn find_unescaped(text: &str, quote: char) -> Option<usize> {
    let mut escaped = false;
    for (index, ch) in text.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
        } else if ch == quote {
            return Some(index);
        }
    }
    None
}

/// `\"` and `\\` inside a quoted argument become the literal character.
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if let Some(next) = chars.next() {
                out.push(next);
                continue;
            }
        }
        out.push(ch);
    }
    out
}

/// `500ms`, `2s`, or a bare number of milliseconds.
fn parse_duration(text: &str) -> Result<u64, String> {
    let (digits, scale) = match text.strip_suffix("ms") {
        Some(digits) => (digits, 1),
        None => match text.strip_suffix('s') {
            Some(digits) => (digits, 1000),
            None => (text, 1),
        },
    };
    digits
        .parse::<u64>()
        .map(|value| value * scale)
        .map_err(|_| format!("`wait {text}` is not a duration — write `500ms` or `2s`"))
}

/// A JavaScript double-quoted string literal for `value` — an author-controlled selector
/// or form value, never interpolated any other way into the compiled script.
fn js_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Split `script` on top-level `;` — one not inside a `'`/`"` quote and not inside
/// `(`/`[`/`{` nesting, so `eval`'s JavaScript (and a quoted argument's own punctuation)
/// can carry a real semicolon without ending the statement early.
fn split_statements(script: &str) -> Vec<String> {
    let mut statements = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut depth: i32 = 0;
    let mut chars = script.chars();
    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            current.push(c);
            if c == '\\' {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            } else if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '\'' | '"' => {
                quote = Some(c);
                current.push(c);
            }
            '(' | '[' | '{' => {
                depth += 1;
                current.push(c);
            }
            ')' | ']' | '}' => {
                depth -= 1;
                current.push(c);
            }
            ';' if depth <= 0 => {
                statements.push(current.trim().to_string());
                current = String::new();
            }
            _ => current.push(c),
        }
    }
    let last = current.trim();
    if !last.is_empty() {
        statements.push(last.to_string());
    }
    statements.into_iter().filter(|s| !s.is_empty()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wait_compiles_to_an_awaited_timeout() {
        let js = compile("wait 500ms").expect("compile");
        assert!(js.contains("await new Promise"), "{js}");
        assert!(js.contains("500"), "{js}");
        assert!(compile("wait 2s").expect("compile").contains("2000"));
    }

    #[test]
    fn click_and_hover_target_a_css_selector() {
        let js = compile("click #login").expect("compile");
        assert!(js.contains("__dxAction(\"#login\""), "{js}");
        assert!(js.contains("el.click()"), "{js}");
        let js = compile("hover .menu").expect("compile");
        assert!(js.contains("__dxAction(\".menu\""), "{js}");
        assert!(js.contains("mouseenter"), "{js}");
    }

    #[test]
    fn type_sets_the_native_value_and_fires_input_and_change() {
        let js = compile(r#"type #email "a@example.com""#).expect("compile");
        assert!(js.contains("__dxAction(\"#email\""), "{js}");
        assert!(js.contains("\"a@example.com\""), "{js}");
        assert!(js.contains("input"), "{js}");
        assert!(js.contains("change"), "{js}");
    }

    #[test]
    fn type_refuses_without_a_selector_or_without_quoted_text() {
        assert!(compile("type \"only text\"").is_err());
        assert!(compile("type #field").is_err());
    }

    #[test]
    fn a_quoted_semicolon_does_not_split_the_statement() {
        let js = compile(r#"type #note "a; b"; click #save"#).expect("compile");
        assert!(js.contains("a; b"), "{js}");
        assert!(js.contains("__dxAction(\"#save\""), "{js}");
    }

    #[test]
    fn key_dispatches_on_the_active_element() {
        let js = compile("key Enter").expect("compile");
        assert!(js.contains("document.activeElement"), "{js}");
        assert!(js.contains("\"Enter\""), "{js}");
    }

    #[test]
    fn scroll_with_no_selector_moves_the_window() {
        let js = compile("scroll 200").expect("compile");
        assert!(js.contains("window.scrollBy(0, 200)"), "{js}");
    }

    #[test]
    fn scroll_with_a_selector_moves_that_element() {
        let js = compile("scroll #list 200").expect("compile");
        assert!(js.contains("__dxAction(\"#list\""), "{js}");
        assert!(js.contains("scrollBy(0, 200)"), "{js}");
    }

    #[test]
    fn eval_is_an_escape_hatch_and_keeps_its_own_semicolons_inside_braces() {
        let js = compile("eval { const x = 1; return x + 1; }").expect("compile");
        assert!(js.contains("const x = 1; return x + 1;"), "{js}");
    }

    #[test]
    fn statements_are_joined_in_order() {
        let js = compile("wait 10ms; click #a; hover #b").expect("compile");
        let wait_at = js.find("Promise").expect("wait");
        let click_at = js.find("#a").expect("click");
        let hover_at = js.find("#b").expect("hover");
        assert!(wait_at < click_at && click_at < hover_at, "{js}");
    }

    #[test]
    fn an_unknown_verb_names_the_vocabulary() {
        let error = compile("dance #floor").expect_err("refused");
        assert!(
            error.contains("wait, click, hover, type, key, scroll, or eval"),
            "{error}"
        );
    }

    #[test]
    fn an_empty_script_is_refused() {
        assert!(compile("").is_err());
        assert!(compile("   ").is_err());
    }

    #[test]
    fn a_selector_containing_quotes_stays_inside_one_js_string_literal() {
        // A CSS attribute selector legitimately contains an escaped quote
        // (`[data-id="x\"y"]`) — the compiled JS string literal must re-escape it rather
        // than let it break out of the literal early.
        let js = compile(r#"click [data-id="x\"y"]"#).expect("compile");
        assert_eq!(
            js,
            r#"await __dxAction("[data-id=\"x\\\"y\"]", el => { el.dispatchEvent(new MouseEvent('mousedown', {bubbles:true,cancelable:true})); el.dispatchEvent(new MouseEvent('mouseup', {bubbles:true,cancelable:true})); el.click(); })"#
        );
    }

    #[test]
    fn js_string_escapes_quotes_backslashes_and_line_separators() {
        assert_eq!(js_string("plain"), "\"plain\"");
        assert_eq!(js_string("a\"b"), "\"a\\\"b\"");
        assert_eq!(js_string("a\\b"), "\"a\\\\b\"");
        assert_eq!(js_string("a\u{2028}b"), "\"a\\u2028b\"");
    }
}
