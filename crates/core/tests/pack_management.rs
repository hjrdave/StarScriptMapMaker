//! Installing and deleting user packs, built-in protection, and usage counts.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests and benches may panic on failure"
)]

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use ssmap_core::asset::{DeleteError, InstallError, PackLibrary, PackRoots};
use ssmap_core::{
    Canvas, DistanceUnit, Id, Layer, Map, Node, Object, ObjectKind, Op, Scale, Stamp, Subtree,
};

fn write_pack(dir: &Path, id: &str) {
    std::fs::create_dir_all(dir.join("art")).unwrap();
    let manifest = format!(
        "[pack]\nid = \"{id}\"\nname = \"Pack {id}\"\nlicence = \"CC0-1.0\"\ncredits = \"Test credits\"\n\
         modified = \"2026-01-01\"\nauthor = \"A\"\n\n[[asset]]\nid = \"one\"\nname = \"One\"\n\
         category = \"c\"\nfile = \"art/one.svg\"\nanchor = [0.5, 0.5]\nwidth = 8\n"
    );
    std::fs::write(dir.join("pack.toml"), manifest).unwrap();
    std::fs::write(dir.join("art/one.svg"), "<svg/>").unwrap();
}

struct Setup {
    _tmp: tempfile::TempDir,
    roots: PackRoots,
    incoming: PathBuf,
}

fn setup() -> (Setup, PackLibrary) {
    let tmp = tempfile::tempdir().unwrap();
    let built_in = tmp.path().join("builtin");
    let user = tmp.path().join("user");
    write_pack(&built_in.join("core"), "core");
    let roots = PackRoots {
        built_in,
        user: Some(user),
    };
    let (library, problems) = PackLibrary::load(&roots);
    assert_eq!(problems, []);
    let incoming = tmp.path().join("incoming");
    (
        Setup {
            _tmp: tmp,
            roots,
            incoming,
        },
        library,
    )
}

#[test]
fn installing_copies_the_pack_and_reports_progress() {
    let (s, mut library) = setup();
    write_pack(&s.incoming, "mine");
    let steps = RefCell::new(Vec::new());
    let pack = library
        .install_pack(&s.incoming, &s.roots, |p| steps.borrow_mut().push(p))
        .unwrap();
    assert_eq!(pack.id, "mine");
    assert!(!pack.is_built_in());
    assert_eq!(pack.asset_count(), 1);
    let user = s.roots.user.as_ref().unwrap();
    assert!(user.join("mine/art/one.svg").is_file());
    assert!(library.pack("mine").is_some());
    let steps = steps.into_inner();
    let last = steps.last().unwrap();
    assert_eq!(last.copied, last.total);
    assert!(last.total > 0);
    let (reloaded, problems) = PackLibrary::load(&s.roots);
    assert_eq!(problems, []);
    assert!(reloaded.pack("mine").is_some());
}

#[test]
fn install_failures_say_why_and_leave_nothing_behind() {
    let (s, mut library) = setup();
    let user = s.roots.user.clone().unwrap();

    let file = s.incoming.with_extension("zip");
    std::fs::write(&file, "zip").unwrap();
    assert!(matches!(
        library.install_pack(&file, &s.roots, |_| {}),
        Err(InstallError::UnsupportedFormat(_))
    ));

    std::fs::create_dir_all(&s.incoming).unwrap();
    assert!(matches!(
        library.install_pack(&s.incoming, &s.roots, |_| {}),
        Err(InstallError::MissingManifest(_))
    ));

    std::fs::write(s.incoming.join("pack.toml"), "not toml [").unwrap();
    assert!(matches!(
        library.install_pack(&s.incoming, &s.roots, |_| {}),
        Err(InstallError::InvalidManifest(_))
    ));

    write_pack(&s.incoming, "core");
    let err = library
        .install_pack(&s.incoming, &s.roots, |_| {})
        .unwrap_err();
    assert!(matches!(err, InstallError::DuplicateId { .. }), "{err}");
    assert!(err.to_string().contains("`core`"));

    assert!(!user.exists() || std::fs::read_dir(&user).unwrap().next().is_none());
    assert_eq!(library.packs().len(), 1);
}

