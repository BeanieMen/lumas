use ahash::AHashMap;
use axum::http::{HeaderMap, HeaderValue, header};
use bytes::Bytes;
use percent_encoding::percent_decode_str;
use rayon::prelude::*;
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    io::Write,
    path::{Path, PathBuf},
};
use tower_http::services::ServeDir;

pub const PAGE_CACHE: &str = "public, max-age=0, must-revalidate";
pub const STATIC_CACHE: &str = "public, max-age=0, must-revalidate";
pub const MIN_COMPRESS: usize = 256;
pub const MAX_IN_MEMORY: u64 = 16 << 20;

pub struct Variant {
    pub body: Bytes,
    pub headers: HeaderMap,
}

pub struct Asset {
    pub plain: Variant,
    pub gzip: Option<Variant>,
    pub br: Option<Variant>,
    pub etag: HeaderValue,
    pub not_modified: HeaderMap,
}

#[derive(Clone, Copy)]
pub enum Entry {
    Mem(&'static Asset),
    Disk,
}

pub struct Site {
    pub map: AHashMap<Box<str>, Entry>,
    pub disk: ServeDir,
}

struct Raw {
    keys: Vec<String>,
    body: Vec<u8>,
    content_type: HeaderValue,
    compress: bool,
    cache: &'static str,
}

pub fn hv(s: String) -> HeaderValue {
    HeaderValue::from_static(Box::leak(s.into_boxed_str()))
}

fn gzip_bytes(data: &[u8]) -> Vec<u8> {
    let mut gz = flate2::write::GzEncoder::new(
        Vec::with_capacity(data.len() / 2),
        flate2::Compression::best(),
    );
    gz.write_all(data).unwrap();
    gz.finish().unwrap()
}

fn brotli_bytes(data: &[u8]) -> Vec<u8> {
    let params = brotli::enc::BrotliEncoderParams {
        quality: 11,
        lgwin: 22,
        size_hint: data.len(),
        ..Default::default()
    };
    let mut out = Vec::with_capacity(data.len() / 2);
    brotli::BrotliCompress(&mut &data[..], &mut out, &params).expect("brotli failed");
    out
}

impl Asset {
    pub fn new(body: Vec<u8>, content_type: HeaderValue, compress: bool, cache: &'static str) -> Self {
        let etag = {
            let mut h = DefaultHasher::new();
            body.hash(&mut h);
            hv(format!("W/\"{:016x}\"", h.finish()))
        };
        let cache = HeaderValue::from_static(cache);

        let (gz, br) = if compress && body.len() >= MIN_COMPRESS {
            (
                Some(gzip_bytes(&body)).filter(|c| c.len() < body.len()),
                Some(brotli_bytes(&body)).filter(|c| c.len() < body.len()),
            )
        } else {
            (None, None)
        };
        let vary = gz.is_some() || br.is_some();

        let make = |bytes: Vec<u8>, enc: Option<&'static str>| {
            let mut h = HeaderMap::with_capacity(8);
            h.insert(header::CONTENT_TYPE, content_type.clone());
            h.insert(header::CONTENT_LENGTH, hv(bytes.len().to_string()));
            h.insert(header::CACHE_CONTROL, cache.clone());
            h.insert(header::ETAG, etag.clone());
            h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
            if vary {
                h.insert(header::VARY, HeaderValue::from_static("accept-encoding"));
            }
            if let Some(e) = enc {
                h.insert(header::CONTENT_ENCODING, HeaderValue::from_static(e));
            }
            Variant {
                body: Bytes::from_static(Box::leak(bytes.into_boxed_slice())),
                headers: h,
            }
        };

        let plain = make(body, None);
        let gzip = gz.map(|b| make(b, Some("gzip")));
        let br = br.map(|b| make(b, Some("br")));

