//! Writes a blank Map file, for checking that installed packages open one:
//! `cargo run -p ssmap-core --example new_map -- <path>.ssmap`

use std::path::PathBuf;

use ssmap_core::file::MapFile;
use ssmap_core::{Canvas, DistanceUnit, Map, Scale};

fn main() {
    let Some(path) = std::env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("usage: new_map <path>.ssmap");
        std::process::exit(2);
    };
    let map = Map::new(Canvas::new(800, 600, Scale::new(1.0, DistanceUnit::Miles)));
    if let Err(err) = MapFile::create(&path, &map, None) {
        eprintln!("couldn't write {}: {err}", path.display());
        std::process::exit(1);
    }
}
