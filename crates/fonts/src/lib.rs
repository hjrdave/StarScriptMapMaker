//! The fonts bundled with the app (ADR 0006), embedded so a Map renders the same everywhere.
//!
//! The files live in `assets/fonts/`, one folder per family with its `OFL.txt`. They are a
//! crate of their own so `cargo deny` checks their licence.

/// One bundled font file.
#[derive(Clone, Copy, Debug)]
pub struct BundledFont {
    /// The family name as Labels store it, such as "Cinzel".
    pub family: &'static str,
    /// The file name under `assets/fonts/<folder>/`.
    pub file: &'static str,
    pub data: &'static [u8],
}

/// One bundled family and its licence.
#[derive(Clone, Copy, Debug)]
pub struct BundledFamily {
    pub name: &'static str,
    /// The folder under `assets/fonts/`.
    pub folder: &'static str,
    /// The full text of its SIL Open Font License, with the copyright notice.
    pub licence: &'static str,
    /// Whether it supplies characters the display families lack, rather than being offered
    /// for Labels in its own right.
    pub fallback: bool,
}

/// The family used for characters a Label's own family lacks.
pub const FALLBACK_FAMILY: &str = "Noto Serif";

macro_rules! fonts {
    ($($family:literal in $folder:literal: [$($file:literal),+ $(,)?]),+ $(,)?) => {
        /// Every bundled font file.
        pub const FONTS: &[BundledFont] = &[$($(BundledFont {
            family: $family,
            file: $file,
            data: include_bytes!(concat!("../../../assets/fonts/", $folder, "/", $file)),
        },)+)+];

        /// Every bundled family, display families first in menu order, then the fallback.
        pub const FAMILIES: &[BundledFamily] = &[$(BundledFamily {
            name: $family,
            folder: $folder,
            licence: include_str!(concat!("../../../assets/fonts/", $folder, "/OFL.txt")),
            fallback: matches!($family.as_bytes(), b"Noto Serif"),
        },)+];
    };
}

fonts! {
    "Cinzel" in "cinzel": ["Cinzel[wght].ttf"],
    "IM Fell Double Pica" in "imfelldoublepica": [
        "IMFELLDoublePica-Regular.ttf",
        "IMFELLDoublePica-Italic.ttf",
    ],
    "Almendra" in "almendra": [
        "Almendra-Regular.ttf",
        "Almendra-Italic.ttf",
        "Almendra-Bold.ttf",
        "Almendra-BoldItalic.ttf",
    ],
    "Uncial Antiqua" in "uncialantiqua": ["UncialAntiqua-Regular.ttf"],
    "MedievalSharp" in "medievalsharp": ["MedievalSharp.ttf"],
    "Noto Serif" in "notoserif": ["NotoSerif[wdth,wght].ttf", "NotoSerif-Italic[wdth,wght].ttf"],
}

/// The family of the UI's proportional font.
pub const UI_PROPORTIONAL_FAMILY: &str = "Inter";
/// The family of the UI's monospace font.
pub const UI_MONOSPACE_FAMILY: &str = "JetBrains Mono";

/// The fonts the UI itself uses. Kept apart from [`FONTS`] and [`FAMILIES`] so the Label font
/// list, the Label font database and the fonts a Map embeds stay as ADR 0006 has them.
pub const UI_FONTS: &[BundledFont] = &[
    BundledFont {
        family: UI_PROPORTIONAL_FAMILY,
        file: "static/Inter_18pt-Regular.ttf",
        data: include_bytes!("../../../assets/fonts/Inter/static/Inter_18pt-Regular.ttf"),
    },
    BundledFont {
        family: UI_MONOSPACE_FAMILY,
        file: "static/JetBrainsMono-Regular.ttf",
        data: include_bytes!(
            "../../../assets/fonts/JetBrains_Mono/static/JetBrainsMono-Regular.ttf"
        ),
    },
];

/// The UI font families and their licences.
pub const UI_FAMILIES: &[BundledFamily] = &[
    BundledFamily {
        name: UI_PROPORTIONAL_FAMILY,
        folder: "Inter",
        licence: include_str!("../../../assets/fonts/Inter/OFL.txt"),
        fallback: false,
    },
    BundledFamily {
        name: UI_MONOSPACE_FAMILY,
        folder: "JetBrains_Mono",
        licence: include_str!("../../../assets/fonts/JetBrains_Mono/OFL.txt"),
        fallback: false,
    },
];

/// The families offered for Labels, in menu order.
pub const LABEL_FAMILIES: &[&str] = &[
    "Cinzel",
    "IM Fell Double Pica",
    "Almendra",
    "Uncial Antiqua",
    "MedievalSharp",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_family_has_fonts_and_an_ofl_licence() {
        for family in FAMILIES {
            assert!(
                FONTS.iter().any(|font| font.family == family.name),
                "{} has no font files",
                family.name
            );
            assert!(
                family
                    .licence
                    .contains("licensed under the SIL Open Font License, Version 1.1"),
                "{} is not under the OFL 1.1",
                family.name
            );
        }
        let (fallback, display): (Vec<&BundledFamily>, Vec<_>) =
            FAMILIES.iter().partition(|f| f.fallback);
        assert_eq!(
            fallback.iter().map(|f| f.name).collect::<Vec<_>>(),
            [FALLBACK_FAMILY]
        );
        assert_eq!(
            display.iter().map(|f| f.name).collect::<Vec<_>>(),
            LABEL_FAMILIES
        );
    }

    #[test]
    fn ui_fonts_are_not_label_fonts() {
        for family in UI_FAMILIES {
            assert!(!LABEL_FAMILIES.contains(&family.name));
            assert!(!FAMILIES.iter().any(|f| f.name == family.name));
            assert!(!FONTS.iter().any(|f| f.family == family.name));
            assert!(
                family
                    .licence
                    .contains("licensed under the SIL Open Font License, Version 1.1")
            );
            assert!(UI_FONTS.iter().any(|f| f.family == family.name));
        }
    }

    #[test]
    fn font_files_are_truetype() {
        for font in FONTS.iter().chain(UI_FONTS) {
            assert_eq!(
                &font.data[..4],
                b"\0\x01\0\0",
                "{} is not a TrueType font",
                font.file
            );
        }
    }
}
