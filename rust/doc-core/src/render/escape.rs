//! HTML escaping and markup sanitizing shared by every renderer surface.
//!
//! Two different jobs live here and must not be confused:
//!
//! - [`escape_html`] turns arbitrary text into inert HTML text. Use it for anything the
//!   author wrote as *prose* — headings, paragraphs, list items, code bodies.
//! - [`sanitize_markup`] keeps author-supplied *markup* (`::html`, `::svg`) working while
//!   removing everything that could execute.
//!
//! # Why this is an allow-list
//! It used to be a deny-list — strip `<script>`, strip `on*=`, rewrite `javascript:` — on the
//! stated assumption that markup "is already local to the reader's own machine". That
//! assumption died when documents began rendering on github.com: the markup now comes from
//! whoever wrote the repository being read, and it is inserted into a page served by
//! github.com. A shadow root does not help — it is the same document and the same origin — so
//! one bypass is arbitrary JavaScript with the reader's github.com session.
//!
//! Deny-lists lose that game. Every one of these got through the old rules, and each is a
//! real, working payload rather than a theoretical one:
//!
//! | Payload | Why it survived |
//! |---|---|
//! | `<img src=x/onerror=alert(1)>` | `/` separates attributes in HTML; the rule needed whitespace |
//! | `<iframe srcdoc="&lt;script&gt;…">` | the script is entity-encoded, and `srcdoc` is same-origin with the embedder |
//! | `<a href="&#106;avascript:…">` | the scheme is entity-encoded and decoded after sanitizing |
//! | `<a href="java&#9;script:…">` | browsers strip tabs out of a URL scheme |
//! | `<base href="https://evil/">`, `<meta http-equiv=refresh>`, `<form action>` | never considered |
//!
//! So the rule is inverted: nothing survives unless it is *named* here. An element not on
//! [`ALLOWED_ELEMENTS`] is dropped, an attribute not on [`ALLOWED_ATTRIBUTES`] is dropped, and
//! a URL whose scheme is not on [`ALLOWED_SCHEMES`] is dropped — after decoding the entities
//! and control characters a browser would decode before acting on it. Adding a capability is
//! then a deliberate edit to a list, reviewed for what it lets through, rather than a payload
//! nobody thought of.

/// Elements author markup may use: presentation, tables, and static SVG.
///
/// Anything absent is unwrapped — its tags are dropped and its text kept — so removing an
/// element never silently deletes a reader's prose. The exceptions are in
/// [`DROPPED_WITH_CONTENT`], whose *content* is not text meant for a reader.
const ALLOWED_ELEMENTS: &[&str] = &[
    // Flow and phrasing.
    "a",
    "abbr",
    "b",
    "bdi",
    "bdo",
    "blockquote",
    "br",
    "caption",
    "cite",
    "code",
    "col",
    "colgroup",
    "dd",
    "del",
    "details",
    "dfn",
    "div",
    "dl",
    "dt",
    "em",
    "figcaption",
    "figure",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "hr",
    "i",
    "img",
    "ins",
    "kbd",
    "li",
    "main",
    "mark",
    "ol",
    "p",
    "pre",
    "q",
    "rp",
    "rt",
    "ruby",
    "s",
    "samp",
    "section",
    "small",
    "span",
    "strong",
    "sub",
    "summary",
    "sup",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "time",
    "tr",
    "u",
    "ul",
    "var",
    "wbr",
    // Static SVG. `foreignObject` is deliberately absent: it is an escape hatch back into
    // HTML, and `script`/`set`/`animate` are absent because SVG can script and animate an
    // attribute into a URL after the markup has been inspected.
    "circle",
    "clippath",
    "defs",
    "desc",
    "ellipse",
    "g",
    "line",
    "lineargradient",
    "marker",
    "mask",
    "path",
    "pattern",
    "polygon",
    "polyline",
    "radialgradient",
    "rect",
    "stop",
    "svg",
    "symbol",
    "text",
    "textpath",
    "title",
    "tspan",
    "use",
];

/// Elements dropped together with everything inside them.
///
/// Unwrapping these would paste program text into the page as prose. `iframe` is here rather
/// than merely unallowed because `srcdoc` carries a whole same-origin document in an
/// attribute, and `style` because CSS is its own execution surface.
const DROPPED_WITH_CONTENT: &[&str] = &[
    "script",
    "style",
    "iframe",
    "object",
    "embed",
    "template",
    "noscript",
    "form",
    "input",
    "button",
    "select",
    "option",
    "textarea",
    "base",
    "meta",
    "link",
    "head",
    "frame",
    "frameset",
    "applet",
    "math",
    "foreignobject",
    "animate",
    "animatetransform",
    "animatemotion",
    "set",
    "handler",
    "listener",
];

