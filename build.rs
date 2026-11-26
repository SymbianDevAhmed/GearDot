// Generate a multi-resolution Windows icon from the project's original vector emblem.
use resvg::{
    tiny_skia::{Pixmap, Transform},
    usvg,
};
use std::{env, fs, io, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=assets/icon.svg");
    println!("cargo:rerun-if-changed=build.rs");
    let out = PathBuf::from(env::var_os("OUT_DIR").ok_or("OUT_DIR missing")?);
    let tree = usvg::Tree::from_str(include_str!("assets/icon.svg"), &usvg::Options::default())?;
    let sizes = [16u32, 24, 32, 48, 64, 128, 256];
    let mut images = Vec::new();
    for size in sizes {
        let mut pixmap = Pixmap::new(size, size).ok_or("icon allocation failed")?;
        let scale = size as f32 / 256.0;
        resvg::render(
            &tree,
            Transform::from_scale(scale, scale),
            &mut pixmap.as_mut(),
        );
        // PNG payloads are supported by modern Windows ICO files.
        images.push((size, pixmap.encode_png()?));
        if size == 64 {
            // egui expects straight-alpha RGBA, not tiny-skia's premultiplied pixels.
            let mut rgba = pixmap.data().to_vec();
            for pixel in rgba.chunks_exact_mut(4) {
                let alpha = u32::from(pixel[3]);
                if alpha != 0 {
                    for channel in &mut pixel[..3] {
                        *channel = ((u32::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
                    }
                }
            }
            fs::write(out.join("app-icon.rgba"), rgba)?;
        }
    }
    let mut ico = vec![0, 0, 1, 0];
    ico.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len() as u32;
    for (size, png) in &images {
        let dimension = if *size == 256 { 0 } else { *size as u8 };
        ico.extend_from_slice(&[dimension, dimension, 0, 0]);
        ico.extend_from_slice(&1u16.to_le_bytes());
        ico.extend_from_slice(&32u16.to_le_bytes());
        ico.extend_from_slice(&(png.len() as u32).to_le_bytes());
        ico.extend_from_slice(&offset.to_le_bytes());
        offset += png.len() as u32;
    }
    for (_, png) in images {
        ico.extend_from_slice(&png);
    }
    let icon_path = out.join("wireless-battery-monitor.ico");
    fs::write(&icon_path, ico)?;
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let path = icon_path
            .to_str()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid icon path"))?;
        let version = format!("{}.{}", env!("CARGO_PKG_VERSION"), 0);
        for (binary, description) in [
            (
                "wireless-battery-monitor",
                "Wireless Peripheral Battery Monitor",
            ),
            ("diagnostic", "Wireless Battery Monitor Diagnostic"),
        ] {
            let resource_dir = out.join(format!("resource-{binary}"));
            fs::create_dir_all(&resource_dir)?;
            let resource_dir = resource_dir.to_str().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "invalid resource directory")
            })?;
            winresource::WindowsResource::new()
                .set_output_directory(resource_dir)
                .set_icon(path)
                .set("ProductName", "Wireless Battery Monitor")
                .set("FileDescription", description)
                .set("CompanyName", "Ahmed Gaber")
                .set("LegalCopyright", "Copyright (c) 2026 Ahmed Gaber")
                .set("OriginalFilename", &format!("{binary}.exe"))
                .set("InternalName", binary)
                .set("FileVersion", &version)
                .set("ProductVersion", &version)
                .compile_for(binary)?;
        }
    }
    Ok(())
}
