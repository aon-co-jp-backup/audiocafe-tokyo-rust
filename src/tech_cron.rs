//! 旧 `aruaru/cron.php`(`--cron-tech`)の Rust 移植(2026-11-27)。
//! Port of the legacy `aruaru/cron.php --cron-tech` (technology ranking, learning prices,
//! AI learning links, price correction, teaching aruaru-llm).
//!
//! 出力の JSON は PHP の `json_encode(JSON_PRETTY_PRINT|JSON_UNESCAPED_UNICODE)` と同じ形
//! (4桁字下げ・`/`は`\/`・キーの順番を保つ)で書く。外部取得は環境変数 `TECH_FIX`
//! (記録したレスポンスのフォルダー)を指定すると、そこから読む(PHP版との一致試験用)。
//! JSON is written exactly like PHP's pretty JSON. Setting `TECH_FIX` replays recorded responses.

use regex::Regex;
use std::path::{Path, PathBuf};
use std::time::Duration;

// ===================== 順序つき JSON / ordered JSON =====================

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum V {
    Null,
    Bool(bool),
    Num(String),
    Str(String),
    Arr(Vec<V>),
    Obj(Vec<(String, V)>),
}

impl V {
    fn obj() -> V {
        V::Obj(Vec::new())
    }
    fn s(x: &str) -> V {
        V::Str(x.to_string())
    }
    fn int(n: i64) -> V {
        V::Num(n.to_string())
    }
    fn get(&self, k: &str) -> Option<&V> {
        match self {
            V::Obj(v) => v.iter().find(|(kk, _)| kk == k).map(|(_, v)| v),
            V::Arr(a) => k.parse::<usize>().ok().and_then(|i| a.get(i)),
            _ => None,
        }
    }
    fn get_mut(&mut self, k: &str) -> Option<&mut V> {
        match self {
            V::Obj(v) => v.iter_mut().find(|(kk, _)| kk == k).map(|(_, v)| v),
            _ => None,
        }
    }
    /// `$a[$k] = $v`(既存なら位置を保つ、無ければ末尾) / set keeping position.
    fn set(&mut self, k: &str, val: V) {
        if let V::Arr(a) = self {
            if a.is_empty() {
                *self = V::obj();
            }
        }
        if let V::Obj(v) = self {
            if let Some(e) = v.iter_mut().find(|(kk, _)| kk == k) {
                e.1 = val;
            } else {
                v.push((k.to_string(), val));
            }
        }
    }
    fn remove(&mut self, k: &str) {
        if let V::Obj(v) = self {
            v.retain(|(kk, _)| kk != k);
        }
    }
    fn is_array(&self) -> bool {
        matches!(self, V::Arr(_) | V::Obj(_))
    }
    /// PHP の `empty()`。
    fn empty(&self) -> bool {
        match self {
            V::Null => true,
            V::Bool(b) => !b,
            V::Str(s) => s.is_empty() || s == "0",
            V::Num(n) => n.parse::<f64>().map(|f| f == 0.0).unwrap_or(false),
            V::Arr(a) => a.is_empty(),
            V::Obj(o) => o.is_empty(),
        }
    }
    /// PHP の `(string)` 変換。
    fn php_string(&self) -> String {
        match self {
            V::Null => String::new(),
            V::Bool(b) => if *b { "1".into() } else { String::new() },
            V::Num(n) => n.clone(),
            V::Str(s) => s.clone(),
            _ => "Array".into(),
        }
    }
    fn php_int(&self) -> i64 {
        match self {
            V::Num(n) => n.parse::<i64>().unwrap_or_else(|_| n.parse::<f64>().map(|f| f as i64).unwrap_or(0)),
            V::Str(s) => leading_int(s),
            V::Bool(b) => *b as i64,
            _ => 0,
        }
    }
    fn is_numeric(&self) -> bool {
        match self {
            V::Num(_) => true,
            V::Str(s) => s.trim_start().parse::<f64>().is_ok() && !s.ends_with(char::is_whitespace),
            _ => false,
        }
    }
    /// 配列の値(リストでも連想でも) / values of a list or map.
    fn values(&self) -> Vec<&V> {
        match self {
            V::Arr(a) => a.iter().collect(),
            V::Obj(o) => o.iter().map(|(_, v)| v).collect(),
            _ => Vec::new(),
        }
    }
    fn values_mut(&mut self) -> Vec<&mut V> {
        match self {
            V::Arr(a) => a.iter_mut().collect(),
            V::Obj(o) => o.iter_mut().map(|(_, v)| v).collect(),
            _ => Vec::new(),
        }
    }
}

fn leading_int(s: &str) -> i64 {
    let t = s.trim_start();
    let mut end = 0;
    for (i, c) in t.char_indices() {
        if c.is_ascii_digit() || (i == 0 && (c == '-' || c == '+')) {
            end = i + c.len_utf8();
        } else {
            break;
        }
    }
    t[..end].parse().unwrap_or(0)
}

/// `array_merge` for maps: later keys overwrite in place, new keys appended.
fn merge(parts: &[&V]) -> V {
    let mut out = V::obj();
    for p in parts {
        if let V::Obj(o) = p {
            for (k, v) in o {
                out.set(k, v.clone());
            }
        }
    }
    out
}

pub(crate) fn parse(s: &str) -> Option<V> {
    let b = s.as_bytes();
    let mut i = 0;
    let v = parse_val(b, &mut i, 0)?;
    skip_ws(b, &mut i);
    (i == b.len()).then_some(v)
}

fn skip_ws(b: &[u8], i: &mut usize) {
    while *i < b.len() && matches!(b[*i], b' ' | b'\t' | b'\n' | b'\r') {
        *i += 1;
    }
}

fn parse_val(b: &[u8], i: &mut usize, depth: usize) -> Option<V> {
    if depth > 512 {
        return None;
    }
    skip_ws(b, i);
    match *b.get(*i)? {
        b'{' => {
            *i += 1;
            let mut o: Vec<(String, V)> = Vec::new();
            skip_ws(b, i);
            if b.get(*i) == Some(&b'}') {
                *i += 1;
                return Some(V::Obj(o));
            }
            loop {
                skip_ws(b, i);
                let k = parse_str(b, i)?;
                skip_ws(b, i);
                if b.get(*i) != Some(&b':') {
                    return None;
                }
                *i += 1;
                let v = parse_val(b, i, depth + 1)?;
                if let Some(e) = o.iter_mut().find(|(kk, _)| *kk == k) {
                    e.1 = v;
                } else {
                    o.push((k, v));
                }
                skip_ws(b, i);
                match b.get(*i)? {
                    b',' => *i += 1,
                    b'}' => {
                        *i += 1;
                        break;
                    }
                    _ => return None,
                }
            }
            // PHP: キーが "0","1",… の連番ならリスト / sequential numeric keys become a list
            if !o.is_empty() && o.iter().enumerate().all(|(n, (k, _))| *k == n.to_string()) {
                return Some(V::Arr(o.into_iter().map(|(_, v)| v).collect()));
            }
            Some(V::Obj(o))
        }
        b'[' => {
            *i += 1;
            let mut a = Vec::new();
            skip_ws(b, i);
            if b.get(*i) == Some(&b']') {
                *i += 1;
                return Some(V::Arr(a));
            }
            loop {
                a.push(parse_val(b, i, depth + 1)?);
                skip_ws(b, i);
                match b.get(*i)? {
                    b',' => *i += 1,
                    b']' => {
                        *i += 1;
                        break;
                    }
                    _ => return None,
                }
            }
            Some(V::Arr(a))
        }
        b'"' => Some(V::Str(parse_str(b, i)?)),
        b't' if b[*i..].starts_with(b"true") => {
            *i += 4;
            Some(V::Bool(true))
        }
        b'f' if b[*i..].starts_with(b"false") => {
            *i += 5;
            Some(V::Bool(false))
        }
        b'n' if b[*i..].starts_with(b"null") => {
            *i += 4;
            Some(V::Null)
        }
        _ => {
            let st = *i;
            while *i < b.len() && matches!(b[*i], b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E') {
                *i += 1;
            }
            let raw = std::str::from_utf8(&b[st..*i]).ok()?;
            raw.parse::<f64>().ok()?;
            Some(V::Num(norm_num(raw)))
        }
    }
}

/// PHP が読み直したときの数値表記 / how PHP re-encodes a number.
fn norm_num(raw: &str) -> String {
    if raw.contains(['.', 'e', 'E']) || raw.parse::<i64>().is_err() {
        let f: f64 = raw.parse().unwrap_or(0.0);
        let s = format!("{f}");
        if f.fract() == 0.0 && f.abs() < 1e15 && !s.contains('.') {
            format!("{s}.0")
        } else {
            s
        }
    } else {
        raw.to_string()
    }
}

