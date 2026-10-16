# StarScript Map Maker

Read `CONTEXT.md` for the domain language and `docs/adr/` for architecture decisions.

## Design standard

All UI work must follow `docs/design/DESIGN-STANDARD.md`. Read it before adding or changing any UI (widgets, layout, colours, spacing, icons, copy, behaviour of UI elements), and look at the matching screen in `docs/design/ui-handoff/screens/` when one exists.

- Use the `Palette` in `crates/app/src/theme.rs` and the spacing, radius and elevation scales. Do not hard-code colours or sizes.
- Use the domain vocabulary in UI copy.
- Code reviewers: check UI changes against the design standard and its checklist, and request changes for deviations.

### Keeping the design standard current

When the owner asks for a design change (layout, colour, spacing, type, icon, copy, or the behaviour of a UI element), update `docs/design/DESIGN-STANDARD.md` in the same change:

1. Edit the relevant section so it states the new rule.
2. Add or update a row in "Decisions since the handoff", with the issue or PR number.
3. If a token changed, also update `docs/design/ui-handoff/tokens.json` and `crates/app/src/theme.rs`.
4. Do not edit the mockup HTML or PNGs; the standard overrides them.

If an implementation task conflicts with the design standard, follow the standard and say so in the PR. If the owner's request itself conflicts with the standard, the request wins: update the standard as above.

## File format compatibility

Backward compatibility of the `.ssmap` map format (and template files) is **not required** while the owner is the only user and the format is still being perfected. When a change needs new fields or a different shape, change the format and bump `format_version`. Do not write migrations for older files, and do not keep old code paths just to open them. Remove this section once the app has outside users.
