//! Generates Silhouettes for an Asset pack's outline art (ADR 0003).
//!
//! ```text
//! cargo run -p ssmap-render --bin silhouettes -- <pack-folder> [--size PX] [--gap PX] [--force]
//! ```
//!
//! For each Asset without a hand-authored Silhouette, rasterises its art with the longer side
//! `--size` pixels (default 512; PNG art keeps its own size), fills it inward from the outer
//! edge, writes `silhouettes/<asset-id>.png`, and sets the Asset's `silhouette` field in
//! `pack.toml`, keeping the rest of the file as written. A `silhouette` naming any other file
//! is hand-authored and left alone. Existing generated files are kept unless `--force` is given.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use ssmap_core::asset::{PACK_MANIFEST, PackSource, parse_manifest};
use ssmap_render::raster::Art;
use ssmap_render::silhouette;

const USAGE: &str = "usage: silhouettes <pack-folder> [--size PX] [--gap PX] [--force]";

struct Args {
    pack: PathBuf,
    size: u32,
    gap: u32,
    force: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut args = std::env::args().skip(1);
    let mut pack = None;
    let (mut size, mut gap, mut force) = (512, 2, false);
    while let Some(arg) = args.next() {
        let mut number = |name: &str| -> Result<u32, String> {
            args.next()
                .and_then(|value| value.parse().ok())
                .filter(|value| *value > 0 || name == "--gap")
                .ok_or_else(|| format!("{name} needs a whole number of pixels"))
        };
        match arg.as_str() {
            "--size" => size = number("--size")?,
            "--gap" => gap = number("--gap")?,
            "--force" => force = true,
            "-h" | "--help" => return Err(USAGE.into()),
            _ if arg.starts_with('-') => return Err(format!("unknown option {arg}\n{USAGE}")),
            _ if pack.is_none() => pack = Some(PathBuf::from(arg)),
            _ => return Err(USAGE.into()),
        }
    }
    Ok(Args {
        pack: pack.ok_or(USAGE)?,
        size,
        gap,
        force,
    })
}

fn main() -> ExitCode {
    match parse_args().and_then(|args| run(&args)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &Args) -> Result<(), String> {
    let manifest_path = args.pack.join(PACK_MANIFEST);
    let text = std::fs::read_to_string(&manifest_path)
        .map_err(|err| format!("{}: cannot be read: {err}", manifest_path.display()))?;
    let pack =
        parse_manifest(&manifest_path, &text, PackSource::User).map_err(|e| e.to_string())?;
    let mut document: toml_edit::DocumentMut = text
        .parse()
        .map_err(|err| format!("{}: {err}", manifest_path.display()))?;

    let mut edited = false;
    for asset in &pack.assets {
        let generated = PathBuf::from(format!("silhouettes/{}.png", asset.id));
        match &asset.silhouette {
            Some(path) if *path != generated => {
                println!("{}: hand-authored Silhouette, skipped", asset.id);
                continue;
            }
            Some(_) if !args.force && args.pack.join(&generated).is_file() => {
                println!("{}: up to date", asset.id);
                continue;
            }
            _ => {}
        }
        let art = Art::load(&pack.path(&asset.file)).map_err(|err| {
            let mut message = err.to_string();
            let mut cause = std::error::Error::source(&err);
            while let Some(inner) = cause {
                message.push_str(&format!(": {inner}"));
                cause = inner.source();
            }
            message
        })?;
        let raster = art.rasterise_fit(args.size);
        let mask = silhouette::generate(&raster, args.gap);
        write_png(&args.pack.join(&generated), &mask)?;
        println!(
            "{}: wrote {} ({}x{})",
            asset.id,
            generated.display(),
            mask.width,
            mask.height
        );
        if asset.silhouette.is_none() {
            set_silhouette(&mut document, &asset.id, &generated);
            edited = true;
        }
    }
    if edited {
        std::fs::write(&manifest_path, document.to_string())
            .map_err(|err| format!("{}: cannot be written: {err}", manifest_path.display()))?;
        println!("updated {}", manifest_path.display());
    }
    Ok(())
}

fn write_png(path: &Path, mask: &ssmap_render::raster::Raster) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|err| format!("{}: cannot be created: {err}", dir.display()))?;
    }
    image::save_buffer(
        path,
        &mask.unpremultiplied(),
        mask.width,
        mask.height,
        image::ExtendedColorType::Rgba8,
    )
    .map_err(|err| format!("{}: cannot be written: {err}", path.display()))
}

fn set_silhouette(document: &mut toml_edit::DocumentMut, id: &str, path: &Path) {
    let assets = document
        .get_mut("asset")
        .and_then(toml_edit::Item::as_array_of_tables_mut)
        .expect("the manifest parsed, so it has [[asset]] tables");
    for table in assets.iter_mut() {
        if table.get("id").and_then(toml_edit::Item::as_str) == Some(id) {
            let value = path
                .to_str()
                .expect("asset ids are ASCII")
                .replace('\\', "/");
            table.insert("silhouette", toml_edit::value(value));
        }
    }
}