/// Attributes any allowed element may carry.
///
/// Presentation and structure only. Every `on*` handler is absent by construction, since no
/// name here begins with `on`, and [`is_allowed_attribute`] refuses that prefix outright so a
/// future addition cannot introduce one by accident.
const ALLOWED_ATTRIBUTES: &[&str] = &[
    // Common.
    "alt",
    "class",
    "colspan",
    "datetime",
    "dir",
    "headers",
    "height",
    "id",
    "lang",
    "role",
    "rowspan",
    "scope",
    "span",
    "start",
    "style",
    "title",
    "type",
    "value",
    "width",
    // SVG geometry and paint.
    "cx",
    "cy",
    "d",
    "dx",
    "dy",
    "fill",
    "fill-opacity",
    "fill-rule",
    "font-family",
    "font-size",
    "font-style",
    "font-weight",
    "gradientunits",
    "letter-spacing",
    "marker-end",
    "marker-mid",
    "marker-start",
    "offset",
    "opacity",
    "orient",
    "patternunits",
    "points",
    "preserveaspectratio",
    "r",
    "refx",
    "refy",
    "rx",
    "ry",
    "stop-color",
    "stop-opacity",
    "stroke",
    "stroke-dasharray",
    "stroke-dashoffset",
    "stroke-linecap",
    "stroke-linejoin",
    "stroke-opacity",
    "stroke-width",
    "text-anchor",
    "transform",
    "vector-effect",
    "viewbox",
    "x",
    "x1",
    "x2",
    "y",
    "y1",
    "y2",
    // Namespace declarations. Inline SVG in HTML does not need them — the parser assigns the
    // namespace — but the same markup is also written to standalone `.svg` and screenshotted,
    // where dropping them turns a drawing into nothing.
    "xmlns",
    "xmlns:xlink",
];

/// Elements that never have content or a closing tag.
///
/// Needed because dropping `<meta>` "and its content" would otherwise discard everything
/// after it — there is no `</meta>` to stop at.
const VOID_ELEMENTS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

/// Attributes whose value is a URL, and so must survive [`is_safe_url`].
const URL_ATTRIBUTES: &[&str] = &["href", "src", "xlink:href"];

/// URL schemes a document may point at.
///
/// `data:` is absent apart from the image forms in [`is_safe_url`]: `data:text/html` is a
/// document, and `data:image/svg+xml` is a document that can script.
const ALLOWED_SCHEMES: &[&str] = &["http", "https", "mailto"];

/// Escape the five HTML-significant characters so `value` renders as literal text.
#[must_use]
pub fn escape_html(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Make a document's own CSS safe to put in a `<style>` element.
///
/// A `::style` block dresses its document on every surface, so this runs on every read and
/// has to hold the same line [`is_safe_style`] holds for an inline `style="…"`: CSS may lay a
/// document out, and may not fetch or execute. Three things are neutralized, and the rest of
/// the author's stylesheet passes through untouched:
///
/// - `</style`, the one sequence that closes the element and escapes into markup.
/// - Remote `url(…)`. A `background: url(https://…)` fires on render, which turns reading a
///   document into telling its author you read it — and on github.com, telling a stranger
///   your IP. A `data:` image and a relative path are kept, because those are how a
///   self-contained document carries its own artwork.
/// - `@import`, `expression(`, `behavior:`, and `-moz-binding`: a fetch and three ways to run
///   script from a declaration.
///
/// Neutralizing rather than deleting is deliberate — a mangled property is a rule the browser
/// ignores, so the surrounding stylesheet still parses and the author sees one thing not work
/// instead of their whole dress falling off at the first bad line.
#[must_use]
pub fn escape_style(value: &str) -> String {
    let mut out = blank_unsafe_urls(value);
    for banned in ["@import", "expression(", "behavior:", "-moz-binding"] {
        out = replace_case_insensitive(&out, banned, &format!("_dx-blocked-{}", &banned[..2]));
    }
    replace_case_insensitive(&out, "</style", "<\\/style")
}

/// Whether a CSS `url(…)` target can be resolved without reaching off the document.
///
/// Stricter than [`is_safe_url`], and deliberately so: that one judges a *link*, which a
/// reader chooses to follow, and `https:` belongs there. This judges a *fetch*, which fires
/// the instant the page paints and which the reader never agreed to. Only an inert `data:`
/// image qualifies — a document that carries its own artwork — and nothing that names a host.
/// The raster media types a page may carry inline as `data:` URIs — the one allow-list
/// shared by author markup (`is_safe_url`, `is_fetchless_url`) and hydration
/// ([`crate::resolve`] embedding `::image src=` files). SVG is deliberately absent: it
/// is a document that can script, not a picture.
pub(crate) const RASTER_IMAGE_MEDIA_TYPES: [&str; 4] =
    ["image/png", "image/jpeg", "image/gif", "image/webp"];

fn is_fetchless_url(target: &str) -> bool {
    let flattened: String = decode_entities(target)
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace() && !ch.is_control())
        .collect::<String>()
        .to_ascii_lowercase();
    if flattened.is_empty() || flattened.starts_with("//") {
        return false;
    }
    match flattened.strip_prefix("data:") {
        Some(rest) => RASTER_IMAGE_MEDIA_TYPES
            .iter()
            .any(|kind| rest.starts_with(kind)),
        // No scheme at all is a relative path inside whatever already loaded the document.
        None => !flattened.contains(':'),
    }
}