fn parse_str(b: &[u8], i: &mut usize) -> Option<String> {
    if b.get(*i) != Some(&b'"') {
        return None;
    }
    *i += 1;
    let mut out: Vec<u8> = Vec::new();
    loop {
        let c = *b.get(*i)?;
        *i += 1;
        match c {
            b'"' => break,
            b'\\' => {
                let e = *b.get(*i)?;
                *i += 1;
                match e {
                    b'"' => out.push(b'"'),
                    b'\\' => out.push(b'\\'),
                    b'/' => out.push(b'/'),
                    b'b' => out.push(8),
                    b'f' => out.push(12),
                    b'n' => out.push(b'\n'),
                    b'r' => out.push(b'\r'),
                    b't' => out.push(b'\t'),
                    b'u' => {
                        let mut cp = hex4(b, i)?;
                        if (0xD800..0xDC00).contains(&cp) && b.get(*i) == Some(&b'\\') && b.get(*i + 1) == Some(&b'u') {
                            *i += 2;
                            let lo = hex4(b, i)?;
                            cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                        }
                        let ch = char::from_u32(cp).unwrap_or('\u{FFFD}');
                        let mut buf = [0u8; 4];
                        out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                    }
                    _ => return None,
                }
            }
            _ => out.push(c),
        }
    }
    String::from_utf8(out).ok()
}

fn hex4(b: &[u8], i: &mut usize) -> Option<u32> {
    let h = std::str::from_utf8(b.get(*i..*i + 4)?).ok()?;
    *i += 4;
    u32::from_str_radix(h, 16).ok()
}

fn esc(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '/' => out.push_str("\\/"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn enc(v: &V, pretty: bool, d: usize, out: &mut String) {
    let ind = |n: usize, out: &mut String| {
        if pretty {
            out.push('\n');
            for _ in 0..n {
                out.push_str("    ");
            }
        }
    };
    match v {
        V::Null => out.push_str("null"),
        V::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        V::Num(n) => out.push_str(n),
        V::Str(s) => esc(s, out),
        V::Arr(a) if a.is_empty() => out.push_str("[]"),
        V::Obj(o) if o.is_empty() => out.push_str("[]"),
        V::Arr(a) => {
            out.push('[');
            for (n, x) in a.iter().enumerate() {
                if n > 0 {
                    out.push(',');
                }
                ind(d + 1, out);
                enc(x, pretty, d + 1, out);
            }
            ind(d, out);
            out.push(']');
        }
        V::Obj(o) => {
            if o.iter().enumerate().all(|(n, (k, _))| *k == n.to_string()) {
                return enc(&V::Arr(o.iter().map(|(_, v)| v.clone()).collect()), pretty, d, out);
            }
            out.push('{');
            for (n, (k, x)) in o.iter().enumerate() {
                if n > 0 {
                    out.push(',');
                }
                ind(d + 1, out);
                esc(k, out);
                out.push_str(if pretty { ": " } else { ":" });
                enc(x, pretty, d + 1, out);
            }
            ind(d, out);
            out.push('}');
        }
    }
}

pub(crate) fn to_json(v: &V, pretty: bool) -> String {
    let mut s = String::new();
    enc(v, pretty, 0, &mut s);
    s
}

fn from_serde(v: &serde_json::Value) -> V {
    match v {
        serde_json::Value::Null => V::Null,
        serde_json::Value::Bool(b) => V::Bool(*b),
        serde_json::Value::Number(n) => V::Num(norm_num(&n.to_string())),
        serde_json::Value::String(s) => V::Str(s.clone()),
        serde_json::Value::Array(a) => V::Arr(a.iter().map(from_serde).collect()),
        serde_json::Value::Object(o) => V::Obj(o.iter().map(|(k, v)| (k.clone(), from_serde(v))).collect()),
    }
}

// ===================== 時刻・小物 / time and helpers =====================

fn jst_now() -> chrono::DateTime<chrono::FixedOffset> {
    chrono::Utc::now().with_timezone(&chrono::FixedOffset::east_opt(9 * 3600).unwrap())
}
fn ymdhis() -> String {
    jst_now().format("%Y-%m-%d %H:%M:%S").to_string()
}
fn date_c() -> String {
    jst_now().format("%Y-%m-%dT%H:%M:%S%:z").to_string()
}

fn norm_key(name: &str) -> String {
    name.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_lowercase()).collect()
}

fn php_trim(s: &str) -> &str {
    s.trim_matches(|c| matches!(c, ' ' | '\t' | '\n' | '\r' | '\0' | '\u{b}'))
}

fn number_format(n: i64) -> String {
    let s = n.abs().to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 {
        format!("-{out}")
    } else {
        out
    }
}

fn ucwords(s: &str) -> String {
    let mut out = String::new();
    let mut up = true;
    for c in s.chars() {
        if up {
            out.push(c.to_ascii_uppercase());
        } else {
            out.push(c);
        }
        up = matches!(c, ' ' | '\t' | '\r' | '\n' | '\u{c}' | '\u{b}');
    }
    out
}

/// PHP の `parse_url($u, PHP_URL_HOST)` 相当(主な形だけ) / host of a URL.
fn url_host(u: &str) -> String {
    let Some(p) = u.find("://") else { return String::new() };
    let rest = &u[p + 3..];
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let mut auth = &rest[..end];
    if let Some(a) = auth.rfind('@') {
        auth = &auth[a + 1..];
    }
    if auth.starts_with('[') {
        return auth.split(']').next().unwrap_or("").to_string() + "]";
    }
    auth.split(':').next().unwrap_or("").to_string()
}

fn strip_www(h: &str) -> String {
    h.strip_prefix("www.").unwrap_or(h).to_string()
}

fn php_round2(x: f64) -> String {
    let r = (x * 100.0).round() / 100.0;
    let s = format!("{r:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    s.to_string()
}

// ===================== 実行状態 / runtime context =====================

pub(crate) struct Ctx {
    dir: PathBuf,
    fix: Option<PathBuf>,
    http: reqwest::Client,
    quota_hit: Option<String>,
    corrections: Vec<(String, String, i64)>,
    llm_prefix: Option<String>,
}

const UA_TECH: &str = "aruaru-tech-sync/1.0";

impl Ctx {
    pub(crate) fn new(dir: &Path) -> Ctx {
        let mut b = reqwest::Client::builder().redirect(reqwest::redirect::Policy::limited(20));
        // db-engines.com は中間証明書を送らないため、PHP版と同じ ca-extra.pem を信頼に足す。
        if let Ok(pem) = std::fs::read(dir.join("ca-extra.pem")) {
            if let Ok(certs) = reqwest::Certificate::from_pem_bundle(&pem) {
                for c in certs {
                    b = b.add_root_certificate(c);
                }
            }
        }
        Ctx {
            dir: dir.to_path_buf(),
            fix: std::env::var_os("TECH_FIX").filter(|s| !s.is_empty()).map(PathBuf::from),
            http: b.build().expect("http client"),
            quota_hit: None,
            corrections: Vec::new(),
            llm_prefix: None,
        }
    }

    fn fix_path(&self, key: &str) -> Option<PathBuf> {
        let d = self.fix.as_ref()?;
        let name: String = key.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
        Some(d.join(name))
    }

    async fn pause(&self, ms: u64) {
        if self.fix.is_none() {
            tokio::time::sleep(Duration::from_millis(ms)).await;
        }
    }

    fn log(&self, msg: &str) {
        use std::io::Write;
        let line = format!("[{}] {}\n", ymdhis(), msg);
        print!("{line}");
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(self.dir.join("cron.log")) {
            let _ = f.write_all(line.as_bytes());
        }
    }

    /// `aruaru_tech_http_get` / `aruaru_learning_prices_http_get`(本文、空/失敗は None)。
    async fn http_get(&self, url: &str, timeout: u64, ua: &str, accept: &str) -> Option<String> {
        if let Some(p) = self.fix_path(url) {
            if let Ok(b) = std::fs::read(&p) {
                return (!b.is_empty()).then(|| String::from_utf8_lossy(&b).into_owned());
            }
        }
        let mut rq = self.http.get(url).timeout(Duration::from_secs(timeout)).header("user-agent", ua).header("accept", accept);
        if url.contains("api.github.com") {
            if let Ok(t) = std::env::var("GITHUB_TOKEN") {
                if !t.is_empty() {
                    rq = rq.header("authorization", format!("Bearer {t}"));
                }
            }
        }
        let body = rq.send().await.ok()?.bytes().await.ok()?;
        (!body.is_empty()).then(|| String::from_utf8_lossy(&body).into_owned())
    }

    fn llm_enabled(&self) -> bool {
        std::env::var("ARUARU_LLM_URL").map(|v| v.to_ascii_lowercase() != "off").unwrap_or(true)
    }

    fn llm_url(&self) -> String {
        match std::env::var("ARUARU_LLM_URL") {
            Ok(u) if !u.is_empty() => u.trim_end_matches('/').to_string(),
            _ => "http://127.0.0.1:4600".into(),
        }
    }

    /// 版なし(`aruaru_llm.Llm`)を先に試し、無ければ旧名(`.v1`)へ。Versionless 方針。
    async fn llm_post(&mut self, method: &str, body: String, timeout: u64) -> Option<(u16, String)> {
        let prefixes: Vec<String> = match &self.llm_prefix {
            Some(p) => vec![p.clone()],
            None => vec!["/aruaru_llm.Llm/".into(), "/aruaru_llm.v1.Llm/".into()],
        };
        for p in prefixes {
            let url = format!("{}{}{}", self.llm_url(), p, method);
            let r = self
                .http
                .post(&url)
                .timeout(Duration::from_secs(timeout))
                .header("content-type", "application/json")
                .body(body.clone())
                .send()
                .await
                .ok()?;
            let code = r.status().as_u16();
            if code == 404 && self.llm_prefix.is_none() {
                continue;
            }
            self.llm_prefix = Some(p);
            return Some((code, r.text().await.unwrap_or_default()));
        }
        None
    }

    async fn llm_complete(&mut self, prompt: &str, timeout: u64) -> Option<String> {
        if !self.llm_enabled() {
            return None;
        }
        let (mut quota, mut tried) = (0, 0);
        for prov in ["gemini", "groq", "mistral"] {
            tried += 1;
            let mut req = V::obj();
            req.set("prompt", V::s(prompt));
            req.set("providers", V::Arr(vec![V::s(prov)]));
            let Some((_, raw)) = self.llm_post("ChatProvidersCompletePriority", to_json(&req, false), timeout).await else {
                continue;
            };
            let Ok(resp) = serde_json::from_str::<serde_json::Value>(&raw) else { continue };
            if !resp.is_object() {
                continue;
            }
            let text = match resp.get("reply") {
                Some(serde_json::Value::Object(o)) => o.get("text").map(|t| from_serde(t).php_string()).unwrap_or_default(),
                Some(serde_json::Value::String(s)) => s.clone(),
                _ => String::new(),
            };
            let text = php_trim(&text);
            if !text.is_empty() {
                return Some(text.to_string());
            }
            if resp.get("all_quota_exceeded").map(|v| !from_serde(v).empty()).unwrap_or(false) {
                quota += 1;
            }
        }
        if tried > 0 && quota == tried {
            self.quota_hit = Some("all_providers_quota_exceeded".into());
        }
        None
    }
}

fn strip_fence(t: &str) -> String {
    static RE: once_cell::sync::Lazy<Regex> = once_cell::sync::Lazy::new(|| Regex::new(r"(?i)\A```(?:json)?\s*|\s*```\n?\z").unwrap());
    RE.replace_all(t, "").into_owned()
}

// ===================== 技術ランキング / tech ranking =====================

fn lang_map() -> &'static [(&'static str, &'static str)] {
    &[
        ("python", "Python"), ("javascript", "JavaScript"), ("java", "Java"), ("c#", "C#"), ("php", "PHP"), ("c++", "C++"),
        ("c", "C"), ("typescript", "TypeScript"), ("ruby", "Ruby"), ("swift", "Swift"), ("kotlin", "Kotlin"), ("go", "Go"),
        ("golang", "Go"), ("rust", "Rust"), ("scala", "Scala"), ("r", "R"), ("dart", "Dart"), ("elixir", "Elixir"),
        ("haskell", "Haskell"), ("lua", "Lua"), ("perl", "Perl"), ("objective-c", "Objective-C"), ("groovy", "Groovy"),
        ("clojure", "Clojure"), ("f#", "F#"), ("julia", "Julia"), ("zig", "Zig"), ("nim", "Nim"), ("crystal", "Crystal"),
        ("ocaml", "OCaml"), ("erlang", "Erlang"), ("matlab", "MATLAB"), ("fortran", "Fortran"), ("powershell", "PowerShell"),
        ("bash", "Bash"), ("shell", "Bash"), ("cobol", "COBOL"), ("vba", "VBA"), ("ada", "Ada"), ("solidity", "Solidity"),
        ("sql", "SQL"), ("assembly", "Assembly"), ("prolog", "Prolog"), ("lisp", "Lisp"), ("scheme", "Scheme"),
        ("webassembly", "WebAssembly"), ("wasm", "WebAssembly"), ("graphql", "GraphQL"), ("terraform", "HCL"), ("hcl", "HCL"),
        ("mdx", "MDX"), ("move-lang", "Move"), ("cairo", "Cairo"),
    ]
}

fn fw_map() -> &'static [(&'static str, &'static str)] {
    &[
        ("reactjs", "React"), ("react-native", "React Native"), ("next.js", "Next.js"), ("vue.js", "Vue.js"),
        ("nuxt.js", "Nuxt.js"), ("angular", "Angular"), ("angularjs", "Angular"), ("svelte", "Svelte"), ("django", "Django"),
        ("flask", "Flask"), ("fastapi", "FastAPI"), ("laravel", "Laravel"), ("symfony", "Symfony"), ("spring", "Spring Boot"),
        ("spring-boot", "Spring Boot"), ("nestjs", "NestJS"), ("express", "Express"), ("node.js", "Express"), ("hono", "Hono"),
        ("flutter", "Flutter"), ("ruby-on-rails", "Ruby on Rails"), ("rails", "Ruby on Rails"), ("gin", "Gin"), ("echo", "Echo"),
        ("fiber", "Fiber"), ("axum", "Axum"), ("actix", "Actix Web"), ("phoenix-framework", "Phoenix"), ("phoenix", "Phoenix"),
        ("blazor", "Blazor"), ("asp.net", "ASP.NET Core"), ("asp.net-core", "ASP.NET Core"), ("dotnet", "ASP.NET Core"),
        ("streamlit", "Streamlit"), ("expo", "Expo"), ("ionic-framework", "Ionic"), ("ionic", "Ionic"), ("capacitor", "Capacitor"),
        ("htmx", "HTMX"), ("qwik", "Qwik"), ("lit", "Lit"), ("preact", "Preact"), ("celery", "Celery"), ("cakephp", "CakePHP"),
        ("playframework", "Play Framework"), ("akka", "Akka"), ("micronaut", "Micronaut"), ("quarkus", "Quarkus"),
        ("remix", "Remix"), ("astro", "Astro"), ("solidjs", "Solid.js"), ("trpc", "tRPC"), ("swiftui", "SwiftUI"),
        ("jetpack-compose", "Jetpack Compose"), ("fastify", "Fastify"), ("sveltekit", "SvelteKit"),
        ("tanstack-query", "TanStack Query"), ("react-query", "TanStack Query"),
    ]
}

