//! 旧 PHP 版(`php -S`、ポート 4401)が配っていた静的ファイルの Rust 移植(2026-11-27)。
//! Static files formerly served by the legacy PHP server (port 4401), ported to Rust.
//!
//! 安全のため、配るのは決めたフォルダーの中の、決めた拡張子のファイルだけ。
//! `..`・ドット始まり・バックアップ(`~`・`.backup`・日付つき)・`.php` は配らない。
//! Only allow-listed folders and extensions; no `..`, dotfiles, backups or `.php`.

use hyper::{Response, StatusCode};
use open_runo_router::hyper_compat as hc;
use std::path::{Path, PathBuf};

/// 配るフォルダー(公開ルート直下) / Allowed top-level folders.
const DIRS: &[&str] = &[
    "analog-amp", "streaming-amp", "top", "world", "manual", "cancer", "images", "video", "mono-sub-out",
];

fn root() -> PathBuf {
    std::env::var_os("AUDIOCAFE_DOCROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/var/www/audiocafe.tokyo"))
}

fn content_type(ext: &str) -> Option<&'static str> {
    Some(match ext {
        "html" => "text/html; charset=UTF-8",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "pdf" => "application/pdf",
        "mp4" => "video/mp4",
        "css" => "text/css; charset=UTF-8",
        "js" => "text/javascript; charset=UTF-8",
        "txt" => "text/plain; charset=UTF-8",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        _ => return None,
    })
}

/// 要求パス(例 `/top/` `/images/a.jpg`)を安全なファイルパスへ。不可なら None。
/// Resolve a request path to a safe file path, or None.
pub(crate) fn resolve(path: &str) -> Option<(PathBuf, &'static str)> {
    let decoded = percent_decode(path)?;
    let segs: Vec<&str> = decoded.split('/').filter(|s| !s.is_empty()).collect();
    let first = *segs.first()?;
    if !DIRS.contains(&first) {
        return None;
    }
    for s in &segs {
        if s.starts_with('.') || s.contains('\\') || s.contains('\0') || s.ends_with('~') {
            return None;
        }
    }
    let mut p = root();
    for s in &segs {
        p.push(s);
    }
    if p.is_dir() {
        p.push("index.html");
    }
    let name = p.file_name()?.to_str()?;
    let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
    let ct = content_type(&ext)?;
    if !p.is_file() {
        return None;
    }
    Some((p, ct))
}

fn percent_decode(s: &str) -> Option<String> {
    let s = s.split('?').next().unwrap_or("");
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let h = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(h, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// `bytes=a-b` を解析 / Parse a single `bytes=` range.
fn parse_range(h: &str, len: u64) -> Option<(u64, u64)> {
    let r = h.strip_prefix("bytes=")?;
    if r.contains(',') || len == 0 {
        return None;
    }
    let (a, b) = r.split_once('-')?;
    let (start, end) = if a.is_empty() {
        let n: u64 = b.parse().ok()?;
        (len.saturating_sub(n), len - 1)
    } else {
        let s: u64 = a.parse().ok()?;
        let e = if b.is_empty() { len - 1 } else { b.parse::<u64>().ok()?.min(len - 1) };
        (s, e)
    };
    (start <= end && start < len).then_some((start, end))
}

pub(crate) async fn serve(path: &str, range: Option<&str>) -> Option<hc::Response> {
    let (p, ct) = resolve(path)?;
    let data = tokio::fs::read(&p).await.ok()?;
    let len = data.len() as u64;
    let builder = Response::builder().header("content-type", ct).header("accept-ranges", "bytes");
    let resp = match range.and_then(|r| parse_range(r, len)) {
        Some((s, e)) => builder
            .status(StatusCode::PARTIAL_CONTENT)
            .header("content-range", format!("bytes {s}-{e}/{len}"))
            .body(hc::fixed_body(bytes::Bytes::copy_from_slice(&data[s as usize..=e as usize]))),
        None => builder.status(StatusCode::OK).body(hc::fixed_body(bytes::Bytes::from(data))),
    };
    resp.ok()
}

#[allow(dead_code)]
pub(crate) fn docroot() -> PathBuf {
    root()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_parse() {
        assert_eq!(parse_range("bytes=0-99", 1000), Some((0, 99)));
        assert_eq!(parse_range("bytes=900-", 1000), Some((900, 999)));
        assert_eq!(parse_range("bytes=-10", 1000), Some((990, 999)));
        assert_eq!(parse_range("bytes=2000-", 1000), None);
        assert_eq!(parse_range("bytes=0-1,5-6", 1000), None);
    }

    #[test]
    fn rejects_unsafe() {
        let d = std::env::temp_dir().join("acr-static-test");
        std::fs::create_dir_all(d.join("top")).unwrap();
        std::fs::write(d.join("top/index.html"), "x").unwrap();
        std::fs::write(d.join("top/.htaccess"), "x").unwrap();
        std::fs::write(d.join("top/contact.php"), "x").unwrap();
        std::fs::write(d.join("top/index.html~"), "x").unwrap();
        std::env::set_var("AUDIOCAFE_DOCROOT", &d);
        assert!(resolve("/top/").is_some());
        assert!(resolve("/top/index.html").is_some());
        assert!(resolve("/top/.htaccess").is_none());
        assert!(resolve("/top/contact.php").is_none());
        assert!(resolve("/top/index.html~").is_none());
        assert!(resolve("/top/../top/index.html").is_none());
        assert!(resolve("/top/%2e%2e/x").is_none());
        assert!(resolve("/data/").is_none());
        let _ = Path::new("");
    }
}
