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

The files are unmodified copies from the `ofl/` folders of
[google/fonts](https://github.com/google/fonts) at commit
`66928797f271fed5ee428544e7229685e2d90127`. The OFL lets us bundle and embed them, and
forbids selling them on their own. A Map file embeds the fonts it uses (ADR 0005).

To add a family, put its files and `OFL.txt` in a new folder here, list it in
`crates/fonts/src/lib.rs`, and add it to `LABEL_FAMILIES` unless it is a fallback.