/// Rewrite every `url(…)` whose target this engine will not fetch into an empty `url()`.
///
/// An empty target is inert: the browser resolves nothing and the declaration is simply
/// ignored, which is exactly what a dropped background should do.
fn blank_unsafe_urls(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(at) = rest.to_ascii_lowercase().find("url(") {
        out.push_str(&rest[..at + 4]);
        rest = &rest[at + 4..];
        let Some(close) = rest.find(')') else {
            // An unclosed `url(` never becomes a request; the remainder is ordinary text.
            break;
        };
        let target = rest[..close].trim().trim_matches(['"', '\''].as_slice());
        if is_fetchless_url(target) {
            out.push_str(&rest[..close]);
        }
        out.push(')');
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    out
}

/// Keep the presentation in author-supplied markup and drop everything that could execute.
///
/// Elements, attributes, and URL schemes are each checked against an allow-list; anything not
/// named is removed. An unknown element is *unwrapped* — its text survives — except for the
/// few in [`DROPPED_WITH_CONTENT`] whose bodies are not prose.
#[must_use]
pub fn sanitize_markup(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] != b'<' {
            let len = char_len_at(value, index);
            // Text between tags passes through exactly as written — nothing here is
            // re-escaped. The one rewrite is below: a `<` that does not open a real tag
            // becomes `&lt;`, so it cannot pair with a later `>` to form an element.
            out.push_str(&value[index..index + len]);
            index += len;
            continue;
        }

        // A `<` that does not begin a tag is literal text, and must not be left as a delimiter
        // that a browser could pair with a later `>` to form an element.
        let Some(tag) = read_tag(value, index) else {
            out.push_str("&lt;");
            index += 1;
            continue;
        };

        if tag.name.is_empty() {
            // A comment, doctype, or processing instruction. Dropped: a comment can hide an
            // unbalanced quote that changes how everything after it parses.
            index = tag.end;
            continue;
        }

        if DROPPED_WITH_CONTENT.contains(&tag.name.as_str()) {
            // A void element has no content and no closing tag; hunting for one would
            // swallow the rest of the document.
            index =
                if tag.is_closing || tag.self_closing || VOID_ELEMENTS.contains(&tag.name.as_str())
                {
                    tag.end
                } else {
                    skip_element_content(value, tag.end, &tag.name)
                };
            continue;
        }

        if !ALLOWED_ELEMENTS.contains(&tag.name.as_str()) {
            index = tag.end; // Unwrap: drop the tag, keep what it wrapped.
            continue;
        }

        out.push_str(&render_tag(&tag));
        index = tag.end;
    }

    out
}

/// Extract the first `<svg>…</svg>` element from `value`, or the empty string when the
/// text contains no SVG root. Used so an `::svg` block cannot smuggle in sibling markup.
#[must_use]
pub fn extract_svg(value: &str) -> String {
    let lower = value.to_ascii_lowercase();
    let start = match lower.find("<svg") {
        Some(index) => index,
        None => return String::new(),
    };
    match lower[start..].find("</svg>") {
        Some(offset) => value[start..start + offset + "</svg>".len()].to_string(),
        None => String::new(),
    }
}

