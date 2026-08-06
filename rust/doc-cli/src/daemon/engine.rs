//! The calls the daemon answers, and the only things it will do.
//!
//! # Why this is an allowlist and not a lookup
//! The daemon listens on a port, and any program on the machine — including a page in the
//! reader's browser — can reach it. So the surface is stated here, in one place, as a fixed
//! set of named calls over data the caller itself supplied. There is no call that reads a
//! file, writes one, or runs anything: the daemon is a pure function from bytes it was given
//! to a rendering of them, and `dx run` — the only thing in `dx` that executes a document —
//! is not reachable from here.
//!
//! That bounds what a caller can *make the daemon do*. It does not, on its own, bound what a
//! caller can *read*: the packs held here were uploaded by the reader's browser and can come
//! from private repositories, and a pack key is guessable
//! (`…/<org>/<repo>/raw/main/.doc/repo.dxcp`). "It only answers about bytes the caller sent"
//! was the wrong reassurance — it answers about bytes *any* caller sent. What actually keeps
//! another program's page out is the access check in
//! [`refuse_unless_local`](super::refuse_unless_local), and that is where to look before
//! adding a call here.
//!
//! # Why these calls
//! They are exactly the engine surface `editor/github/resolve.js` depends on, and they match
//! `doc-wasm`'s exports name for name and result for result. The browser shim can therefore
//! send the same call to whichever engine is reachable — the daemon or its bundled wasm —
//! and `resolve.js` cannot tell the difference. One engine, two doors.

use serde_json::Value;

use super::packs::Packs;

/// What a call produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The call's result, shaped exactly as `doc-wasm` returns it.
    Value(Value),
    /// The call named a pack the daemon is not holding; the client should upload it and
    /// retry. This is a normal step, not a failure: the daemon never fetches anything
    /// itself, so the first mention of any pack necessarily misses.
    NeedPack(String),
}

/// The names [`call`] accepts.
///
/// States the surface in one place, names it back to a caller who got it wrong, and gives
/// [`every_advertised_call_is_answered`](tests::every_advertised_call_is_answered) something
/// to check the match arms against.
pub const CALLS: [&str; 7] = [
    "pack_document",
    "pack_paths",
    "sha256_hex",
    "render_html",
    "references",
    "file_references",
    "stylesheet",
];

/// Run the call named `name` with `args` against the packs in `packs`.
///
/// # Errors
/// When `name` is not in [`CALLS`], when `args` are not the shapes that call takes, or when
/// the call itself fails — a path the pack does not hold, for instance. The message is the
/// one the reader is eventually shown, so it says what happened rather than that something
/// did.
pub fn call(packs: &mut Packs, name: &str, args: &[Value]) -> Result<Outcome, String> {
    match name {
        "pack_document" => {
            let key = pack_key(args, 0)?;
            let path = text(args, 1)?;
            let Some(pack) = packs.get(key) else {
                return Ok(Outcome::NeedPack(key.to_string()));
            };
            pack.source(path)
                .map(|source| Outcome::Value(Value::String(source)))
                .ok_or_else(|| format!("the pack holds no document at {path}"))
        }
        "pack_paths" => {
            let key = pack_key(args, 0)?;
            let Some(pack) = packs.get(key) else {
                return Ok(Outcome::NeedPack(key.to_string()));
            };
            let paths: Vec<&str> = pack
                .entries
                .iter()
                .map(|entry| entry.path.as_str())
                .collect();
            // A JSON *string*, as `doc-wasm` returns, because the caller parses it.
            let encoded = serde_json::to_string(&paths)
                .map_err(|error| format!("listing the pack's documents: {error}"))?;
            Ok(Outcome::Value(Value::String(encoded)))
        }
        "sha256_hex" => Ok(Outcome::Value(Value::String(doc_core::digest::sha256_hex(
            bytes(args, 0)?,
        )))),
        "render_html" => {
            let source = text(args, 0)?;
            let theme = text(args, 1)?;
            let fragment = flag(args, 2)?;
            // Optional fourth argument: resources answering the document's references,
            // in the same JSON shape `doc-wasm`'s `render_html` takes — the two doors
            // must stay indistinguishable to the caller.
            let resources = args.get(3).and_then(Value::as_str);
            let mut document = doc_core::format::parse(source);
            doc_core::resolve::hydrate(&mut document, &provided_from(resources));
            Ok(Outcome::Value(Value::String(doc_core::render::html(
                &document,
                &doc_core::render::HtmlOptions {
                    theme: doc_core::render::Theme::parse(theme),
                    fragment,
                    ..doc_core::render::HtmlOptions::default()
                },
            ))))
        }
        "references" => {
            let source = text(args, 0)?;
            let document = doc_core::format::parse(source);
            let rows: Vec<Value> = doc_core::resolve::references(&document)
                .iter()
                .map(|reference| match reference {
                    doc_core::resolve::Reference::File(path) => {
                        serde_json::json!({"kind": "file", "path": path})
                    }
                    doc_core::resolve::Reference::Document(path) => {
                        serde_json::json!({"kind": "document", "path": path})
                    }
                })
                .collect();
            // A JSON *string*, as `doc-wasm` returns, because the caller parses it.
            let encoded = serde_json::to_string(&rows)
                .map_err(|error| format!("listing the document's references: {error}"))?;
            Ok(Outcome::Value(Value::String(encoded)))
        }
        "file_references" => {
            // The second half of the prefetch protocol: what a fetched file (a `::view`
            // page) names in turn. Still a pure function of bytes the caller supplied.
            let path = text(args, 0)?;
            let content = text(args, 1)?;
            let rows: Vec<Value> = doc_core::resolve::file_references(path, content)
                .iter()
                .map(|reference| match reference {
                    doc_core::resolve::Reference::File(path) => {
                        serde_json::json!({"kind": "file", "path": path})
                    }
                    doc_core::resolve::Reference::Document(path) => {
                        serde_json::json!({"kind": "document", "path": path})
                    }
                })
                .collect();
            // A JSON *string*, as `doc-wasm` returns, because the caller parses it.
            let encoded = serde_json::to_string(&rows)
                .map_err(|error| format!("listing the file's references: {error}"))?;
            Ok(Outcome::Value(Value::String(encoded)))
        }
        "stylesheet" => Ok(Outcome::Value(Value::String(
            doc_core::render::stylesheet().to_string(),
        ))),
        _ => Err(format!(
            "`{name}` is not a call the dx engine answers. It answers: {}",
            CALLS.join(", ")
        )),
    }
}

