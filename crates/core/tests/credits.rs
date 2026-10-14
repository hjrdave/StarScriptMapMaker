//! The credits query and per-pack credit entries.

use std::path::{Path, PathBuf};

use ssmap_core::asset::{PackLibrary, PackSource, parse_manifest};
use ssmap_core::{
    Canvas, DistanceUnit, Id, Layer, Map, Node, Object, ObjectKind, Op, Scale, Stamp, Subtree,
};

fn credit_pack(id: &str, attribution_required: bool, authors: &[(&str, &str)]) -> String {
    let mut text = format!(
        "[pack]\nid = \"{id}\"\nname = \"Pack {id}\"\nlicence = \"CC-BY-3.0\"\n\
         licence_url = \"https://creativecommons.org/licenses/by/3.0/\"\n\
         credits = \"Credit line for {id}.\"\nattribution_required = {attribution_required}\n\
         modified = \"2026-01-01\"\n"
    );
    for (asset, author) in authors {
        text.push_str(&format!(
            "\n[[asset]]\nid = \"{asset}\"\nname = \"Asset {asset}\"\ncategory = \"c\"\n\
             file = \"{asset}.svg\"\nanchor = [0.5, 0.5]\nwidth = 8\nauthor = \"{author}\"\n\
             source_url = \"https://example.com/{asset}\"\n"
        ));
    }
    text
}

fn library_of(manifests: &[String]) -> PackLibrary {
    let mut library = PackLibrary::default();
    let mut problems = Vec::new();
    for text in manifests {
        let pack = parse_manifest(Path::new("p/pack.toml"), text, PackSource::BuiltIn).unwrap();
        library.add(pack, &mut problems);
    }
    assert_eq!(problems, []);
    library
}

fn place(map: &mut Map, parent: Id, asset: &str) {
    let subtree = Subtree {
        id: Id::new(),
        node: Node::Object(Object::new(ObjectKind::Stamp(Stamp::new(
            asset.parse().unwrap(),
            [0.0, 0.0],
        )))),
        children: Vec::new(),
    };
    let index = map.children(Some(parent)).len();
    map.apply(Op::Insert {
        parent: Some(parent),
        index,
        subtree,
    })
    .unwrap();
}

#[test]
fn the_query_returns_the_attributed_assets_a_map_uses() {
    let library = library_of(&[
        credit_pack(
            "icons",
            true,
            &[("sword", "Lorc"), ("shield", "Delapouite")],
        ),
        credit_pack("free", false, &[("tree", "Someone")]),
    ]);
    let mut map = Map::new(Canvas::new(64, 64, Scale::new(1.0, DistanceUnit::Miles)));
    let layer = Id::new();
    map.apply(Op::Insert {
        parent: None,
        index: 0,
        subtree: Subtree {
            id: layer,
            node: Node::Layer(Layer::new("Stamps")),
            children: Vec::new(),
        },
    })
    .unwrap();
    assert_eq!(library.attribution_required_assets(&map), []);

    for asset in [
        "icons/sword",
        "free/tree",
        "icons/sword",
        "unloaded/thing",
        "icons/missing",
    ] {
        place(&mut map, layer, asset);
    }
    let used = library.attribution_required_assets(&map);
    assert_eq!(used.len(), 1, "{used:#?}");
    let sword = &used[0];
    assert_eq!(sword.reference.to_string(), "icons/sword");
    assert_eq!(sword.author, "Lorc");
    assert_eq!(sword.licence, "CC-BY-3.0");
    assert_eq!(sword.pack_name, "Pack icons");
    assert_eq!(sword.credits, "Credit line for icons.");
    assert_eq!(
        sword.source_url.as_deref(),
        Some("https://example.com/sword")
    );
    assert_eq!(
        sword.licence_url.as_deref(),
        Some("https://creativecommons.org/licenses/by/3.0/")
    );

    place(&mut map, layer, "icons/shield");
    let refs: Vec<String> = library
        .attribution_required_assets(&map)
        .iter()
        .map(|a| a.reference.to_string())
        .collect();
    assert_eq!(refs, ["icons/shield", "icons/sword"]);
}

#[test]
fn pack_credits_list_every_distinct_author() {
    let library = library_of(&[credit_pack(
        "icons",
        true,
        &[("a", "Lorc"), ("b", "Delapouite"), ("c", "Lorc")],
    )]);
    let credits = library.credits();
    assert_eq!(credits.len(), 1);
    assert_eq!(credits[0].authors, ["Delapouite", "Lorc"]);
    assert_eq!(credits[0].licences, ["CC-BY-3.0"]);
    assert_eq!(credits[0].sources.len(), 3);
    assert!(credits[0].attribution_required);
}

#[test]
fn the_game_icons_pack_manifest_is_valid_and_flags_attribution() {
    let dir =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/optional-packs/game-icons");
    let file = dir.join("pack.toml");
    let text = std::fs::read_to_string(&file).unwrap();
    let pack = parse_manifest(&file, &text, PackSource::User).unwrap();
    assert_eq!(pack.id, "game-icons");
    assert!(pack.attribution_required);
    assert_eq!(pack.licence, "CC-BY-3.0");
    assert_eq!(
        pack.licence_url.as_deref(),
        Some("https://creativecommons.org/licenses/by/3.0/")
    );
    assert!(pack.credits.contains("https://game-icons.net"));
}
