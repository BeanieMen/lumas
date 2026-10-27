use bytes::Bytes;
use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    io::Write,
    path::Path,
};

#[derive(Clone)]
pub struct Asset {
    pub content_type: &'static str,
    pub etag: String,
    pub plain: Bytes,
    pub gzip: Option<Bytes>,
    pub br: Option<Bytes>,
}

impl Asset {
    pub fn new(data: Vec<u8>, content_type: &'static str, compress: bool) -> Self {
        let etag = format!("W/\"{:016x}\"", {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            data.hash(&mut h);
            h.finish()
        });

        let (gzip, br) = if compress && data.len() >= 256 {
            let gz = gzip_compress(&data)
                .filter(|c| c.len() < data.len())
                .map(Bytes::from);
            let br = brotli_compress(&data)
                .filter(|c| c.len() < data.len())
                .map(Bytes::from);
            (gz, br)
        } else {
            (None, None)
        };

        Self {
            content_type,
            etag,
            plain: Bytes::from(data),
            gzip,
            br,
        }
    }
}

fn gzip_compress(data: &[u8]) -> Option<Vec<u8>> {
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    gz.write_all(data).ok()?;
    gz.finish().ok()
}

fn brotli_compress(data: &[u8]) -> Option<Vec<u8>> {
    let params = brotli::enc::BrotliEncoderParams {
        quality: 11,
        lgwin: 22,
        size_hint: data.len(),
        ..Default::default()
    };
    let mut out = Vec::with_capacity(data.len() / 2);
    brotli::BrotliCompress(&mut &data[..], &mut out, &params).ok()?;
    Some(out)
}

pub fn guess_mime(path: &Path) -> &'static str {
    match path.extension().and_then(|s| s.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "ttf" => "font/ttf",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "ico" => "image/x-icon",
        _ => "application/octet-stream",
    }
}

pub struct Site {
    pub pages: HashMap<String, Asset>,
}

fn walk_static(dir: &Path, base: &Path, pages: &mut HashMap<String, Asset>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_static(&path, base, pages);
        } else if path.is_file() {
            let Ok(rel) = path.strip_prefix(base) else {
                continue;
            };
            let key = format!("/{}", rel.to_string_lossy());
            if let Ok(bytes) = std::fs::read(&path) {
                let mime = guess_mime(&path);
                let compress = mime.starts_with("text/")
                    || mime.contains("javascript")
                    || mime.contains("font")
                    || mime.contains("svg")
                    || mime.contains("json");
                pages.insert(key, Asset::new(bytes, mime, compress));
            }
        }
    }
}

impl Site {
    pub fn load() -> Self {
        let mut pages = HashMap::new();

        walk_static(Path::new("static"), Path::new("static"), &mut pages);

        if let Ok(entries) = std::fs::read_dir("templates") {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) != Some("html") {
                    continue;
                }
                let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                    continue;
                };
                if name == "base.html" || name.starts_with('_') {
                    continue;
                }
                let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                    continue;
                };

                if let Ok(html) = crate::view::render(name, false) {
                    let asset = Asset::new(html.into_bytes(), "text/html; charset=utf-8", true);
                    if stem == "index" {
                        pages.insert("/".into(), asset.clone());
                        pages.insert("/index".into(), asset.clone());
                        pages.insert("/index.html".into(), asset);
                    } else {
                        pages.insert(format!("/{stem}"), asset.clone());
                        pages.insert(format!("/{stem}.html"), asset);
                    }
                }
            }
        }

        Site { pages }
    }
}
