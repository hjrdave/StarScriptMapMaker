//! What a Map lacks for copied branches to paste into it as they were: the Assets, fonts and
//! Label styles they name that the Map does not have. Pasting into another Map or tab can
//! leave such gaps; [`Map::fit_for_paste`] closes them.

use std::collections::BTreeSet;

use crate::asset::AssetRef;
use crate::label::LabelStyle;
use crate::map::{Map, Subtree};
use crate::node::{Node, ObjectKind};

/// Things copied branches need that the target Map lacks.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct PasteGaps {
    /// Assets neither embedded nor installed. Their Stamps paste as placeholders.
    pub assets: Vec<AssetRef>,
    /// Font families Labels set themselves that the Map cannot draw. They get the fallback font.
    pub fonts: Vec<String>,
    /// How many Labels follow a style the Map lacks. They follow the default style instead,
    /// keeping their overrides.
    pub label_styles: usize,
}

impl PasteGaps {
    /// Whether the Map lacks nothing the paste needs.
    pub fn is_empty(&self) -> bool {
        self.assets.is_empty() && self.fonts.is_empty() && self.label_styles == 0
    }
}

impl Map {
    /// What `subtrees` need that this Map lacks. `asset_available` says whether an Asset can
    /// be drawn here (embedded or installed); `font_available` whether a bundled family exists.
    /// A family the Map embeds counts as available too.
    pub fn paste_gaps(
        &self,
        subtrees: &[Subtree],
        asset_available: impl Fn(&AssetRef) -> bool,
        font_available: impl Fn(&str) -> bool,
    ) -> PasteGaps {
        let mut assets = BTreeSet::new();
        let mut fonts = BTreeSet::new();
        let mut label_styles = 0;
        for_each_object(subtrees, &mut |kind| match kind {
            ObjectKind::Stamp(stamp) => {
                if !self.embedded().assets.contains_key(&stamp.asset)
                    && !asset_available(&stamp.asset)
                {
                    assets.insert(stamp.asset.clone());
                }
            }
            ObjectKind::Label(label) => {
                if self.label_styles().get(label.style).is_none() {
                    label_styles += 1;
                }
                if let Some(font) = &label.font
                    && !self.font_available(&font.family, &font_available)
                {
                    fonts.insert(font.family.clone());
                }
            }
            _ => {}
        });
        PasteGaps {
            assets: assets.into_iter().collect(),
            fonts: fonts.into_iter().collect(),
            label_styles,
        }
    }

    /// Closes the gaps [`Map::paste_gaps`] reports: a Label with a style this Map lacks follows
    /// the default style (its overrides stay, and the style is not copied), and a font the
    /// Map cannot draw becomes `fallback_family`. Stamps keep their Asset, which draws as a
    /// placeholder until the Asset is installed.
    pub fn fit_for_paste(
        &self,
        subtrees: &mut [Subtree],
        font_available: impl Fn(&str) -> bool,
        fallback_family: &str,
    ) {
        self.fit_trees(subtrees, &font_available, fallback_family);
    }

    fn fit_trees(
        &self,
        subtrees: &mut [Subtree],
        font_available: &dyn Fn(&str) -> bool,
        fallback_family: &str,
    ) {
        for tree in subtrees {
            if let Node::Object(object) = &mut tree.node
                && let ObjectKind::Label(label) = &mut object.kind
            {
                if self.label_styles().get(label.style).is_none() {
                    label.style = LabelStyle::DEFAULT_ID;
                }
                if let Some(font) = &mut label.font
                    && !self.font_available(&font.family, &font_available)
                {
                    font.family = fallback_family.to_owned();
                }
            }
            self.fit_trees(&mut tree.children, font_available, fallback_family);
        }
    }

    fn font_available(&self, family: &str, bundled: &impl Fn(&str) -> bool) -> bool {
        bundled(family)
            || self
                .embedded()
                .fonts
                .values()
                .any(|font| font.family == family)
    }
}

fn for_each_object(subtrees: &[Subtree], visit: &mut impl FnMut(&ObjectKind)) {
    for tree in subtrees {
        if let Node::Object(object) = &tree.node {
            visit(&object.kind);
        }
        for_each_object(&tree.children, visit);
    }
}