fn db_map() -> &'static [(&'static str, &'static str)] {
    &[
        ("postgresql", "PostgreSQL"), ("mysql", "MySQL"), ("redis", "Redis"), ("mongodb", "MongoDB"), ("sqlite", "SQLite"),
        ("elasticsearch", "Elasticsearch"), ("cassandra", "Cassandra"), ("dynamodb", "DynamoDB"), ("firebase", "Firestore"),
        ("firestore", "Firestore"), ("mariadb", "MariaDB"), ("oracle", "Oracle DB"), ("sql-server", "SQL Server"),
        ("bigquery", "BigQuery"), ("snowflake", "Snowflake"), ("neo4j", "Neo4j"), ("influxdb", "InfluxDB"),
        ("clickhouse", "ClickHouse"), ("couchbase", "Couchbase"), ("duckdb", "DuckDB"), ("timescaledb", "TimescaleDB"),
        ("cockroachdb", "CockroachDB"), ("supabase", "Supabase"), ("planetscale", "PlanetScale"), ("tidb", "TiDB"),
        ("weaviate", "Weaviate"), ("pinecone", "Pinecone"), ("graphql-database", "FaunaDB"), ("fauna", "FaunaDB"),
        ("arangodb", "ArangoDB"), ("yugabytedb", "YugabyteDB"), ("scylladb", "ScyllaDB"), ("hbase", "Apache HBase"),
        ("druid", "Apache Druid"), ("cosmosdb", "Azure Cosmos DB"), ("documentdb", "Amazon DocumentDB"),
    ]
}