/// One parsed tag.
struct Tag {
    /// The element name, lowercased for matching; empty for a comment or doctype.
    name: String,
    /// The element name exactly as written, which is what is emitted.
    ///
    /// SVG names are case-sensitive — `linearGradient` and `viewBox` are not the same as their
    /// lowercase spellings outside an HTML parser — so matching lowercases and emitting does
    /// not.
    raw_name: String,
    /// Whether this is a closing tag.
    is_closing: bool,
    /// Whether the tag closed itself (`<br/>`).
    self_closing: bool,
    /// The attributes as `(lowercased name, name as written, value)`.
    attributes: Vec<(String, String, String)>,
    /// Byte index just past the tag's `>`.
    end: usize,
}

/// Parse the tag starting at `start`, or `None` when `<` is not the start of one.
///
/// Quoted attribute values are honored, so a `>` inside one cannot end the tag early — a
/// parser that got that wrong would let markup after the quote escape inspection entirely.
fn read_tag(value: &str, start: usize) -> Option<Tag> {
    let bytes = value.as_bytes();
    let mut index = start + 1;

    // Comments, doctypes, and processing instructions: no name, consumed to their end.
    if bytes.get(index) == Some(&b'!') || bytes.get(index) == Some(&b'?') {
        let end = if value[index..].starts_with("!--") {
            value[index..]
                .find("-->")
                .map_or(bytes.len(), |at| index + at + 3)
        } else {
            value[index..]
                .find('>')
                .map_or(bytes.len(), |at| index + at + 1)
        };
        return Some(Tag {
            name: String::new(),
            raw_name: String::new(),
            is_closing: false,
            self_closing: false,
            attributes: Vec::new(),
            end,
        });
    }

    let is_closing = bytes.get(index) == Some(&b'/');
    if is_closing {
        index += 1;
    }

    let name_start = index;
    while index < bytes.len() && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b':') {
        index += 1;
    }
    if index == name_start {
        return None; // `<` followed by something that cannot start a name: literal text.
    }
    let raw_name = value[name_start..index].to_string();
    let name = raw_name.to_ascii_lowercase();

    let mut attributes = Vec::new();
    let mut self_closing = false;
    loop {
        // Attribute separators: whitespace *and* `/`, which is what `src=x/onerror=…` abused.
        while index < bytes.len() && (bytes[index].is_ascii_whitespace() || bytes[index] == b'/') {
            if bytes[index] == b'/' {
                self_closing = true;
            }
            index += 1;
        }
        if index >= bytes.len() {
            return Some(Tag {
                name,
                raw_name,
                is_closing,
                self_closing,
                attributes,
                end: bytes.len(),
            });
        }
        if bytes[index] == b'>' {
            return Some(Tag {
                name,
                raw_name,
                is_closing,
                self_closing,
                attributes,
                end: index + 1,
            });
        }

        let attribute_start = index;
        while index < bytes.len()
            && !bytes[index].is_ascii_whitespace()
            && bytes[index] != b'='
            && bytes[index] != b'>'
            && bytes[index] != b'/'
        {
            index += 1;
        }
        if index == attribute_start {
            index += 1; // Nothing consumed; step past the byte so the loop terminates.
            continue;
        }
        let raw_attribute = value[attribute_start..index].to_string();
        let attribute = raw_attribute.to_ascii_lowercase();

        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        let mut attribute_value = String::new();
        if bytes.get(index) == Some(&b'=') {
            index += 1;
            while index < bytes.len() && bytes[index].is_ascii_whitespace() {
                index += 1;
            }
            let (text, next) = read_attribute_value(value, index);
            attribute_value = text;
            index = next;
        }
        attributes.push((attribute, raw_attribute, attribute_value));
    }
}

/// Read an attribute value at `index`, returning it and the index just past it.
fn read_attribute_value(value: &str, index: usize) -> (String, usize) {
    let bytes = value.as_bytes();
    if index >= bytes.len() {
        return (String::new(), index);
    }
    let quote = bytes[index];
    if quote == b'"' || quote == b'\'' {
        let start = index + 1;
        let mut end = start;
        while end < bytes.len() && bytes[end] != quote {
            end += 1;
        }
        return (
            value[start..end.min(bytes.len())].to_string(),
            (end + 1).min(bytes.len()),
        );
    }
    let start = index;
    let mut end = index;
    while end < bytes.len()
        && !bytes[end].is_ascii_whitespace()
        && bytes[end] != b'>'
        && bytes[end] != b'/'
    {
        end += 1;
    }
    (value[start..end].to_string(), end)
}

