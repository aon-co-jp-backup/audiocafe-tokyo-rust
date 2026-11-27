//! 旧 `aruaru-lady/cron.php`(`aruaru-lady/index.php --cron-all`)の Rust 移植(2026-11-27)。
//! Rust port of the legacy `aruaru-lady/cron.php` (`--cron-all`).
//!
//! 4つのランキング(キャバクラ・熟女キャバ・TVチャットレディ2種)を、固定の一覧(`data/lady/*.json`、
//! PHP の master_pool から書き出したもの)と、週・日ごとの種(seed)から作り、PHP と同じ形の JSON で保存する。
//! Builds four rankings from fixed lists plus a weekly/daily seed and writes the same JSON as PHP.

use serde_json::Value;

/// 項目の順番を保つ1行 / One row with key order kept.
pub(crate) type Row = Vec<(String, Value)>;

fn load(raw: &str) -> Vec<Row> {
    let v: Vec<Vec<(String, Value)>> = serde_json::from_str(raw).expect("data/lady の JSON が壊れている");
    v
}

fn pool(name: &str) -> Vec<Row> {
    load(match name {
        "caba" => include_str!("../data/lady/caba.json"),
        "jukujo" => include_str!("../data/lady/jukujo.json"),
        "group" => include_str!("../data/lady/group.json"),
        "normal" => include_str!("../data/lady/normal.json"),
        _ => unreachable!(),
    })
}

fn get_i64(r: &Row, k: &str) -> i64 {
    r.iter().find(|(kk, _)| kk == k).and_then(|(_, v)| v.as_i64()).unwrap_or(0)
}

fn remove(r: &mut Row, k: &str) {
    r.retain(|(kk, _)| kk != k);
}

fn set_rank(rows: &mut [Row]) {
    for (i, r) in rows.iter_mut().enumerate() {
        r.push(("rank".into(), Value::from(i as i64 + 1)));
    }
}

/// `aruaru_caba_select_top50`
pub(crate) fn caba_top50(mut pool: Vec<Row>, seed: i64) -> Vec<Row> {
    pool.sort_by(|a, b| get_i64(b, "w").cmp(&get_i64(a, "w")));
    let n = pool.len() as i64;
    let off = seed % n.max(1);
    let mut rot: Vec<Row> = (0..n).map(|i| pool[((off + i) % n) as usize].clone()).collect();
    rot.sort_by(|a, b| get_i64(b, "w").cmp(&get_i64(a, "w")));
    rot.truncate(50);
    for r in rot.iter_mut() {
        remove(r, "w");
        remove(r, "region");
    }
    set_rank(&mut rot);
    rot
}

/// `aruaru_jukujo_caba_select_top50`(primary=`lux`, mul=100000)と tvchat 系(primary=`w`, mul=1000)の共通形。
pub(crate) fn keyed_top50(mut pool: Vec<Row>, seed: i64, primary: &str, mul: i64) -> Vec<Row> {
    let keys: Vec<i64> = pool
        .iter()
        .enumerate()
        .map(|(i, r)| get_i64(r, primary) * mul + ((seed >> (i % 12)) & 4095))
        .collect();
    let mut idx: Vec<usize> = (0..pool.len()).collect();
    idx.sort_by(|&a, &b| {
        get_i64(&pool[b], primary)
            .cmp(&get_i64(&pool[a], primary))
            .then(keys[b].cmp(&keys[a]))
    });
    let mut top: Vec<Row> = idx.into_iter().take(50).map(|i| std::mem::take(&mut pool[i])).collect();
    set_rank(&mut top);
    top
}

fn region_rows(region: &str) -> Vec<Row> {
    pool("caba")
        .into_iter()
        .filter(|r| r.iter().any(|(k, v)| k == "region" && v.as_str() == Some(region)))
        .collect()
}

/// PHP の `json_encode(…, JSON_PRETTY_PRINT | JSON_UNESCAPED_UNICODE)` と同じ書き方。
/// Same output as PHP's pretty JSON with unescaped unicode (slashes escaped as `\/`).
pub(crate) enum J {
    Str(String),
    Raw(Value),
    Obj(Vec<(String, J)>),
    Rows(Vec<Row>),
}

fn php_str(s: &str) -> String {
    serde_json::to_string(s).unwrap().replace('/', "\\/")
}

