# Labels use named Label styles and bundled fonts only

Status: accepted

A Label is an Object sized in Canvas units, so the screen matches the export. It stores:
- text, with explicit line breaks;
- position, alignment (left, centre, right) and rotation;
- font family and style, size, letter spacing, line height and a non-destructive all-caps flag;
- fill colour, opacity, and halo width and colour (the halo is drawn as a stroke).

Typography is driven by **named Label styles** stored in the Map ("Realm", "City", "Sea", "Range" in the template). Each Label references a style and may override any field, so a whole map restyles in one Command. We preferred this to a single Map-wide default because map typography is a hierarchy. Deleting a style in use copies its values into the affected Labels as overrides and moves them to the default style, in one Undo step, so nothing visibly changes.

v1 offers **bundled OFL fonts only**: Cinzel, IM Fell Double Pica, Almendra, Uncial Antiqua and MedievalSharp, plus Noto Serif as a fallback for characters the display fonts lack. We rejected system fonts because Maps embed their fonts (ADR 0005), and many commercial fonts forbid embedding. User and system fonts belong with the post-v1 custom-asset import effort.

## Consequences

- Text is edited in a box overlaid at the Label (UI font) with the real render shown live, or in the properties panel. Committing is one Command. True WYSIWYG editing on the Canvas is post-v1.
- v1 has no auto-placement and no attaching Labels to other Objects.
- Curved text arrives later as an optional baseline path, which is a minor file-version addition under ADR 0005. Nothing is reserved for it now.