/// Skip everything up to and including the matching close tag for `name`.
fn skip_element_content(value: &str, from: usize, name: &str) -> usize {
    let close = format!("</{name}");
    let lower = value.to_ascii_lowercase();
    match lower[from..].find(&close) {
        Some(at) => {
            let tag_start = from + at;
            lower[tag_start..]
                .find('>')
                .map_or(value.len(), |end| tag_start + end + 1)
        }
        // Unterminated: everything after it belonged to the dropped element.
        None => value.len(),
    }
}

/// Re-emit an allowed tag with only its allowed attributes.
///
/// The tag is rebuilt rather than copied, so anything the parser did not understand cannot
/// ride along inside it, and every value is re-quoted and escaped.
fn render_tag(tag: &Tag) -> String {
    if tag.is_closing {
        return format!("</{}>", tag.raw_name);
    }
    let mut out = format!("<{}", tag.raw_name);
    for (name, raw_name, value) in &tag.attributes {
        if !is_allowed_attribute(name) {
            continue;
        }
        if URL_ATTRIBUTES.contains(&name.as_str()) && !is_safe_url(value) {
            continue;
        }
        if name == "style" && !is_safe_style(value) {
            continue;
        }
        out.push_str(&format!(" {}=\"{}\"", raw_name, escape_html(value)));
    }
    if tag.self_closing {
        out.push('/');
    }
    out.push('>');
    out
}

/// Whether `name` may be kept.
fn is_allowed_attribute(name: &str) -> bool {
    // Refused by prefix as well as by absence: an `on*` name must never be reachable, even if
    // someone later adds one to the list by mistake.
    if name.starts_with("on") {
        return false;
    }
    // The whole `aria-*` family is allowed by prefix. Every one of them is inert — they
    // annotate meaning for assistive technology and cannot name a URL or run anything — and
    // enumerating them would guarantee that the one a document needs is the one missing.
    // Dropping `aria-label` to be safe is not safe; it is a document a screen reader can no
    // longer describe.
    if name.starts_with("aria-") {
        return true;
    }
    ALLOWED_ATTRIBUTES.contains(&name) || URL_ATTRIBUTES.contains(&name)
}

/// Whether a URL attribute value is safe to keep.
///
/// The value is normalized the way a browser normalizes one before acting on it — HTML
/// entities decoded, ASCII whitespace and control characters removed — because
/// `&#106;avascript:` and `java&#9;script:` are both `javascript:` by the time anything runs.
fn is_safe_url(value: &str) -> bool {
    let decoded: String = decode_entities(value)
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace() && !ch.is_control())
        .collect();
    let lower = decoded.to_ascii_lowercase();

    // A `data:` image is useful and inert — except SVG, which is a document that can script.
    if let Some(rest) = lower.strip_prefix("data:") {
        return RASTER_IMAGE_MEDIA_TYPES
            .iter()
            .any(|kind| rest.starts_with(kind));
    }

    match lower.split_once(':') {
        // No scheme: a relative or fragment URL, which cannot name a new context.
        None => true,
        Some((scheme, _)) => {
            // A colon later than the first path separator is part of the path, not a scheme.
            if scheme.contains('/') || scheme.contains('?') || scheme.contains('#') {
                return true;
            }
            ALLOWED_SCHEMES.contains(&scheme)
        }
    }
}

/// Whether an inline `style` value is safe to keep.
///
/// CSS is its own execution and exfiltration surface: `url(…)` fetches, `@import` pulls in a
/// stylesheet, and the legacy `expression()` runs script. None of them are needed to lay out a
/// document, so a value mentioning any of them is dropped whole rather than picked apart.
fn is_safe_style(value: &str) -> bool {
    let flattened: String = decode_entities(value)
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace() && !ch.is_control())
        .collect::<String>()
        .to_ascii_lowercase();
    ![
        "url(",
        "@import",
        "expression(",
        "javascript:",
        "behavior:",
        "-moz-binding",
    ]
    .iter()
    .any(|banned| flattened.contains(banned))
}

