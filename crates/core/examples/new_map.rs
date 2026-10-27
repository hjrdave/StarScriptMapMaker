//! Writes a blank Map file, for checking that installed packages open one:
//! `cargo run -p ssmap-core --example new_map -- <path>.ssmap`
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "existing invariant panics; replace with error handling over time (#351)"
)]

use std::path::PathBuf;

use ssmap_core::file::MapFile;
use ssmap_core::{Canvas, DistanceUnit, Map, Scale};

fn main() {
    let path = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .expect("usage: new_map <path>.ssmap"),
    );
    let map = Map::new(Canvas::new(800, 600, Scale::new(1.0, DistanceUnit::Miles)));
    if let Err(err) = MapFile::create(&path, &map, None) {
        eprintln!("couldn't write {}: {err}", path.display());
        std::process::exit(1);
    }
}
