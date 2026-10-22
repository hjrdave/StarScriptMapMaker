//! Embedding Assets and fonts in a Map, and updating them from a pack.

use std::collections::HashSet;

use crate::embedded::{Blob, ContentHash, EmbedAsset, EmbedFont, EmbeddedFont};
use crate::map::Map;
use crate::op::Op;

use super::CommandError;

pub(super) fn embed_assets(
    map: &Map,
    assets: &[EmbedAsset],
    fonts: &[EmbedFont],
) -> Result<Vec<Op>, CommandError> {
    let mut ops = Vec::new();
    let mut stored = HashSet::new();
    let mut put_blob = |ops: &mut Vec<Op>, blob: &Blob, expected: ContentHash| {
        let hash = blob.hash();
        if hash != expected {
            return Err(CommandError::InvalidEmbed(format!(
                "a file does not match its recorded hash {expected}"
            )));
        }
        if !map.embedded().blobs.contains_key(&hash) && stored.insert(hash) {
            ops.push(Op::SetBlob {
                hash,
                data: Some(blob.clone()),
            });
        }
        Ok(())
    };
    let mut seen_assets = HashSet::new();
    for embed in assets {
        // The embedded copy wins until the user updates it from the pack.
        if map.embedded().assets.contains_key(&embed.reference)
            || !seen_assets.insert(&embed.reference)
        {
            continue;
        }
        put_blob(&mut ops, &embed.art, embed.record.art)?;
        match (&embed.silhouette, embed.record.silhouette) {
            (Some(blob), Some(hash)) => put_blob(&mut ops, blob, hash)?,
            (None, None) => {}
            _ => {
                return Err(CommandError::InvalidEmbed(format!(
                    "{}: the Silhouette file and its record disagree",
                    embed.reference
                )));
            }
        }
        ops.push(Op::SetEmbeddedAsset {
            asset: embed.reference.clone(),
            record: Some(Box::new(embed.record.clone())),
        });
    }
    let mut seen_fonts = HashSet::new();
    for font in fonts {
        if map.embedded().fonts.contains_key(&font.file) || !seen_fonts.insert(&font.file) {
            continue;
        }
        put_blob(&mut ops, &font.data, font.data.hash())?;
        ops.push(Op::SetEmbeddedFont {
            file: font.file.clone(),
            record: Some(Box::new(EmbeddedFont {
                family: font.family.clone(),
                file: font.file.clone(),
                data: font.data.hash(),
                licence: font.licence.clone(),
                extra: Default::default(),
            })),
        });
    }
    Ok(ops)
}

pub(super) fn update_from_pack(map: &Map, assets: &[EmbedAsset]) -> Result<Vec<Op>, CommandError> {
    let embedded = map.embedded();
    let mut records = embedded.assets.clone();
    let mut updated = Vec::new();
    let mut new_blobs: Vec<(ContentHash, &Blob)> = Vec::new();
    for embed in assets {
        if updated.contains(&&embed.reference) {
            continue;
        }
        if !embedded.assets.contains_key(&embed.reference) {
            return Err(CommandError::InvalidEmbed(format!(
                "{} is not embedded in the Map",
                embed.reference
            )));
        }
        if embed.silhouette.is_some() != embed.record.silhouette.is_some() {
            return Err(CommandError::InvalidEmbed(format!(
                "{}: the Silhouette file and its record disagree",
                embed.reference
            )));
        }
        let files = [
            Some((&embed.art, embed.record.art)),
            embed.silhouette.as_ref().zip(embed.record.silhouette),
        ];
        for (blob, expected) in files.into_iter().flatten() {
            if blob.hash() != expected {
                return Err(CommandError::InvalidEmbed(format!(
                    "a file does not match its recorded hash {expected}"
                )));
            }
            if !embedded.blobs.contains_key(&expected)
                && !new_blobs.iter().any(|(hash, _)| *hash == expected)
            {
                new_blobs.push((expected, blob));
            }
        }
        records.insert(embed.reference.clone(), embed.record.clone());
        updated.push(&embed.reference);
    }
    let mut kept: HashSet<ContentHash> = embedded.fonts.values().map(|font| font.data).collect();
    for record in records.values() {
        kept.extend(std::iter::once(record.art).chain(record.silhouette));
    }
    let mut ops = Vec::new();
    for (hash, blob) in new_blobs {
        ops.push(Op::SetBlob {
            hash,
            data: Some(blob.clone()),
        });
    }
    for reference in updated {
        ops.push(Op::SetEmbeddedAsset {
            asset: reference.clone(),
            record: Some(Box::new(records[reference].clone())),
        });
    }
    for hash in embedded.blobs.keys().filter(|hash| !kept.contains(hash)) {
        ops.push(Op::SetBlob {
            hash: *hash,
            data: None,
        });
    }
    Ok(ops)
}
