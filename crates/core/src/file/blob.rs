//! The binary outline blob of a Landmass or Lake.
//!
//! All numbers are little-endian. A blob is a `u32` Polygon count, then for each Polygon a
//! `u32` ring count (the exterior first, then its holes), then for each ring a `u32` point
//! count followed by that many `f64` `x, y` pairs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "existing invariant panics; replace with error handling over time (#351)"
)]

use thiserror::Error;

use crate::geometry::{MultiPolygon, Point, Polygon};

/// Why an outline blob could not be decoded.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum BlobError {
    /// The blob ends before the outline does.
    #[error("the blob ends early")]
    EndsEarly,
    /// A polygon has no exterior ring.
    #[error("a polygon has no exterior ring")]
    NoExteriorRing,
    /// Bytes remain after the outline.
    #[error("{0} bytes left over")]
    LeftOver(usize),
}

pub(super) fn encode(geometry: &MultiPolygon) -> Vec<u8> {
    let points: usize = geometry
        .0
        .iter()
        .map(|polygon| polygon.exterior.len() + polygon.holes.iter().map(Vec::len).sum::<usize>())
        .sum();
    let mut out = Vec::with_capacity(4 + geometry.0.len() * 8 + points * 16);
    push_len(&mut out, geometry.0.len());
    for polygon in &geometry.0 {
        push_len(&mut out, 1 + polygon.holes.len());
        for ring in std::iter::once(&polygon.exterior).chain(&polygon.holes) {
            push_len(&mut out, ring.len());
            for [x, y] in ring {
                out.extend_from_slice(&x.to_le_bytes());
                out.extend_from_slice(&y.to_le_bytes());
            }
        }
    }
    out
}

pub(super) fn decode(bytes: &[u8]) -> Result<MultiPolygon, BlobError> {
    let mut reader = Reader { bytes };
    let polygons = reader.len(4)?;
    let mut geometry = Vec::with_capacity(polygons);
    for _ in 0..polygons {
        let rings = reader.len(4)?;
        if rings == 0 {
            return Err(BlobError::NoExteriorRing);
        }
        let mut rings = (0..rings).map(|_| reader.ring());
        let exterior = rings.next().expect("at least one ring")?;
        let holes = rings.collect::<Result<_, _>>()?;
        geometry.push(Polygon { exterior, holes });
    }
    if !reader.bytes.is_empty() {
        return Err(BlobError::LeftOver(reader.bytes.len()));
    }
    Ok(MultiPolygon(geometry))
}

fn push_len(out: &mut Vec<u8>, len: usize) {
    let len = u32::try_from(len).expect("fewer than 2^32 parts, rings or points");
    out.extend_from_slice(&len.to_le_bytes());
}

struct Reader<'a> {
    bytes: &'a [u8],
}

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], BlobError> {
        let Some((head, rest)) = self.bytes.split_first_chunk::<N>() else {
            return Err(BlobError::EndsEarly);
        };
        self.bytes = rest;
        Ok(*head)
    }

    /// Reads a count and checks that the blob could hold that many items of at least
    /// `item_bytes` each, so a damaged count can't ask for a huge allocation.
    fn len(&mut self, item_bytes: usize) -> Result<usize, BlobError> {
        let len = u32::from_le_bytes(self.take()?) as usize;
        if len.saturating_mul(item_bytes) > self.bytes.len() {
            return Err(BlobError::EndsEarly);
        }
        Ok(len)
    }

    fn ring(&mut self) -> Result<Vec<Point>, BlobError> {
        let points = self.len(16)?;
        (0..points)
            .map(|_| {
                let x = f64::from_le_bytes(self.take()?);
                let y = f64::from_le_bytes(self.take()?);
                Ok([x, y])
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_exactly() {
        let geometry = MultiPolygon(vec![
            Polygon {
                exterior: vec![[0.0, 0.0], [10.5, -0.0], [f64::MIN_POSITIVE, 1e300]],
                holes: vec![vec![[1.0, 1.0], [2.0, 1.0], [1.5, 2.0]], vec![]],
            },
            Polygon::default(),
        ]);
        let decoded = decode(&encode(&geometry)).unwrap();
        assert_eq!(decoded, geometry);
        let bits = |g: &MultiPolygon| g.0[0].exterior[1][1].to_bits();
        assert_eq!(bits(&decoded), bits(&geometry), "-0.0 keeps its sign");
    }

    #[test]
    fn rejects_damaged_blobs() {
        let blob = encode(&MultiPolygon(vec![Polygon {
            exterior: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            holes: vec![],
        }]));
        assert!(decode(&blob[..blob.len() - 1]).is_err());
        assert!(decode(&[blob.as_slice(), &[0]].concat()).is_err());
        assert!(decode(&[0xff, 0xff, 0xff, 0xff]).is_err());
        assert!(decode(&[1, 0, 0, 0, 0, 0, 0, 0]).is_err());
    }
}
