//! Colours as users pick them.

use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// An sRGB colour with straight (not premultiplied) alpha. Stored as `"#rrggbb"`, or
/// `"#rrggbbaa"` when not opaque.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const BLACK: Color = Color::rgb(0, 0, 0);
    pub const WHITE: Color = Color::rgb(0xff, 0xff, 0xff);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02x}{:02x}{:02x}", self.r, self.g, self.b)?;
        if self.a != 255 {
            write!(f, "{:02x}", self.a)?;
        }
        Ok(())
    }
}

impl FromStr for Color {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        let invalid = || format!("{s:?} is not a colour like \"#rrggbb\" or \"#rrggbbaa\"");
        let hex = s.strip_prefix('#').ok_or_else(invalid)?;
        if !(hex.len() == 6 || hex.len() == 8) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(invalid());
        }
        let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).expect("checked hex");
        Ok(Self {
            r: channel(0),
            g: channel(2),
            b: channel(4),
            a: if hex.len() == 8 { channel(6) } else { 255 },
        })
    }
}

impl Serialize for Color {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = Cow::<str>::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for Color {
    fn schema_name() -> Cow<'static, str> {
        "Color".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "pattern": "^#[0-9a-fA-F]{6}([0-9a-fA-F]{2})?$",
            "description": "sRGB as \"#rrggbb\", or \"#rrggbbaa\" with straight alpha",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_hex() {
        for (color, text) in [
            (Color::rgb(0x12, 0xab, 0xff), "\"#12abff\""),
            (Color::rgba(0, 0, 0, 0x80), "\"#00000080\""),
        ] {
            assert_eq!(serde_json::to_string(&color).unwrap(), text);
            assert_eq!(serde_json::from_str::<Color>(text).unwrap(), color);
        }
        assert_eq!("#ABCDEF".parse::<Color>(), Ok(Color::rgb(0xab, 0xcd, 0xef)));
    }

    #[test]
    fn rejects_other_forms() {
        for text in ["12abff", "#12abf", "#12abffz0", "#+1abff"] {
            assert!(text.parse::<Color>().is_err(), "{text}");
        }
    }
}
