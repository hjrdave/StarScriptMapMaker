//! What Copy and Cut take: branches of the Layer tree, kept in memory and mirrored to the
//! system clipboard as a versioned JSON text payload (no image).

use serde::{Deserialize, Serialize};
use ssmap_core::Subtree;

/// Names the payload so other text on the system clipboard is never mistaken for layers.
const FORMAT: &str = "starscript-map-maker/layers";
/// Bumped whenever the payload changes shape in a way an older app cannot read.
const VERSION: u32 = 1;

/// Copied Layers and Objects, in stacking order (bottom first), with their original ids.
#[derive(Clone, PartialEq, Debug)]
pub struct Clipboard {
    pub subtrees: Vec<Subtree>,
    /// The middle of the copied Objects' combined bounds, in Canvas units. Paste puts this
    /// point under the cursor. `None` when nothing copied has an extent.
    pub centre: Option<[f64; 2]>,
    /// Whether any copied branch is an Object rather than a Layer, so it needs a Layer around it.
    pub has_objects: bool,
}

#[derive(Serialize, Deserialize)]
struct Payload {
    format: String,
    version: u32,
    #[serde(default)]
    centre: Option<[f64; 2]>,
    layers: Vec<Subtree>,
}

impl Clipboard {
    pub fn new(subtrees: Vec<Subtree>, centre: Option<[f64; 2]>) -> Self {
        let has_objects = subtrees.iter().any(|s| !s.node.is_layer());
        Self {
            subtrees,
            centre,
            has_objects,
        }
    }

    /// The text mirrored to the system clipboard.
    #[allow(clippy::expect_used, reason = "a tree of plain data serialises")]
    pub fn to_text(&self) -> String {
        serde_json::to_string(&Payload {
            format: FORMAT.to_owned(),
            version: VERSION,
            centre: self.centre,
            layers: self.subtrees.clone(),
        })
        .expect("a tree of plain data serialises")
    }

    /// Reads text written by [`Clipboard::to_text`]. `None` for any other text, an empty
    /// payload or a version this app does not know.
    pub fn from_text(text: &str) -> Option<Self> {
        let payload: Payload = serde_json::from_str(text).ok()?;
        (payload.format == FORMAT && payload.version == VERSION && !payload.layers.is_empty())
            .then(|| Self::new(payload.layers, payload.centre))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssmap_core::{Id, Layer, Node};

    fn layer(name: &str) -> Subtree {
        Subtree {
            id: Id::new(),
            node: Node::Layer(Layer::new(name)),
            children: Vec::new(),
        }
    }

    #[test]
    fn the_text_round_trips() {
        let clipboard = Clipboard::new(vec![layer("Pines")], Some([3.0, 4.0]));
        let text = clipboard.to_text();
        assert_eq!(Clipboard::from_text(&text), Some(clipboard));
    }

    #[test]
    fn other_text_and_other_versions_are_not_layers() {
        assert_eq!(Clipboard::from_text("hello"), None);
        assert_eq!(Clipboard::from_text("{}"), None);
        let text = Clipboard::new(vec![layer("A")], None)
            .to_text()
            .replace("\"version\":1", "\"version\":99");
        assert_eq!(Clipboard::from_text(&text), None);
    }
}