#[test]
fn user_packs_are_trashed_and_removed() {
    let (s, mut library) = setup();
    write_pack(&s.incoming, "mine");
    library.install_pack(&s.incoming, &s.roots, |_| {}).unwrap();
    assert!(library.can_delete("mine", &s.roots));
    let trashed = RefCell::new(None);
    library
        .delete_pack("mine", &s.roots, |dir| {
            *trashed.borrow_mut() = Some(dir.to_path_buf());
            std::fs::remove_dir_all(dir).map_err(Into::into)
        })
        .unwrap();
    assert!(trashed.into_inner().unwrap().ends_with("mine"));
    assert!(library.pack("mine").is_none());
    assert!(matches!(
        library.delete_pack("mine", &s.roots, |_| Ok(())),
        Err(DeleteError::NotFound(_))
    ));
}

#[test]
fn a_failed_trash_keeps_the_pack() {
    let (s, mut library) = setup();
    write_pack(&s.incoming, "mine");
    library.install_pack(&s.incoming, &s.roots, |_| {}).unwrap();
    let err = library
        .delete_pack("mine", &s.roots, |_| Err("no trash".into()))
        .unwrap_err();
    assert!(matches!(&err, DeleteError::Trash(cause) if cause.to_string() == "no trash"));
    assert_eq!(
        err.to_string(),
        "cannot move the pack to the Trash: no trash"
    );
    assert!(library.pack("mine").is_some());
}

#[test]
fn built_in_packs_cannot_be_deleted() {
    let (s, mut library) = setup();
    assert!(library.pack("core").unwrap().is_built_in());
    assert!(!library.can_delete("core", &s.roots));
    let err = library
        .delete_pack("core", &s.roots, |_| {
            panic!("must not trash a built-in pack")
        })
        .unwrap_err();
    assert!(matches!(&err, DeleteError::BuiltIn(id) if id == "core"));
    assert!(library.pack("core").is_some());
}

#[test]
fn a_user_pack_inside_the_built_in_folder_cannot_be_deleted() {
    let (s, _) = setup();
    // Loaded as a user pack, but it lives in the real built-in folder.
    let (mut user_loaded, _) = PackLibrary::load(&PackRoots {
        built_in: s.roots.built_in.join("nowhere"),
        user: Some(s.roots.built_in.clone()),
    });
    assert!(!user_loaded.pack("core").unwrap().is_built_in());
    let err = user_loaded
        .delete_pack("core", &s.roots, |_| panic!("must not trash"))
        .unwrap_err();
    assert!(matches!(&err, DeleteError::BuiltIn(id) if id == "core"));
}

#[test]
fn usage_counts_stamps_of_the_pack() {
    let (_s, library) = setup();
    let mut map = Map::new(Canvas::new(64, 64, Scale::new(1.0, DistanceUnit::Miles)));
    let layer = Id::new();
    map.apply(Op::Insert {
        parent: None,
        index: 0,
        subtree: Subtree {
            id: layer,
            node: Node::Layer(Layer::new("L")),
            children: Vec::new(),
        },
    })
    .unwrap();
    for asset in ["core/one", "core/one", "other/two"] {
        let index = map.children(Some(layer)).len();
        map.apply(Op::Insert {
            parent: Some(layer),
            index,
            subtree: Subtree {
                id: Id::new(),
                node: Node::Object(Object::new(ObjectKind::Stamp(Stamp::new(
                    asset.parse().unwrap(),
                    [0.0, 0.0],
                )))),
                children: Vec::new(),
            },
        })
        .unwrap();
    }
    assert_eq!(library.pack_usage(&map, "core"), 2);
    assert_eq!(library.pack_usage(&map, "other"), 1);
    assert_eq!(library.pack_usage(&map, "none"), 0);
    assert_eq!(library.pack_assets_used(&map, "core").len(), 1);
}
