//! Fields this version doesn't know about.

/// Fields read from a file saved by a newer minor format version, which this version doesn't
/// know. Every persisted JSON object keeps them in a serde catch-all, so opening and saving the
/// Map loses nothing (ADR 0005).
pub type Extra = serde_json::Map<String, serde_json::Value>;