fn php_scalar(v: &Value) -> String {
    match v {
        Value::String(s) => php_str(s),
        other => other.to_string(),
    }
}

fn write(j: &J, ind: usize, out: &mut String) {
    let pad = |n: usize| " ".repeat(n * 4);
    match j {
        J::Str(s) => out.push_str(&php_str(s)),
        J::Raw(v) => out.push_str(&php_scalar(v)),
        J::Obj(items) => {
            if items.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push_str("{\n");
            for (i, (k, v)) in items.iter().enumerate() {
                out.push_str(&pad(ind + 1));
                out.push_str(&php_str(k));
                out.push_str(": ");
                write(v, ind + 1, out);
                if i + 1 < items.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str(&pad(ind));
            out.push('}');
        }
        J::Rows(rows) => {
            if rows.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push_str("[\n");
            for (i, r) in rows.iter().enumerate() {
                out.push_str(&pad(ind + 1));
                let obj = J::Obj(r.iter().map(|(k, v)| (k.clone(), J::Raw(v.clone()))).collect());
                write(&obj, ind + 1, out);
                if i + 1 < rows.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str(&pad(ind));
            out.push(']');
        }
    }
}

pub(crate) fn to_php_json(j: &J) -> String {
    let mut s = String::new();
    write(j, 0, &mut s);
    s
}

const CABA_DISCLAIMER: &str = "日本全国　キャバレー　キャバクラは、無料の自動車送迎がある場合も御座います。時給・勤務条件・年齢等は法令・自治体規制および各店・求人媒体の表記により異なります。下表は高額時給帯の募集例が出やすいエリアの検索窓口の目安ランキングです（週次で自動更新。特定店の推奨・保証ではありません）。";
const JUKUJO_DISCLAIMER: &str = "熟女キャバレー／熟女キャバクラにおいて、「高級」の目安・熟年応募可否・自動車無料送迎の有無は店舗および求人ごとに異なります。本表は検索への窓口目安のみで、順位や店を推奨・保証するものではありません。風俗営業適正化法等に従ってご利用ください。";
const GROUP_DISCLAIMER: &str = "TVチャットレディ【グループチャット（パーティーチャット）版】（ノンアダルト・在宅／駅前体験）における「実質時給目安」は、運営サイトの報酬率・最低保証・同時接続効率・指定機材により大きく異なります。本表は検索への窓口目安のみで、順位・サイト・店舗を推奨または保証するものではありません。年齢制限・身分証/契約の取り扱い・各種法令を必ずご確認ください。";
const NORMAL_DISCLAIMER: &str = "TVチャットレディ【通常版／1対1】（ノンアダルト・在宅／駅前体験）における「実質時給目安」は、運営サイトの報酬率・最低保証・稼働効率・指定機材により大きく異なります。本表は検索への窓口目安のみで、順位・サイト・店舗を推奨または保証するものではありません。年齢制限・身分証/契約の取り扱い・各種法令を必ずご確認ください。";

/// `(int)gmdate('oW')` / ISO 年+週
pub(crate) fn iso_week_seed(t: chrono::DateTime<chrono::Utc>) -> i64 {
    t.format("%G%V").to_string().parse().unwrap()
}

/// `(int)gmdate('Yz')`(z は 0 始まり・ゼロ埋めなし)
pub(crate) fn year_day_seed(t: chrono::DateTime<chrono::Utc>) -> i64 {
    use chrono::Datelike;
    format!("{}{}", t.year(), t.ordinal0()).parse().unwrap()
}

/// 4つのファイルの中身を作る / Build the four cache files: (file name, JSON text).
pub(crate) fn build_all(t: chrono::DateTime<chrono::Utc>) -> Vec<(&'static str, String)> {
    let week = iso_week_seed(t);
    let day = year_day_seed(t);
    let updated = J::Str(format!("{} UTC", t.format("%Y-%m-%d %H:%M:%S")));
    let up = || match &updated {
        J::Str(s) => J::Str(s.clone()),
        _ => unreachable!(),
    };
    let caba = J::Obj(vec![
        ("updated_at".into(), up()),
        ("disclaimer".into(), J::Str(CABA_DISCLAIMER.into())),
        ("tokyo_23".into(), J::Obj(vec![("rows".into(), J::Rows(caba_top50(region_rows("tokyo_23"), week ^ 7919)))])),
        ("tokyo_tama".into(), J::Obj(vec![("rows".into(), J::Rows(caba_top50(region_rows("tokyo_tama"), week ^ 8923)))])),
        ("national".into(), J::Obj(vec![("rows".into(), J::Rows(caba_top50(region_rows("national"), week ^ 9937)))])),
    ]);
    let simple = |rows: Vec<Row>, d: &str| {
        J::Obj(vec![("updated_at".into(), up()), ("rows".into(), J::Rows(rows)), ("disclaimer".into(), J::Str(d.into()))])
    };
    vec![
        ("aruaru-caba-ranking-cache.json", to_php_json(&caba)),
        ("aruaru-jukujo-caba-ranking-cache.json", to_php_json(&simple(keyed_top50(pool("jukujo"), week ^ 17837, "lux", 100000), JUKUJO_DISCLAIMER))),
        ("aruaru-tvchat-group-ranking-cache.json", to_php_json(&simple(keyed_top50(pool("group"), day ^ 39067, "w", 1000), GROUP_DISCLAIMER))),
        ("aruaru-tvchat-normal-ranking-cache.json", to_php_json(&simple(keyed_top50(pool("normal"), day ^ 28541, "w", 1000), NORMAL_DISCLAIMER))),
    ]
}

/// `--cron-lady`: `<docroot>/aruaru-lady/` へ4ファイルを書く(PHP と同じ場所・同じ記録の文言)。
pub(crate) fn run(dir: &std::path::Path) -> std::io::Result<()> {
    let now = || chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    println!("\n[{}] ========== aruaru-lady --cron-all 開始 ==========", now());
    let labels = ["キャバクラランキング", "熟女キャバランキング", "TVチャットレディ（グループ）", "TVチャットレディ（通常）"];
    for (i, (name, body)) in build_all(chrono::Utc::now()).into_iter().enumerate() {
        println!("[{}] [{}/4] {} クロール...", now(), i + 1, labels[i]);
        let dst = dir.join(name);
        let tmp = dir.join(format!("{name}.tmp{}", std::process::id()));
        std::fs::write(&tmp, body)?;
        std::fs::rename(&tmp, &dst)?;
        println!("[{}] [{}/4] 完了", now(), i + 1);
    }
    println!("[{}] ========== aruaru-lady --cron-all 完了（0秒）==========\n", now());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pools_load() {
        for n in ["caba", "jukujo", "group", "normal"] {
            assert!(pool(n).len() >= 50, "{n}");
        }
    }

    /// PHP 本番(8.3)で同じ種から作った結果(`tests/fixtures/lady-expected.txt`)と1文字単位で一致すること。
    #[test]
    fn matches_php_output() {
        let expected = include_str!("../tests/fixtures/lady-expected.txt");
        let mut got = Vec::new();
        for s in [202648i64, 202601, 202652, 1, 99999] {
            let j = J::Obj(vec![
                ("seed".into(), J::Raw(Value::from(s))),
                ("caba23".into(), J::Rows(caba_top50(region_rows("tokyo_23"), s ^ 7919))),
                ("cabatama".into(), J::Rows(caba_top50(region_rows("tokyo_tama"), s ^ 8923))),
                ("cabanat".into(), J::Rows(caba_top50(region_rows("national"), s ^ 9937))),
                ("jukujo".into(), J::Rows(keyed_top50(pool("jukujo"), s ^ 17837, "lux", 100000))),
                ("group".into(), J::Rows(keyed_top50(pool("group"), s ^ 39067, "w", 1000))),
                ("normal".into(), J::Rows(keyed_top50(pool("normal"), s ^ 28541, "w", 1000))),
            ]);
            got.push(to_php_json(&j));
        }
        let got = got.join("
=====
");
        assert_eq!(got.len(), expected.len());
        assert!(got == expected);
    }

    #[test]
    fn seeds() {
        use chrono::TimeZone;
        let t = chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        assert_eq!(iso_week_seed(t), 202601);
        assert_eq!(year_day_seed(t), 20260);
        let t = chrono::Utc.with_ymd_and_hms(2027, 1, 1, 0, 0, 0).unwrap();
        assert_eq!(iso_week_seed(t), 202653);
    }
}
