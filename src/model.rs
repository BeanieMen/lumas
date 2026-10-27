use ahash::AHashMap;
use rayon::prelude::*;
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    io::Write,
    path::{Path, PathBuf},
};

pub const PAGE_CACHE: &str = "public, max-age=0, must-revalidate";
pub const STATIC_CACHE: &str = "public, max-age=0, must-revalidate";
pub const MIN_COMPRESS: usize = 256;

// ───────────────────────── prebuilt assets ─────────────────────────

pub struct Blob {
    pub bytes: &'static [u8], // status line + headers + body, ready to send
    pub head: usize,          // length of status line + headers (for HEAD)
}

pub struct Asset {
    pub plain: Blob,
    pub gzip: Option<Blob>,
    pub br: Option<Blob>,
    pub etag: &'static [u8],
    pub not_modified: &'static [u8],
}

pub struct Site {
    pub map: AHashMap<Box<str>, &'static Asset>,
}

pub struct Raw {
    pub keys: Vec<String>,
    pub body: Vec<u8>,
    pub content_type: String,
    pub compress: bool,
    pub cache: &'static str,
}

pub fn leak(v: Vec<u8>) -> &'static [u8] {
    Box::leak(v.into_boxed_slice())
}

pub fn gzip_bytes(data: &[u8]) -> Vec<u8> {
    let mut gz = flate2::write::GzEncoder::new(
        Vec::with_capacity(data.len() / 2),
        flate2::Compression::best(),
    );
    gz.write_all(data).unwrap();
    gz.finish().unwrap()
}

pub fn brotli_bytes(data: &[u8]) -> Vec<u8> {
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

pub fn build(body: &[u8], ct: &str, cache: &str, etag: &str, enc: Option<&str>, vary: bool) -> Blob {
    let mut v = Vec::with_capacity(body.len() + 256);
    write!(
        v,
        "HTTP/1.1 200 OK\r\ncontent-type: {ct}\r\ncontent-length: {}\r\ncache-control: {cache}\r\netag: {etag}\r\nx-content-type-options: nosniff\r\n",
        body.len()
    )
    .unwrap();
    if vary {
        v.extend_from_slice(b"vary: accept-encoding\r\n");
    }
    if let Some(e) = enc {
        write!(v, "content-encoding: {e}\r\n").unwrap();
    }
    v.extend_from_slice(b"\r\n");
    let head = v.len();
    v.extend_from_slice(body);
    Blob { bytes: leak(v), head }
}

impl Asset {
    pub fn new(body: Vec<u8>, ct: String, compress: bool, cache: &str) -> Self {
        let etag = {
            let mut h = DefaultHasher::new();
            body.hash(&mut h);
            format!("W/\"{:016x}\"", h.finish())
        };

        // Keep a compressed variant only if it is actually smaller.
        let (gz, br) = if compress && body.len() >= MIN_COMPRESS {
            (
                Some(gzip_bytes(&body)).filter(|c| c.len() < body.len()),
                Some(brotli_bytes(&body)).filter(|c| c.len() < body.len()),
            )
        } else {
            (None, None)
        };
        let vary = gz.is_some() || br.is_some();

        let plain = build(&body, &ct, cache, &etag, None, vary);
        let gzip = gz.map(|b| build(&b, &ct, cache, &etag, Some("gzip"), vary));
        let br = br.map(|b| build(&b, &ct, cache, &etag, Some("br"), vary));

        let mut nm = Vec::new();
        write!(nm, "HTTP/1.1 304 Not Modified\r\netag: {etag}\r\ncache-control: {cache}\r\n").unwrap();
        if vary {
            nm.extend_from_slice(b"vary: accept-encoding\r\n");
        }
        nm.extend_from_slice(b"\r\n");

        Self {
            plain,
            gzip,
            br,
            etag: leak(etag.into_bytes()),
            not_modified: leak(nm),
        }
    }
}

pub fn compressible(m: &str) -> bool {
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

pub fn content_type(path: &Path) -> (String, bool) {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let m = mime.essence_str();
    let ct = if m.starts_with("text/") || m.ends_with("javascript") {
        format!("{m}; charset=utf-8")
    } else {
        m.to_owned()
    };
    (ct, compressible(m))
}

pub fn walk(dir: &Path, base: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        // Never serve dotfiles (.git, .env, .DS_Store ...).
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

        let base = Path::new("static");
        let mut files = Vec::new();
        walk(base, base, &mut files);
        for (key, path) in files {
            let Ok(body) = std::fs::read(&path) else { continue };
            let (ct, compress) = content_type(&path);
            raws.push(Raw { keys: vec![key], body, content_type: ct, compress, cache: STATIC_CACHE });
        }

        // Templates are rendered ONCE at startup and win key conflicts. Errors fail fast.
        if let Ok(entries) = std::fs::read_dir("templates") {
            let env = crate::view::create_env();

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
                    content_type: "text/html; charset=utf-8".to_string(),
                    compress: true,
                    cache: PAGE_CACHE,
                });
            }
        }

        // Compress in parallel (brotli q11 is slow; this keeps startup short).
        let built: Vec<(Vec<String>, &'static Asset)> = raws
            .into_par_iter()
            .map(|r| {
                let a: &'static Asset =
                    Box::leak(Box::new(Asset::new(r.body, r.content_type, r.compress, r.cache)));
                (r.keys, a)
            })
            .collect();

        let mut map = AHashMap::new();
        for (keys, a) in built {
            for k in keys {
                map.insert(k.into_boxed_str(), a);
            }
        }
        Site { map }
    }

    pub fn lookup(&self, path: &str) -> Option<&'static Asset> {
        if let Some(a) = self.map.get(path) {
            return Some(*a);
        }
        // Cold path: percent-encoded file names ("my%20file.png").
        if path.contains('%') {
            let d = percent_encoding::percent_decode_str(path).decode_utf8().ok()?;
            return self.map.get(&*d).copied();
        }
        None
    }
}
