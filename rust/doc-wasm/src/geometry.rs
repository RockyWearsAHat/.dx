//! JSON shapes for `board_edge_layout` and `board_edge_preview`.
//!
//! `doc-core`'s `render::board` geometry (`Rect`, `Side`, `Measured`, `EdgeSpec`,
//! `EdgeLayout`, …) is deliberately serde-free, the same reason [`crate::dto`] mirrors
//! [`doc_core::model`] instead of deriving on it directly — these are the mirrors for the
//! geometry door, `camelCase` on the wire like every other DTO in this crate.

use doc_core::render::{DragEnd, Rect, Side};
use serde::{Deserialize, Serialize};

/// A box on the canvas, as an editing surface measured it.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct RectDto {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl From<RectDto> for Rect {
    fn from(dto: RectDto) -> Rect {
        Rect {
            x: dto.x,
            y: dto.y,
            w: dto.w,
            h: dto.h,
        }
    }
}

/// The side a caller named, by letter or by word — [`doc_core::render::side_named`]
/// is the one vocabulary, so `"b"` and `"bottom"` answer the same [`Side`] here that they
/// would from a node line or a `dx board --link` call.
fn side_of(word: &str) -> Result<Side, String> {
    doc_core::render::side_named(word).ok_or_else(|| format!("`{word}` is not a side"))
}

/// One node [`board_edge_layout`] measured.
#[derive(Debug, Clone, Deserialize)]
pub struct MeasuredDto {
    id: String,
    #[serde(flatten)]
    rect: RectDto,
}

/// One edge [`board_edge_layout`] is asked to route.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EdgeSpecDto {
    from: String,
    to: String,
    #[serde(default)]
    from_side: Option<String>,
    #[serde(default)]
    to_side: Option<String>,
    #[serde(default)]
    label: String,
}

/// `board_edge_layout`'s whole request: the boxes a surface measured, the pairs it wants
/// routed, and the fit the canvas will display at.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayoutRequest {
    #[serde(default = "one")]
    pub scale: f64,
    pub nodes: Vec<MeasuredDto>,
    pub edges: Vec<EdgeSpecDto>,
}

/// The default `scale` a request that omits it gets: a legible fit, same as a fresh board.
fn one() -> f64 {
    1.0
}