/// 挿入順を保つスコア表 / insertion-ordered score map.
#[derive(Default, Clone)]
struct Scores(Vec<(String, f64)>);
impl Scores {
    fn get(&self, k: &str) -> Option<f64> {
        self.0.iter().find(|(kk, _)| kk == k).map(|(_, v)| *v)
    }
    fn set(&mut self, k: &str, v: f64) {
        if let Some(e) = self.0.iter_mut().find(|(kk, _)| kk == k) {
            e.1 = v;
        } else {
            self.0.push((k.to_string(), v));
        }
    }
    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

const SO_URL: &str = "https://api.stackexchange.com/2.3/tags?order=desc&sort=popular&site=stackoverflow&pagesize=100&page=";

async fn fetch_stackoverflow(ctx: &Ctx, map: &[(&str, &str)], pages: u32) -> Scores {
    let mut sc = Scores::default();
    for page in 1..=pages {
        let Some(raw) = ctx.http_get(&format!("{SO_URL}{page}"), 20, UA_TECH, "application/json,text/html").await else { break };
        let Ok(d) = serde_json::from_str::<serde_json::Value>(&raw) else { break };
        let Some(items) = d.get("items").and_then(|x| x.as_array()) else { break };
        for it in items {
            let tag = it.get("name").map(|v| from_serde(v).php_string()).unwrap_or_default().to_ascii_lowercase();
            let count = it.get("count").and_then(|v| v.as_f64()).unwrap_or(0.0);
            if tag.is_empty() || count <= 0.0 {
                continue;
            }
            let Some((_, canon)) = map.iter().find(|(t, _)| *t == tag) else { continue };
            let key = norm_key(canon);
            let cur = sc.get(&key).unwrap_or(0.0);
            sc.set(&key, cur.max(count));
        }
        ctx.pause(300).await;
    }
    sc
}

async fn fetch_dbengines(ctx: &Ctx) -> (Scores, &'static str) {
    let mut sc = Scores::default();
    let Some(raw) = ctx.http_get("https://db-engines.com/en/ranking", 25, UA_TECH, "application/json,text/html").await else {
        return (sc, "db-engines.com (fetch failed)");
    };
    let re = Regex::new(r#"(?i)<a\s+href="/en/sys/[^"]+">([^<]+)</a>"#).unwrap();
    let mut rank = 0i64;
    for c in re.captures_iter(&raw) {
        let name = html_escape::decode_html_entities(&c[1]).into_owned();
        let name = php_trim(&name);
        if name.is_empty() {
            continue;
        }
        rank += 1;
        let key = norm_key(name);
        let v = ((51 - rank.min(50)) * 10000) as f64;
        let cur = sc.get(&key).unwrap_or(0.0);
        sc.set(&key, cur.max(v));
        if rank >= 60 {
            break;
        }
    }
    (sc, "db-engines.com")
}

struct External {
    langs: Scores,
    fws: Scores,
    dbs: Scores,
    sources: Vec<String>,
    errors: Vec<String>,
}

async fn fetch_external(ctx: &Ctx) -> External {
    let mut sources: Vec<String> = Vec::new();
    let mut errors = Vec::new();
    let langs = fetch_stackoverflow(ctx, lang_map(), 3).await;
    if !langs.is_empty() {
        sources.push("stackoverflow.com".into());
    } else {
        errors.push("Stack Overflow (languages): no data".to_string());
    }
    let fws = fetch_stackoverflow(ctx, fw_map(), 3).await;
    if !fws.is_empty() {
        sources.push("stackoverflow.com".into());
    } else {
        errors.push("Stack Overflow (frameworks): no data".to_string());
    }
    let db_so = fetch_stackoverflow(ctx, db_map(), 2).await;
    let (mut dbs, src) = fetch_dbengines(ctx).await;
    for (k, v) in &db_so.0 {
        let cur = dbs.get(k).unwrap_or(0.0);
        dbs.set(k, cur + v * 0.3);
    }
    if !dbs.is_empty() {
        sources.push(src.into());
        if !db_so.is_empty() {
            sources.push("stackoverflow.com (DB tags)".into());
        }
    } else {
        errors.push("DB rankings: no external data".to_string());
    }
    let mut uniq: Vec<String> = Vec::new();
    for s in sources {
        if !uniq.contains(&s) {
            uniq.push(s);
        }
    }
    External { langs, fws, dbs, sources: uniq, errors }
}

fn build_top50(category: &str, baseline: &[V], ext: &Scores) -> Vec<V> {
    let mut seed: Vec<(String, V)> = Vec::new();
    for row in baseline {
        let key = norm_key(&row.get("name").map(V::php_string).unwrap_or_default());
        if let Some(e) = seed.iter_mut().find(|(k, _)| *k == key) {
            e.1 = row.clone();
        } else {
            seed.push((key, row.clone()));
        }
    }
    let mut keys: Vec<String> = Vec::new();
    for k in ext.0.iter().map(|(k, _)| k).chain(seed.iter().map(|(k, _)| k)) {
        if !keys.contains(k) {
            keys.push(k.clone());
        }
    }
    let mut combined: Vec<(String, f64)> = keys
        .iter()
        .map(|k| {
            let e = ext.get(k).unwrap_or(0.0);
            let base_rank = seed.iter().find(|(kk, _)| kk == k).map(|(_, r)| r.get("rank").map(V::php_int).unwrap_or(99)).unwrap_or(99);
            let base = (51 - base_rank.min(50)).max(0) * 500;
            (k.clone(), e + base as f64)
        })
        .collect();
    combined.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let mut out = Vec::new();
    for (i, (key, _)) in combined.into_iter().take(80).enumerate() {
        let rank = V::int(i as i64 + 1);
        if let Some((_, r)) = seed.iter().find(|(k, _)| *k == key) {
            let mut row = r.clone();
            row.set("rank", rank);
            if category == "framework" && row.get("memory").map(V::empty).unwrap_or(true) {
                row.set("memory", V::s("中程度"));
            }
            if category == "database" && row.get("memory").map(V::empty).unwrap_or(true) {
                row.set("memory", V::s("中〜大"));
            }
            out.push(row);
            continue;
        }
        let display = ucwords(&key.replace(['-', '_'], " "));
        let mut row = V::obj();
        row.set("rank", rank);
        row.set("name", V::Str(display));
        row.set("_needs_ai", V::Bool(true));
        row.set("_key", V::Str(key));
        out.push(row);
    }
    out
}

fn complete(category: &str, row: &V) -> bool {
    let req: &[&str] = match category {
        "language" => &["team_dev", "maintenance", "beginner", "speed", "dev_scale", "traits", "oss_note", "async_support", "throughput_100k", "memory", "ai_comment"],
        "framework" => &["team_dev", "maintenance", "beginner", "speed", "large_scale", "memory", "ai_comment"],
        _ => &["speed", "scale", "distributed", "memory", "ai_comment"],
    };
    req.iter().all(|f| !row.get(f).map(V::empty).unwrap_or(true))
}

fn fallback_row(category: &str, name: &str) -> V {
    let mut r = V::obj();
    r.set("rank", V::int(0));
    r.set("name", V::s(name));
    let kv: &[(&str, &str)] = match category {
        "language" => &[
            ("team_dev", "要調査"), ("maintenance", "要調査"), ("beginner", "要調査"), ("speed", "要調査"), ("dev_scale", "要調査"),
            ("traits", "外部ランキングで新規検出。詳細は公式ドキュメントを参照。"), ("oss_note", "要調査"), ("async_support", "要調査"),
            ("throughput_100k", "要調査"), ("memory", "中程度"), ("ai_comment", "外部データで検出（OPENAI_API_KEY未設定のため簡易表示）"),
        ],
        "framework" => &[
            ("team_dev", "要調査"), ("maintenance", "要調査"), ("beginner", "要調査"), ("speed", "要調査"), ("large_scale", "要調査"),
            ("memory", "中程度"), ("ai_comment", "外部データで検出（OPENAI_API_KEY未設定）"),
        ],
        _ => &[("speed", "要調査"), ("scale", "要調査"), ("distributed", "要調査"), ("memory", "中〜大"), ("ai_comment", "外部データで検出（OPENAI_API_KEY未設定）")],
    };
    for (k, v) in kv {
        r.set(k, V::s(v));
    }
    r
}

async fn ai_enrich(ctx: &mut Ctx, category: &str, names: &[String]) -> Vec<(String, V)> {
    if names.is_empty() || !ctx.llm_enabled() {
        return Vec::new();
    }
    let list = names.iter().take(12).cloned().collect::<Vec<_>>().join(", ");
    let schema = match category {
        "language" => r#"{"name":"","team_dev":"","maintenance":"","beginner":"","speed":"","dev_scale":"","traits":"","oss_note":"","async_support":"","throughput_100k":"","memory":"","ai_comment":""}"#,
        "framework" => r#"{"name":"","team_dev":"","maintenance":"","beginner":"","speed":"","large_scale":"","memory":"","ai_comment":""}"#,
        _ => r#"{"name":"","speed":"","scale":"","distributed":"","memory":"","ai_comment":""}"#,
    };
    let prompt = format!(
        "2026年時点の日本のITエンジニア向けに、次の{category}についてJSON配列のみで回答してください（説明文禁止）。\n対象: {list}\n各要素のスキーマ: {schema}\ntraitsは日本語80字以内。ai_commentは日本語60字以内。memoryは「極小/小容量/中程度/大容量必要」などから選ぶ。"
    );
    let Some(text) = ctx.llm_complete(&format!("JSON配列のみ返す。\n{prompt}"), 90).await else { return Vec::new() };
    let Some(arr) = parse(&strip_fence(&text)) else { return Vec::new() };
    let mut out: Vec<(String, V)> = Vec::new();
    for item in arr.values() {
        if !matches!(item, V::Obj(_)) || item.get("name").map(V::empty).unwrap_or(true) {
            continue;
        }
        let k = norm_key(&item.get("name").unwrap().php_string());
        if let Some(e) = out.iter_mut().find(|(kk, _)| *kk == k) {
            e.1 = item.clone();
        } else {
            out.push((k, item.clone()));
        }
    }
    out
}

async fn apply_ai(ctx: &mut Ctx, category: &str, mut rows: Vec<V>) -> Vec<V> {
    let mut need: Vec<String> = Vec::new();
    for r in &rows {
        if !complete(category, r) || !r.get("_needs_ai").map(V::empty).unwrap_or(true) {
            let n = r.get("name").map(V::php_string).unwrap_or_default();
            if !need.contains(&n) {
                need.push(n);
            }
        }
    }
    let strip = |r: &mut V| {
        r.remove("_needs_ai");
        r.remove("_key");
    };
    if need.is_empty() {
        rows.iter_mut().for_each(strip);
        return rows;
    }
    let mut enriched: Vec<(String, V)> = Vec::new();
    for chunk in need.chunks(10) {
        for (k, v) in ai_enrich(ctx, category, chunk).await {
            if let Some(e) = enriched.iter_mut().find(|(kk, _)| *kk == k) {
                e.1 = v;
            } else {
                enriched.push((k, v));
            }
        }
        if ctx.llm_enabled() && chunk.len() > 1 {
            ctx.pause(500).await;
        }
    }
    for r in rows.iter_mut() {
        let name = r.get("name").map(V::php_string).unwrap_or_default();
        let key = norm_key(&name);
        if complete(category, r) && r.get("_needs_ai").map(V::empty).unwrap_or(true) {
            strip(r);
            continue;
        }
        let rank = r.get("rank").cloned().unwrap_or(V::Null);
        let mut tail = V::obj();
        *r = match enriched.iter().find(|(k, _)| *k == key).map(|(_, v)| v) {
            Some(ai) if ai.is_array() => {
                tail.set("name", r.get("name").cloned().unwrap_or(V::Null));
                tail.set("rank", rank);
                merge(&[r, ai, &tail])
            }
            _ => {
                tail.set("rank", rank);
                merge(&[&fallback_row(category, &name), r, &tail])
            }
        };
        strip(r);
    }
    rows
}

fn baseline() -> V {
    parse(include_str!("../data/tech/baseline.json")).expect("baseline.json")
}

async fn refresh_rankings(ctx: &mut Ctx) -> V {
    let mut base = baseline();
    let ext = fetch_external(ctx).await;
    let errors = V::Arr(ext.errors.iter().map(|e| V::s(e)).collect());
    if ext.langs.is_empty() && ext.fws.is_empty() && ext.dbs.is_empty() {
        ctx.log("外部データ取得失敗 — ベースラインのみ保存");
        let mut m = V::obj();
        m.set("sources", V::Arr(vec![V::s("baseline-only")]));
        m.set("fetched_at", V::Str(date_c()));
        m.set("openai_used", V::Bool(false));
        m.set("errors", errors);
        base.set("ranking_meta", m);
        return base;
    }
    let rows = |k: &str| base.get(k).map(|v| v.values().into_iter().cloned().collect::<Vec<_>>()).unwrap_or_default();
    let (bl, bf, bd) = (rows("languages"), rows("frameworks"), rows("databases"));
    let langs = build_top50("language", &bl, &ext.langs);
    let fws = build_top50("framework", &bf, &ext.fws);
    let dbs = build_top50("database", &bd, &ext.dbs);
    let ai_used = ctx.llm_enabled();
    let langs = apply_ai(ctx, "language", langs).await;
    let fws = apply_ai(ctx, "framework", fws).await;
    let dbs = apply_ai(ctx, "database", dbs).await;
    ctx.log(&format!(
        "同期完了: 言語{} / FW{} / DB{} / ソース: {}",
        langs.len(),
        fws.len(),
        dbs.len(),
        ext.sources.join(", ")
    ));
    let mut out = V::obj();
    out.set("updated_at", V::Str(ymdhis()));
    out.set("languages", V::Arr(langs));
    out.set("frameworks", V::Arr(fws));
    out.set("databases", V::Arr(dbs));
    let mut m = V::obj();
    m.set("sources", V::Arr(ext.sources.iter().map(|s| V::s(s)).collect()));
    m.set("fetched_at", V::Str(date_c()));
    m.set("openai_used", V::Bool(ai_used));
    m.set("github_token", V::Bool(std::env::var("GITHUB_TOKEN").map(|t| !t.is_empty()).unwrap_or(false)));
    m.set("errors", errors);
    m.set(
        "note",
        V::s("順位は Stack Overflow / DB-Engines 等の外部人気データとベースラインを合成。紹介文の新規生成は OpenAI（キー設定時）。"),
    );
    out.set("ranking_meta", m);
    out
}

// ===================== 学習価格 / learning prices =====================

const PRICE_KEYS: [&str; 3] = ["progate_monthly", "dotinstall_monthly", "paiza_monthly"];
const PRICE_DEFAULTS: [i64; 3] = [1490, 1480, 1490];

fn read_json(p: &Path) -> Option<V> {
    let s = std::fs::read_to_string(p).ok()?;
    if s.is_empty() {
        return None;
    }
    parse(&s)
}

fn prices_extract(key: &str, html: &str) -> Option<i64> {
    let re = match key {
        "dotinstall_monthly" => r#"class="plan-price-value"\s+data-cycle="1"\s*>\s*([0-9]{1,3}(?:,[0-9]{3})*|[0-9]+)\s*<"#,
        "progate_monthly" => r#""amount":([0-9]{3,6}),"currency":"jpy","price":"[^"]*","interval":"month","interval_count":1,"#,
        "paiza_monthly" => r#"(?s)1ヶ月プラン.{0,400}?<strong>\s*([0-9]{1,3}(?:,[0-9]{3})+|[0-9]{4,5})\s*</strong>\s*円"#,
        _ => return None,
    };
    let c = Regex::new(re).ok()?.captures(html)?;
    let n: i64 = c[1].replace(',', "").parse().ok()?;
    (1000..=99999).contains(&n).then_some(n)
}

async fn prices_refresh(ctx: &Ctx) {
    let path = ctx.dir.join("aruaru-learning-prices-cache.json");
    let mut cur: Vec<(&str, i64)> = PRICE_KEYS.iter().copied().zip(PRICE_DEFAULTS).collect();
    if let Some(old) = read_json(&path) {
        if old.is_array() {
            for (k, v) in cur.iter_mut() {
                if let Some(x) = old.get(k).filter(|x| x.is_numeric()) {
                    *v = x.php_int().clamp(100, 999999);
                }
            }
        }
    }
    let urls = [
        ("dotinstall_monthly", "https://dotinstall.com/premium"),
        ("progate_monthly", "https://progate.com/plans/for_users"),
        ("paiza_monthly", "https://paiza.jp/works/paid_plan_lp/settlement"),
    ];
    let mut sources = V::obj();
    for (key, url) in urls {
        let html = ctx
            .http_get(url, 12, "Mozilla/5.0 (compatible; aruaru-learning-prices/1.0)", "text/html,application/xhtml+xml")
            .await;
        let prev = cur.iter().find(|(k, _)| *k == key).unwrap().1;
        let Some(html) = html else {
            sources.set(key, V::s("fetch_failed"));
            continue;
        };
        let Some(one) = prices_extract(key, &html) else {
            sources.set(key, V::s("no_match"));
            continue;
        };
        cur.iter_mut().find(|(k, _)| *k == key).unwrap().1 = one;
        sources.set(key, V::Str(if one == prev { "unchanged".into() } else { format!("updated:{one}") }));
    }
    let mut payload = V::obj();
    for (k, v) in &cur {
        payload.set(k, V::int(*v));
    }
    payload.set("updated_at", V::Str(date_c()));
    payload.set("sources", sources.clone());
    let _ = std::fs::write(&path, to_json(&payload, true));
    ctx.log(&format!("[learning-prices] {} → saved", to_json(&sources, false)));
}

/// host → 検証済み月額 / verified monthly price per host.
fn verified_prices(ctx: &Ctx) -> Vec<(&'static str, i64)> {
    let mut cur: Vec<(&str, i64)> = PRICE_KEYS.iter().copied().zip(PRICE_DEFAULTS).collect();
    if let Some(old) = read_json(&ctx.dir.join("aruaru-learning-prices-cache.json")) {
        for (k, v) in cur.iter_mut() {
            if let Some(x) = old.get(k).filter(|x| x.is_numeric()) {
                *v = x.php_int();
            }
        }
    }
    let g = |k: &str| cur.iter().find(|(kk, _)| *kk == k).unwrap().1;
    vec![
        ("dotinstall.com", g("dotinstall_monthly")),
        ("progate.com", g("progate_monthly")),
        ("prog-8.com", g("progate_monthly")),
        ("paiza.jp", g("paiza_monthly")),
    ]
}

// ===================== AI学習リンク / AI learning links =====================

fn free_host_tier(host: &str) -> char {
    const PAID: &[&str] = &[
        "udemy.com", "coursera.org", "schoo.jp", "codecademy.com", "techacademy.jp", "progate.com", "prog-8.com", "dotinstall.com",
        "paiza.jp", "pluralsight.com", "linkedin.com", "skillshare.com", "treehouse.com", "datacamp.com", "edx.org", "udacity.com",
        "tutorialspoint.com", "geeksforgeeks.org", "baeldung.com", "educative.io", "frontendmasters.com", "egghead.io", "sejuku.net",
        "aidemy.net", "tech-camp.in", "kikagaku.co.jp", "zerotohero.dev", "scrimba.com", "laracasts.com", "pragprog.com", "oreilly.com",
    ];
    const FREE: &[&str] = &[
        "qiita.com", "zenn.dev", "youtube.com", "freecodecamp.org", "github.com", "developer.mozilla.org", "learn.microsoft.com",
        "docs.microsoft.com", "developer.android.com", "developer.apple.com", "w3schools.com", "wikipedia.org", "go.dev", "dart.dev",
        "flutter.dev", "kotlinlang.org", "typescriptlang.org", "php.net", "python.org", "python.jp", "rust-lang.org", "nextjs.org",
        "react.dev", "vuejs.org", "nodejs.org", "swift.org", "ruby-lang.org", "railsguides.jp", "exercism.org", "learnxinyminutes.com",
        "elixirschool.com", "lua.org", "ziglang.org", "dlang.org", "crystal-lang.org", "scala-lang.org", "fortran-lang.org",
        "swi-prolog.org", "soliditylang.org", "openjdk.org", "oracle.com", "postgresql.org", "mysql.com", "mongodb.com", "sqlite.org",
        "redis.io", "docker.com", "kubernetes.io", "git-scm.com", "rust-lang.org", "mozilla.org",
    ];
    let host = strip_www(&host.to_lowercase());
    let hit = |d: &&str| host == *d || host.ends_with(&format!(".{d}"));
    if PAID.iter().any(hit) {
        return 'B';
    }
    if FREE.iter().any(hit) {
        return 'A';
    }
    static RE: once_cell::sync::Lazy<Regex> = once_cell::sync::Lazy::new(|| Regex::new(r"^(docs?|developers?|learn|reference|guide)\.").unwrap());
    if RE.is_match(&host) {
        return 'A';
    }
    'C'
}

fn is_free_claim(price: &str) -> bool {
    let p = php_trim(price).to_lowercase();
    p == "無料" || p == "無償" || p == "free"
}

fn item_str(it: &V, k: &str) -> String {
    it.get(k).map(V::php_string).unwrap_or_default()
}

async fn scan_signals(ctx: &Ctx, urls: &[String]) -> Vec<(String, Option<bool>)> {
    let mut uniq: Vec<String> = Vec::new();
    for u in urls {
        if !uniq.contains(u) {
            uniq.push(u.clone());
        }
    }
    let re = Regex::new(r"(?i)(有料|月額|年額|料金プラン|料金表|プレミアム|サブスク|会員登録が必要|ログインが必要|購入|カートに入れる|[¥￥]\s?[0-9]|[0-9][0-9,]*\s?円|pricing|subscribe|subscription|premium|buy now|add to cart|\$\s?[0-9]{1,4}\b|per month|/\s?month|paywall|members only|free trial|無料体験|無料トライアル)").unwrap();
    let strip = [
        Regex::new(r"(?is)<script\b.*?</script>").unwrap(),
        Regex::new(r"(?is)<style\b.*?</style>").unwrap(),
        Regex::new(r"<[^>]+>").unwrap(),
    ];
    let mut out = Vec::new();
    let mut live: Vec<String> = Vec::new();
    for u in &uniq {
        match ctx.fix_path(&format!("SIG:{u}")).and_then(|p| std::fs::read_to_string(p).ok()) {
            Some(c) => out.push((u.clone(), if c.is_empty() { None } else { Some(c == "1") })),
            None => live.push(u.clone()),
        }
    }
    for batch in live.chunks(30) {
        let mut set = tokio::task::JoinSet::new();
        for (i, u) in batch.iter().enumerate() {
            let rq = ctx
                .http
                .get(u)
                .timeout(Duration::from_secs(12))
                .header("range", "bytes=0-153599")
                .header("user-agent", "Mozilla/5.0 (compatible; aruaru-free-check/1.0)");
            set.spawn(async move {
                let r = match rq.send().await {
                    Ok(r) => r,
                    Err(_) => return (i, None),
                };
                let code = r.status().as_u16();
                let body = r.bytes().await.unwrap_or_default();
                (i, ((200..400).contains(&code) && !body.is_empty()).then(|| String::from_utf8_lossy(&body).into_owned()))
            });
        }
        let mut res: Vec<Option<String>> = vec![None; batch.len()];
        while let Some(Ok((i, b))) = set.join_next().await {
            res[i] = b;
        }
        for (u, b) in batch.iter().zip(res) {
            let v = b.map(|body| {
                let mut t = body;
                for r in &strip {
                    t = r.replace_all(&t, " ").into_owned();
                }
                re.is_match(&t)
            });
            out.push((u.clone(), v));
        }
    }
    out
}

/// 生成AIが作った URL のうち、実際に開けるものだけ残す / keep only live URLs.
async fn filter_live(ctx: &Ctx, items: Vec<V>) -> Vec<V> {
    if items.is_empty() {
        return items;
    }
    let client = reqwest::Client::builder().redirect(reqwest::redirect::Policy::limited(4)).build().unwrap_or_else(|_| ctx.http.clone());
    let mut set = tokio::task::JoinSet::new();
    for (i, it) in items.iter().enumerate() {
        let rq = client
            .get(item_str(it, "url"))
            .timeout(Duration::from_secs(8))
            .header("range", "bytes=0-2047")
            .header("user-agent", "Mozilla/5.0 (compatible; aruaru-link-check/1.0)");
        set.spawn(async move { (i, rq.send().await.map(|r| (200..400).contains(&r.status().as_u16())).unwrap_or(false)) });
    }
    let mut ok = vec![false; items.len()];
    while let Some(Ok((i, b))) = set.join_next().await {
        ok[i] = b;
    }
    items.into_iter().zip(ok).filter(|(_, b)| *b).map(|(v, _)| v).collect()
}

async fn sanitize_links(ctx: &mut Ctx, items: &mut V, sigs: Option<&[(String, Option<bool>)]>) {
    let verified = verified_prices(ctx);
    let owned;
    let sigs = match sigs {
        Some(s) => s,
        None => {
            let need: Vec<String> = items
                .values()
                .iter()
                .filter(|it| is_free_claim(&item_str(it, "price")) && free_host_tier(&url_host(&item_str(it, "url"))) == 'C')
                .map(|it| item_str(it, "url"))
                .collect();
            owned = if need.is_empty() { Vec::new() } else { scan_signals(ctx, &need).await };
            &owned[..]
        }
    };
    static AMT: once_cell::sync::Lazy<Regex> = once_cell::sync::Lazy::new(|| Regex::new(r"([0-9][0-9,]{2,5})\s*円").unwrap());
    static DONE: once_cell::sync::Lazy<Regex> = once_cell::sync::Lazy::new(|| {
        Regex::new(r"^(一部無料・有料プランあり|有料プランの記載あり|無料（ページ内に料金の記載なし|月額[0-9,]+円（税込・公式)").unwrap()
    });
    for it in items.values_mut() {
        let url = item_str(it, "url");
        let host = strip_www(&url_host(&url).to_lowercase());
        let price = php_trim(&item_str(it, "price")).to_string();
        let v = verified.iter().find(|(h, _)| *h == host).map(|(_, y)| *y).unwrap_or(0);
        if v > 0 {
            let said: Vec<i64> = AMT.captures_iter(&price).filter_map(|c| c[1].replace(',', "").parse().ok()).collect();
            if !said.is_empty() && !said.contains(&v) {
                ctx.corrections.push((host.clone(), price.clone(), v));
            }
            it.set("price", V::Str(format!("月額{}円（税込・公式ページで確認）", number_format(v))));
        } else if is_free_claim(&price) {
            match free_host_tier(&host) {
                'B' => it.set("price", V::s("一部無料・有料プランあり（公式サイトで要確認）")),
                'C' => {
                    let sig = sigs.iter().find(|(u, _)| *u == url).and_then(|(_, s)| *s);
                    it.set(
                        "price",
                        V::s(match sig {
                            None => "料金は公式サイトで要確認",
                            Some(true) => "有料プランの記載あり（公式サイトで要確認）",
                            Some(false) => "無料（ページ内に料金の記載なし・自動確認）",
                        }),
                    );
                }
                _ => {}
            }
        } else if !DONE.is_match(&price) {
            it.set("price", V::s("料金は公式サイトで要確認"));
        }
    }
}

fn dedupe_urls(items: &V) -> Vec<V> {
    static HTTPS: once_cell::sync::Lazy<Regex> = once_cell::sync::Lazy::new(|| Regex::new(r"(?i)^https://").unwrap());
    let mut seen: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for it in items.values() {
        if !it.is_array() {
            continue;
        }
        let url = php_trim(&item_str(it, "url")).to_string();
        if url.is_empty() || !HTTPS.is_match(&url) {
            continue;
        }
        let key = format!("{}\0{}", url_host(&url).to_lowercase(), url);
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        let field = |k: &str, d: &str| match it.get(k) {
            Some(V::Null) | None => d.to_string(),
            Some(x) => x.php_string(),
        };
        let mut o = V::obj();
        o.set("name", V::Str(field("name", "リンク")));
        o.set("url", V::Str(url));
        o.set("features", V::Str(field("features", "オンライン学習")));
        o.set("price", V::Str(field("price", "要確認")));
        out.push(o);
    }
    out
}

fn ai_cache_path(ctx: &Ctx) -> PathBuf {
    ctx.dir.join("aruaru-tech-learning-ai-cache.json")
}

fn load_ai_cache(ctx: &Ctx) -> V {
    let mut def = V::obj();
    def.set("entries", V::Arr(vec![]));
    def.set("meta", V::Arr(vec![]));
    match read_json(&ai_cache_path(ctx)) {
        Some(mut j @ V::Obj(_)) => {
            for k in ["entries", "meta"] {
                if j.get(k).is_none() {
                    j.set(k, V::Arr(vec![]));
                }
            }
            j
        }
        _ => def,
    }
}

fn save_ai_cache(ctx: &Ctx, data: &mut V) {
    if data.get("meta").is_none() {
        data.set("meta", V::obj());
    }
    data.get_mut("meta").unwrap().set("saved_at", V::Str(date_c()));
    let _ = std::fs::write(ai_cache_path(ctx), to_json(data, true));
}

async fn fetch_links(ctx: &mut Ctx, name: &str, kind: &str) -> Option<(Vec<V>, Vec<V>)> {
    if !ctx.llm_enabled() {
        return None;
    }
    let kind_ja = match kind {
        "language" => "プログラミング言語",
        "framework" => "フレームワーク／ライブラリ",
        "database" => "データベース／ストレージ",
        _ => "クラウド／インフラ",
    };
    let vp = verified_prices(ctx);
    let g = |h: &str| vp.iter().find(|(k, _)| *k == h).map(|(_, v)| *v).unwrap_or(0);
    let price_rule = format!(
        "【料金のルール】price は推測で書かない。確認済みの金額: ドットインストール月額{}円、Progate月額{}円、paiza月額{}円（いずれも税込）。無料のものは \"無料\"、金額が分からないものは \"要確認\" と書く。",
        g("dotinstall.com"),
        g("progate.com"),
        g("paiza.jp")
    );
    let prompt = format!(
        "技術: 「{name}」（種別: {kind_ja}）\n2026年時点で、初心者〜中級者がオンラインで学べる「日本語の学習ページ・公式ドキュメント・動画学習サービス（Progate / ドットインストール / paiza / Schoo / Udemy 等）」を中心に、実在する https URL を選んでください。\n公式ドキュメントの日本語版があれば優先。\n{price_rule}\n\n次のJSONのみを返す（説明文禁止）。各配列は最大10件。url は必ず https で始まる実在URL。\n{{\n  \"ja\": [ {{\"name\":\"サイト名\",\"url\":\"https://...\",\"features\":\"一言\",\"price\":\"無料／月額目安など\"}} ],\n  \"en\": [ {{\"name\":\"Site\",\"url\":\"https://...\",\"features\":\"one line\",\"price\":\"Free / paid hint\"}} ]\n}}"
    );
    let text = ctx
        .llm_complete(&format!("You output JSON only. Never invent domains; use well-known real URLs only.\n\n{prompt}"), 90)
        .await?;
    let obj = parse(&strip_fence(&text))?;
    let (ja, en) = match (obj.get("ja"), obj.get("en")) {
        (Some(a), Some(b)) if !matches!(a, V::Null) && !matches!(b, V::Null) && obj.is_array() => (a.clone(), b.clone()),
        _ => return None,
    };
    let mut out = Vec::new();
    for list in [ja, en] {
        let l = if list.is_array() { list } else { V::Arr(vec![]) };
        let mut live = V::Arr(filter_live(ctx, dedupe_urls(&l)).await);
        sanitize_links(ctx, &mut live, None).await;
        out.push(match live {
            V::Arr(a) => a,
            _ => vec![],
        });
    }
    let en = out.pop().unwrap();
    let ja = out.pop().unwrap();
    if ja.is_empty() && en.is_empty() {
        return None;
    }
    Some((ja, en))
}

async fn learning_ai_refresh(ctx: &mut Ctx, ranking: &V, max: usize) {
    if !ctx.llm_enabled() {
        ctx.log("[learning-ai] ARUARU_LLM_URL=off のためスキップ");
        return;
    }
    let mut keys: Vec<String> = Vec::new();
    for (sec, pfx) in [("languages", "lang"), ("frameworks", "fw"), ("databases", "db")] {
        if let Some(list) = ranking.get(sec) {
            for row in list.values() {
                let n = php_trim(&item_str(row, "name")).to_string();
                if !n.is_empty() {
                    keys.push(format!("{pfx}:{n}"));
                }
            }
        }
    }
    keys.extend(["lang:Mojo", "fw:WunderGraph", "fw:VersionlessAPI", "cloud:AWS"].map(String::from));
    let mut cache = load_ai_cache(ctx);
    let mut entries = match cache.get("entries") {
        Some(e @ V::Obj(_)) => e.clone(),
        _ => V::obj(),
    };
    let now = chrono::Utc::now().timestamp();
    let mut done = 0usize;
    for key in keys {
        if done >= max {
            break;
        }
        if let Some(prev @ V::Obj(_)) = entries.get(&key) {
            if prev.get("updated").map(V::php_int).unwrap_or(0) + 86400 > now {
                continue;
            }
        }
        let Some((pfx, name)) = key.split_once(':') else { continue };
        if name.is_empty() || name.contains('\n') || !["lang", "fw", "db", "cloud"].contains(&pfx) {
            continue;
        }
        let kind = match pfx {
            "lang" => "language",
            "fw" => "framework",
            "db" => "database",
            _ => "cloud",
        };
        let fetched = fetch_links(ctx, name, kind).await;
        if let Some(q) = &ctx.quota_hit {
            ctx.log(&format!("[learning-ai] 利用上限に達したため今回は打ち切り ({q})"));
            break;
        }
        let Some((ja, en)) = fetched else { continue };
        let mut e = V::obj();
        e.set("ja", V::Arr(ja));
        e.set("en", V::Arr(en));
        e.set("updated", V::int(now));
        e.set("source", V::s("aruaru-llm"));
        entries.set(&key, e);
        done += 1;
        ctx.pause(350).await;
    }
    cache.set("entries", entries);
    if !matches!(cache.get("meta"), Some(V::Obj(_))) {
        cache.set("meta", V::obj());
    }
    let meta = cache.get_mut("meta").unwrap();
    meta.set("last_cron_refresh", V::int(now));
    meta.set("last_refresh_count", V::int(done as i64));
    save_ai_cache(ctx, &mut cache);
    ctx.log(&format!("[learning-ai] 更新 {done} 件 → {}", ai_cache_path(ctx).display()));
}

async fn fix_cache(ctx: &mut Ctx) -> usize {
    let mut cache = load_ai_cache(ctx);
    let before = ctx.corrections.len();
    let mut need = Vec::new();
    if let Some(entries) = cache.get("entries") {
        for e in entries.values() {
            for lang in ["ja", "en"] {
                if let Some(l) = e.get(lang) {
                    for it in l.values() {
                        if is_free_claim(&item_str(it, "price")) && free_host_tier(&url_host(&item_str(it, "url"))) == 'C' {
                            need.push(item_str(it, "url"));
                        }
                    }
                }
            }
        }
    }
    let sigs = if need.is_empty() { Vec::new() } else { scan_signals(ctx, &need).await };
    if let Some(entries) = cache.get_mut("entries") {
        let mut es: Vec<&mut V> = entries.values_mut();
        for e in es.iter_mut() {
            for lang in ["ja", "en"] {
                if let Some(l) = e.get_mut(lang) {
                    if l.is_array() {
                        let mut tmp = std::mem::replace(l, V::Null);
                        sanitize_links(ctx, &mut tmp, Some(&sigs)).await;
                        *e.get_mut(lang).unwrap() = tmp;
                    }
                }
            }
        }
    }
    save_ai_cache(ctx, &mut cache);
    ctx.corrections.len() - before
}

async fn teach_prices(ctx: &mut Ctx) -> usize {
    let today = jst_now().format("%Y-%m-%d").to_string();
    let pages = [
        ("dotinstall.com", "Dotinstall", "ドットインストール", "https://dotinstall.com/premium"),
        ("progate.com", "Progate", "Progate", "https://progate.com/plans/for_users"),
        ("paiza.jp", "paiza", "paizaラーニング", "https://paiza.jp/works/paid_plan_lp/settlement"),
    ];
    let mut facts: Vec<(String, Vec<String>, String)> = vec![(
        "Rule: never guess prices of learning services (Progate, dotinstall, paiza, Udemy, Schoo). State only prices verified on the official page; otherwise write \"要確認\". 学習サービスの料金は推測で書かない。公式ページで確認できた金額だけを書き、不明なら「要確認」とする。".into(),
        ["rule", "price", "learning", "progate", "dotinstall", "paiza", "udemy", "schoo"].map(String::from).to_vec(),
        "aruaru-ai-teach".into(),
    )];
    for (host, yen) in verified_prices(ctx) {
        let Some((_, en, ja, url)) = pages.iter().find(|p| p.0 == host) else { continue };
        if yen <= 0 {
            continue;
        }
        facts.push((
            format!("Verified price ({today}): {en} monthly plan costs {yen} yen including tax. {ja}の月額プランは{yen}円（税込）。公式ページ {url} で確認。"),
            vec![en.to_lowercase(), "price".into(), "verified".into(), "learning".into()],
            url.to_string(),
        ));
    }
    for (host, ai, v) in ctx.corrections.iter().take(20) {
        facts.push((
            format!("Correction: AI wrote price \"{ai}\" for {host} but the verified monthly price is {v} yen. Do not repeat this mistake. AIが{host}の料金を「{ai}」と書いたが誤り。検証済みの月額は{v}円。"),
            vec!["correction".into(), "price".into(), "learning".into()],
            "aruaru-ai-teach".into(),
        ));
    }
    let mut n = 0;
    if ctx.llm_enabled() {
        for (text, tags, src) in facts {
            let mut b = V::obj();
            b.set("text", V::Str(text));
            b.set("tags", V::Arr(tags.into_iter().map(V::Str).collect()));
            b.set("source", V::Str(src));
            if let Some((code, _)) = ctx.llm_post("KnowledgeAdd", to_json(&b, false), 15).await {
                if (200..300).contains(&code) {
                    n += 1;
                }
            }
        }
    }
    ctx.log(&format!("[learning-ai] aruaru-llmへ学習させた件数: {n}（GitHubへ自動バックアップ）"));
    n
}

// ===================== 入口 / entry =====================

pub(crate) async fn run(dir: &Path) -> std::io::Result<()> {
    let t0 = std::time::Instant::now();
    let p = |m: &str| println!("[{}] {}", ymdhis(), m);
    p("========== aruaru cron-tech 開始 ==========");
    let writable = std::fs::metadata(dir).map(|m| !m.permissions().readonly()).unwrap_or(false);
    p(&format!("書込権限: {}", if writable { "OK" } else { "NG(書込不可)" }));
    let mut ctx = Ctx::new(dir);

    p("[1/3] AIテクノロジーランキング 更新...");
    let data = refresh_rankings(&mut ctx).await;
    let tech = dir.join("ai-tech-ranking-cache.json");
    std::fs::write(&tech, to_json(&data, true))?;
    let srcs = match data.get("ranking_meta").and_then(|m| m.get("sources")) {
        Some(s) => s.values().iter().map(|v| v.php_string()).collect::<Vec<_>>().join(", "),
        None => "baseline".into(),
    };
    p(&format!("[1/3] 完了（外部: {srcs}）"));

    p("[2/3] 学習価格情報 更新...");
    prices_refresh(&ctx).await;
    p("[2/3] 完了");

    p("[3/3] AI学習コメント 更新...");
    learning_ai_refresh(&mut ctx, &data, 40).await;
    let fixed = fix_cache(&mut ctx).await;
    let taught = teach_prices(&mut ctx).await;
    p(&format!("[3/3] 完了（価格訂正 {fixed} 件 / aruaru-llmへ学習 {taught} 件）"));

    let root = dir.parent().unwrap_or(dir);
    for f in [tech, dir.join("aruaru-learning-prices-cache.json")] {
        if !f.is_file() {
            println!("    (無し) {}", f.display());
            continue;
        }
        let dst = root.join(f.file_name().unwrap());
        let tmp = root.join(format!("{}.tmp{}", f.file_name().unwrap().to_string_lossy(), std::process::id()));
        if std::fs::copy(&f, &tmp).is_ok() && std::fs::rename(&tmp, &dst).is_ok() {
            println!("    複製: {}  {}", dst.display(), ymdhis());
        } else {
            let _ = std::fs::remove_file(&tmp);
            println!("    複製失敗: {}", dst.display());
        }
    }
    p(&format!("========== aruaru cron-tech 完了（{}秒）==========", php_round2(t0.elapsed().as_secs_f64())));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_roundtrip_like_php() {
        let ls = char::from_u32(0x2028).unwrap();
        let input = format!(r#"{{"a":"x/y","b":[],"c":{{}},"d":{{"0":1,"1":2}},"e":1.50,"f":"{ls}é"}}"#);
        let v = parse(&input).unwrap();
        let bs = '\\';
        let want = format!(r#"{{"a":"x{bs}/y","b":[],"c":[],"d":[1,2],"e":1.5,"f":"{bs}u2028é"}}"#);
        assert_eq!(to_json(&v, false), want);
        assert_eq!(to_json(&parse("[1,{\"k\":true}]").unwrap(), true), "[\n    1,\n    {\n        \"k\": true\n    }\n]");
    }

    #[test]
    fn helpers() {
        assert_eq!(norm_key("C++ / Next.js"), "cnextjs");
        assert_eq!(number_format(1480), "1,480");
        assert_eq!(url_host("https://user@www.Example.com:8080/a?b"), "www.Example.com");
        assert_eq!(free_host_tier("www.udemy.com"), 'B');
        assert_eq!(free_host_tier("docs.foo.io"), 'A');
        assert_eq!(free_host_tier("example.org"), 'C');
        assert!(is_free_claim(" Free "));
        assert_eq!(php_round2(8.0349), "8.03");
        assert_eq!(php_round2(12.0), "12");
        assert_eq!(strip_fence("```json\n[1]\n```"), "[1]");
    }

    #[test]
    fn price_extract() {
        assert_eq!(prices_extract("dotinstall_monthly", r#"<span class="plan-price-value" data-cycle="1">1,480</span>"#), Some(1480));
        assert_eq!(
            prices_extract("progate_monthly", r#"{"amount":1490,"currency":"jpy","price":"x","interval":"month","interval_count":1,"#),
            Some(1490)
        );
        assert_eq!(prices_extract("paiza_monthly", "<h3>1ヶ月プラン</h3><p><strong>1,490</strong>円（税込）"), Some(1490));
    }

    #[test]
    fn top50_order_and_fallback() {
        let base = parse(r#"[{"rank":1,"name":"Go","memory":""},{"rank":2,"name":"Rust"}]"#).unwrap();
        let mut ext = Scores::default();
        ext.set("zig", 1e9);
        ext.set("rust", 10.0);
        let rows = build_top50("framework", &base.values().into_iter().cloned().collect::<Vec<_>>(), &ext);
        let names: Vec<String> = rows.iter().map(|r| item_str(r, "name")).collect();
        assert_eq!(names, ["Zig", "Go", "Rust"]);
        assert_eq!(item_str(&rows[1], "memory"), "中程度");
        assert!(rows[0].get("_needs_ai").is_some());
    }

    #[test]
    fn baseline_loads() {
        let b = baseline();
        assert_eq!(b.get("languages").unwrap().values().len(), 80);
    }
}