/// Build the resolver `render_html` hydrates against from its resources JSON, exactly
/// as `doc-wasm`'s `provided_from` reads the same shape. Malformed JSON resolves
/// nothing, so the mistake shows on the page as sentences rather than vanishing.
fn provided_from(resources: Option<&str>) -> doc_core::resolve::Provided {
    let mut provided = doc_core::resolve::Provided::new();
    let Some(raw) = resources else {
        return provided;
    };
    let Ok(value) = serde_json::from_str::<Value>(raw) else {
        return provided;
    };
    if let Some(entries) = value.get("files").and_then(Value::as_object) {
        for (path, text) in entries {
            if let Some(text) = text.as_str() {
                provided.add_file(path, text);
            }
        }
    }
    if let Some(entries) = value.get("documents").and_then(Value::as_object) {
        for (path, source) in entries {
            if let Some(source) = source.as_str() {
                provided.add_document(path, source);
            }
        }
    }
    provided
}

/// The argument at `index` as a string.
fn text(args: &[Value], index: usize) -> Result<&str, String> {
    args.get(index)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("argument {index} must be a string"))
}

/// The argument at `index` as a boolean.
fn flag(args: &[Value], index: usize) -> Result<bool, String> {
    args.get(index)
        .and_then(Value::as_bool)
        .ok_or_else(|| format!("argument {index} must be true or false"))
}

/// The argument at `index` as a pack key, written `{"pack": "<key>"}`.
///
/// Bytes are never sent inline with a call — that is the whole point of the daemon. A call
/// names a pack, and the pack was uploaded once.
fn pack_key(args: &[Value], index: usize) -> Result<&str, String> {
    args.get(index)
        .and_then(|argument| argument.get("pack"))
        .and_then(Value::as_str)
        .ok_or_else(|| format!("argument {index} must be {{\"pack\": \"<key>\"}}"))
}

