//! `luma-wallpaper unpack <scene.pkg> <папка>`: распаковка архива, .tex сохраняются как .png.
use crate::{pkg::Pkg, tex};
use anyhow::Result;
use std::{fs, path::Path};

fn safe(name: &str) -> bool {
    !name.is_empty() && !name.contains("..") && !name.contains(':') && !name.starts_with('/') && !name.starts_with('\\')
}

pub fn run(pkg: &Path, out: &Path) -> Result<()> {
    let p = Pkg::parse(fs::read(pkg)?)?;
    let (mut ok, mut raw, mut skipped) = (0, 0, 0);
    for e in &p.entries {
        if !safe(&e.name) {
            skipped += 1;
            continue;
        }
        let dest = out.join(&e.name);
        if let Some(dir) = dest.parent() {
            fs::create_dir_all(dir)?;
        }
        let data = p.bytes(e);
        if e.name.ends_with(".tex") {
            match tex::decode(data) {
                Ok(img) => {
                    image::save_buffer(dest.with_extension("png"), &img.rgba, img.width, img.height, image::ColorType::Rgba8)?;
                    ok += 1;
                    continue;
                }
                Err(err) => println!("{}: {err} (сохранён как есть)", e.name),
            }
        }
        fs::write(&dest, data)?;
        raw += 1;
    }
    println!("Готово: {} файлов в архиве, текстур в PNG: {ok}, сохранено как есть: {raw}, пропущено: {skipped}", p.entries.len());
    Ok(())
}