impl LayoutRequest {
    /// This request's nodes and edges, translated into what [`doc_core::render::board`]
    /// takes — the one place a malformed side word becomes the error a caller sees.
    ///
    /// # Errors
    /// Returns a sentence when an edge names a side that is not one.
    pub fn into_core(
        self,
    ) -> Result<
        (
            Vec<doc_core::render::Measured>,
            Vec<doc_core::render::EdgeSpec>,
            f64,
        ),
        String,
    > {
        let nodes = self
            .nodes
            .into_iter()
            .map(|node| doc_core::render::Measured {
                id: node.id,
                rect: node.rect.into(),
            })
            .collect();
        let edges = self
            .edges
            .into_iter()
            .map(|edge| {
                let from_pin = edge.from_side.as_deref().map(side_of).transpose()?;
                let to_pin = edge.to_side.as_deref().map(side_of).transpose()?;
                Ok(doc_core::render::EdgeSpec {
                    from: edge.from,
                    to: edge.to,
                    from_pin,
                    to_pin,
                    label: edge.label,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok((nodes, edges, self.scale))
    }
}

/// Where an edge's words sit, on the wire.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LabelDto {
    x: f64,
    y: f64,
    font: f64,
}

/// One routed edge, on the wire — echoes the `from`/`to` it answers so a caller can match
/// an entry back to its request without relying on the array's order.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct EdgeLayoutDto {
    from: String,
    to: String,
    from_side: char,
    to_side: char,
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<LabelDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stroke: Option<f64>,
}

impl From<doc_core::render::EdgeLayout> for EdgeLayoutDto {
    fn from(layout: doc_core::render::EdgeLayout) -> EdgeLayoutDto {
        EdgeLayoutDto {
            from: layout.from,
            to: layout.to,
            from_side: layout.from_side.letter(),
            to_side: layout.to_side.letter(),
            path: layout.path,
            label: layout.label.map(|label| LabelDto {
                x: label.x,
                y: label.y,
                font: label.font,
            }),
            stroke: layout.stroke,
        }
    }
}

/// `board_edge_layout`'s answer: every routed edge, JSON-serialized.
///
/// # Errors
/// Returns a sentence when `request` is not valid JSON of [`LayoutRequest`]'s shape, or
/// names a side that is not one.
pub fn layout(request: &str) -> Result<String, String> {
    let request: LayoutRequest =
        serde_json::from_str(request).map_err(|error| format!("not a layout request: {error}"))?;
    let (nodes, edges, scale) = request.into_core()?;
    let answer: Vec<EdgeLayoutDto> = doc_core::render::edge_layout(&nodes, &edges, scale)
        .into_iter()
        .map(EdgeLayoutDto::from)
        .collect();
    serde_json::to_string(&answer).map_err(|error| format!("could not encode the layout: {error}"))
}

/// One end of a line being dragged, on the wire: attached to a measured node's side, or
/// following the pointer over open paper.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum DragEndDto {
    /// `{"box": {...}, "side": "r"}` — attached to a node.
    On {
        #[serde(rename = "box")]
        rect: RectDto,
        side: String,
    },
    /// `{"x": .., "y": ..}` — free, following the pointer.
    At { x: f64, y: f64 },
}

/// `board_edge_preview`'s whole request: the side the line leaves, where it currently ends,
/// and whatever it should dodge.
#[derive(Debug, Clone, Deserialize)]
struct PreviewRequest {
    from: DragFromDto,
    to: DragEndDto,
    #[serde(default)]
    obstacles: Vec<RectDto>,
}

/// The end of a line that is anchored — always a node's side, since a drag always starts
/// on one.
#[derive(Debug, Clone, Deserialize)]
struct DragFromDto {
    #[serde(rename = "box")]
    rect: RectDto,
    side: String,
}

/// `board_edge_preview`'s answer: `{"path": "M … C …, …, … L …"}`.
///
/// # Errors
/// Returns a sentence when `request` is not valid JSON of the expected shape, or names a
/// side that is not one.
pub fn preview(request: &str) -> Result<String, String> {
    let request: PreviewRequest =
        serde_json::from_str(request).map_err(|error| format!("not a preview request: {error}"))?;
    let from_side = side_of(&request.from.side)?;
    let to = match request.to {
        DragEndDto::On { rect, side } => DragEnd::On(rect.into(), side_of(&side)?),
        DragEndDto::At { x, y } => DragEnd::At(x, y),
    };
    let obstacles: Vec<Rect> = request.obstacles.into_iter().map(Rect::from).collect();
    let path = doc_core::render::drag_path((request.from.rect.into(), from_side), to, &obstacles);
    serde_json::to_string(&serde_json::json!({ "path": path }))
        .map_err(|error| format!("could not encode the preview: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two nodes and one unpinned edge, routed end to end through the JSON boundary — the
    /// same request shape `editor/surface/edit.js` sends.
    #[test]
    fn a_layout_request_answers_one_routed_edge_per_input_edge() {
        let request = serde_json::json!({
            "scale": 1.0,
            "nodes": [
                {"id": "a", "x": 0.0, "y": 0.0, "w": 200.0, "h": 100.0},
                {"id": "b", "x": 600.0, "y": 0.0, "w": 200.0, "h": 100.0},
            ],
            "edges": [
                {"from": "a", "to": "b"},
            ],
        })
        .to_string();
        let answer = layout(&request).expect("a valid request lays out");
        let parsed: serde_json::Value = serde_json::from_str(&answer).expect("valid JSON back");
        let edges = parsed.as_array().expect("an array of edges");
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0]["from"], "a");
        assert_eq!(edges[0]["to"], "b");
        assert!(edges[0]["path"].as_str().expect("a path").starts_with('M'));
        assert!(
            edges[0].get("label").is_none(),
            "an unlabelled edge carries no label key"
        );
        assert!(
            edges[0].get("stroke").is_none(),
            "a legible fit carries no stroke key"
        );
    }

    /// A pinned side round-trips through the wire's letter spelling and its word spelling
    /// alike — the vocabulary `side_named` promises every host shares.
    #[test]
    fn a_pinned_side_is_honoured_by_letter_or_by_word() {
        let request = serde_json::json!({
            "nodes": [
                {"id": "a", "x": 0.0, "y": 0.0, "w": 200.0, "h": 100.0},
                {"id": "b", "x": 0.0, "y": 400.0, "w": 200.0, "h": 100.0},
            ],
            "edges": [{"from": "a", "to": "b", "fromSide": "right"}],
        })
        .to_string();
        let answer = layout(&request).expect("a valid request lays out");
        let parsed: serde_json::Value = serde_json::from_str(&answer).expect("valid JSON");
        assert_eq!(parsed[0]["fromSide"], "r");
    }

    /// A side that is not one is the caller's error, not a silent default.
    #[test]
    fn an_unknown_side_word_is_refused() {
        let request = serde_json::json!({
            "nodes": [
                {"id": "a", "x": 0.0, "y": 0.0, "w": 200.0, "h": 100.0},
                {"id": "b", "x": 300.0, "y": 0.0, "w": 200.0, "h": 100.0},
            ],
            "edges": [{"from": "a", "to": "b", "fromSide": "sideways"}],
        })
        .to_string();
        let error = layout(&request).expect_err("an unknown side must refuse");
        assert!(error.contains("sideways"), "{error}");
    }

    /// Malformed JSON is an error naming the problem, never an empty answer that reads as
    /// "no edges" when the real story is "no request was understood".
    #[test]
    fn malformed_json_is_an_error_not_an_empty_layout() {
        let error = layout("not json").expect_err("malformed input must refuse");
        assert!(error.contains("not a layout request"), "{error}");
    }

    /// A drag preview attached to a node's side draws a path from that side, straight,
    /// with no obstacle to bend around.
    #[test]
    fn a_preview_over_open_paper_draws_a_path_from_the_source_side() {
        let request = serde_json::json!({
            "from": {"box": {"x": 0.0, "y": 0.0, "w": 200.0, "h": 100.0}, "side": "r"},
            "to": {"x": 500.0, "y": 40.0},
        })
        .to_string();
        let answer = preview(&request).expect("a valid preview request");
        let parsed: serde_json::Value = serde_json::from_str(&answer).expect("valid JSON");
        assert!(parsed["path"].as_str().expect("a path").starts_with('M'));
    }

    /// A drag preview dropped over a node's side lands exactly on it, the same as a
    /// committed edge would.
    #[test]
    fn a_preview_over_a_node_lands_on_the_side_named() {
        let request = serde_json::json!({
            "from": {"box": {"x": 0.0, "y": 0.0, "w": 200.0, "h": 100.0}, "side": "r"},
            "to": {"box": {"x": 900.0, "y": 0.0, "w": 200.0, "h": 100.0}, "side": "l"},
        })
        .to_string();
        let answer = preview(&request).expect("a valid preview request");
        let parsed: serde_json::Value = serde_json::from_str(&answer).expect("valid JSON");
        let path = parsed["path"].as_str().expect("a path");
        // The arrival anchor is the last `L x y` pair — the middle of the target's left
        // side.
        let tail = path.rsplit("L ").next().expect("a straight tail");
        let mut numbers = tail.split_whitespace();
        let x: f64 = numbers.next().expect("x").parse().expect("a number");
        let y: f64 = numbers.next().expect("y").parse().expect("a number");
        assert!((x - 900.0).abs() < 1e-6, "{path}");
        assert!((y - 50.0).abs() < 1e-6, "{path}");
    }

    /// A malformed preview request is an error, not a straight line to nowhere.
    #[test]
    fn a_malformed_preview_request_is_an_error() {
        let error = preview("{}").expect_err("a request with no `from`/`to` must refuse");
        assert!(error.contains("not a preview request"), "{error}");
    }
}
