//! Assets and fonts embedded in a Map (ADR 0005), so a Map renders identically even if a pack
//! is updated, removed or was never installed.
//!
//! Embedded copies are Map state, changed only by Ops: a content-hash-keyed store of blobs
//! (art, Silhouettes, font files), plus one record per `pack-id/asset-id` and per font file
//! holding its manifest entry, credits and licence. Identical bytes are stored once. The
//! embedded copy wins over an installed pack's art until the user updates it from the pack.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Write as _};
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

use crate::asset::{ArtFormat, Asset, AssetRef, Pack};
use crate::extra::Extra;
use crate::map::Map;
use crate::node::{Node, ObjectKind};

/// The SHA-256 of a blob's bytes, written as 64 lowercase hex digits.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContentHash([u8; 32]);

impl ContentHash {
    pub fn of(bytes: &[u8]) -> Self {
        Self(sha256(bytes))
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.iter().try_for_each(|byte| write!(f, "{byte:02x}"))
    }
}

impl fmt::Debug for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ContentHash({self})")
    }
}

impl FromStr for ContentHash {
    type Err = ParseHexError;

    fn from_str(s: &str) -> Result<Self, ParseHexError> {
        let bytes = from_hex(s)?;
        let bytes = <[u8; 32]>::try_from(bytes).map_err(|_| ParseHexError::WrongLength)?;
        Ok(Self(bytes))
    }
}

impl Serialize for ContentHash {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ContentHash {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for ContentHash {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "ContentHash".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({ "type": "string", "pattern": "^[0-9a-f]{64}$" })
    }
}

/// The bytes of one embedded file, cheap to clone. In JSON it is a hex string.
#[derive(Clone, PartialEq, Eq)]
pub struct Blob(Arc<[u8]>);

impl Blob {
    pub fn new(bytes: impl Into<Arc<[u8]>>) -> Self {
        Self(bytes.into())
    }

    pub fn hash(&self) -> ContentHash {
        ContentHash::of(&self.0)
    }
}

impl std::ops::Deref for Blob {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for Blob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Blob({} bytes)", self.0.len())
    }
}

impl Serialize for Blob {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut text = String::with_capacity(self.0.len() * 2);
        for byte in self.0.iter() {
            let _ = write!(text, "{byte:02x}");
        }
        serializer.serialize_str(&text)
    }
}

impl<'de> Deserialize<'de> for Blob {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        from_hex(&text)
            .map(|bytes| Blob(bytes.into()))
            .map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for Blob {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Blob".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({ "type": "string", "pattern": "^([0-9a-f]{2})*$" })
    }
}

/// Why a string is not valid hex, or not a hash.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Error)]
pub enum ParseHexError {
    #[error("expected an even number of hex digits")]
    OddLength,
    #[error("expected hex digits")]
    NotHex,
    #[error("a hash is 32 bytes")]
    WrongLength,
}

fn from_hex(text: &str) -> Result<Vec<u8>, ParseHexError> {
    if !text.len().is_multiple_of(2) || !text.is_ascii() {
        return Err(ParseHexError::OddLength);
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).map_err(|_| ParseHexError::NotHex))
        .collect()
}

/// An Asset's manifest entry, its pack's credits and licence, and the hashes of its files, as
/// they were when embedded.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
pub struct EmbeddedAsset {
    pub name: String,
    pub category: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub format: ArtFormat,
    /// The art file.
    pub art: ContentHash,
    pub silhouette: Option<ContentHash>,
    /// The ground point, as fractions of the art's width and height from its top-left.
    pub anchor: [f64; 2],
    /// Default width in Canvas units.
    pub width: f64,
    pub variant_group: Option<String>,
    pub author: String,
    pub source_url: Option<String>,
    pub licence: String,
    pub modified: String,
    pub pack_name: String,
    pub pack_licence: String,
    pub pack_licence_url: Option<String>,
    pub pack_credits: String,
    pub attribution_required: bool,
    #[serde(flatten)]
    pub extra: Extra,
}

/// One embedded font file and its licence.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
pub struct EmbeddedFont {
    /// The family name Labels store, such as "Cinzel".
    pub family: String,
    /// The font's file name, which is also its key.
    pub file: String,
    pub data: ContentHash,
    /// The full licence text.
    pub licence: String,
    #[serde(flatten)]
    pub extra: Extra,
}

/// Everything embedded in a Map.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct Embedded {
    /// The files, by content hash.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub blobs: BTreeMap<ContentHash, Blob>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub assets: BTreeMap<AssetRef, EmbeddedAsset>,
    /// By file name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fonts: BTreeMap<String, EmbeddedFont>,
}

impl Embedded {
    pub fn is_empty(&self) -> bool {
        self.blobs.is_empty() && self.assets.is_empty() && self.fonts.is_empty()
    }

    pub fn blob(&self, hash: &ContentHash) -> Option<&Blob> {
        self.blobs.get(hash)
    }

    /// The art file of an embedded Asset.
    pub fn art(&self, reference: &AssetRef) -> Option<&Blob> {
        self.blob(&self.assets.get(reference)?.art)
    }

    /// The Silhouette file of an embedded Asset, if it has one.
    pub fn silhouette(&self, reference: &AssetRef) -> Option<&Blob> {
        self.blob(&self.assets.get(reference)?.silhouette?)
    }

