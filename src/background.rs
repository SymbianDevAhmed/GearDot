//! Local import and display of the Devices background.
use eframe::egui;
use image::{DynamicImage, ImageFormat, ImageReader, Limits};
use std::{
    fs,
    io::{self, Cursor, Read},
    path::Path,
};
use wireless_battery_monitor::preferences::{data_dir, local_id};

const MAX_FILE_BYTES: u64 = 24 * 1024 * 1024;

fn decode(path: &Path) -> io::Result<DynamicImage> {
    let mut data = Vec::new();
    fs::File::open(path)?
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut data)?;
    if data.len() as u64 > MAX_FILE_BYTES {
        return Err(io::Error::other("Images must be smaller than 24 MB."));
    }
    let mut reader = ImageReader::new(Cursor::new(data)).with_guessed_format()?;
    if !matches!(reader.format(), Some(ImageFormat::Png | ImageFormat::Jpeg)) {
        return Err(io::Error::other("Choose a PNG or JPEG image."));
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    reader.decode().map_err(io::Error::other)
}

pub fn choose(frame: &eframe::Frame) -> io::Result<Option<String>> {
    let Some(path) = crate::runtime::choose_file(
        frame,
        "Image files (*.png;*.jpg;*.jpeg)\0*.png;*.jpg;*.jpeg\0\0",
    )?
    else {
        return Ok(None);
    };
    let image = decode(&path)?.thumbnail(1600, 1600);
    let mut encoded = Cursor::new(Vec::new());
    image
        .write_to(&mut encoded, ImageFormat::Png)
        .map_err(io::Error::other)?;
    let id = local_id("", encoded.get_ref());
    let dir = data_dir()?.join("backgrounds");
    fs::create_dir_all(&dir)?;
    let target = dir.join(format!("{id}.png"));
    if !target.exists() {
        let temporary = dir.join(format!("{id}.{}.tmp", std::process::id()));
        let result =
            fs::write(&temporary, encoded.get_ref()).and_then(|()| fs::rename(&temporary, target));
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result?;
    }
    Ok(Some(id))
}

pub fn texture(ctx: &egui::Context, id: &str) -> io::Result<egui::TextureHandle> {
    let image = decode(&data_dir()?.join("backgrounds").join(format!("{id}.png")))?.to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    Ok(ctx.load_texture(
        "devices-background",
        egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw()),
        egui::TextureOptions::LINEAR,
    ))
}