        let mut not_modified = HeaderMap::with_capacity(3);
        not_modified.insert(header::ETAG, etag.clone());
        not_modified.insert(header::CACHE_CONTROL, cache.clone());
        if vary {
            not_modified.insert(header::VARY, HeaderValue::from_static("accept-encoding"));
        }

        Self { plain, gzip, br, etag, not_modified }
    }
}

fn compressible(m: &str) -> bool {
    m.starts_with("text/")
        || m.ends_with("javascript")
        || m.ends_with("json")
        || m.ends_with("xml")
        || matches!(
            m,
            "image/x-icon"
                | "image/vnd.microsoft.icon"
                | "image/bmp"
                | "application/wasm"
                | "font/ttf"
                | "font/otf"
                | "application/x-font-ttf"
                | "application/vnd.ms-fontobject"
        )
}

fn content_type(path: &Path) -> (HeaderValue, bool) {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let m = mime.essence_str();
    let ct = if m.starts_with("text/") || m.ends_with("javascript") {
        format!("{m}; charset=utf-8")
    } else {
        m.to_owned()
    };
    (hv(ct), compressible(m))
}

fn walk(dir: &Path, base: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        if e.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let p = e.path();
        if p.is_dir() {
            walk(&p, base, out);
        } else if p.is_file() {
            let Ok(rel) = p.strip_prefix(base) else { continue };
            let url = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            out.push((format!("/{url}"), p));
        }
    }
}

impl Site {
    pub fn load() -> Self {
        let mut raws: Vec<Raw> = Vec::new();
        let mut disk_keys: Vec<String> = Vec::new();

        let base = Path::new("static");
        let mut files = Vec::new();
        walk(base, base, &mut files);
        for (key, path) in files {
            let Ok(meta) = std::fs::metadata(&path) else { continue };
            if meta.len() > MAX_IN_MEMORY {
                disk_keys.push(key);
                continue;
            }
            let Ok(body) = std::fs::read(&path) else { continue };
            let (ct, compress) = content_type(&path);
            raws.push(Raw { keys: vec![key], body, content_type: ct, compress, cache: STATIC_CACHE });
        }

        if let Ok(entries) = std::fs::read_dir("templates") {
            let env = crate::view::create_env();
            let html_ct = HeaderValue::from_static("text/html; charset=utf-8");

            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) != Some("html") {
                    continue;
                }
                let Some(file_name) = path.file_name().and_then(|s| s.to_str()) else { continue };
                if file_name == "base.html" || file_name.starts_with('_') {
                    continue;
                }
                let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else { continue };

                let html = crate::view::render_template(&env, file_name);

                let keys = if stem == "index" {
                    vec!["/".to_string()]
                } else {
                    vec![format!("/{stem}"), format!("/{stem}.html")]
                };
                raws.push(Raw {
                    keys,
                    body: html.into_bytes(),
                    content_type: html_ct.clone(),
                    compress: true,
                    cache: PAGE_CACHE,
                });
            }
        }

        let built: Vec<(Vec<String>, &'static Asset)> = raws
            .into_par_iter()
            .map(|r| {
                let a: &'static Asset =
                    Box::leak(Box::new(Asset::new(r.body, r.content_type, r.compress, r.cache)));
                (r.keys, a)
            })
            .collect();

        let mut map: AHashMap<Box<str>, Entry> = AHashMap::new();
        for k in disk_keys {
            map.insert(k.into_boxed_str(), Entry::Disk);
        }
        for (keys, a) in built {
            for k in keys {
                map.insert(k.into_boxed_str(), Entry::Mem(a));
            }
        }

        Site {
            map,
            disk: ServeDir::new("static").precompressed_br().precompressed_gzip(),
        }
    }

    pub fn lookup(&self, path: &str) -> Option<Entry> {
        match self.map.get(path) {
            Some(e) => Some(*e),
            None if path.contains('%') => percent_decode_str(path)
                .decode_utf8()
                .ok()
                .and_then(|p| self.map.get(&*p).copied()),
            None => None,
        }
    }
}