    /// The font files of a family, in file-name order.
    pub fn family_files<'a>(&'a self, family: &'a str) -> impl Iterator<Item = &'a Blob> + 'a {
        self.fonts
            .values()
            .filter(move |font| font.family == family)
            .filter_map(|font| self.blob(&font.data))
    }

    /// A file some record names that the store lacks, if any.
    pub fn first_missing_blob(&self) -> Option<ContentHash> {
        let assets = self
            .assets
            .values()
            .flat_map(|asset| std::iter::once(asset.art).chain(asset.silhouette));
        let fonts = self.fonts.values().map(|font| font.data);
        assets
            .chain(fonts)
            .find(|hash| !self.blobs.contains_key(hash))
    }
}

/// An Asset to embed: its record and its files.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
pub struct EmbedAsset {
    pub reference: AssetRef,
    pub record: EmbeddedAsset,
    pub art: Blob,
    pub silhouette: Option<Blob>,
}

/// A font file to embed.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
pub struct EmbedFont {
    pub family: String,
    pub file: String,
    pub licence: String,
    pub data: Blob,
}

impl EmbedAsset {
    /// Reads `asset` of `pack` from disk. Fails if its art or Silhouette cannot be read.
    pub fn read(pack: &Pack, asset: &Asset) -> std::io::Result<Self> {
        let art = Blob::new(std::fs::read(pack.path(&asset.file))?);
        let silhouette = asset
            .silhouette
            .as_deref()
            .map(|file: &Path| std::fs::read(pack.path(file)).map(Blob::new))
            .transpose()?;
        let reference = AssetRef::new(&pack.id, &asset.id)
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
        let record = EmbeddedAsset {
            name: asset.name.clone(),
            category: asset.category.clone(),
            tags: asset.tags.clone(),
            format: asset.format,
            art: art.hash(),
            silhouette: silhouette.as_ref().map(Blob::hash),
            anchor: asset.anchor,
            width: asset.width,
            variant_group: asset.variant_group.clone(),
            author: asset.author.clone(),
            source_url: asset.source_url.clone(),
            licence: asset.licence.clone(),
            modified: asset.modified.clone(),
            pack_name: pack.name.clone(),
            pack_licence: pack.licence.clone(),
            pack_licence_url: pack.licence_url.clone(),
            pack_credits: pack.credits.clone(),
            attribution_required: pack.attribution_required,
            extra: Extra::new(),
        };
        Ok(Self {
            reference,
            record,
            art,
            silhouette,
        })
    }
}

impl Map {
    /// The Assets the Map's Stamps place, each once.
    pub fn used_assets(&self) -> BTreeSet<AssetRef> {
        self.ids()
            .filter_map(|id| match self.node(id)? {
                Node::Object(object) => match &object.kind {
                    ObjectKind::Stamp(stamp) => Some(stamp.asset.clone()),
                    _ => None,
                },
                Node::Layer(_) => None,
            })
            .collect()
    }

    /// The font families the Map's Labels use, each once.
    pub fn used_font_families(&self) -> BTreeSet<String> {
        self.ids()
            .filter_map(|id| match self.node(id)? {
                Node::Object(object) => match &object.kind {
                    ObjectKind::Label(label) => {
                        Some(label.resolve(self.label_styles()).font.family)
                    }
                    _ => None,
                },
                Node::Layer(_) => None,
            })
            .collect()
    }
}

fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut state: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut compress = |block: &[u8]| {
        let mut w = [0u32; 64];
        for (i, word) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (s, v) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *s = s.wrapping_add(v);
        }
    };
    let mut blocks = data.chunks_exact(64);
    for block in &mut blocks {
        compress(block);
    }
    let mut tail = blocks.remainder().to_vec();
    tail.push(0x80);
    while tail.len() % 64 != 56 {
        tail.push(0);
    }
    tail.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for block in tail.chunks_exact(64) {
        compress(block);
    }
    let mut out = [0u8; 32];
    for (chunk, word) in out.chunks_exact_mut(4).zip(state) {
        chunk.copy_from_slice(&word.to_be_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_match_the_sha256_test_vectors() {
        assert_eq!(
            ContentHash::of(b"").to_string(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            ContentHash::of(b"abc").to_string(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            ContentHash::of(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")
                .to_string(),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        let million_a = vec![b'a'; 1_000_000];
        assert_eq!(
            ContentHash::of(&million_a).to_string(),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    #[test]
    fn blobs_and_hashes_round_trip_through_json() {
        let blob = Blob::new(vec![0u8, 1, 0xfe, 0xff]);
        let json = serde_json::to_string(&blob).unwrap();
        assert_eq!(json, "\"0001feff\"");
        assert_eq!(serde_json::from_str::<Blob>(&json).unwrap(), blob);
        let hash = blob.hash();
        let json = serde_json::to_string(&hash).unwrap();
        assert_eq!(serde_json::from_str::<ContentHash>(&json).unwrap(), hash);
        assert!(serde_json::from_str::<Blob>("\"abc\"").is_err());
        assert!(serde_json::from_str::<ContentHash>("\"00\"").is_err());
    }
}
