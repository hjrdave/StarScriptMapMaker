# Bundled fonts

The fonts Labels can use (ADR 0006), each under the SIL Open Font License 1.1 with its
`OFL.txt` beside it. The app embeds them through the `ssmap-fonts` crate
(`crates/fonts`), which is also how `cargo deny` checks their licence.

| Family | Folder | Role |
| --- | --- | --- |
| Cinzel | `cinzel/` | Display (variable weight) |
| IM Fell Double Pica | `imfelldoublepica/` | Display, with italic |
| Almendra | `almendra/` | Display, with italic and bold |
| Uncial Antiqua | `uncialantiqua/` | Display |
| MedievalSharp | `medievalsharp/` | Display |
| Noto Serif | `notoserif/` | Fallback for characters the display families lack |
| Inter | `Inter/` | UI proportional font, not a Label font. Source: Google Fonts download |
| JetBrains Mono | `JetBrains_Mono/` | UI monospace font, not a Label font. Source: Google Fonts download |

Inter and JetBrains Mono are registered as UI families (`UI_FAMILIES` in `crates/fonts`),
apart from the Label families, so the Label font list and the fonts a Map embeds do not
change. Their folders are the Google Fonts downloads as committed, with `OFL.txt`,
`README.txt`, the variable fonts and `static/`; the app embeds only
`Inter/static/Inter_18pt-Regular.ttf` and
`JetBrains_Mono/static/JetBrainsMono-Regular.ttf`. The rest stay in place.

The Label fonts above are unmodified copies from the `ofl/` folders of
[google/fonts](https://github.com/google/fonts) at commit
`66928797f271fed5ee428544e7229685e2d90127`. The OFL lets us bundle and embed them, and
forbids selling them on their own. A Map file embeds the fonts it uses (ADR 0005).

To add a family, put its files and `OFL.txt` in a new folder here, list it in
`crates/fonts/src/lib.rs`, and add it to `LABEL_FAMILIES` unless it is a fallback.