/// Decode the HTML entities a browser decodes inside an attribute value.
///
/// Numeric entities in both bases, plus the named ones that can appear in a URL. This exists
/// only so [`is_safe_url`] and [`is_safe_style`] judge what a browser will actually see.
///
/// One deliberate gap: a browser also decodes a numeric entity **without** its trailing `;`
/// (`&#106avascript:` is `javascript:` to an HTML parser), and this decoder does not — the
/// unterminated form passes through undecoded, so [`is_safe_url`] never sees the scheme it
/// hides. That is safe only because [`render_tag`] re-escapes every value it emits with
/// [`escape_html`], turning the `&` into `&amp;` so the browser reads literal text rather
/// than an entity. If that re-escaping ever changes, this gap becomes an XSS door;
/// `an_unterminated_numeric_entity_cannot_smuggle_a_scheme` pins the dependency.
fn decode_entities(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;

    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let end = after.find(';').unwrap_or(0);
        let body = &after[..end];

        let decoded = if let Some(digits) = body.strip_prefix("#x").or(body.strip_prefix("#X")) {
            u32::from_str_radix(digits, 16)
                .ok()
                .and_then(char::from_u32)
        } else if let Some(digits) = body.strip_prefix('#') {
            digits.parse::<u32>().ok().and_then(char::from_u32)
        } else {
            match body {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "colon" => Some(':'),
                "tab" => Some('\t'),
                "newline" => Some('\n'),
                _ => None,
            }
        };

        match decoded {
            Some(ch) => {
                out.push(ch);
                rest = &after[end + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Case-insensitive literal replacement, preserving all other bytes.
fn replace_case_insensitive(value: &str, needle: &str, replacement: &str) -> String {
    let lower_needle = needle.to_ascii_lowercase();
    let mut out = String::with_capacity(value.len());
    let mut rest = value;

    loop {
        let lower = rest.to_ascii_lowercase();
        match lower.find(&lower_needle) {
            Some(index) => {
                out.push_str(&rest[..index]);
                out.push_str(replacement);
                rest = &rest[index + needle.len()..];
            }
            None => {
                out.push_str(rest);
                return out;
            }
        }
    }
}

/// Byte length of the UTF-8 character starting at `index`.
fn char_len_at(value: &str, index: usize) -> usize {
    value[index..].chars().next().map_or(1, char::len_utf8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_every_significant_character() {
        assert_eq!(
            escape_html("<a href=\"x\">&'</a>"),
            "&lt;a href=&quot;x&quot;&gt;&amp;&#39;&lt;/a&gt;"
        );
    }

    #[test]
    fn escape_style_blanks_a_remote_url_fetch() {
        let clean = escape_style("div { background: url(https://evil.example/spy.png); }");
        assert!(!clean.contains("evil.example"), "{clean}");
        assert!(clean.contains("url()"), "{clean}");
    }

    #[test]
    fn escape_style_keeps_a_data_image_url() {
        let css = "div { background: url(data:image/png;base64,QQ==); }";
        assert_eq!(escape_style(css), css);
    }

    #[test]
    fn escape_style_keeps_a_relative_path_url() {
        let css = "div { background: url(../art/tile.png); }";
        assert_eq!(escape_style(css), css);
    }

    #[test]
    fn escape_style_neutralizes_import() {
        let clean = escape_style("@import url(https://evil.example/x.css);");
        assert!(!clean.contains("@import"), "{clean}");
    }

    #[test]
    fn escape_style_neutralizes_expression() {
        let clean = escape_style("width: expression(alert(1));");
        assert!(!clean.contains("expression("), "{clean}");
    }

    #[test]
    fn escape_style_neutralizes_behavior() {
        let clean = escape_style("behavior: url(evil.htc);");
        assert!(!clean.contains("behavior:"), "{clean}");
    }

    #[test]
    fn escape_style_neutralizes_moz_binding() {
        let clean = escape_style("-moz-binding: url(evil.xml#x);");
        assert!(!clean.contains("-moz-binding"), "{clean}");
    }

    #[test]
    fn escape_style_neutralizes_case_and_whitespace_variants() {
        for payload in [
            "@IMPORT url(x.css);",
            "EXPRESSION(alert(1));",
            "BEHAVIOR:url(x.htc);",
            "-MOZ-BINDING:url(x.xml);",
        ] {
            let clean = escape_style(payload).to_ascii_lowercase();
            assert!(
                !clean.contains("@import")
                    && !clean.contains("expression(")
                    && !clean.contains("behavior:")
                    && !clean.contains("-moz-binding"),
                "{clean}"
            );
        }
    }

    #[test]
    fn escape_style_escapes_a_closing_style_tag() {
        let clean = escape_style("p { color: red } </style><script>alert(1)</script>");
        assert!(!clean.contains("</style>"), "{clean}");
        assert!(clean.contains("<\\/style>"), "{clean}");
    }

    #[test]
    fn escape_style_leaves_ordinary_css_untouched() {
        let css = "p { color: red; font-size: 14px; }";
        assert_eq!(escape_style(css), css);
    }

    #[test]
    fn presentation_markup_survives() {
        assert_eq!(
            sanitize_markup("<p class=\"note\">hi <b>there</b></p>"),
            "<p class=\"note\">hi <b>there</b></p>"
        );
        assert_eq!(
            sanitize_markup("<table><tr><td colspan=\"2\">x</td></tr></table>"),
            "<table><tr><td colspan=\"2\">x</td></tr></table>"
        );
    }

    #[test]
    fn svg_drawings_survive() {
        let svg = "<svg viewBox=\"0 0 10 10\"><path d=\"M0 0 L10 10\" stroke=\"red\"/></svg>";
        let clean = sanitize_markup(svg);
        assert!(clean.contains("viewBox=\"0 0 10 10\""), "{clean}");
        assert!(clean.contains("d=\"M0 0 L10 10\""), "{clean}");
    }

    #[test]
    fn links_and_images_survive() {
        assert_eq!(
            sanitize_markup("<a href=\"https://example.com/x?a=1#b\">x</a>"),
            "<a href=\"https://example.com/x?a=1#b\">x</a>"
        );
        assert_eq!(
            sanitize_markup("<a href=\"../notes.dx\">x</a>"),
            "<a href=\"../notes.dx\">x</a>"
        );
    }

    /// Every one of these was a working payload against the deny-list this replaced.
    #[test]
    fn the_payloads_that_defeated_the_deny_list_do_not_survive() {
        let cases = [
            "<img src=x/onerror=alert(1)>",
            "<iframe srcdoc=\"&lt;script&gt;alert(1)&lt;/script&gt;\"></iframe>",
            "<a href=\"&#106;avascript:alert(1)\">x</a>",
            "<a href=\"java&#9;script:alert(1)\">x</a>",
            "<a href=\"&#x6a;avascript:alert(1)\">x</a>",
            "<svg><foreignObject><img src=x onerror=alert(1)></foreignObject></svg>",
            "<svg><a><set attributeName=\"href\" to=\"javascript:alert(1)\"/><text>x</text></a></svg>",
            "<base href=\"https://evil.example/\">",
            "<meta http-equiv=\"refresh\" content=\"0;url=https://evil.example\">",
            "<form action=\"https://evil.example\"><input name=p></form>",
            "<object data=\"data:text/html,<script>alert(1)</script>\"></object>",
            "<embed src=\"data:text/html,x\">",
            "<img src=x\nonerror=alert(1)>",
            "<div style=\"background:url(javascript:alert(1))\">x</div>",
            "<a href=\"data:text/html,<script>alert(1)</script>\">x</a>",
            "<a href=\"data:image/svg+xml,<svg onload=alert(1)>\">x</a>",
            "<img src=\"x\" ONERROR=\"alert(1)\">",
            "<svg><script>alert(1)</script></svg>",
        ];
        for attack in cases {
            let clean = sanitize_markup(attack);
            let flattened = clean.to_ascii_lowercase().replace(char::is_whitespace, "");
            assert!(!flattened.contains("onerror"), "handler survived: {clean}");
            assert!(!flattened.contains("onload"), "handler survived: {clean}");
            assert!(
                !flattened.contains("javascript:"),
                "scheme survived: {clean}"
            );
            assert!(!flattened.contains("srcdoc"), "srcdoc survived: {clean}");
            assert!(!flattened.contains("<script"), "script survived: {clean}");
            assert!(!flattened.contains("<iframe"), "iframe survived: {clean}");
            assert!(!flattened.contains("<base"), "base survived: {clean}");
            assert!(!flattened.contains("<meta"), "meta survived: {clean}");
            assert!(!flattened.contains("<form"), "form survived: {clean}");
            assert!(!flattened.contains("<object"), "object survived: {clean}");
            assert!(!flattened.contains("<embed"), "embed survived: {clean}");
            assert!(
                !flattened.contains("data:text/html"),
                "html data url survived: {clean}"
            );
            assert!(
                !flattened.contains("foreignobject"),
                "foreignObject survived: {clean}"
            );
        }
    }

    #[test]
    fn a_dropped_element_takes_its_program_text_with_it() {
        // Unwrapping a script would paste `alert(1)` into the page as prose.
        assert_eq!(sanitize_markup("a<script>alert(1)</script>b"), "ab");
        assert_eq!(sanitize_markup("a<style>p{color:red}</style>b"), "ab");
        // An unterminated one takes the rest with it rather than leaving it live.
        assert_eq!(sanitize_markup("a<script>alert(1)"), "a");
    }

    #[test]
    fn an_unknown_element_is_unwrapped_so_prose_is_never_lost() {
        assert_eq!(sanitize_markup("<marquee>read me</marquee>"), "read me");
        assert_eq!(sanitize_markup("<custom-thing>text</custom-thing>"), "text");
    }

    #[test]
    fn an_attribute_value_may_hold_the_tag_delimiter() {
        // A parser that ended the tag at the first `>` would let everything after the quote
        // escape inspection, which is a bypass in itself.
        let clean = sanitize_markup("<p title=\"a > b\" onclick=\"evil()\">x</p>");
        assert!(clean.contains("title=\"a &gt; b\""), "{clean}");
        assert!(!clean.contains("onclick"), "{clean}");
    }

    #[test]
    fn a_bare_less_than_stays_text_and_cannot_form_a_tag() {
        assert_eq!(sanitize_markup("1 < 2 and 3 > 2"), "1 &lt; 2 and 3 > 2");
    }

    #[test]
    fn comments_are_dropped_so_they_cannot_hide_an_unbalanced_quote() {
        assert_eq!(sanitize_markup("a<!-- <img src=x onerror=y> -->b"), "ab");
    }

    #[test]
    fn safe_data_images_survive_and_scriptable_ones_do_not() {
        let png = "<img src=\"data:image/png;base64,iVBORw0KGgo=\">";
        assert!(sanitize_markup(png).contains("data:image/png"));
        assert!(!sanitize_markup("<img src=\"data:image/svg+xml,x\">").contains("data:"));
    }

    /// A drawing that a screen reader can no longer describe is a document made worse, not
    /// safer. `aria-*` is inert — it names meaning, never a URL and never behavior — so the
    /// whole family is kept rather than enumerated, and `xmlns` stays because the same markup
    /// is also written to standalone SVG.
    #[test]
    fn accessibility_and_namespaces_survive_the_allow_list() {
        let chart = "<svg viewBox=\"0 0 10 10\" xmlns=\"http://www.w3.org/2000/svg\" \
                     role=\"img\" aria-label=\"Request latency\" aria-describedby=\"d\"></svg>";
        let clean = sanitize_markup(chart);
        for kept in [
            "viewBox=\"0 0 10 10\"",
            "xmlns=\"http://www.w3.org/2000/svg\"",
            "role=\"img\"",
            "aria-label=\"Request latency\"",
            "aria-describedby=\"d\"",
        ] {
            assert!(clean.contains(kept), "{kept} was dropped: {clean}");
        }
    }

    #[test]
    fn a_style_attribute_that_only_lays_out_survives() {
        assert_eq!(
            sanitize_markup("<div style=\"text-align:center\">x</div>"),
            "<div style=\"text-align:center\">x</div>"
        );
        assert!(!sanitize_markup("<div style=\"@import 'x'\">y</div>").contains("import"));
    }

    #[test]
    fn entities_are_decoded_the_way_a_browser_decodes_them() {
        assert_eq!(decode_entities("&#106;avascript&colon;x"), "javascript:x");
        assert_eq!(decode_entities("&#x6A;&amp;"), "j&");
        assert_eq!(
            decode_entities("plain & unterminated"),
            "plain & unterminated"
        );
    }

    /// A numeric entity with no trailing `;` is not decoded here, but a browser decodes it —
    /// so a smuggled `&#106avascript:` href must reach the page with its `&` escaped to
    /// `&amp;`, where it is literal text. This is the re-escaping [`decode_entities`]'s
    /// safety depends on; if it weakens, this test is the alarm.
    #[test]
    fn an_unterminated_numeric_entity_cannot_smuggle_a_scheme() {
        let sanitized = sanitize_markup("<a href=\"&#106avascript:alert(1)\">x</a>");
        assert!(
            !sanitized.contains("href=\"&#106"),
            "the raw entity survived unescaped, and a browser will decode it: {sanitized}"
        );
        if sanitized.contains("href") {
            assert!(sanitized.contains("&amp;#106"), "{sanitized}");
        }
    }

    #[test]
    fn extracts_only_the_first_svg_root() {
        assert_eq!(extract_svg("a<svg>x</svg>b<svg>y</svg>"), "<svg>x</svg>");
        assert_eq!(extract_svg("no svg here"), "");
    }
}
