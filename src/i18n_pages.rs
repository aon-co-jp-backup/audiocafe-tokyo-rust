//! 旧 PHP 版(`audiocafe-tokyo-php` の `*/index-<言語>.php`、51ページ)の Rust 移植。
//! 本文は `templates/i18n/<dir>/<言語>.html`(PHP から機械的に抜き出したもの)で、
//! `{{LANG_NAV:<現在の言語>}}` を各ディレクトリの `lang-nav.php` と同じ HTML に、
//! `{{YEAR}}` を今年に置き換える。出力は PHP 版と同じバイト列になるよう試験で確認する。

include!("i18n_pages_list.rs");

const ARUARU: &[(&str, &str)] = &[
    ("ja", "🇯🇵 日本語"), ("en", "🇺🇸 English"), ("en-gb", "🇬🇧 English (UK)"), ("ko", "🇰🇷 한국어"),
    ("zh-cn", "🇨🇳 简体中文"), ("zh-tw", "🇹🇼 繁體中文"), ("ru", "🇷🇺 Русский"), ("uk", "🇺🇦 Українська"),
    ("de", "🇩🇪 Deutsch"), ("at", "🇦🇹 Deutsch (Österreich)"), ("ch", "🇨🇭 Deutsch (Schweiz)"), ("it", "🇮🇹 Italiano"),
    ("fr", "🇫🇷 Français"), ("es", "🇪🇸 Español"), ("tl", "🇵🇭 Filipino"), ("ar", "🇸🇦 العربية"),
    ("fa", "🇮🇷 فارسی"), ("he", "🇮🇱 עברית"),
];
const RAKUTEN: &[(&str, &str)] = &[
    ("ja", "🇯🇵 日本語"), ("en", "🇺🇸 English"), ("en-gb", "🇬🇧 English (UK)"), ("it", "🇮🇹 Italiano"),
    ("de", "🇩🇪 Deutsch"), ("at", "🇦🇹 Deutsch (Österreich)"), ("ch", "🇨🇭 Deutsch (Schweiz)"), ("fr", "🇫🇷 Français"),
    ("ru", "🇷🇺 Русский"), ("uk", "🇺🇦 Українська"), ("ar", "🇸🇦 العربية"), ("fa", "🇮🇷 فارسی"),
    ("ko", "🇰🇷 한국어"), ("zh-cn", "🇨🇳 简体中文"), ("zh-tw", "🇹🇼 繁體中文"), ("es", "🇪🇸 Español"),
    ("tl", "🇵🇭 Filipino"), ("he", "🇮🇱 עברית"),
];

fn lang_nav(dir: &str, current: &str) -> String {
    let (langs, tail) = if dir == "rakuten-mobile" {
        (RAKUTEN, r#"<a href="https://audiocafe.tokyo/aruaru/">📊 aruaru（IT技術情報）</a>"#)
    } else {
        (ARUARU, r#"<a href="https://aruaru.tokyo/">🎲 aruaru.tokyo</a>"#)
    };
    let mut s = String::from("<div class=\"lang-switch\">\n");
    for (code, label) in langs {
        if *code == current {
            continue;
        }
        let href = if *code == "ja" { format!("/{dir}/") } else { format!("/{dir}/index-{code}.php") };
        s.push_str(&format!("  <a href=\"{href}\">{label}</a>\n"));
    }
    s.push_str(&format!("  {tail}\n</div>\n"));
    s
}

/// `dir` と `file`(例 `index-en.php`)からページを描く。該当なしは `None`。
pub(crate) fn render(dir: &str, file: &str, year: i32) -> Option<String> {
    let lang = file.strip_prefix("index-")?.strip_suffix(".php")?;
    let (_, _, tpl) = PAGES.iter().find(|(d, l, _)| *d == dir && *l == lang)?;
    let mut out = tpl.replace("{{YEAR}}", &year.to_string());
    while let Some(i) = out.find("{{LANG_NAV:") {
        let j = i + out[i..].find("}}")?;
        let cur = out[i + 11..j].to_string();
        out.replace_range(i..j + 2, &lang_nav(dir, &cur));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_pages_render_without_php() {
        assert_eq!(PAGES.len(), 51);
        for (d, l, _) in PAGES {
            let s = render(d, &format!("index-{l}.php"), 2026).unwrap();
            assert!(!s.contains("<?") && !s.contains("{{"), "{d}/{l}");
        }
        assert!(render("aruaru", "index-xx.php", 2026).is_none());
        assert!(render("aruaru", "../etc", 2026).is_none());
    }
}
