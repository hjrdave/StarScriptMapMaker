//! Credits: what the loaded packs ask to be credited for, and which of it a Map uses.

use std::collections::BTreeSet;

use crate::map::Map;
use crate::node::{Node, ObjectKind};

use super::{AssetRef, Pack, PackLibrary};

/// One pack's entry on the credits screen.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PackCredit {
    pub pack_id: String,
    pub name: String,
    pub licence: String,
    /// Where the licence text can be read, if the pack says.
    pub licence_url: Option<String>,
    /// The pack's free-form credit line.
    pub credits: String,
    pub attribution_required: bool,
    /// Every distinct Asset author, sorted.
    pub authors: Vec<String>,
    /// Every distinct licence the pack's Assets are under, sorted. Usually just the pack's own.
    pub licences: Vec<String>,
    /// Every distinct Asset source link, sorted.
    pub sources: Vec<String>,
}

/// An Asset from a pack that must be credited.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AttributedAsset {
    pub reference: AssetRef,
    pub pack_name: String,
    pub asset_name: String,
    pub author: String,
    pub licence: String,
    pub licence_url: Option<String>,
    pub source_url: Option<String>,
    /// The pack's credit line.
    pub credits: String,
}

impl AttributedAsset {
    /// The line crediting this Asset, such as `Sword by Lorc, Icons (CC-BY-3.0) https://...`.
    pub(crate) fn credit_line(&self) -> String {
        let mut line = format!(
            "{} by {}, {} ({})",
            self.asset_name, self.author, self.pack_name, self.licence
        );
        if let Some(url) = self.source_url.as_ref().or(self.licence_url.as_ref()) {
            line.push(' ');
            line.push_str(url);
        }
        line
    }
}

/// The credit lines for `assets`: one per Asset, then each pack's own credit line once.
pub fn credit_lines(assets: &[AttributedAsset]) -> Vec<String> {
    let mut lines: Vec<String> = assets.iter().map(AttributedAsset::credit_line).collect();
    let mut seen = BTreeSet::new();
    for asset in assets {
        let credit = asset.credits.trim();
        if !credit.is_empty() && seen.insert(credit) {
            lines.push(credit.to_owned());
        }
    }
    lines
}

impl Pack {
    /// This pack's credits entry.
    pub fn credit(&self) -> PackCredit {
        let distinct = |values: Vec<&str>| -> Vec<String> {
            values
                .into_iter()
                .map(str::to_owned)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect()
        };
        PackCredit {
            pack_id: self.id.clone(),
            name: self.name.clone(),
            licence: self.licence.clone(),
            licence_url: self.licence_url.clone(),
            credits: self.credits.clone(),
            attribution_required: self.attribution_required,
            authors: distinct(self.assets.iter().map(|a| a.author.as_str()).collect()),
            licences: distinct(self.assets.iter().map(|a| a.licence.as_str()).collect()),
            sources: distinct(
                self.assets
                    .iter()
                    .filter_map(|a| a.source_url.as_deref())
                    .collect(),
            ),
        }
    }
}

impl PackLibrary {
    /// The credits entry of every loaded pack, in load order.
    pub fn credits(&self) -> Vec<PackCredit> {
        self.packs().iter().map(Pack::credit).collect()
    }

    /// The Assets `map` places as Stamps, from packs flagged `attribution_required`, each once,
    /// sorted by reference. Hidden and locked Stamps count: an export may still show them.
    /// Stamps of Assets no loaded pack provides are skipped, as there is nothing to credit.
    pub fn attribution_required_assets(&self, map: &Map) -> Vec<AttributedAsset> {
        let used: BTreeSet<&AssetRef> = map
            .ids()
            .filter_map(|id| match map.node(id)? {
                Node::Object(object) => match &object.kind {
                    ObjectKind::Stamp(stamp) => Some(&stamp.asset),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        used.into_iter()
            .filter_map(|reference| {
                let (pack, asset) = self.asset(reference)?;
                pack.attribution_required.then(|| AttributedAsset {
                    reference: reference.clone(),
                    pack_name: pack.name.clone(),
                    asset_name: asset.name.clone(),
                    author: asset.author.clone(),
                    licence: asset.licence.clone(),
                    licence_url: pack.licence_url.clone(),
                    source_url: asset.source_url.clone(),
                    credits: pack.credits.clone(),
                })
            })
            .collect()
    }
}