/// The argument at `index` as bytes, written `{"text": "…"}` and taken as UTF-8.
///
/// The one call that hashes bytes hashes a document's source, which is text, so this is the
/// only encoding needed and JSON carries it without a second layer.
fn bytes(args: &[Value], index: usize) -> Result<&[u8], String> {
    args.get(index)
        .and_then(|argument| argument.get("text"))
        .and_then(Value::as_str)
        .map(str::as_bytes)
        .ok_or_else(|| format!("argument {index} must be {{\"text\": \"…\"}}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use doc_core::chunk::Pack as CorePack;

    /// A real encoded pack holding one document at `path`.
    fn pack_bytes(path: &str, source: &str) -> Vec<u8> {
        let document = doc_core::format::parse(source);
        let pack = CorePack::build([(path, &document)]);
        doc_core::chunk::encode_pack(&pack)
    }

    /// A cache holding one pack under `repo@main`.
    fn loaded() -> Packs {
        let mut packs = Packs::new();
        packs
            .store(
                "repo@main",
                &pack_bytes("notes.dx", "::heading level=1\nHello\n"),
            )
            .expect("a real pack decodes");
        packs
    }

    /// `{"pack": key}`.
    fn pack(key: &str) -> Value {
        serde_json::json!({ "pack": key })
    }

    #[test]
    fn a_held_pack_answers_with_the_document_source() {
        let outcome = call(
            &mut loaded(),
            "pack_document",
            &[pack("repo@main"), Value::String("notes.dx".into())],
        )
        .expect("the call succeeds");
        let Outcome::Value(Value::String(source)) = outcome else {
            panic!("expected a source string, got {outcome:?}");
        };
        assert!(source.contains("Hello"), "{source}");
    }

    #[test]
    fn a_pack_the_daemon_does_not_hold_asks_for_it_rather_than_failing() {
        let outcome = call(
            &mut loaded(),
            "pack_document",
            &[pack("repo@feature"), Value::String("notes.dx".into())],
        )
        .expect("a miss is not an error");
        assert_eq!(outcome, Outcome::NeedPack("repo@feature".to_string()));
    }

    #[test]
    fn a_path_the_pack_does_not_hold_says_so() {
        let error = call(
            &mut loaded(),
            "pack_document",
            &[pack("repo@main"), Value::String("absent.dx".into())],
        )
        .expect_err("a missing path is an error");
        assert!(error.contains("absent.dx"), "{error}");
    }

    #[test]
    fn pack_paths_returns_a_json_string_the_caller_parses() {
        let outcome =
            call(&mut loaded(), "pack_paths", &[pack("repo@main")]).expect("the call succeeds");
        let Outcome::Value(Value::String(encoded)) = outcome else {
            panic!("expected a JSON string, got {outcome:?}");
        };
        assert_eq!(encoded, r#"["notes.dx"]"#);
    }

    #[test]
    fn sha256_matches_the_reference_the_pointer_carries() {
        let outcome = call(
            &mut Packs::new(),
            "sha256_hex",
            &[serde_json::json!({ "text": "abc" })],
        )
        .expect("the call succeeds");
        assert_eq!(
            outcome,
            Outcome::Value(Value::String(doc_core::digest::sha256_hex(b"abc")))
        );
    }

    #[test]
    fn render_html_is_the_same_rendering_the_cli_produces() {
        let source = "::heading level=1\nHello\n";
        let outcome = call(
            &mut Packs::new(),
            "render_html",
            &[
                Value::String(source.into()),
                Value::String("dark".into()),
                Value::Bool(true),
                Value::Bool(false),
            ],
        )
        .expect("the call succeeds");
        let expected = doc_core::render::html(
            &doc_core::format::parse(source),
            &doc_core::render::HtmlOptions {
                theme: doc_core::render::Theme::parse("dark"),
                fragment: true,
                ..doc_core::render::HtmlOptions::default()
            },
        );
        assert_eq!(outcome, Outcome::Value(Value::String(expected)));
    }

    #[test]
    fn an_unknown_call_is_refused_by_name() {
        let error = call(&mut Packs::new(), "read_file", &[]).expect_err("unknown calls fail");
        assert!(error.contains("read_file"), "{error}");
    }

    #[test]
    fn every_advertised_call_is_answered() {
        for name in CALLS {
            let outcome = call(&mut Packs::new(), name, &[]);
            let refused_as_unknown =
                matches!(&outcome, Err(message) if message.contains("is not a call"));
            assert!(
                !refused_as_unknown,
                "`{name}` is advertised but not answered"
            );
        }
    }

    #[test]
    fn a_wrongly_shaped_argument_says_what_it_should_have_been() {
        let error = call(
            &mut loaded(),
            "pack_document",
            &[
                Value::String("repo@main".into()),
                Value::String("notes.dx".into()),
            ],
        )
        .expect_err("a bare string is not a pack reference");
        assert!(error.contains("\"pack\""), "{error}");
    }
}
