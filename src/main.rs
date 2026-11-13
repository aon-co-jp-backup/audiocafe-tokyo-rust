//! audiocafe-tokyo-server — audiocafe.tokyo のPHPモノリス(index.php、
//! 445KB・189関数)をRust+Poemへ段階的に移行するための新サイト。
//! aruaru-tokyo-server/aon-tokyo/karu-tokyo と同じ技術スタック・実装方針
//! (DB非依存・1バイナリ完結・テンプレートエンジン不使用)を踏襲する。
//!
//! ## 移行方針(2026-07-17、第一段)
//! 既存PHP側はDB接続をほとんど持たず、実データの大半は
//! `*-cache.json`(ファイルベースキャッシュ、`https://audiocafe.tokyo/`
//! 直下に静的公開済み)によるジャンル別ランキング表示。このRust側は
//! そのキャッシュJSONをHTTP経由で取得し(サーバー間の直接ファイル共有を
//! 前提にしない、疎結合)、[`rust_json`](https://github.com/aon-co-jp/Rust-JSON)
//! (`parse_strict`)でパースして汎用的にレンダリングする。
//!
//! キャッシュのスキーマは完全には統一されていない(地域別
//! `tokyo_23`/`tokyo_tama`/`national`に分かれるもの、フラットな`rows`
//! だけのもの、`ai-tech-ranking`のような複数の異なる配列を持つもの、
//! `rakuten-*`系のようなスカラー値+日英併記フィールドの塊等)。
//! 2026-07-17、全8種のキャッシュに対応する**完全再帰の汎用レンダラー**
//! (`render_value_generic`)に書き換えた——形状ごとに専用コードを書く
//! のではなく、JSON構造から機械的にHTMLへ変換する(スカラー値は`<p>`、
//! 文字列配列は`<ul>`、同一キー構成のオブジェクト配列は表、
//! それ以外のオブジェクト配列・ネストしたオブジェクトは再帰)。

mod cron;
mod scraper;
mod seed_urls;

use open_runo_router::hyper_compat::{self, Params};
use hyper::{Method, StatusCode};
// JSONは速い値の型(`sonic_rs::Value`)で読む(RS-JSONの方針、2026-10-16)。
// 旧`serde_json::Value`はキーをアルファベット順に持っていたため、表の列順などが変わらないよう
// `sorted_entries`で同じ順序に揃えている。
use sonic_rs::{JsonContainerTrait, JsonValueTrait, Value};
use std::sync::Arc;
use once_cell::sync::Lazy;
use serde::Deserialize;

const CACHE_BASE: &str = "https://audiocafe.tokyo";
/// 実バグ修正(2026-09-28、続き): `audiocafe-tokyo-rust.service`
/// (このアプリ本体、`WorkingDirectory=/root/audiocafe-tokyo-rust`)と
/// `audiocafe-cron-rust.service`(`--cron-all`、`WorkingDirectory=
/// /var/www/audiocafe.tokyo`)が別々の作業ディレクトリを持つため、
/// CWD相対パスでの読み書きでは一致しない(このアプリのCWDから見た
/// `aruaru/doda-jobs-cache.json`は存在しないファイルを指してしまう)。
/// 静的Webルート(nginx等の実配信ディレクトリ、cronの書き込み先と一致)
/// を絶対パスで固定し、CWDに依存せず常に同じ場所を読むようにする。
const CACHE_DIR: &str = "/var/www/audiocafe.tokyo";
const ARUARU_TOKYO_URL: &str = "https://aruaru.tokyo/";
/// 東京都西部の暮らし・テレワーク紹介 + open-cosmoエコシステムの入口
/// (2026-07-20追記、ユーザー指示: 「aruaru.tokyo と runo.tokyo へのリンクを
/// audiocafe.tokyo内にも貼って」)。
const RUNO_TOKYO_URL: &str = "https://runo.tokyo/";

/// ユーザー提供のブログ記事(タイトルをリンクテキストにし、URLそのものは
/// 表示しない、2026-07-20追記)。トップページのYouTube紹介タイトルの
/// 上に配置する(ユーザー指示)。
const TOP_AMP_HTML: &str = r#"<p class="note" style="font-size:1.2rem"><strong>最新: VER8 と AVアンプ (SV-VER10) — 電源入力の設計案 (100〜240V 自動対応・50/60Hz・電力線での機器認識・双方が対応のときだけ 120Hz) / Latest: power-input design</strong><br><a href="https://github.com/aon-co-jp/open-network-amp/tree/main/VER8" target="_blank" rel="noopener noreferrer">VER8</a> · <a href="https://github.com/aon-co-jp/open-network-amp/blob/main/VER8/power_input.jpg" target="_blank" rel="noopener noreferrer">VER8 概念図と8言語の説明</a> · <a href="https://github.com/aon-co-jp/open-network-amp/blob/main/VER8/power-input-design-ja.md" target="_blank" rel="noopener noreferrer">詳細 (日本語)</a> · <a href="https://github.com/aon-co-jp/open-av-amp/tree/main/SV-VER10" target="_blank" rel="noopener noreferrer">AVアンプ SV-VER10</a> · <a href="https://github.com/aon-co-jp/open-av-amp/blob/main/SV-VER10/power_input.jpg" target="_blank" rel="noopener noreferrer">SV-VER10 概念図と8言語の説明</a></p>
<p class="note" style="font-size:1.2rem"><strong>ネットワーク対応プリメインアンプ・サブウーファー専用端子の回路図 (VER1〜VER7) / Schematics VER1-VER7 / Schaltpläne / Схемы</strong><br>回路図 (JPG・PDF) と説明書は GitHub で公開しています。 / Schematics (JPG, PDF) and the manual are on GitHub.<br></p>
<a href="https://github.com/aon-co-jp/open-network-amp/tree/main/VER1" target="_blank" rel="noopener noreferrer">VER1</a> · <a href="https://github.com/aon-co-jp/open-network-amp/tree/main/VER2" target="_blank" rel="noopener noreferrer">VER2</a> · <a href="https://github.com/aon-co-jp/open-network-amp/tree/main/VER3" target="_blank" rel="noopener noreferrer">VER3</a> · <a href="https://github.com/aon-co-jp/open-network-amp/tree/main/VER4" target="_blank" rel="noopener noreferrer">VER4</a> · <a href="https://github.com/aon-co-jp/open-network-amp/tree/main/VER5" target="_blank" rel="noopener noreferrer">VER5</a> · <a href="https://github.com/aon-co-jp/open-network-amp/tree/main/VER6" target="_blank" rel="noopener noreferrer">VER6</a> · <a href="https://github.com/aon-co-jp/open-network-amp/tree/main/VER7" target="_blank" rel="noopener noreferrer">VER7</a><br><a href="https://github.com/aon-co-jp/open-network-amp" target="_blank" rel="noopener noreferrer">open-network-amp (README)</a><br><strong>VER6 とアナログ版 (A級・AB級・D級アナログ・真空管) / VER6 and analog versions</strong><br><a href="https://github.com/aon-co-jp/open-network-amp/tree/main/VER6" target="_blank" rel="noopener noreferrer">VER6</a> · <a href="https://github.com/aon-co-jp/open-network-amp/tree/main/VER7" target="_blank" rel="noopener noreferrer">VER7</a> · <a href="https://github.com/aon-co-jp/open-network-amp/blob/main/VER6/amp_ver6_classA.jpg" target="_blank" rel="noopener noreferrer">A級 / Class A</a> · <a href="https://github.com/aon-co-jp/open-network-amp/blob/main/VER6/amp_ver6_classAB.jpg" target="_blank" rel="noopener noreferrer">AB級 / Class AB</a> · <a href="https://github.com/aon-co-jp/open-network-amp/tree/main/classd-analog/CD-VER2" target="_blank" rel="noopener noreferrer">D級アナログ / Class D analog (CD-VER2)</a> · <a href="https://github.com/aon-co-jp/open-network-amp/blob/main/VER6/amp_ver6_tube.jpg" target="_blank" rel="noopener noreferrer">真空管 / Vacuum tube</a> · <a href="https://github.com/aon-co-jp/open-network-amp/tree/main/VER5/analog-amp" target="_blank" rel="noopener noreferrer">VER5 アナログ版すべて / VER5 analog figures</a><br><strong>VER7 (12畳以上・小音量から実用音量 / 8言語) / VER7</strong><br><a href="https://github.com/aon-co-jp/open-network-amp/tree/main/VER7" target="_blank" rel="noopener noreferrer">VER7</a> · <a href="https://github.com/aon-co-jp/open-network-amp/blob/main/VER7/amp_ver7_classA.jpg" target="_blank" rel="noopener noreferrer">A級 (VER7)</a> · <a href="https://github.com/aon-co-jp/open-network-amp/blob/main/VER7/amp_ver7_classA_AB.jpg" target="_blank" rel="noopener noreferrer">A級+AB級 (VER7)</a> · <a href="https://github.com/aon-co-jp/open-network-amp/blob/main/VER7/amp_ver7_classAB.jpg" target="_blank" rel="noopener noreferrer">AB級 (VER7)</a> · <a href="https://github.com/aon-co-jp/open-network-amp/blob/main/VER7/amp_ver7_classD.jpg" target="_blank" rel="noopener noreferrer">D級アナログ (VER7)</a> · <a href="https://github.com/aon-co-jp/open-network-amp/blob/main/VER7/amp_ver7_tube_AB.jpg" target="_blank" rel="noopener noreferrer">真空管プリ+AB級 (VER7)</a> · <a href="https://github.com/aon-co-jp/open-network-amp/blob/main/VER7/amp_ver7_tube_A_AB.jpg" target="_blank" rel="noopener noreferrer">真空管プリ+A+AB級 (VER7)</a> · <a href="https://github.com/aon-co-jp/open-network-amp/blob/main/VER7/amp_ver7_tube_D.jpg" target="_blank" rel="noopener noreferrer">真空管プリ+クラスDアナログ (VER7)</a> · <a href="https://github.com/aon-co-jp/open-network-amp/blob/main/VER7/amp_ver7_sim.jpg" target="_blank" rel="noopener noreferrer">仮想シミュレーション (5回)</a> · <a href="https://github.com/aon-co-jp/open-network-amp/blob/main/VER7/amp_ver7b_sim.jpg" target="_blank" rel="noopener noreferrer">追加の仮想シミュレーション TEST 6-10</a> · <a href="https://github.com/aon-co-jp/open-network-amp/blob/main/VER7/manual-v7.jpg" target="_blank" rel="noopener noreferrer">説明書 8言語 / Manual</a><br><strong>ヘッドホンアンプ (前面・全バージョンとAVアンプ共通) / Headphone amplifier</strong><br><a href="https://github.com/aon-co-jp/open-network-amp/tree/main/headphone/HP-VER2" target="_blank" rel="noopener noreferrer">HP-VER2</a> · <a href="https://github.com/aon-co-jp/open-network-amp/blob/main/headphone/HP-VER2/amp_hp2.jpg" target="_blank" rel="noopener noreferrer">回路図 (3.5・4.4・6.35mm・XLR)</a> · <a href="https://github.com/aon-co-jp/open-network-amp/blob/main/headphone/HP-VER2/amp_hp2_sim.jpg" target="_blank" rel="noopener noreferrer">仮想シミュレーション</a> · <a href="https://github.com/aon-co-jp/open-network-amp/blob/main/headphone/HP-VER2/amp_sw.jpg" target="_blank" rel="noopener noreferrer">ON/OFFスイッチ (スピーカー・ヘッドホン・サブウーファー)</a> · <a href="https://github.com/aon-co-jp/open-network-amp/blob/main/headphone/HP-VER2/amp_sw_sim.jpg" target="_blank" rel="noopener noreferrer">スイッチの仮想シミュレーション</a> · <a href="https://github.com/aon-co-jp/open-network-amp/blob/main/headphone/HP-VER2/manual-hp2.jpg" target="_blank" rel="noopener noreferrer">説明書 8言語</a> · <a href="https://github.com/aon-co-jp/open-network-amp/tree/main/headphone/HP-VER3" target="_blank" rel="noopener noreferrer">HP-VER3 ゲインと音量の調整 (各1dB刻み・3モード)</a> · <a href="https://github.com/aon-co-jp/open-network-amp/tree/main/headphone/HP-VER4" target="_blank" rel="noopener noreferrer">HP-VER4 音量・低音ゲイン・高音ゲイン</a> · <a href="https://github.com/aon-co-jp/open-network-amp/blob/main/headphone/HP-VER4/amp_gain4.jpg" target="_blank" rel="noopener noreferrer">回路図</a> · <a href="https://github.com/aon-co-jp/open-network-amp/blob/main/headphone/HP-VER4/amp_gain4_sim.jpg" target="_blank" rel="noopener noreferrer">仮想シミュレーション</a> · <a href="https://github.com/aon-co-jp/open-network-amp/blob/main/headphone/HP-VER3/amp_gain3.jpg" target="_blank" rel="noopener noreferrer">ゲイン調整の回路図</a> · <a href="https://github.com/aon-co-jp/open-network-amp/blob/main/headphone/HP-VER3/amp_gain3_sim.jpg" target="_blank" rel="noopener noreferrer">仮想シミュレーション</a><br><span style="font-size:1rem">真空管はプリアンプ段のみ (パワー段は半導体) / The valve is the preamplifier stage only.</span><br><strong>サラウンドAVアンプ (完全バランス/アンバランス) / Surround AV amplifier</strong><br><a href="https://github.com/aon-co-jp/open-av-amp" target="_blank" rel="noopener noreferrer">open-av-amp</a> · <a href="https://github.com/aon-co-jp/open-av-amp/tree/main/SV-VER1" target="_blank" rel="noopener noreferrer">SV-VER1</a> · <a href="https://github.com/aon-co-jp/open-av-amp/tree/main/SV-VER2" target="_blank" rel="noopener noreferrer">SV-VER2 (D級)</a> · <a href="https://github.com/aon-co-jp/open-av-amp/tree/main/SV-VER3" target="_blank" rel="noopener noreferrer">SV-VER3</a> · <a href="https://github.com/aon-co-jp/open-av-amp/tree/main/SV-VER4" target="_blank" rel="noopener noreferrer">SV-VER4</a> · <a href="https://github.com/aon-co-jp/open-av-amp/tree/main/SV-VER5" target="_blank" rel="noopener noreferrer">SV-VER5 (VER7 対応・D級アナログのみ)</a> · <a href="https://github.com/aon-co-jp/open-av-amp/tree/main/SV-VER6" target="_blank" rel="noopener noreferrer">SV-VER6 (Android OS・ブラウザ・Wi-Fi・Bluetooth・LAN)</a> · <a href="https://github.com/aon-co-jp/open-av-amp/tree/main/SV-VER7" target="_blank" rel="noopener noreferrer">SV-VER7 (前面ヘッドホンアンプ)</a> · <a href="https://github.com/aon-co-jp/open-av-amp/tree/main/SV-VER8" target="_blank" rel="noopener noreferrer">SV-VER8 (図にヘッドホン・ON/OFF・ゲインと音量を描き込み)</a> · <a href="https://github.com/aon-co-jp/open-av-amp/tree/main/SV-VER9" target="_blank" rel="noopener noreferrer">SV-VER9 (音量と低音・高音ゲイン)</a>"#;

const TOP_VIDEO_HTML: &str = r#"<p style="max-width:960px;margin:1rem auto .8rem;font-size:1.3rem;font-weight:700;text-align:center;">ネットから転載致しました。</p>
<div style="max-width:960px;margin:0 auto 1.4rem;padding:1.4rem 1.6rem;background:#fff;color:#1e2a5e;border-radius:12px;text-align:left;line-height:1.6;box-shadow:0 1px 6px rgba(0,0,0,.25);">
<p style="margin:0 0 .3rem;font-weight:700;color:#e07b39;font-size:1rem;">フリーランス市場の現在地</p>
<h2 style="margin:0 0 .5rem;font-size:clamp(1.5rem,4.2vw,2.3rem);line-height:1.3;color:#1e2a5e;">案件は減っていない。求められる役割が変わった。</h2>
<p style="margin:0 0 1rem;color:#5a6178;font-size:1rem;">直近半年でエンジニア歴の長い方ほど実感されている変化を、市場データと合わせて整理しました。</p>
<div style="display:flex;flex-wrap:wrap;gap:.9rem;margin-bottom:1.2rem;">
<div style="flex:1 1 220px;background:#f3f5fb;border-radius:10px;padding:.9rem 1.1rem;"><div style="font-size:1.9rem;font-weight:800;color:#1e2a5e;">横ばい</div><div style="font-weight:700;margin-top:.2rem;">案件の母数</div><div style="color:#5a6178;font-size:.9rem;">総数は減っていない</div></div>
<div style="flex:1 1 220px;background:#f3f5fb;border-radius:10px;padding:.9rem 1.1rem;"><div style="font-size:1.9rem;font-weight:800;color:#e07b39;">減少</div><div style="font-weight:700;margin-top:.2rem;">実装がメインの案件</div><div style="color:#5a6178;font-size:.9rem;">AI による代替が進行</div></div>
<div style="flex:1 1 220px;background:#f3f5fb;border-radius:10px;padding:.9rem 1.1rem;"><div style="font-size:1.9rem;font-weight:800;color:#1e2a5e;">増加</div><div style="font-weight:700;margin-top:.2rem;">上流・設計の案件</div><div style="color:#5a6178;font-size:.9rem;">「何をどう作るか」</div></div>
</div>
<div style="display:flex;flex-wrap:wrap;gap:1rem;align-items:stretch;margin-bottom:1.1rem;">
<div style="flex:1 1 280px;"><p style="margin:0 0 .4rem;font-weight:700;color:#4a5170;">これまで求められていたもの</p><div style="border:1px solid #cfd5e6;border-radius:10px;padding:.9rem 1rem;color:#3a4160;"><ul style="margin:0;padding-left:1.3rem;color:#3a4160;"><li style="margin:.35rem 0;">技術に詳しく、任せれば実装しきれる人</li><li style="margin:.35rem 0;">決まった仕様を、漏れなく品質高く形にする</li><li style="margin:.35rem 0;">言語・フレームワークの経験年数で評価</li><li style="margin:.35rem 0;">チームの一員としての開発工数の提供</li></ul></div></div>
<div style="align-self:center;color:#e07b39;font-size:2rem;">&#10140;</div>
<div style="flex:1 1 280px;"><p style="margin:0 0 .4rem;font-weight:700;color:#e07b39;">いま求められているもの</p><div style="background:#1e2a5e;border-radius:10px;padding:.9rem 1rem;color:#fff;"><ul style="margin:0;padding-left:1.3rem;color:#fff;"><li style="margin:.35rem 0;">ビジネス側と伴走し、要求を仕様に落とせる</li><li style="margin:.35rem 0;">ふわっとした依頼を組み替えて形にする柔軟性</li><li style="margin:.35rem 0;">複雑なドメイン・業務フローを理解しにいける</li><li style="margin:.35rem 0;">実装は AI 前提。人は意思決定と設計に時間を使う</li></ul></div></div>
</div>
<div style="background:#fbeee4;border-radius:10px;padding:.8rem 1rem;color:#3a4160;"><b style="color:#e07b39;">同時に起きていること</b>　企業側の内製化で業務委託枠が縮小／PoC 止まりの AI 案件も減り、実務に組み込んで効果を出すフェーズへ</div>
</div>
<div style="max-width:960px;margin:0 auto 1.4rem;padding:1.4rem 1.6rem;background:#fff;color:#1e2a5e;border-radius:12px;text-align:left;line-height:1.6;box-shadow:0 1px 6px rgba(0,0,0,.25);">
<p style="margin:0 0 .3rem;font-weight:700;color:#e07b39;font-size:1rem;">条件面のリアル</p>
<h2 style="margin:0 0 .5rem;font-size:clamp(1.5rem,4.2vw,2.3rem);line-height:1.3;color:#1e2a5e;">報酬・稼働・選考、この半年で動いた 3 つの数字</h2>
<p style="margin:0 0 1rem;color:#5a6178;font-size:1rem;">下がったのは「入口」。稼働後の単価アップの水準は以前と大きく変わっていません。</p>
<div style="display:flex;flex-wrap:wrap;gap:1.2rem;margin-bottom:1.2rem;">
<div style="flex:1 1 320px;"><p style="margin:0 0 .5rem;font-weight:700;">① 報酬レンジ（税別・週 5 稼働）</p><div style="background:#f3f5fb;border-radius:10px;padding:1rem 1.1rem;"><div style="display:flex;flex-wrap:wrap;align-items:center;gap:.8rem;"><div><div style="color:#7a8199;font-size:.85rem;">以前</div><div style="font-size:1.7rem;font-weight:800;color:#9aa1b8;">80～90 万</div></div><div style="color:#b6bfd6;font-size:1.6rem;">&#10140;</div><div><div style="color:#e07b39;font-size:.85rem;font-weight:700;">現在</div><div style="font-size:2rem;font-weight:800;">70～85 万</div></div></div><div style="margin-top:.8rem;background:#fbeee4;border-radius:8px;padding:.5rem .8rem;font-size:.9rem;">開発メイン案件は 10～15 万ダウン。予算は AI 関連の投資へ移動。</div></div></div>
<div style="flex:1 1 320px;"><p style="margin:0 0 .5rem;font-weight:700;">② 稼働形態の内訳</p>
<div style="display:flex;align-items:center;gap:.6rem;margin:.5rem 0;"><div style="width:9.5rem;text-align:right;font-size:.9rem;">フルリモート</div><div style="height:26px;width:15%;background:#c9d8fb;"></div><div style="font-size:.9rem;font-weight:700;">15%</div></div>
<div style="display:flex;align-items:center;gap:.6rem;margin:.5rem 0;"><div style="width:9.5rem;text-align:right;font-size:.9rem;">常駐</div><div style="height:26px;width:20%;background:#7a86ab;"></div><div style="font-size:.9rem;font-weight:700;">20%</div></div>
<div style="display:flex;align-items:center;gap:.6rem;margin:.5rem 0;"><div style="width:9.5rem;text-align:right;font-size:.9rem;">出社・リモート併用</div><div style="height:26px;width:65%;background:#1e2a5e;"></div><div style="font-size:.9rem;font-weight:700;">65%</div></div>
<p style="margin:.6rem 0 0;color:#5a6178;font-size:.9rem;">「月 1～週 1 なら出社可」と言えるだけで、決まるスピードが段違いに変わります。</p></div>
</div>
<p style="margin:0 0 .5rem;font-weight:700;">③ 選考の通り方（10 件エントリーした場合の目安）</p>
<div style="display:flex;flex-wrap:wrap;gap:.5rem;margin-bottom:1.1rem;"><div style="flex:1 1 150px;background:#1e2a5e;color:#fff;border-radius:10px;padding:.9rem 1rem;"><div style="font-size:1.6rem;font-weight:800;">10 件</div><div style="font-size:.9rem;">エントリー</div></div><div style="align-self:center;color:#b6bfd6;font-size:1.6rem;">&#9654;</div><div style="flex:1 1 150px;background:#1e2a5e;color:#fff;border-radius:10px;padding:.9rem 1rem;"><div style="font-size:1.6rem;font-weight:800;">10～15%</div><div style="font-size:.9rem;">書類通過率</div></div><div style="align-self:center;color:#b6bfd6;font-size:1.6rem;">&#9654;</div><div style="flex:1 1 150px;background:#c9d8fb;color:#1e2a5e;border-radius:10px;padding:.9rem 1rem;"><div style="font-size:1.6rem;font-weight:800;">1～2 件</div><div style="font-size:.9rem;">企業面談</div></div><div style="align-self:center;color:#b6bfd6;font-size:1.6rem;">&#9654;</div><div style="flex:1 1 150px;background:#e07b39;color:#fff;border-radius:10px;padding:.9rem 1rem;"><div style="font-size:1.6rem;font-weight:800;">約 30%</div><div style="font-size:.9rem;">面談後の内定率</div></div></div>
<div style="background:#f3f5fb;border-radius:10px;padding:.8rem 1rem;"><b>難所は「書類」</b>　経歴書が AI できれいに作れるようになり、「まず会って判断」から「書かれていなければ見送り」へ。</div>
</div>
<div style="max-width:960px;margin:0 auto 1.4rem;padding:1.4rem 1.6rem;background:#fff;color:#1e2a5e;border-radius:12px;text-align:left;line-height:1.6;box-shadow:0 1px 6px rgba(0,0,0,.25);">
<p style="margin:0 0 .3rem;font-weight:700;color:#e07b39;font-size:1rem;">AI時代に生き残るフリーランスエンジニアの条件｜案件データで見えた市場の変化</p>
<h2 style="margin:0 0 .5rem;font-size:clamp(1.5rem,4.2vw,2.3rem);line-height:1.3;color:#1e2a5e;">生成AI関連の開発案件は前年比8.4倍、<br>平均単価は約1.8倍に上昇</h2>
<p style="margin:0 0 1rem;color:#5a6178;font-size:1rem;">出典：クラウドソーシング各社の公開集計（2025年）を基に本メディア集計</p>
<p style="margin:0 0 .5rem;font-weight:700;">案件カテゴリ別の変化</p>
<div style="overflow-x:auto;"><table style="border-collapse:collapse;width:100%;min-width:620px;font-size:.95rem;">
<thead><tr><th style="padding:.55rem .7rem;text-align:left;background:#1e2a5e;color:#fff;font-size:.9rem;white-space:nowrap;">案件カテゴリ</th><th style="padding:.55rem .7rem;text-align:left;background:#1e2a5e;color:#fff;font-size:.9rem;white-space:nowrap;">案件数の変化</th><th style="padding:.55rem .7rem;text-align:left;background:#1e2a5e;color:#fff;font-size:.9rem;white-space:nowrap;">単価の変化</th><th style="padding:.55rem .7rem;text-align:left;background:#1e2a5e;color:#fff;font-size:.9rem;white-space:nowrap;">主な理由</th></tr></thead>
<tbody>
<tr style="background:#f3f5fb;"><td style="padding:.55rem .7rem;font-weight:700;">生成AI実装・ファインチューニング</td><td style="padding:.55rem .7rem;color:#e07b39;font-weight:700;">急増(前年比+740%)</td><td style="padding:.55rem .7rem;color:#e07b39;font-weight:700;">大幅上昇</td><td style="padding:.55rem .7rem;font-size:.92rem;">需要増に対して対応できる人材が不足</td></tr>
<tr style="background:#fff;"><td style="padding:.55rem .7rem;font-weight:700;">AIエージェント・自動化ツール開発</td><td style="padding:.55rem .7rem;color:#e07b39;font-weight:700;">急増</td><td style="padding:.55rem .7rem;color:#e07b39;font-weight:700;">高単価維持</td><td style="padding:.55rem .7rem;font-size:.92rem;">企業の業務効率化投資が集中</td></tr>
<tr style="background:#f3f5fb;"><td style="padding:.55rem .7rem;font-weight:700;">既存SaaSへのAI機能追加</td><td style="padding:.55rem .7rem;color:#e07b39;font-weight:700;">増加</td><td style="padding:.55rem .7rem;color:#e07b39;font-weight:700;">やや上昇</td><td style="padding:.55rem .7rem;font-size:.92rem;">API統合スキルが求められる</td></tr>
<tr style="background:#fff;"><td style="padding:.55rem .7rem;font-weight:700;">フルスタック開発(独自案件)</td><td style="padding:.55rem .7rem;color:#5a6178;font-weight:700;">横ばい〜微増</td><td style="padding:.55rem .7rem;color:#5a6178;font-weight:700;">横ばい</td><td style="padding:.55rem .7rem;font-size:.92rem;">差別化できる人材への引き合いが多い</td></tr>
<tr style="background:#f3f5fb;"><td style="padding:.55rem .7rem;font-weight:700;">テンプレサイト制作・WordPressカスタマイズ</td><td style="padding:.55rem .7rem;color:#3b5bdb;font-weight:700;">減少</td><td style="padding:.55rem .7rem;color:#3b5bdb;font-weight:700;">下落</td><td style="padding:.55rem .7rem;font-size:.92rem;">発注側がAIツールで自作できる範囲が拡大</td></tr>
<tr style="background:#fff;"><td style="padding:.55rem .7rem;font-weight:700;">簡単なバグ修正・コード改修</td><td style="padding:.55rem .7rem;color:#3b5bdb;font-weight:700;">大幅減</td><td style="padding:.55rem .7rem;color:#3b5bdb;font-weight:700;">下落</td><td style="padding:.55rem .7rem;font-size:.92rem;">AIコーディングツールで対応できるケースが増加</td></tr>
<tr style="background:#f3f5fb;"><td style="padding:.55rem .7rem;font-weight:700;">HTMLコーディング(デザインカンプ実装)</td><td style="padding:.55rem .7rem;color:#3b5bdb;font-weight:700;">減少</td><td style="padding:.55rem .7rem;color:#3b5bdb;font-weight:700;">下落傾向</td><td style="padding:.55rem .7rem;font-size:.92rem;">AIコーディングツールの精度向上</td></tr>
</tbody></table></div>
<p style="margin:1rem 0 0;font-size:.95rem;">転載元: <a href="https://withcode.tech/media/freelance-engineer-survive-ai-era/" target="_blank" rel="noopener noreferrer" style="color:#3b5bdb;">https://withcode.tech/media/freelance-engineer-survive-ai-era/</a></p>
</div>
<div style="max-width:960px;margin:0 auto 1.4rem;padding:1rem 1.2rem;border:1px solid rgba(128,128,128,.5);border-radius:10px;text-align:left;line-height:1.8;font-size:1.05rem;">
<p style="margin:0 0 .7rem;padding-left:.8rem;border-left:3px solid rgba(128,128,128,.6);">CLAUDEでASICや組み込みアプリやデバイスドライバ開発はC++やRustで可能ですか?</p>
<p style="margin:0 0 .7rem;padding-left:.8rem;border-left:3px solid rgba(128,128,128,.6);">スマホアプリやWEBサイトも、HTML5やCSS3やTypeScriptやNode.jsやRestAPIなどを不使用で、Rust+RPoemなどで開発するには、ClaudeにRustやRPoemやPoemなどの公式URLやGitHubやBlogなどのURLを提示して必要に応じて読んで参考にしてもらいながら、README・CLAUDE・PORTINGなどのファイルを定期的に読み書きしてもらいながら開発していった方が歩留まりが少ないようですが、いかがですか?</p>
<p style="margin:0 0 .7rem;">RPoem の GitHub 公式リポジトリの README・PORTING・CLAUDE などを読んだうえでの回答です。</p>
<p style="margin:0 0 .7rem;">RPoem は、GraphQL COSMOでREST API を不要として、Poem と Tauri の互換機能を Rust で一から自作したものです。管理画面は TypeScript・Node.js を使わず、Rust から WebAssembly で作っています。</p>
<p style="margin:0 0 .7rem;">RPoem は、REST を不要にするための WunderGraph COSMO 互換です。</p>
<p style="margin:1.1rem 0 .4rem;font-size:1.2rem;font-weight:700;">RPoem が実際にどういうものか</p>
<ul style="margin:0 0 .8rem;padding-left:1.4rem;"><li style="margin:.3rem 0;">Rust 製の GraphQL COSMO プラットフォームです。</li><li style="margin:.3rem 0;">Poem(バックエンド)も Tauri(フロントエンド)も、パッケージには依存せず、互換の機能を一から自作しています。</li><li style="margin:.3rem 0;">REST は不要にできます。PORTING には「REST API不要」とあります。</li><li style="margin:.3rem 0;">管理画面は Rust から WebAssembly を作って動かし、「TypeScript/Node.js 不使用」と書かれています。</li><li style="margin:.3rem 0;">Android 用のフォルダ(android/)と、Windows のトレイアプリ(apps/desktop-tray)があります。</li></ul>
<p style="margin:1.1rem 0 .4rem;font-size:1.2rem;font-weight:700;">1. ASIC・組み込み・デバイスドライバは、C++やRustで開発できますか?</p>
<p style="margin:0 0 .7rem;">はい、できます。ただし、できることの範囲には差があります。RPoem の資料に組み込みやドライバの開発の話は出てきません(RPoem は Web とサーバー向けの基盤です)ので、以下は Claude 一般の能力の話です。</p>
<ul style="margin:0 0 .8rem;padding-left:1.4rem;"><li style="margin:.3rem 0;">組み込み・デバイスドライバ: C/C++ と Rust のどちらでも、コードの作成、レビュー、バグ調査を手伝えます。<ul style="margin:0 0 .8rem;padding-left:1.4rem;"><li style="margin:.3rem 0;">組み込み Rust(no_std、embedded-hal、RTIC、Embassy など)や、Linux カーネルモジュール、Windows ドライバー(WDK)も対象です。</li><li style="margin:.3rem 0;">実機が絡む部分(レジスタの挙動、タイミング、割り込み)は、データシートや実機のログがないと間違えやすくなります。</li><li style="margin:.3rem 0;">書き込んで動かす検証は、最終的に人が行う必要があります。</li></ul></li><li style="margin:.3rem 0;">ASIC: 実際に設計するのはハードウェア記述言語(Verilog、SystemVerilog、VHDL)です。Chisel のようなハードウェア用の言語もあります。<ul style="margin:0 0 .8rem;padding-left:1.4rem;"><li style="margin:.3rem 0;">RTL、テストベンチ、検証スクリプトは書けます。</li><li style="margin:.3rem 0;">論理合成、配置配線、タイミング収束には EDA ツール(多くは有償)が必要です。</li><li style="margin:.3rem 0;">Claude はそのツールを代わりに動かせず、製造に出す設計は専門家のレビューが必須です。</li></ul></li><li style="margin:.3rem 0;">安全性が重い分野(車載、医療、航空): 生成したコードをそのまま使わず、規格に沿った検証とレビューを前提にしてください。</li></ul>
<p style="margin:1.1rem 0 .4rem;font-size:1.2rem;font-weight:700;">2. 公式URLを渡し、README・CLAUDE・PORTING を読み書きさせる進め方は、歩留まりがよいですか?</p>
<p style="margin:0 0 .7rem;">はい、その進め方で歩留まりは上がります。RPoem 自身が、PORTING.md を「このファイル1枚で導入できる」と書いて、その運用を前提にしています。</p>
<ul style="margin:0 0 .8rem;padding-left:1.4rem;"><li style="margin:.3rem 0;">公式URLや GitHub を渡す:<ul style="margin:0 0 .8rem;padding-left:1.4rem;"><li style="margin:.3rem 0;">RPoem のような新しい自作の部品は、Claude が事前に知らない可能性が高いです。</li><li style="margin:.3rem 0;">該当コードや README を読ませれば、推測で書く間違いが減ります。</li><li style="margin:.3rem 0;">読ませるときは、バージョンやコミットも指定してください。古い情報を参照するのを防げます。</li><li style="margin:.3rem 0;">RPoem の README の冒頭には「2026-10-28 に依存を最新のメジャー版へ更新」とあり、直近の変更で前提が変わっていることがあります。</li></ul></li><li style="margin:.3rem 0;">README・CLAUDE.md・PORTING.md を読み書きさせる:<ul style="margin:0 0 .8rem;padding-left:1.4rem;"><li style="margin:.3rem 0;">会話が長くなると前の内容を要約で失うため、ファイルに残す方法は有効です。</li><li style="margin:.3rem 0;">決めたこと、環境の落とし穴、未了の作業、再開の手順をファイルに書いておくと、次のセッションでもすぐ再開できます。</li><li style="margin:.3rem 0;">CLAUDE.md は日付つきの履歴で約350KBあります。全部は読ませず、最新の項目と該当箇所を検索して読ませる使い方が効率的です。</li><li style="margin:.3rem 0;">古い記述が残ると誤った前提になるので、更新を依頼のたびに行わせてください。</li></ul></li><li style="margin:.3rem 0;">ほかにも効く工夫:<ul style="margin:0 0 .8rem;padding-left:1.4rem;"><li style="margin:.3rem 0;">テストとビルドを毎回動かさせる。</li><li style="margin:.3rem 0;">実機やブラウザでの確認を省かない。</li><li style="margin:.3rem 0;">依頼を小さく分ける。</li></ul></li></ul>
<p style="margin:1.1rem 0 .4rem;font-size:1.2rem;font-weight:700;">3. スマホアプリやWEBサイトを、HTML5・CSS3・TypeScript・Node.js・REST API なしで作れますか?</p>
<ul style="margin:0 0 .8rem;padding-left:1.4rem;"><li style="margin:.3rem 0;">TypeScript・Node.js: 管理画面は Rust から WebAssembly を作って動かしており、資料に不使用と明記されています。</li><li style="margin:.3rem 0;">REST API: 不使用です。RPoem は REST を不要にするための WunderGraph COSMO 互換で、アプリの API は GraphQL にまとめます。</li><li style="margin:.3rem 0;">HTML5・CSS3: ブラウザの表示の土台は HTML と CSS のままです。WebAssembly も HTML から読み込む構成です。「使わない」のではなく「自分では書かない」が正確です。</li><li style="margin:.3rem 0;">サーバー側: Rust+RPoem だけで作れます。</li><li style="margin:.3rem 0;">スマホ: docs/tauri-parity.md の目標は「iOS/Android を単一コードベースで」です。android/ のフォルダはありますが、iOS まで実機で完成しているかは、読んだ範囲では確認できませんでした。配布の手続きは各OS(署名、ストア)に従う必要があります。</li></ul>
<p style="margin:1.1rem 0 .4rem;font-size:1.2rem;font-weight:700;">お勧め</p>
<p style="margin:0 0 .7rem;">RPoem は汎用ロジックを共有部品にする方針と、README・CLAUDE・PORTING の運用をそのまま続けるのがよいです。画面の見た目だけは、HTML5+CSS3 で書く方が安定します(上のスライドも HTML5+CSS3 で作っています)。</p>
<div lang="en" style="margin-top:1.4rem;padding-top:1rem;border-top:1px solid rgba(128,128,128,.4);">
<p style="margin:0 0 .7rem;padding-left:.8rem;border-left:3px solid rgba(128,128,128,.6);">Can Claude develop ASICs, embedded apps and device drivers in C++ or Rust?</p>
<p style="margin:0 0 .7rem;padding-left:.8rem;border-left:3px solid rgba(128,128,128,.6);">To build smartphone apps and websites in Rust + RPoem without HTML5, CSS3, TypeScript, Node.js or REST APIs, it seems the yield is better if I give Claude the official URLs, GitHub and blog URLs of Rust, RPoem and Poem to read as needed, and have it regularly read and write files such as README, CLAUDE and PORTING while developing. What do you think?</p>
<p style="margin:0 0 .7rem;">This answer is based on reading the README, PORTING and CLAUDE files in the official RPoem GitHub repository.</p>
<p style="margin:0 0 .7rem;">RPoem is a Rust platform that makes REST APIs unnecessary through GraphQL COSMO, with compatible Poem and Tauri functionality built from scratch in Rust. Its admin UI uses neither TypeScript nor Node.js; it is built in Rust and compiled to WebAssembly.</p>
<p style="margin:0 0 .7rem;">RPoem is compatible with WunderGraph COSMO, whose purpose is to make REST unnecessary.</p>
<p style="margin:1.1rem 0 .4rem;font-size:1.2rem;font-weight:700;">What RPoem actually is</p>
<ul style="margin:0 0 .8rem;padding-left:1.4rem;"><li style="margin:.3rem 0;">A GraphQL COSMO platform written in Rust.</li><li style="margin:.3rem 0;">Both Poem (backend) and Tauri (frontend) are rebuilt from scratch as compatible functionality, without depending on those packages.</li><li style="margin:.3rem 0;">REST can be made unnecessary. PORTING says "REST API not needed".</li><li style="margin:.3rem 0;">The admin UI is built in Rust, compiled to WebAssembly, and is documented as "no TypeScript/Node.js".</li><li style="margin:.3rem 0;">There is an Android folder (android/) and a Windows tray app (apps/desktop-tray).</li></ul>
<p style="margin:1.1rem 0 .4rem;font-size:1.2rem;font-weight:700;">1. Can ASICs, embedded software and device drivers be developed in C++ or Rust?</p>
<p style="margin:0 0 .7rem;">Yes, but the range of what can be done differs. The RPoem documents say nothing about embedded or driver development (RPoem is a platform for the web and servers), so the following describes Claude in general.</p>
<ul style="margin:0 0 .8rem;padding-left:1.4rem;"><li style="margin:.3rem 0;">Embedded and device drivers: Claude can help write, review and debug code in either C/C++ or Rust.<ul style="margin:0 0 .8rem;padding-left:1.4rem;"><li style="margin:.3rem 0;">This covers embedded Rust (no_std, embedded-hal, RTIC, Embassy, etc.), Linux kernel modules and Windows drivers (WDK).</li><li style="margin:.3rem 0;">Hardware-dependent parts (register behavior, timing, interrupts) are easy to get wrong without datasheets or logs from the real device.</li><li style="margin:.3rem 0;">Flashing and running it on real hardware must ultimately be verified by a person.</li></ul></li><li style="margin:.3rem 0;">ASIC: the actual design is done in hardware description languages (Verilog, SystemVerilog, VHDL), or hardware languages such as Chisel.<ul style="margin:0 0 .8rem;padding-left:1.4rem;"><li style="margin:.3rem 0;">Claude can write RTL, testbenches and verification scripts.</li><li style="margin:.3rem 0;">Logic synthesis, place and route and timing closure need EDA tools (many are paid).</li><li style="margin:.3rem 0;">Claude cannot run those tools for you, and any design sent to manufacturing requires expert review.</li></ul></li><li style="margin:.3rem 0;">Safety-critical fields (automotive, medical, aviation): do not use generated code as is; assume verification and review according to the relevant standards.</li></ul>
<p style="margin:1.1rem 0 .4rem;font-size:1.2rem;font-weight:700;">2. Is it better to give official URLs and have Claude read and write README, CLAUDE and PORTING files?</p>
<p style="margin:0 0 .7rem;">Yes, this approach improves the yield. RPoem itself says PORTING.md alone is enough to adopt it, and relies on that way of working.</p>
<ul style="margin:0 0 .8rem;padding-left:1.4rem;"><li style="margin:.3rem 0;">Provide official URLs and GitHub:<ul style="margin:0 0 .8rem;padding-left:1.4rem;"><li style="margin:.3rem 0;">New, self-made components such as RPoem are likely unknown to Claude beforehand.</li><li style="margin:.3rem 0;">Having it read the relevant code and README reduces mistakes made by guessing.</li><li style="margin:.3rem 0;">Specify the version or commit when you have it read, to avoid outdated information.</li><li style="margin:.3rem 0;">The top of the RPoem README says dependencies were updated to the latest major versions on 2026-10-28, so premises may have changed recently.</li></ul></li><li style="margin:.3rem 0;">Have it read and write README, CLAUDE.md and PORTING.md:<ul style="margin:0 0 .8rem;padding-left:1.4rem;"><li style="margin:.3rem 0;">When a conversation gets long, earlier content is lost to summarization, so keeping it in files is effective.</li><li style="margin:.3rem 0;">Writing down decisions, environment pitfalls, unfinished work and resume steps lets the next session resume quickly.</li><li style="margin:.3rem 0;">CLAUDE.md is a dated history of about 350 KB. Do not have it read everything; have it search for and read the latest entries and the relevant parts.</li><li style="margin:.3rem 0;">Stale entries become wrong premises, so have it update the files with every request.</li></ul></li><li style="margin:.3rem 0;">Other things that help:<ul style="margin:0 0 .8rem;padding-left:1.4rem;"><li style="margin:.3rem 0;">Run tests and builds every time.</li><li style="margin:.3rem 0;">Do not skip checks on real devices and in the browser.</li><li style="margin:.3rem 0;">Split requests into small pieces.</li></ul></li></ul>
<p style="margin:1.1rem 0 .4rem;font-size:1.2rem;font-weight:700;">3. Can smartphone apps and websites be built without HTML5, CSS3, TypeScript, Node.js or REST APIs?</p>
<ul style="margin:0 0 .8rem;padding-left:1.4rem;"><li style="margin:.3rem 0;">TypeScript and Node.js: the admin UI is built in Rust and compiled to WebAssembly, and the documents explicitly say they are not used.</li><li style="margin:.3rem 0;">REST API: not used. RPoem is compatible with WunderGraph COSMO, which makes REST unnecessary, and the app's API is consolidated into GraphQL.</li><li style="margin:.3rem 0;">HTML5 and CSS3: the foundation of browser display remains HTML and CSS, and WebAssembly is also loaded from HTML. "Not written by hand" is more accurate than "not used".</li><li style="margin:.3rem 0;">Server side: it can be built with Rust + RPoem alone.</li><li style="margin:.3rem 0;">Smartphones: the goal in docs/tauri-parity.md is "iOS/Android from a single codebase". An android/ folder exists, but from what I read I could not confirm that iOS is complete on real devices. Distribution must follow each OS's process (signing, stores).</li></ul>
<p style="margin:1.1rem 0 .4rem;font-size:1.2rem;font-weight:700;">Recommendation</p>
<p style="margin:0 0 .7rem;">Keep to the policy of making general-purpose logic a shared component in RPoem, and keep the README, CLAUDE and PORTING workflow as it is. Only the visual appearance is more stable written in HTML5 + CSS3 (the slides above are also made with HTML5 + CSS3).</p>
</div>
<p style="margin:1rem 0 0;font-size:.95rem;">リソース元 / Sources: <a href="https://github.com/aon-co-jp/RPoem" target="_blank" rel="noopener noreferrer">RPoem リポジトリ / repository</a> ・ <a href="https://github.com/aon-co-jp/RPoem/blob/main/README.md" target="_blank" rel="noopener noreferrer">README.md</a> ・ <a href="https://github.com/aon-co-jp/RPoem/blob/main/PORTING.md" target="_blank" rel="noopener noreferrer">PORTING.md</a> ・ <a href="https://github.com/aon-co-jp/RPoem/blob/main/CLAUDE.md" target="_blank" rel="noopener noreferrer">CLAUDE.md</a> ・ <a href="https://github.com/aon-co-jp/RPoem/blob/main/docs/poem-parity.md" target="_blank" rel="noopener noreferrer">docs/poem-parity.md</a> ・ <a href="https://github.com/aon-co-jp/RPoem/blob/main/docs/tauri-parity.md" target="_blank" rel="noopener noreferrer">docs/tauri-parity.md</a> ・ <a href="https://wundergraph.com/cosmo" target="_blank" rel="noopener noreferrer">WunderGraph COSMO 公式 / official</a> ・ <a href="https://cosmo.wundergraph.com/onboarding/1" target="_blank" rel="noopener noreferrer">COSMO onboarding</a> ・ <a href="https://github.com/wundergraph/cosmo" target="_blank" rel="noopener noreferrer">wundergraph/cosmo (GitHub)</a></p>
</div>
<div class="top-intro" style="margin:1rem auto;padding:.8rem 1rem;max-width:42rem;text-align:left;line-height:1.8;font-size:1.1rem;">
    <p style="margin:0 0 .8rem;">CLAUDEと言うAIをマスターして使いこなしておりますので、<br>様々なプログラム言語とフレームワーク対応可能で、<br>様々なWEBサイトやスマホアプリ開発が可能で御座います。</p>
    <p style="margin:0;"><a href="mailto:norukia.jp@gmail.com">norukia.jp@gmail.com</a><br>
    石　塚　正　浩<br>
    郵便番号　197-0828<br>
    東京都あきる野市秋留5-3-11<br>
    <a href="tel:09075555011">090-7555-5011</a></p>
</div>
<div class="ai-driven-research" style="margin:1rem auto;padding:.8rem 1rem;max-width:42rem;border:1px solid rgba(128,128,128,.5);border-radius:8px;text-align:left;">
    <strong>🔬 研究中：Claudeによる、WEBスキルが無くても日本語（母国語）でWEBサイトやスマホアプリを開発する為のAI駆動開発（研究の一部）</strong><br />
    📝 <a href="https://ameblo.jp/www-aon/entry-12973252437.html" target="_blank" rel="noopener noreferrer">プログラム言語やフレームワークなどの全てをRust（Poemやhyper）版に移植するメリット？</a><br />
    <a href="https://ameblo.jp/www-aon/entry-12973252437.html" target="_blank" rel="noopener noreferrer">https://ameblo.jp/www-aon/entry-12973252437.html</a>
  </div>
<p style="font-size:1.2rem;margin:1rem 0;">政府は、LINEアプリ化して、99％リストラして少子高齢化と年金と後期高齢者無償化予算確保。<br>
    <a href="https://ameblo.jp/www-aon/entry-12981307682.html" target="_blank" rel="noopener noreferrer">https://ameblo.jp/www-aon/entry-12981307682.html</a></p>
<div class="online-learning" style="margin:1rem auto;padding:.8rem 1rem;max-width:42rem;border:1px solid rgba(128,128,128,.5);border-radius:8px;text-align:left;">
    <strong>🎓 オンライン学習サイト / Online learning</strong><br />
    AI駆動開発も学べます。WEBスキルなしで日本語でWEBサイトやスマホアプリが開発出来る様になります。<br />
    <a href="https://progate.com/courses/feature/ai-driven-development/exercises/atmoSFuL468uLDi-Pd-lI" target="_blank" rel="noopener noreferrer">▶️ Progate「AI駆動開発」</a><br />
    HTML,CSSを学ぼう<br />
    <a href="https://progate.com/courses/feature/webpage/exercises/hJQNO8xPIcha_idvTX2dn" target="_blank" rel="noopener noreferrer">▶️ Progate「HTML・CSS」</a><br />
    Pythonなら、無料ならPaizaの<br />
    <a href="https://paiza.jp/works/search_courses/2208" target="_blank" rel="noopener noreferrer">▶️ paizaラーニング「Python3」入門講座一覧</a><br />
    Python プロゲート<br />
    <a href="https://progate.com/courses/python" target="_blank" rel="noopener noreferrer">▶️ Progate「Python入門」</a><br />
    その他無料でPythonが学べます。<br />
    <a href="https://www.python.jp/train/index.html" target="_blank" rel="noopener noreferrer">▶️ python.jp「Python入門講座」</a>
  </div>
<p style="font-size:1.2rem;margin:1rem 0;text-align:center;">
<a href="https://www.youtube.com/watch?v=mrCAz7mU9Zo" target="_blank" rel="noopener noreferrer">▶️ 【1日密着】Claude Codeに取り憑かれたエンジニア｜その衝撃の開発手法に迫る</a><br>
<a href="https://www.facebook.com/reel/1753405299237847?locale=ja_JP" target="_blank" rel="noopener noreferrer">Facebook Backup</a><br>
<a href="https://www.youtube.com/results?search_query=AI%E9%A7%86%E5%8B%95%E9%96%8B%E7%99%BA%20CLAUDE%20CODE%20DESKTOP" target="_blank" rel="noopener noreferrer">▶️ AI駆動開発 CLAUDE CODE DESKTOP</a><br>
<a href="/aruaru">audiocafe.tokyo/aruaru(IT・建築系求人 日本語版)</a><br>
<a href="https://audiocafe.tokyo/aruaru/index-en.php">🌐 audiocafe.tokyo/aruaru (English, translated by Claude Code)</a><br>
<a href="/aruaru-lady">audiocafe.tokyo/aruaru-lady(女性向け求人 日本語版)</a><br>
<a href="https://audiocafe.tokyo/aruaru-lady/index-en.php">🌐 audiocafe.tokyo/aruaru-lady (English, translated by Claude Code)</a></p>

<section class="top-news" style="max-width:640px;margin:1rem auto;text-align:center;">
<div style="max-width:360px;margin:0 auto 1.4rem;">
<p style="font-size:1.2rem;font-weight:700;margin:0 0 .6rem;"><a href="https://www.youtube.com/shorts/FiW4F_XqHdc" target="_blank" rel="noopener noreferrer">ポーランド美女が一番嬉しかった瞬間？</a></p>
<div style="position:relative;width:100%;aspect-ratio:9/16;background:#000;border-radius:8px;overflow:hidden;">
<button type="button" aria-label="動画を再生 / Play video" style="all:unset;position:absolute;inset:0;cursor:pointer;" onclick="var f=document.createElement('iframe');f.src='https://www.youtube-nocookie.com/embed/FiW4F_XqHdc?autoplay=1&amp;playsinline=1&amp;rel=0';f.allow='autoplay; encrypted-media; picture-in-picture';f.allowFullscreen=true;f.title='ポーランド美女が一番嬉しかった瞬間？';f.style.cssText='position:absolute;inset:0;width:100%;height:100%;border:0';this.replaceWith(f)"><img src="https://i.ytimg.com/vi/FiW4F_XqHdc/oar2.jpg" onerror="this.onerror=null;this.src='https://i.ytimg.com/vi/FiW4F_XqHdc/hqdefault.jpg'" alt="ポーランド美女が一番嬉しかった瞬間？" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block;"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);width:64px;height:64px;border-radius:50%;background:rgba(0,0,0,.65);color:#fff;font-size:30px;line-height:64px;text-align:center;">▶</span></button>
</div>
</div>
<div style="max-width:640px;margin:0 auto 1.4rem;">
<p style="font-size:1.2rem;font-weight:700;margin:0 0 .6rem;"><a href="https://www.youtube.com/watch?v=mitYwJ83TaE" target="_blank" rel="noopener noreferrer">不朽の名作 - カウボーイビバップOP | カウボーイビバップ | Netflix Japan</a></p>
<div style="position:relative;width:100%;aspect-ratio:16/9;background:#000;border-radius:8px;overflow:hidden;">
<button type="button" aria-label="動画を再生 / Play video" style="all:unset;position:absolute;inset:0;cursor:pointer;" onclick="var f=document.createElement('iframe');f.src='https://www.youtube-nocookie.com/embed/mitYwJ83TaE?autoplay=1&amp;playsinline=1&amp;rel=0';f.allow='autoplay; encrypted-media; picture-in-picture';f.allowFullscreen=true;f.title='不朽の名作 - カウボーイビバップOP | カウボーイビバップ | Netflix Japan';f.style.cssText='position:absolute;inset:0;width:100%;height:100%;border:0';this.replaceWith(f)"><img src="https://i.ytimg.com/vi/mitYwJ83TaE/hqdefault.jpg" alt="不朽の名作 - カウボーイビバップOP | カウボーイビバップ | Netflix Japan" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block;"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);width:64px;height:64px;border-radius:50%;background:rgba(0,0,0,.65);color:#fff;font-size:30px;line-height:64px;text-align:center;">▶</span></button>
</div>
</div>
<div style="max-width:640px;margin:0 auto 1.4rem;">
<p style="font-size:1.2rem;font-weight:700;margin:0 0 .6rem;"><a href="https://www.youtube.com/watch?v=HDQL_r-FGBM" target="_blank" rel="noopener noreferrer">【ルパン三世カリオストロの城】炎のたからもの【高音質】</a></p>
<div style="position:relative;width:100%;aspect-ratio:16/9;background:#000;border-radius:8px;overflow:hidden;">
<button type="button" aria-label="動画を再生 / Play video" style="all:unset;position:absolute;inset:0;cursor:pointer;" onclick="var f=document.createElement('iframe');f.src='https://www.youtube-nocookie.com/embed/HDQL_r-FGBM?autoplay=1&amp;playsinline=1&amp;rel=0';f.allow='autoplay; encrypted-media; picture-in-picture';f.allowFullscreen=true;f.title='【ルパン三世カリオストロの城】炎のたからもの【高音質】';f.style.cssText='position:absolute;inset:0;width:100%;height:100%;border:0';this.replaceWith(f)"><img src="https://i.ytimg.com/vi/HDQL_r-FGBM/hqdefault.jpg" alt="【ルパン三世カリオストロの城】炎のたからもの【高音質】" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block;"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);width:64px;height:64px;border-radius:50%;background:rgba(0,0,0,.65);color:#fff;font-size:30px;line-height:64px;text-align:center;">▶</span></button>
</div>
</div>
</section>
"#;

const BLOG_POST_URL: &str = "https://ameblo.jp/www-aon/entry-12973252437.html";
const BLOG_POST_TITLE_JA: &str = "プログラム言語やフレームワークなどの全てをRust(Poemやhyper)版に移植するメリット?";
/// ユーザー指示(2026-07-20)により日英両方で掲載。ブログ本文自体は
/// 日本語のみだが、リンクのラベルは英語話者にも内容が伝わるよう
/// 意訳したもの(URLは日英共通、リンク先は変えない)。
const BLOG_POST_TITLE_EN: &str = "The benefits of migrating everything — programming languages, frameworks, and more — to Rust";

/// 2件目のブログ記事(ユーザー指示、2026-08-04追記)。
const BLOG_POST2_URL: &str = "https://ameblo.jp/www-aon/entry-12974607800.html";
const BLOG_POST2_TITLE_JA: &str = "上下水道配管や屋根瓦などのハイテク新素材。パナホームとヤマダホームのコーキングレス外壁";

/// このRust側が対応済みのランキング一覧(表示名・キャッシュファイル名)。
/// 2026-07-17、汎用レンダラーへの書き換えにより全8種類に対応。
const RANKINGS: &[(&str, &str, &str)] = &[
    ("aruaru-caba", "aruaru-caba-ranking-cache.json", "キャバクラ求人 時給帯ランキング(地域別)"),
    ("aruaru-eikaiwa", "aruaru-eikaiwa-ranking-cache.json", "英会話スクール ランキング"),
    ("aruaru-jukujo-caba", "aruaru-jukujo-caba-ranking-cache.json", "熟女キャバ 求人ランキング"),
    ("ai-tech-ranking", "ai-tech-ranking-cache.json", "プログラミング言語・フレームワーク・DBランキング"),
    ("aruaru-learning-prices", "aruaru-learning-prices-cache.json", "学習サービス月額料金"),
    ("rakuten-mobile", "rakuten-mobile-cache.json", "楽天モバイル プラン情報"),
    ("rakuten-intl-call", "rakuten-intl-call-cache.json", "楽天モバイル 国際通話"),
    ("rakuten-platinum", "rakuten-platinum-cache.json", "楽天モバイル プラチナバンド・衛星"),
];

fn page_shell(title: &str, body: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="ja">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
<style>
body {{ font-family: -apple-system, "Hiragino Sans", "Yu Gothic", sans-serif; max-width: 900px; margin: 2rem auto; padding: 0 1rem; line-height: 1.7; color: #222; }}
h1 {{ font-size: 1.6rem; }}
h2 {{ font-size: 1.15rem; margin-top: 2rem; border-bottom: 2px solid #eee; padding-bottom: 0.3rem; }}
table {{ border-collapse: collapse; width: 100%; margin: 1rem 0; }}
th, td {{ border: 1px solid #ddd; padding: 0.4rem 0.6rem; text-align: left; font-size: 0.92rem; }}
th {{ background: #f5f5f5; }}
.disclaimer {{ font-size: 0.8rem; color: #777; }}
a {{ color: #222; }}
a:visited {{ color: #222; }}
nav a {{ margin-right: 1rem; }}
.nav-php-link {{ font-size: 1.4em; font-weight: 800; }}
.site-header-band {{ background: #000; color: #fff; width: 100vw; margin: -2rem calc(-50vw + 50%) 0; padding: 1rem 1rem 0.8rem; box-sizing: border-box; }}
.site-header-band nav a {{ color: #7dd3fc; }}
.site-header-band .nav-php-link {{ color: #fbbf24 !important; }}
.site-header-band .ac-credit {{ color: #fff; background: #000; font-size: 0.85rem; margin-top: 0.5rem; }}
.site-header-band .ac-credit a {{ color: #7dd3fc; }}
</style>
</head>
<body>
{TOP_VIDEO_HTML}
<div class="site-header-band">
<nav><a href="/">TOP</a> <a href="/discover">Discover</a> <a href="/help">困った時は</a> <a href="{ARUARU_TOKYO_URL}">aruaru.tokyo</a> <a href="{RUNO_TOKYO_URL}">runo.tokyo</a> <a href="https://karu.tokyo/" target="_blank" rel="noopener noreferrer">karu.tokyo</a> <a href="/index.php" class="nav-php-link" target="_blank" rel="noopener noreferrer">PHP</a></nav>
<p class="ac-credit">Claude Code DESKTOPというAIに、ITスキルがほとんど無くてもアプリやWEBサイトが作れる技術で、PHP版をRust＋RPoem
（audiocafe.tokyoのRustへの移植プロジェクト:
<a href="https://github.com/aon-co-jp/audiocafe-tokyo-rust" target="_blank" rel="noopener noreferrer">audiocafe-tokyo-rust</a>）
へ日本語で命令して、移植が成功致しました。関連プロジェクト:
<a href="https://github.com/aon-co-jp/RTypeScript" target="_blank" rel="noopener noreferrer">RTypeScript</a>・
<a href="https://github.com/aon-co-jp/RPoem" target="_blank" rel="noopener noreferrer">RPoem</a>・
<a href="https://github.com/aon-co-jp/open-web-server" target="_blank" rel="noopener noreferrer">open-web-server</a>・
<a href="https://github.com/aon-co-jp/RFrontEnd" target="_blank" rel="noopener noreferrer">RFrontEnd</a>・
<a href="https://github.com/aon-co-jp/RReact" target="_blank" rel="noopener noreferrer">RReact</a></p>
</div>
{body}
</body>
</html>"#
    )
}

/// 実バグ修正(2026-09-28): このサーバー自身が`CACHE_BASE`
/// (`https://audiocafe.tokyo`)へ自己参照HTTPリクエストを送る設計だったが、
/// サブディレクトリ付きのパス(`aruaru/doda-jobs-cache.json`等)は
/// リバースプロキシ側の`/aruaru/`向けlocationがこのRustアプリへ丸ごと
/// 転送する設定になっており、アプリ自身は`/aruaru`という固定ページ
/// ルートしか持たないため、キャッシュJSONへの自己リクエストは常に
/// 404で失敗していた(ルートtouch level直下のファイルだけがnginx側の
/// 静的配信で200になり、偶然動いているように見えていた)。この
/// サーバーはキャッシュファイルと同じマシン上で動いているため、HTTP
/// 経由の自己参照は本来不要——まずローカルファイルとして
/// (`--cron-all`の書き込み先と同じ相対パスで)直接読み、失敗した場合
/// のみ従来通りHTTP経由でのフォールバックを試みる。
async fn fetch_cache(filename: &str) -> Result<Value, String> {
    let local_path = std::path::Path::new(CACHE_DIR).join(filename);
    if let Ok(text) = std::fs::read_to_string(&local_path) {
        return parse_cache_json(&text);
    }
    let url = format!("{CACHE_BASE}/{filename}");
    let text = reqwest::get(&url).await.map_err(|e| e.to_string())?.text().await.map_err(|e| e.to_string())?;
    parse_cache_json(&text)
}

/// キャッシュJSONを、`RS-JSON`の安全な厳密モード+速い値の型で読む。
/// 上限は16 MiB・深さ32・重複キー拒否(ローカルのキャッシュは信頼できるが、HTTP経由の
/// フォールバックは外部の入力なので、どちらも同じ上限で読む)。
fn parse_cache_json(text: &str) -> Result<Value, String> {
    let limits = rust_json::Limits { max_bytes: 16 * 1024 * 1024, max_depth: 32, reject_duplicate_keys: true };
    rust_json::parse_secure_sonic(text, &limits).map_err(|e| e.to_string())
}

/// オブジェクトの(キー, 値)を**キーのアルファベット順**で返す。旧`serde_json::Value`
/// (`preserve_order`なし)がこの順序で保持していたため、表の列順・出力順が切替前後で
/// 変わらないよう揃える(`sonic_rs::Value`はJSONの出現順で保持する)。
fn sorted_entries(obj: &sonic_rs::Object) -> Vec<(&str, &Value)> {
    let mut v: Vec<(&str, &Value)> = obj.iter().collect();
    v.sort_by(|a, b| a.0.cmp(b.0));
    v
}

/// 値を表示用の文字列にする(`serde_json::Value::to_string()`相当: 文字列は引用符つきJSON、
/// 数値・真偽値・nullはそのまま、配列・オブジェクトはJSON)。
fn value_to_json_string(v: &Value) -> String {
    sonic_rs::to_string(v).unwrap_or_default()
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// オブジェクト配列を、可能なら表として描画する。各要素が同じキー集合を
/// 持つ場合(このキャッシュ群の大半のケース)は列を揃えた表に、そうでない
/// 場合は要素ごとの箇条書き(キー:値)にフォールバックする。
fn render_object_array(items: &[Value]) -> String {
    let Some(first) = items.first().and_then(|v| v.as_object()) else {
        return String::new();
    };
    let columns: Vec<&str> = sorted_entries(first).into_iter().map(|(k, _)| k).collect();
    let uniform = items.iter().all(|v| {
        v.as_object().map(|o| {
            let keys: Vec<&str> = sorted_entries(o).into_iter().map(|(k, _)| k).collect();
            keys == columns
        }).unwrap_or(false)
    });

    if uniform {
        let mut out = String::from("<table><tr>");
        for col in &columns {
            out.push_str(&format!("<th>{}</th>", html_escape(col)));
        }
        out.push_str("</tr>");
        for item in items {
            out.push_str("<tr>");
            for col in &columns {
                let cell = item.get(*col);
                let rendered = match cell {
                    Some(v) if v.as_str().map(|s| s.starts_with("http://") || s.starts_with("https://")).unwrap_or(false) => {
                        format!(r#"<a href="{0}" target="_blank" rel="noopener noreferrer">🔎 リンク</a>"#, html_escape(v.as_str().unwrap_or("")))
                    }
                    Some(v) if v.as_str().is_some() => html_escape(v.as_str().unwrap_or("")),
                    Some(v) => html_escape(&value_to_json_string(v)),
                    None => String::new(),
                };
                out.push_str(&format!("<td>{rendered}</td>"));
            }
            out.push_str("</tr>");
        }
        out.push_str("</table>");
        out
    } else {
        let mut out = String::from("<ul>");
        for item in items {
            if let Some(obj) = item.as_object() {
                let pairs: Vec<String> = sorted_entries(obj)
                    .into_iter()
                    .map(|(k, v)| format!("{}: {}", html_escape(k), html_escape(&value_to_json_string(v))))
                    .collect();
                out.push_str(&format!("<li>{}</li>", pairs.join(" / ")));
            }
        }
        out.push_str("</ul>");
        out
    }
}

/// JSON値を再帰的にHTMLへ変換する完全汎用レンダラー。8種類全てのキャッシュ
/// 形状(地域別ネスト・フラットrows・複数配列の塊・スカラー値+日英併記
/// フィールド)をこの1関数だけでカバーする。
fn render_value_generic(data: &Value, depth: u8) -> String {
    let mut out = String::new();
    let Some(obj) = data.as_object() else {
        return html_escape(&value_to_json_string(data));
    };

    // 先に見出し的なスカラーフィールド(updated_at/disclaimer等)を出す。
    for key in ["updated_at", "crawled_at", "disclaimer"] {
        if let Some(v) = obj.get(&key).and_then(|v| v.as_str()) {
            out.push_str(&format!("<p class=\"disclaimer\">{}: {}</p>", html_escape(key), html_escape(v)));
        }
    }

    for (key, value) in sorted_entries(obj) {
        if matches!(key, "updated_at" | "crawled_at" | "disclaimer") {
            continue;
        }
        if let Some(s) = value.as_str() {
            if s.starts_with("http://") || s.starts_with("https://") {
                out.push_str(&format!(
                    r#"<p><strong>{}:</strong> <a href="{}" target="_blank" rel="noopener noreferrer">🔎 リンク</a></p>"#,
                    html_escape(key), html_escape(s)
                ));
            } else {
                out.push_str(&format!("<p><strong>{}:</strong> {}</p>", html_escape(key), html_escape(s)));
            }
        } else if value.is_number() || value.is_boolean() {
            out.push_str(&format!("<p><strong>{}:</strong> {}</p>", html_escape(key), html_escape(&value_to_json_string(value))));
        } else if let Some(arr) = value.as_array() {
            let items = arr.as_slice();
            if items.iter().all(|v| v.is_str()) {
                let list: String = items.iter().map(|v| format!("<li>{}</li>", html_escape(v.as_str().unwrap_or("")))).collect();
                out.push_str(&format!("<h{n}>{k}</h{n}><ul>{list}</ul>", n = (depth + 2).min(6), k = html_escape(key)));
            } else if !items.is_empty() && items.iter().all(|v| v.is_object()) {
                out.push_str(&format!("<h{n}>{k}</h{n}>", n = (depth + 2).min(6), k = html_escape(key)));
                out.push_str(&render_object_array(items));
            }
        } else if value.is_object() {
            out.push_str(&format!("<h{n}>{k}</h{n}>", n = (depth + 2).min(6), k = html_escape(key)));
            out.push_str(&render_value_generic(value, depth + 1));
        }
    }
    out
}

fn render_ranking_body(label: &str, data: &Value) -> String {
    format!("<h1>{}</h1>{}", html_escape(label), render_value_generic(data, 0))
}

/// PHP側の`/aruaru/`・`/aruaru-lady/`・`/rakuten-mobile/`サブページは、
/// 複数のキャッシュファイルを1ページに束ねて表示する構成になっている
/// (調査済み、2026-07-17)。同じ構成をRust側でも複合ページとして再現する。
/// キャッシュパスは`CACHE_BASE`からの相対パス(サブディレクトリ込み)。
struct CompositePage {
    slug: &'static str,
    title: &'static str,
    sections: &'static [(&'static str, &'static str)], // (見出し, キャッシュの相対パス)
}

const COMPOSITE_PAGES: &[CompositePage] = &[
    CompositePage {
        slug: "rakuten-mobile",
        title: "楽天モバイル 総合ページ",
        sections: &[
            ("基本プラン", "rakuten-mobile-cache.json"),
            ("国際通話", "rakuten-intl-call-cache.json"),
            ("プラチナバンド・衛星", "rakuten-platinum-cache.json"),
        ],
    },
    CompositePage {
        slug: "aruaru",
        title: "aruaru(IT・建築系求人 総合ページ)",
        sections: &[
            ("キャバクラ求人 時給帯ランキング", "aruaru-caba-ranking-cache.json"),
            ("熟女キャバ 求人ランキング", "aruaru-jukujo-caba-ranking-cache.json"),
            ("英会話スクール ランキング", "aruaru-eikaiwa-ranking-cache.json"),
            ("学習サービス月額料金", "aruaru-learning-prices-cache.json"),
            ("プログラミング言語・フレームワーク・DBランキング", "ai-tech-ranking-cache.json"),
            ("楽天モバイル 基本プラン", "aruaru/rakuten-mobile-cache.json"),
            ("楽天モバイル 国際通話", "aruaru/rakuten-intl-call-cache.json"),
            ("楽天モバイル プラチナバンド・衛星", "aruaru/rakuten-platinum-cache.json"),
            ("楽天モバイル スマホキャンペーン", "aruaru/rakuten-smartphone-cache.json"),
            ("doda 求人情報", "aruaru/doda-jobs-cache.json"),
        ],
    },
    CompositePage {
        slug: "aruaru-lady",
        title: "aruaru-lady(女性向け求人 総合ページ)",
        sections: &[
            ("キャバクラ求人 時給帯ランキング", "aruaru-lady/aruaru-caba-ranking-cache.json"),
            ("熟女キャバ 求人ランキング", "aruaru-lady/aruaru-jukujo-caba-ranking-cache.json"),
            ("TVチャット(通常) ランキング", "aruaru-lady/aruaru-tvchat-normal-ranking-cache.json"),
            ("TVチャット(グループ) ランキング", "aruaru-lady/aruaru-tvchat-group-ranking-cache.json"),
            ("楽天モバイル 基本プラン", "aruaru-lady/rakuten-mobile-cache.json"),
            ("楽天モバイル 国際通話", "aruaru-lady/rakuten-intl-call-cache.json"),
            ("楽天モバイル プラチナバンド・衛星", "aruaru-lady/rakuten-platinum-cache.json"),
        ],
    },
];

/// JSON文字列フィールドを取得し、無ければ`default`を返す(PHP側の
/// `$_rm_rk['price'] ?? '最大3,278円（税込）'`等のnull合体演算子と同じ挙動)。
fn get_str(v: &Value, key: &str, default: &str) -> String {
    v.get(key).and_then(|x| x.as_str()).unwrap_or(default).to_string()
}

fn get_bool(v: &Value, key: &str) -> bool {
    v.get(key).and_then(|x| x.as_bool()).unwrap_or(false)
}

/// PHPの`rawurlencode()`相当(Google検索リンク生成用)。UTF-8バイト単位で
/// 非予約文字を`%XX`に置き換える単純な実装。
fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(*b as char),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

fn google_search_url(query: &str) -> String {
    format!("https://www.google.com/search?q={}", percent_encode(query))
}

/// JSON値を種類を問わず表示用文字列にする(`get_str`は文字列型専用のため、
/// ランキングキャッシュの`rank`のような数値フィールドは拾えない。
/// aruaru-ladyのランキング表描画で数値・文字列混在フィールドを
/// まとめて扱うために追加)。
fn get_disp(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(x) if x.as_str().is_some() => x.as_str().unwrap_or("").to_string(),
        Some(x) if x.is_number() || x.is_boolean() => value_to_json_string(x),
        _ => String::new(),
    }
}

/// aruaru-ladyの各種TOP50ランキング(キャバクラ/熟女キャバ/TVチャットレディ
/// グループ・通常)を、PHP版と同じ列構成(順位+指定カラム+検索リンク)の
/// 表としてレンダリングする。`cols`は(JSONキー, 表示ラベル)のペア。
fn render_rank_table(rows: &[Value], head_color: &str, cols: &[(&str, &str)]) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let mut out = String::from(r#"<div class="ox"><table><thead><tr>"#);
    out.push_str(&format!(r#"<th style="width:48px;color:{head_color};">順位</th>"#));
    for (_, label) in cols {
        out.push_str(&format!(r#"<th style="color:{head_color};">{}</th>"#, html_escape(label)));
    }
    out.push_str(&format!(r#"<th style="color:{head_color};width:110px;">検索</th></tr></thead><tbody>"#));
    for row in rows {
        out.push_str(r#"<tr style="border-bottom:1px solid #4c1d45;">"#);
        out.push_str(&format!(
            r#"<td style="padding:6px 8px;text-align:center;font-weight:bold;color:{head_color};">{}</td>"#,
            html_escape(&get_disp(row, "rank"))
        ));
        for (key, _) in cols {
            out.push_str(&format!(r#"<td style="padding:6px 8px;">{}</td>"#, html_escape(&get_disp(row, key))));
        }
        let url = get_disp(row, "url");
        out.push_str(&format!(
            r#"<td style="padding:6px 8px;"><a href="{}" target="_blank" rel="noopener noreferrer" style="color:{head_color};font-weight:bold;">Google で検索</a></td>"#,
            html_escape(&url)
        ));
        out.push_str("</tr>");
    }
    out.push_str("</tbody></table></div>");
    out
}

/// PHP版`aruaru-lady/index.php`(2916行)の`<style>`ブロック
/// (1424〜1449行目、および1450〜1469行目の一部)から、レイアウトの
/// 核となるダークテーマCSSを移植したもの。Google翻訳ウィジェット/
/// OPEN・CLOSEパネルのJS演出用CSS(1470行目以降)は対象外
/// (今回のスコープは見た目の一致であり、翻訳ウィジェット自体は
/// 別機能のため)。`page_shell`のヘッダー内`<style>`より後(body内)に
/// 出力することで、同名セレクタ(`body`・`a`・`table`等)を上書きする。
/// 多言語版(`audiocafe-tokyo-php`側の`index-*.php`、18言語)への
/// 言語切替ナビ(2026-08-04新設)。このRustサイトの`/aruaru`・
/// `/aruaru-lady`・`/rakuten-mobile`は日本語版のみを描画しており、
/// PHP側で先に用意されていた多言語版ページ(英/英(UK)/独/オーストリア/
/// スイス/伊/仏/西/フィリピン語/露/ウクライナ/アラビア/ペルシャ/
/// ヘブライ語/韓/中国語簡体字・繁体字)への導線が無かったため
/// (ユーザー指示「この3つの英語を基本として翻訳したサイトの上に
/// 世界十数カ国の言語名をナビとして並べてクリックすると読めるように
/// して」)、日本語ページの先頭にこのナビを追加する。リンク先は
/// `open-web-server`側`domains.toml`に追加した`/aruaru/index-`等の
/// 専用prefixルート経由でPHP-FPM(127.0.0.1:4401)へ到達する
/// (`audiocafe-tokyo-php/CLAUDE.md` 2026-08-04続きエントリ参照)。
const LANG_NAV_STYLE: &str = r#"<style>
.lang-nav-bar{display:flex;flex-wrap:wrap;gap:.35rem;justify-content:center;margin:0 0 1.2rem;padding:.6rem;background:rgba(255,255,255,.04);border-radius:.6rem}
.lang-nav-bar a{display:inline-block;padding:.3rem .7rem;border-radius:999px;background:rgba(255,255,255,.08);color:inherit;text-decoration:none;font-size:.82rem;font-weight:600}
.lang-nav-bar a:hover{background:rgba(255,255,255,.18);text-decoration:none}
</style>"#;

fn lang_nav_bar(base: &str) -> String {
    let langs: &[(&str, &str)] = &[
        ("en", "🇺🇸 English"),
        ("en-gb", "🇬🇧 English (UK)"),
        ("de", "🇩🇪 Deutsch"),
        ("at", "🇦🇹 Deutsch (AT)"),
        ("ch", "🇨🇭 Deutsch (CH)"),
        ("it", "🇮🇹 Italiano"),
        ("fr", "🇫🇷 Français"),
        ("es", "🇪🇸 Español"),
        ("tl", "🇵🇭 Filipino"),
        ("ru", "🇷🇺 Русский"),
        ("uk", "🇺🇦 Українська"),
        ("ar", "🇸🇦 العربية"),
        ("fa", "🇮🇷 فارسی"),
        ("he", "🇮🇱 עברית"),
        ("ko", "🇰🇷 한국어"),
        ("zh-cn", "🇨🇳 简体中文"),
        ("zh-tw", "🇹🇼 繁體中文"),
    ];
    let links: String = langs
        .iter()
        .map(|(code, label)| format!(r#"<a href="{base}/index-{code}.php">{label}</a>"#))
        .collect();
    format!(r#"{LANG_NAV_STYLE}<div class="lang-nav-bar">{links}</div>"#)
}

const ARUARU_LADY_STYLE: &str = r#"<style>
.aruaru-lady-page{margin:-2rem -1rem;background:#0a0f1e;color:#e2e8f0;font-family:'Helvetica Neue',Arial,'Hiragino Kaku Gothic ProN',sans-serif;line-height:1.7}
.aruaru-lady-page a{color:#fda4af;text-decoration:none}
.aruaru-lady-page a:hover{text-decoration:underline}
.aruaru-lady-page .wrap{max-width:1200px;margin:0 auto;padding:16px}
.aruaru-lady-page .hero{background:linear-gradient(135deg,#1a0a14 0%,#2d0a1e 50%,#1a0a14 100%);padding:40px 20px;text-align:center;border-bottom:2px solid #be185d}
.aruaru-lady-page .hero h1{font-size:clamp(22px,5vw,36px);color:#fda4af;font-weight:900;line-height:1.3;margin-bottom:12px}
.aruaru-lady-page .hero p{color:#fcd7e0;font-size:15px;opacity:.9;max-width:700px;margin:0 auto}
.aruaru-lady-page .notice{background:rgba(30,58,138,.25);border:1.5px solid #3b82f6;border-radius:12px;padding:12px 16px;margin:20px 0;font-size:15px;color:#bfdbfe;display:flex;align-items:center;gap:10px;flex-wrap:wrap}
.aruaru-lady-page .notice a{color:#93c5fd;font-weight:700;text-decoration:underline}
.aruaru-lady-page .card{background:#110818;border:1px solid #4c1d45;border-radius:16px;padding:24px;margin:28px 0}
.aruaru-lady-page h2{font-size:clamp(18px,4vw,24px);color:#fda4af;margin-bottom:12px;line-height:1.35}
.aruaru-lady-page h3{font-size:clamp(15px,3.5vw,19px);color:#f472b6;margin:24px 0 10px}
.aruaru-lady-page table{width:100%;border-collapse:collapse;font-size:15px}
.aruaru-lady-page thead tr{background:#2d0a1e;text-align:left}
.aruaru-lady-page th{padding:8px 6px;color:#fda4af;white-space:nowrap}
.aruaru-lady-page .ox{overflow-x:auto}
.aruaru-lady-page .toc{display:flex;flex-wrap:wrap;gap:8px;margin:16px 0}
.aruaru-lady-page .toc a{display:inline-block;padding:6px 14px;border-radius:20px;background:rgba(190,24,93,.18);border:1px solid #be185d;color:#fda4af;font-size:15px;font-weight:700}
.aruaru-lady-page .toc a:hover{background:rgba(190,24,93,.35)}
.aruaru-lady-page .cron-box{background:#0c1228;border:1px solid #334155;border-radius:12px;padding:16px 18px;margin-top:32px;font-size:15px}
.aruaru-lady-page .cron-box code{background:#0a1628;padding:2px 8px;border-radius:4px;color:#a5f3fc;font-size:15px}
.aruaru-lady-page footer{text-align:center;padding:32px 16px;color:#64748b;font-size:15px;border-top:1px solid #1e293b;margin-top:48px}
@media(max-width:640px){.aruaru-lady-page .hero{padding:28px 14px}.aruaru-lady-page .card{padding:16px}}
</style>"#;

/// PHP側`aruaru-lady/index.php`(2916行)が実際に表示している内容を移植する。
/// 汎用JSONダンプ(`render_value_generic`)ではPHP版と全く別のページに
/// なってしまうため(2026-07-19監査、`/rakuten-mobile`と同じ問題)、この
/// 関数はPHP版のセクション構成・見出し・静的マーケティング文言(体験入店の
/// エリア一覧、政策提案カード等)を1対1に近い形で再現しつつ、データ部分
/// (キャバクラ/熟女キャバ/TVチャットレディ グループ・通常の各TOP50表)は
/// 既存の`fetch_cache`アーキテクチャ(HTTP経由で`aruaru-lady/*-cache.json`
/// 取得)をそのまま使う。CSSも実PHP版の`<style>`ブロックから移植済み
/// (`ARUARU_LADY_STYLE`、ユーザー指示によりスコープ拡大: 2026-07-19)。
async fn render_aruaru_lady_body() -> String {
    let lang_nav = lang_nav_bar("/aruaru-lady");
    let caba = fetch_cache("aruaru-lady/aruaru-caba-ranking-cache.json").await;
    let jukujo = fetch_cache("aruaru-lady/aruaru-jukujo-caba-ranking-cache.json").await;
    let tv_group = fetch_cache("aruaru-lady/aruaru-tvchat-group-ranking-cache.json").await;
    let tv_normal = fetch_cache("aruaru-lady/aruaru-tvchat-normal-ranking-cache.json").await;

    let empty = Value::default(); // null
    let caba = caba.as_ref().unwrap_or(&empty);
    let jukujo = jukujo.as_ref().unwrap_or(&empty);
    let tv_group = tv_group.as_ref().unwrap_or(&empty);
    let tv_normal = tv_normal.as_ref().unwrap_or(&empty);

    let empty_rows: Vec<Value> = Vec::new();
    let caba_tokyo23: &[Value] = caba.get("tokyo_23").and_then(|v| v.get("rows")).and_then(|v| v.as_array()).map(|a| a.as_slice()).unwrap_or(&empty_rows);
    let caba_tama: &[Value] = caba.get("tokyo_tama").and_then(|v| v.get("rows")).and_then(|v| v.as_array()).map(|a| a.as_slice()).unwrap_or(&empty_rows);
    let caba_national: &[Value] = caba.get("national").and_then(|v| v.get("rows")).and_then(|v| v.as_array()).map(|a| a.as_slice()).unwrap_or(&empty_rows);
    let caba_disclaimer = get_disp(caba, "disclaimer");
    let caba_updated = get_disp(caba, "updated_at");

    let jukujo_rows: &[Value] = jukujo.get("rows").and_then(|v| v.as_array()).map(|a| a.as_slice()).unwrap_or(&empty_rows);
    let jukujo_disclaimer = get_disp(jukujo, "disclaimer");
    let jukujo_updated = get_disp(jukujo, "updated_at");

    let tvg_rows: &[Value] = tv_group.get("rows").and_then(|v| v.as_array()).map(|a| a.as_slice()).unwrap_or(&empty_rows);
    let tvg_disclaimer = get_disp(tv_group, "disclaimer");
    let tvg_updated = get_disp(tv_group, "updated_at");

    let tvn_rows: &[Value] = tv_normal.get("rows").and_then(|v| v.as_array()).map(|a| a.as_slice()).unwrap_or(&empty_rows);
    let tvn_disclaimer = get_disp(tv_normal, "disclaimer");
    let tvn_updated = get_disp(tv_normal, "updated_at");

    let caba_cols: &[(&str, &str)] = &[("area", "エリア"), ("title", "エリア名称・特徴"), ("band", "時給目安帯"), ("pickup", "補足")];
    let jukujo_cols: &[(&str, &str)] = &[("area", "エリア"), ("title", "エリア名称・特徴"), ("lux_band", "高級帯目安"), ("pickup", "補足")];
    let tv_cols: &[(&str, &str)] = &[("plat", "プラットフォーム"), ("title", "サイト/エリア・特徴"), ("band", "時給目安帯"), ("pickup", "補足")];

    let caba_tokyo23_table = render_rank_table(caba_tokyo23, "#fda4af", caba_cols);
    let caba_tama_table = render_rank_table(caba_tama, "#fda4af", caba_cols);
    let caba_national_table = render_rank_table(caba_national, "#fda4af", caba_cols);
    let jukujo_table = render_rank_table(jukujo_rows, "#fb7185", jukujo_cols);
    let tvg_table = render_rank_table(tvg_rows, "#fbbf24", tv_cols);
    let tvn_table = render_rank_table(tvn_rows, "#34d399", tv_cols);

    let tv_chat_nonadult_url = google_search_url("TVチャットレディ　ノンアダルト");
    let we2plus_url = google_search_url("楽天モバイル お勧め 1円 高性能CPU 富士通製 スマホ we2 plus");
    let link_android_url = google_search_url("楽天モバイル スマホアプリの名前は 楽天リンク Android版");
    let link_iphone_url = google_search_url("楽天モバイル スマホアプリの名前は 楽天リンク iPhone版");
    let jobs_ja_url = google_search_url("女性向け 求人情報");
    let jobs_en_url = google_search_url("Job postings for women in Japan");

    // 体験入店ゾーン一覧(PHP版`aruaru_taiken_zone_definitions()`、
    // 990〜980行目付近の13ゾーンをタイトル・紹介文のみ移植した簡略版。
    // PHP版は各ゾーン内をさらに市区町村単位のカードへ展開するが、今回は
    // コンテンツの一致(ゾーン名・紹介文・検索導線)を優先し、市区町村
    // カードの完全複製は省略した)。
    let taiken_zones: &[(&str, &str)] = &[
        ("東京都・23区内", "港区・新宿・渋谷・銀座ほか、区ごとの体験入店・体験勤務の募集例を検索する窓口です。"),
        ("東京都・23区外（多摩地域ほか）", "立川・八王子・町田・吉祥寺など、都心外の多摩・西東京エリアです。"),
        ("首都圏（神奈川・千葉・埼玉）主要都市", "横浜・川崎・千葉・さいたま・川口など、東京23区外の首都圏中核都市です。"),
        ("北海道", "札幌すすきのを中心に、旭川・函館など道内主要・区外エリアです。"),
        ("東北", "仙台・盛岡・青森など東北各県の主要歓楽街・駅前エリアです。"),
        ("関東（茨城・栃木・群馬・山梨）", "北関東・山梨の県都・歓楽街エリアです。"),
        ("甲信越・北陸（長野・新潟ほか）", "長野・新潟・金沢・富山など、中部山地と日本海側の主要エリアです。"),
        ("東海（愛知・静岡・岐阜・三重）", "名古屋・浜松・静岡・岐阜・四日市など東海地方の主要・区外都市です。"),
        ("関西（大阪・京都・神戸・奈良ほか）", "ミナミ・梅田・祇園・三宮など関西の中核と、堺・枚方など府県内区外です。"),
        ("中国地方", "広島・岡山・山口など中国地方の主要都市です。"),
        ("四国", "高松・松山・高知・徳島など四国4県の主要エリアです。"),
        ("九州（福岡・熊本・鹿児島ほか）", "中洲・熊本・天文館など九州各県の主要歓楽街・駅前です。"),
        ("沖縄", "那覇を中心に、沖縄本島の主要エリアです（離島・条例は各店で要確認）。"),
    ];
    let taiken_list: String = taiken_zones
        .iter()
        .map(|(title, intro)| {
            let url = google_search_url(&format!("{title} キャバクラ 熟女キャバ TVチャットレディ 体験入店"));
            format!(
                r#"<li style="margin:0 0 10px;"><a href="{url}" target="_blank" rel="noopener noreferrer" style="color:#bae6fd;font-weight:bold;">📍 {t}</a><br><span style="opacity:.8;font-size:14px;color:#cbd5e1;">{i}</span></li>"#,
                t = html_escape(title),
                i = html_escape(intro)
            )
        })
        .collect();

    format!(
        r##"{ARUARU_LADY_STYLE}
<div class="aruaru-lady-page">
{lang_nav}
<div class="hero">
  <h1>💃 女性向けお仕事情報</h1>
  <p>キャバレー・キャバクラ・熟女キャバ・TVチャットレディの求人・体験入店・時給目安など。各リンクは Google 検索窓口です。必ず各店の公式サイト・求人票・法令をご確認ください。</p>
</div>
<div class="wrap">

<div class="notice">
  <span>🔗</span>
  <span>IT技術・プログラミング・英会話など女性向け以外の情報は</span>
  <a href="/aruaru">audiocafe.tokyo/aruaru</a>
  <span>をご覧ください。</span>
</div>

<div class="toc">
  <a href="#lady-rakuten-mobile-corner">📶 楽天モバイル</a>
  <a href="#lady-jobs-corner">💼 女性向け求人</a>
  <a href="#tv-corner">📱 TVチャットレディ</a>
  <a href="#taiken-corner">🗺️ 体験入店</a>
  <a href="#caba-corner">🚗 キャバクラ時給TOP50</a>
  <a href="#jukujo-corner">✨ 熟女キャバTOP50</a>
</div>

<div class="card" id="lady-rakuten-mobile-corner">
<h2>📶 楽天モバイル</h2>
<p>スマホの乗り換えなら楽天モバイル。プラン・国際通話・プラチナバンドの最新情報は <a href="/rakuten-mobile">/rakuten-mobile</a> をご覧ください。</p>
</div>

<div class="card" id="lady-jobs-corner">
<h2>💼 女性向け・外資系 求人情報</h2>
<p style="color:#cbd5e1;font-size:14px;opacity:.92;">各リンクは Google 検索または転職サイトへの入口です。応募前に必ず各社の公式求人票・条件をご確認ください。</p>
<ul style="font-size:15px;line-height:2.1;padding-left:1.2rem;">
<li><a href="{jobs_ja_url}" target="_blank" rel="noopener noreferrer">🔍 女性向け 求人情報（Google検索）</a></li>
<li><a href="{jobs_en_url}" target="_blank" rel="noopener noreferrer">🔍 Job postings for women in Japan（Google Search）</a></li>
<li><a href="https://www.daijob.com/" target="_blank" rel="noopener noreferrer">🌐 Daijob.com　日本語</a></li>
<li><a href="https://www.daijob.com/en/" target="_blank" rel="noopener noreferrer">🌐 Daijob.com　ENGLISH</a></li>
</ul>
</div>

<div class="card" id="tv-corner">
<h2>📱 TVチャットレディ（ノンアダルト・在宅／駅前体験）</h2>
<p style="opacity:.9;font-size:15px;color:#e0f2fe;">日本全国の駅前には体験できるお店がある場合もあります。事前に検索し、各店の公式サイト（HP）で募集要項・年齢制限・機材・契約内容をご確認ください。自宅や病院で入院中でも、畳一畳分のスペースとスマートフォンがあれば始められる例もあります。</p>
<ul style="font-size:15px;line-height:2.1;padding-left:1.2rem;color:#cbd5e1;">
<li><a href="{tv_chat_nonadult_url}" target="_blank" rel="noopener noreferrer" style="color:#38bdf8;font-weight:bold;">🔍 Google検索：TVチャットレディ　ノンアダルト</a></li>
<li><a href="https://atgroup.jp/" target="_blank" rel="noopener noreferrer" style="color:#7dd3fc;font-weight:bold;">📱 ノンアダルト チャットレディ — atgroup.jp</a></li>
<li><a href="{we2plus_url}" target="_blank" rel="noopener noreferrer">🔍 楽天モバイル お勧め 富士通製 We2 Plus（1円・高性能CPU）</a></li>
<li><a href="{link_android_url}" target="_blank" rel="noopener noreferrer">🔍 楽天リンク Android版</a></li>
<li><a href="{link_iphone_url}" target="_blank" rel="noopener noreferrer">🔍 楽天リンク iPhone版</a></li>
</ul>
<p style="opacity:.88;font-size:15px;color:#cbd5e1;">楽天リンクアプリ経由の通話で電話放題プランなどを利用できる場合があります。<strong style="color:#fef08a;">TVチャットレディ</strong>は、まず<strong>スマートフォン</strong>から始めやすく、できれば<strong>WEBカメラ付きのオンラインPC環境</strong>もあると画質・安定性の面でお勧めです（各求人・各店の指定機材を必ずご確認ください）。</p>
</div>

<div class="card" id="taiken-corner">
<h2>🗺️ 体験入店・体験可能店（全国エリア別）</h2>
<p style="opacity:.88;font-size:15px;color:#c7d2fe;">体験入店・体験勤務の可否・年齢・契約・風俗営業適正化法等は店舗・自治体・求人媒体ごとに異なります。下記は各エリア名での Google 検索窓口のみで、特定店の推奨・保証ではありません。応募前に公式HP・求人票で必ずご確認ください。</p>
<div style="margin-bottom:14px;">
  <span style="color:#7dd3fc;font-weight:bold;margin-right:12px;">TVチャットレディ</span>
  <span style="color:#fb7185;font-weight:bold;margin-right:12px;">熟女キャバ</span>
  <span style="color:#fda4af;font-weight:bold;">キャバレー・キャバクラ</span>
</div>
<ul style="list-style:none;padding:0;">{taiken_list}</ul>
</div>

<div class="card" id="tvchat-group-corner">
<h2>📱 TVチャットレディ【グループチャット（パーティーチャット）版】 全国・高額時給目安ランキング TOP50</h2>
<div style="display:flex;align-items:center;gap:12px;background:linear-gradient(135deg,#7c1d4e 0%,#3b0764 100%);border:2px solid #fbbf24;border-radius:12px;padding:14px 18px;margin-bottom:14px;">
  <div style="flex-shrink:0;background:#fbbf24;color:#1c0a14;font-weight:900;font-size:18px;border-radius:50%;width:52px;height:52px;display:flex;align-items:center;justify-content:center;">No.1</div>
  <div>
    <div style="color:#fbbf24;font-weight:bold;">🏆 高額時給 No.1 — グループチャット（パーティーチャット）</div>
    <div style="color:#fef3c7;font-size:15px;">複数人同時参加で収入が大幅アップ。人数が多い場合、<strong style="color:#fbbf24;">時給 36,000円〜177,000円</strong> の例もあります。</div>
    <div style="margin-top:8px;"><a href="https://atgroup.jp/money" target="_blank" rel="noopener noreferrer" style="display:inline-block;background:#fbbf24;color:#1c0a14;font-weight:bold;padding:7px 20px;border-radius:8px;">💰 ATグループ 高額報酬ページを見る →</a></div>
  </div>
</div>
<p style="opacity:.88;font-size:15px;color:#fde68a;">{tvg_disclaimer}</p>
<p style="opacity:.65;font-size:15px;color:#cbd5e1;">📅 リスト更新（キャッシュ・毎日自動）: {tvg_updated} — 「高額時給順」は報酬率・同時接続効率の目安インデックスによる並びで、特定サイト/店舗の格付けではありません。</p>
{tvg_table}
</div>

<div class="card" id="tvchat-normal-corner">
<h2>📲 TVチャットレディ【通常版／1対1】 全国・高額時給目安ランキング TOP50</h2>
<p style="opacity:.88;font-size:15px;color:#fde68a;">{tvn_disclaimer}</p>
<p style="opacity:.65;font-size:15px;color:#cbd5e1;">📅 リスト更新（キャッシュ・毎日自動）: {tvn_updated} — 「高額時給順」は報酬率・稼働効率の目安インデックスによる並びで、特定サイト/店舗の格付けではありません。</p>
{tvn_table}
</div>

<div class="card" id="caba-corner">
<h2>🚗 キャバクラ・キャバレー 高額時給目安ランキング TOP50</h2>
<div style="margin-bottom:16px;padding:12px 14px;background:rgba(190,24,93,.12);border:1px solid rgba(190,24,93,.35);border-radius:10px;">
  <p style="font-size:15px;color:#fcd7e0;font-weight:700;">▶ 近年、若い男性受け・おじ様向けにも需要ニーズが増加中のスタイル — YouTube で検索：</p>
  <a href="https://www.youtube.com/results?search_query=%E3%82%B3%E3%82%B9%E3%83%97%E3%83%AC+%E3%83%9F%E3%83%8B%E3%82%B9%E3%82%AB+%E5%AD%A6%E5%9C%92+%E3%82%AD%E3%83%A3%E3%83%90" target="_blank" rel="noopener noreferrer" style="display:inline-flex;align-items:center;gap:6px;padding:7px 14px;border-radius:20px;background:#be185d;color:#fff;font-weight:800;">▶ コスプレ　ミニスカ　学園　キャバ</a>
</div>
<p style="opacity:.82;font-size:15px;color:#fecdd3;">いずれも Google 検索窓口へのリンクです。時給は目安であり、各店の求人票を必ずご確認ください。</p>
<h3 id="aruaru-caba-tokyo23">🚗 高額時給目安ランキング TOP50（東京23区内・キャバレー／キャバクラ・検索窓口）</h3>
<p style="opacity:.88;font-size:15px;color:#fde68a;">{caba_disclaimer}</p>
<p style="opacity:.65;font-size:15px;color:#cbd5e1;">📅 リスト更新: {caba_updated} — TTL 約7日</p>
{caba_tokyo23_table}
<h3 id="aruaru-caba-tokyo-tama">🚗 高額時給目安ランキング TOP50（東京23区外・多摩／八王子・立川ほか・検索窓口）</h3>
<p style="opacity:.65;font-size:15px;color:#cbd5e1;">📅 リスト更新: {caba_updated} — TTL 約7日</p>
{caba_tama_table}
<h3 id="aruaru-caba-national">🚗 高額時給目安ランキング TOP50（大阪〜北海道・沖縄・日本全国・検索窓口）</h3>
<p style="opacity:.65;font-size:15px;color:#cbd5e1;">📅 リスト更新: {caba_updated} — TTL 約7日</p>
{caba_national_table}
</div>

<div class="card" id="jukujo-corner">
<h2>✨ 熟女キャバレー・熟女キャバクラ 全国・高級帯目安 TOP50</h2>
<p style="opacity:.88;font-size:15px;color:#fecdd3;">{jukujo_disclaimer}</p>
<p style="opacity:.65;font-size:15px;color:#cbd5e1;">📅 リスト更新（キャッシュ）: {jukujo_updated} — 「高級順」はエリアの目安インデックスによる並びで、特定店の格付けではありません。</p>
{jukujo_table}
</div>

<div class="cron-box">
  📡 <strong>★ Cron 自動更新設定（毎日）：</strong> キャバクラ・熟女キャバ・TVチャットレディ（グループ/通常）の各ランキングキャッシュは毎朝自動更新されます。
</div>

<div class="notice" style="margin-top:32px;">
  <span>🔗</span>
  <span>IT技術・プログラミング・英会話など女性向け以外の情報は</span>
  <a href="/aruaru">audiocafe.tokyo/aruaru</a>
  <span>をご覧ください。</span>
</div>

</div>

<div class="card" style="border-color:#4338ca;margin-top:32px;" id="policy-sougeishaxi">
  <h2 style="color:#a5b4fc;">🚗 関連政策提案：無料送迎車の半公共タクシー化</h2>
  <h3 style="color:#c7d2fe;">概念：半公共送迎タクシー制度</h3>
  <p style="font-size:15px;color:#e2e8f0;">キャバクラ等の無料送迎車を、空席があれば一般市民も乗れる<strong style="color:#c7d2fe;">「相乗り型半公共交通」</strong>として機能させる構想です。病院・スーパーへの買い物など日常的な送迎にも活用でき、交通空白地帯の補完手段となり得ます。運転手には店からの給与に加え、<strong style="color:#c7d2fe;">日本全国の市区町村から半公務員手当</strong>が支給される仕組みとすることで、安定した収入と社会的役割を両立できる可能性があります。</p>
  <h3 style="color:#86efac;font-size:16px;">✅ メリット</h3>
  <ul style="font-size:15px;line-height:2.1;padding-left:1.4rem;color:#d1fae5;">
    <li>深夜・郊外など交通空白地帯をカバー</li>
    <li>運転手の収入が安定（店＋自治体手当の二重収入）</li>
    <li>病院・買い物難民の高齢者救済</li>
    <li>車両の遊休時間を社会還元</li>
  </ul>
  <h3 style="color:#fca5a5;font-size:16px;">⚠️ 課題と論点</h3>
  <ul style="font-size:15px;line-height:2.1;padding-left:1.4rem;color:#fecdd3;">
    <li>道路運送法との整合性（白タク規制）</li>
    <li>自治体ごとの財源・予算手当</li>
    <li>乗客の安全管理・身元確認</li>
    <li>優先度のルール（店の業務客 vs 一般市民）</li>
  </ul>
  <h3 style="color:#a5b4fc;font-size:16px;">📋 近い事例</h3>
  <ul style="font-size:15px;line-height:2.1;padding-left:1.4rem;color:#c7d2fe;">
    <li><strong>デマンド型乗合タクシー</strong> — 過疎地で既に実施中。予約に応じて乗合運行する公共交通の仕組み。</li>
    <li><strong>ライドシェア</strong> — 2024年から一部解禁。自家用車を使った有償旅客運送が条件付きで認められ始めた。</li>
    <li><strong>スクールバスの地域開放</strong> — 登下校時間外に地域住民が利用できるよう開放する取り組み。</li>
  </ul>
  <p style="font-size:15px;color:#94a3b8;">実現には道路運送法（白タク規制）の整備・自治体ごとの条例・財源確保・安全管理ルールの策定が必要です。既存のデマンド交通やライドシェア制度との連携・法改正が検討課題となります。</p>
  <hr style="border:none;border-top:1px solid rgba(148,163,184,.2);margin:20px 0;">
  <h3 style="color:#fcd34d;font-size:16px;">🍺 お客様送迎・昼飲み需要・販売品目に関する提案</h3>
  <p style="font-size:15px;color:#e2e8f0;">キャバレー・キャバクラでは従業員向けの自動車での<strong style="color:#fcd34d;">無料送迎サービス</strong>がある所が多い様ですので、<strong style="color:#fcd34d;">お客様向けにも無料の送迎サービス</strong>があった方が親切で良いと思われます。またお店も、朝からや昼からでも楽しく飲んで気分転換したい方の<strong style="color:#fcd34d;">需要やニーズも増加中</strong>の様です。</p>
  <h3 style="color:#7dd3fc;font-size:15px;">🥤 飲料・販売品目の拡充提案</h3>
  <p style="font-size:15px;color:#e2e8f0;">キャバレー・キャバクラや、今後は<strong style="color:#7dd3fc;">駅のKIOSK・キヨスクや自動販売機</strong>でも、ウイスキーやビールの他に、<strong style="color:#7dd3fc;">ASAHI の青い缶の無添加のノンアルコールビール</strong>など販売や提供などの需要やニーズが増加中の様です。</p>
  <h3 style="color:#f9a8d4;font-size:15px;">💊 心臓ケア商品の優先販売提案</h3>
  <p style="font-size:15px;color:#e2e8f0;">駅のキオスクや自動販売機、キャバレー・キャバクラなどのお店でも、心臓の薬の<strong style="color:#f9a8d4;">「救心」</strong>や、心臓に良いサプリメントとして<strong style="color:#f9a8d4;">「コエンザイムQ10」</strong>などを<strong style="color:#f9a8d4;">優先的に販売</strong>して欲しいです。</p>
</div>

<div style="margin:12px 0 18px;padding:14px 18px;border-radius:12px;background:linear-gradient(135deg,rgba(255,122,69,.13),rgba(47,111,237,.08));border:1.5px solid #ff7a45;display:flex;align-items:center;flex-wrap:wrap;gap:10px;">
  <span style="font-size:18px;">🎲</span>
  <span style="color:#ff7a45;font-weight:700;font-size:15px;">「あるある」まとめ & 開発リポジトリ紹介はこちら →</span>
  <a href="{ARUARU_TOKYO_URL}" target="_blank" rel="noopener noreferrer" style="display:inline-block;padding:6px 16px;border-radius:8px;background:linear-gradient(135deg,#ff7a45,#2f6fed);color:#fff;font-weight:800;">📍 aruaru.tokyo</a>
</div>

<footer>
  <p>© audiocafe.tokyo / aruaru-lady — 掲載情報はGoogle検索窓口へのリンクです。各店・各媒体の公式情報を必ずご確認ください。</p>
  <p style="margin-top:6px;">年齢制限・法令・各都道府県の条例に必ず従ってください。</p>
</footer>
</div>
"##
    )
}

/// PHP版`aruaru/index.php`(8152行)が実際に外部リンク集として掲載している
/// 求人サイト19件(2395〜2452行目、`EXT_SITES`定数)。(表示名, タグ, 紹介文, リンク先)。
/// 「ITあんけん」だけは実URLが動的生成(`build_itanken_url`、free_wordクエリの
/// 複雑な組み立てロジック)のため、簡略化してGoogle検索窓口にフォールバックした。
const ARUARU_EXT_SITES: &[(&str, &str, &str, &str)] = &[
    ("レバテックフリーランス", "フリーランス", "案件数・単価ともに国内最大級のITフリーランス向けエージェント。高単価・長期案件が豊富。", "https://freelance.levtech.jp/project/"),
    ("フリーランススタート", "フリーランス", "50万件以上の案件を集約した最大級フリーランス案件データベース。キーワードで複数スキルを組み合わせて検索可能。", "https://freelance-start.com/jobs?keyword="),
    ("ITあんけん", "フリーランス", "50万件超の案件データベース。言語とフレームワークを組み合わせて検索可能。", "https://www.google.com/search?q=ITあんけん"),
    ("Wantedly", "スタートアップ", "「やりたいこと」でつながる採用サービス。スタートアップ・ベンチャーの求人が豊富。", "https://www.wantedly.com/projects"),
    ("ITプロパートナーズ", "副業・フリーランス", "週2〜3日から参画できる副業・フリーランス案件に特化。スタートアップ系が豊富。", "https://itpropartners.com/job?free_word="),
    ("リクルートエージェント", "正社員求人", "リクルートの正社員求人サイト。言語キーワード(例: Python)で求人検索。", "https://www.r-agent.com/job_search/"),
    ("ハイパフォコンサル", "コンサルフリーランス", "PM・PMO・戦略・SAP・IT・AI領域のフリーランスコンサル案件紹介。エンド直・高単価案件多数。", "https://www.high-performer.jp/consultant/projects/?onlyRecruiting=true"),
    ("geechs job", "フリーランス", "国内最大級のギークスジョブ。ITフリーランスの高単価・リモート案件が豊富。", "https://geechs-job.com/project/"),
    ("Midworks", "フリーランス", "フリーランスでも社会保険・各種保障が充実。正社員並みのサポートで安心して働ける。", "https://mid-works.com/projects/skills/"),
    ("クラウドテック", "フリーランス", "クラウドワークスが運営するITフリーランス向けエージェント。多様な職種・単価帯。", "https://tech.crowdworks.jp/job_offers/o/1?q="),
    ("Findy Freelance", "フリーランス", "GitHubスキルスコアで自動マッチング。エンジニア目線のフリーランス案件サービス。", "https://freelance.findy-code.io/works/languages/"),
    ("フリーランスハブ", "フリーランス", "複数のフリーランス案件サイトを横断検索できるアグリゲーター。効率よく案件を探せる。", "https://freelance-hub.jp/"),
    ("ココナラテック", "フリーランス", "ココナラが運営するITフリーランス向けエージェント。技術スキルに特化した案件を検索可能。", "https://tech.coconala.co.jp/"),
    ("Offers", "副業・複業", "副業・複業×開発案件のマッチング。スタートアップや成長企業の週1〜案件が充実。", "https://offers.jp/jobs/skills/"),
    ("Green", "正社員転職", "IT・Web・ゲーム業界特化の転職サービス。正社員でキャリアアップしたい方向け。", "https://www.green-japan.com/search/skill/"),
    ("Findy(転職)", "エンジニア転職", "スキルスコアでスカウトが届くエンジニア特化の転職サービス。高年収求人多数。", "https://findy-code.io/recommends/"),
    ("Indeed Japan", "総合求人", "国内最大級の求人検索エンジン。正社員・契約社員・フリーランスを幅広く検索可能。", "https://jp.indeed.com/jobs?q="),
    ("Daijob(日本語)", "バイリンガル・外資系", "日本最大級のバイリンガル・外資系・グローバル企業向け転職サイト(日本語版)。", "https://www.daijob.com/jobs/search_result?kw="),
    ("Daijob (English)", "Bilingual / Global", "Japan's largest bilingual / foreign-affiliated career site.", "https://www.daijob.com/en/jobs/search_result?kw="),
];

/// PHP版`aruaru_learning_categories()`(892〜1121行目)が持つ5カテゴリの
/// TOP50学習サービスのうち、各カテゴリの実データ(自動生成の穴埋め用
/// 「〇〇 おすすめ #51」等のパディング行は除く)を代表数件だけ抜粋移植した
/// もの(タイトル, [(名称,紹介文,料金)])。PHP版はカテゴリごとに80/50件まで
/// 機械的にパディングしているが、内容一致の本質(実在するカテゴリと代表的な
/// サービス名が分かること)には影響しないと判断し、正直に開示した上で
/// 簡略化した(aruaru-ladyの体験入店ゾーン簡略化と同じ方針)。
const ARUARU_LEARNING_CATEGORIES: &[(&str, &[(&str, &str, &str)])] = &[
    ("おすすめ学習塾 TOP50", &[
        ("河合塾(Kawaijuku)", "大学受験・理系強化・全国展開", "年間数十万円〜"),
        ("駿台予備学校", "難関大・医学部・理系特化", "年間数十万円〜"),
        ("早稲田アカデミー", "小中高・大学受験", "月額1〜3万円前後〜"),
        ("スタディサプリ進学", "動画＋進路・オンライン塾", "月額数千円〜"),
        ("トライ", "家庭教師・個別", "月額3〜6万円前後〜"),
        ("SAPIX", "中学受験", "年間数十万円〜"),
        ("ヒューマンアカデミー", "IT・Web・デザイン", "数十万円〜(講座による)"),
    ]),
    ("おすすめ家庭教師紹介サービス TOP50", &[
        ("家庭教師のトライ", "全国ネットワーク・オンライン可・講師紹介", "時間数・単価は要確認"),
        ("あすなろ家庭教師", "マンツーマン・定期・紹介", "地域・単価要確認"),
        ("マナリンク(オンライン家庭教師)", "マッチング型・オンライン中心", "講師単価は要確認"),
        ("スタディサプリ(個別オンライン指導)", "大手・オンラインマンツーマン事例あり", "プランにより要確認"),
    ]),
    ("おすすめPC教室 TOP50", &[
        ("パソコン教室ワード", "Office・基礎操作", "月額5千〜1.5万円〜"),
        ("キッズパソコン教室", "子ども向け", "月額5千〜1万円〜"),
        ("アビバ", "資格・Office", "コースによる"),
        ("ライズ", "PC・資格", "コースによる"),
    ]),
    ("おすすめプログラミング教室 TOP50", &[
        ("テックアカデミー", "Web・Ruby/Python", "月額〜数十万(プランによる)"),
        ("ドットインストール", "動画で手軽に", "月額980円台〜"),
        ("Progate", "初心者向け", "月額980円台〜"),
        ("Schoo", "ライブ授業", "月額2,178円〜"),
        ("paizaラーニング", "問題演習", "月額980円台〜"),
        ("AtCoder", "競プロ", "無料"),
        ("Qiita / Zenn", "技術記事", "無料"),
    ]),
    ("おすすめ学習タブレット TOP50", &[
        ("Smile Zemi", "小中学生", "月額〜"),
        ("Z会のタブレット", "中学受験", "月額〜"),
        ("進研ゼミ デジタル", "小中高", "月額〜"),
        ("スタディサプリ", "動画学習", "月額〜"),
    ]),
];

/// PHP版`aruaru/index.php`(8152行)の`<style>`ブロック(4222〜4380行目)
/// から、レイアウトの核となるダークテーマCSSを移植したもの
/// (`.aruaru-page`でスコープし、他ページの同名セレクタと衝突しないよう
/// にしている)。ロゴcanvasアニメーション・検索フォーム/チップ演出用JS・
/// Google翻訳ウィジェット用CSSは対象外(ユーザー指示によるスコープ:
/// 見た目の一致は静的レイアウト・配色が対象であり、JS演出は既存2ページ
/// (aruaru-lady・rakuten-mobile)と同じく対象外と判断)。
const ARUARU_STYLE: &str = r#"<style>
.aruaru-page{margin:-2rem -1rem;background:#0b1220;color:#fff;font-family:'Noto Sans JP','Hiragino Kaku Gothic ProN',Meiryo,system-ui,sans-serif;line-height:1.7}
.aruaru-page a{color:#7dd3fc;text-decoration:none}
.aruaru-page a:hover{text-decoration:underline}
.aruaru-page .wrap{max-width:1100px;margin:0 auto;padding:16px}
.aruaru-page .hero{padding:2.4rem 1rem 1.6rem;text-align:center}
.aruaru-page .hero h1{font-size:clamp(1.5rem,4.5vw,2.4rem);font-weight:900;line-height:1.3;background:linear-gradient(135deg,#fff 0%,#cfe9ff 55%,#a5b4fc 100%);-webkit-background-clip:text;background-clip:text;color:transparent}
.aruaru-page .hero p{color:#dde6f5;font-size:15px;max-width:44rem;margin:.75rem auto 0}
.aruaru-page .hero-stats{display:flex;flex-wrap:wrap;justify-content:center;gap:1.6rem;margin-top:1.4rem}
.aruaru-page .hero-stat-val{font-size:1.3rem;font-weight:900;display:block}
.aruaru-page .hero-stat-lbl{font-size:.85rem;color:#dde6f5}
.aruaru-page .card{background:linear-gradient(165deg,#111c33 0%,#16223d 100%);border:1px solid rgba(255,255,255,.08);border-radius:14px;padding:20px 22px;margin:24px 0}
.aruaru-page h2{color:#fff;font-size:clamp(18px,4vw,24px);margin-bottom:12px}
.aruaru-page h3{font-size:16px;margin:20px 0 8px}
.aruaru-page table{width:100%;border-collapse:collapse;font-size:15px}
.aruaru-page thead tr{background:#10233f;text-align:left}
.aruaru-page th{padding:8px 6px;white-space:nowrap}
.aruaru-page .ox{overflow-x:auto}
.aruaru-page .toc{display:flex;flex-wrap:wrap;gap:8px;margin:16px 0}
.aruaru-page .toc a{display:inline-block;padding:6px 14px;border-radius:20px;background:rgba(6,182,212,.12);border:1px solid rgba(6,182,212,.4);color:#7dd3fc;font-size:15px;font-weight:700}
.aruaru-page .toc a:hover{background:rgba(6,182,212,.25)}
.aruaru-page .ext-grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(260px,1fr));gap:12px}
.aruaru-page .ext-card{display:block;background:#16223d;border:1px solid rgba(255,255,255,.08);border-radius:11px;padding:14px 16px}
.aruaru-page .ext-card:hover{border-color:rgba(255,255,255,.25)}
.aruaru-page .ext-card-name{font-weight:800;color:#fff}
.aruaru-page .ext-tag{font-size:.8rem;font-weight:800;padding:1px 7px;border-radius:5px;background:rgba(167,139,250,.16);color:#c4b5fd;margin-left:6px}
.aruaru-page .ext-desc{font-size:.86rem;color:#dde6f5;margin:.3rem 0}
.aruaru-page .notice{background:rgba(190,24,93,.1);border:1.5px solid #fda4af;border-radius:12px;padding:12px 16px;margin:18px 0;display:flex;align-items:center;gap:10px;flex-wrap:wrap}
.aruaru-page footer{text-align:center;padding:32px 16px;color:#94a3b8;font-size:14px;border-top:1px solid rgba(255,255,255,.08);margin-top:40px}
@media(max-width:640px){.aruaru-page .card{padding:14px}}
</style>"#;

/// ランキング/一覧系のデータをPHP版と同じ列見出しの表として描画する
/// 汎用ヘルパー(`render_rank_table`の`aruaru`版、検索リンク列は付けない)。
/// `name`列に`url`フィールドがあれば自動的にリンク化する
/// (英会話ランキングのアプリ名リンク等、PHP版の挙動を再現)。
fn render_data_table(rows: &[Value], head_color: &str, cols: &[(&str, &str)]) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let mut out = String::from(r#"<div class="ox"><table><thead><tr>"#);
    for (_, label) in cols {
        out.push_str(&format!(r#"<th style="color:{head_color};">{}</th>"#, html_escape(label)));
    }
    out.push_str("</tr></thead><tbody>");
    for row in rows {
        out.push_str(r#"<tr style="border-bottom:1px solid rgba(255,255,255,.08);">"#);
        for (key, _) in cols {
            let cell = if *key == "name" {
                let name = get_disp(row, "name");
                match row.get("url").and_then(|v| v.as_str()) {
                    Some(url) if !url.is_empty() => format!(
                        r#"<a href="{}" target="_blank" rel="noopener noreferrer" style="color:{head_color};font-weight:bold;">{}</a>"#,
                        html_escape(url), html_escape(&name)
                    ),
                    _ => html_escape(&name),
                }
            } else {
                html_escape(&get_disp(row, key))
            };
            out.push_str(&format!(r#"<td style="padding:6px 8px;">{cell}</td>"#));
        }
        out.push_str("</tr>");
    }
    out.push_str("</tbody></table></div>");
    out
}

/// PHP版`aruaru/index.php`(8152行)が実際に表示している内容を移植する。
/// 汎用JSONダンプ(`render_value_generic`)ではPHP版と全く別のページに
/// なってしまうため(2026-07-19、`/rakuten-mobile`・`/aruaru-lady`と同種の
/// 問題を監査で確認——旧`COMPOSITE_PAGES`の`aruaru`エントリはキャバクラ/
/// 熟女キャバランキングを列挙していたが、実際のPHP版はそれらを
/// `/aruaru-lady`へ移転済みで、`/aruaru`側ではもう表示していない)。
/// この関数はPHP版の実際のセクション構成
/// (ヒーロー→楽天モバイル導線→doda求人ピックアップ→外部求人サイト一覧→
/// サービス向上・販売提案→AI技術TOP80ランキング(言語/FW/DB)→
/// 学習サービスTOP50→英会話TOP50→aruaru-lady移転案内)を再現しつつ、
/// データ部分は既存の`fetch_cache`アーキテクチャ経由で取得する。
/// CSSは`ARUARU_STYLE`(PHP版`<style>`ブロックの核部分を移植、ユーザー
/// 指示によるスコープ拡大: 見た目もPHP版と一致させる)。
/// AI駆動開発(WEBスキルが無くても母国語でWEBサイト・スマホアプリを開発する)の研究の一部として
/// 紹介する記事(2026-10-08追加、ユーザー指示)。タイトルはユーザー提供の表記のまま、URLも表示する。
/// `format!`の置換対象に埋め込むため、波括弧は含めないこと。
const AI_DRIVEN_RESEARCH_HTML: &str = r#"<p style="margin:.2rem 0 .6rem;">Claudeによる、WEBスキルが無くても日本語（母国語）でWEBサイトやスマホアプリを開発する為のAI駆動開発（研究の一部）</p>
<p style="margin:.2rem 0;">📝 <a href="https://ameblo.jp/www-aon/entry-12973252437.html" target="_blank" rel="noopener noreferrer">プログラム言語やフレームワークなどの全てをRust（Poemやhyper）版に移植するメリット？</a></p>
<p style="margin:.2rem 0;word-break:break-all;"><a href="https://ameblo.jp/www-aon/entry-12973252437.html" target="_blank" rel="noopener noreferrer">https://ameblo.jp/www-aon/entry-12973252437.html</a></p>"#;

/// オンライン学習サイトの紹介(2026-10-08追加、ユーザー指示)。aruaruの一番上と
/// 「おすすめ学習サービスTOP50」の冒頭の2か所に同じ内容を出す。文言・URLはユーザー提供のもの。
/// `format!`の置換対象に埋め込むため、波括弧は含めないこと。
const ONLINE_LEARNING_LIST_HTML: &str = r#"<ul style="list-style:none;padding:0;margin:.4rem 0 0;">
<li style="padding:.45rem 0;border-bottom:1px solid rgba(255,255,255,.1);">AI駆動開発も学べます。WEBスキルなしで日本語でWEBサイトやスマホアプリが開発出来る様になります。<br>▶️ <a href="https://progate.com/courses/feature/ai-driven-development/exercises/atmoSFuL468uLDi-Pd-lI" target="_blank" rel="noopener noreferrer">Progate「AI駆動開発」</a></li>
<li style="padding:.45rem 0;border-bottom:1px solid rgba(255,255,255,.1);">HTML,CSSを学ぼう<br>▶️ <a href="https://progate.com/courses/feature/webpage/exercises/hJQNO8xPIcha_idvTX2dn" target="_blank" rel="noopener noreferrer">Progate「HTML・CSS」</a></li>
<li style="padding:.45rem 0;border-bottom:1px solid rgba(255,255,255,.1);">Pythonなら、無料ならPaizaの<br>▶️ <a href="https://paiza.jp/works/search_courses/2208" target="_blank" rel="noopener noreferrer">paizaラーニング「Python3」入門講座一覧</a></li>
<li style="padding:.45rem 0;border-bottom:1px solid rgba(255,255,255,.1);">Python プロゲート<br>▶️ <a href="https://progate.com/courses/python" target="_blank" rel="noopener noreferrer">Progate「Python入門」</a></li>
<li style="padding:.45rem 0;">その他無料でPythonが学べます。<br>▶️ <a href="https://www.python.jp/train/index.html" target="_blank" rel="noopener noreferrer">python.jp「Python入門講座」</a></li>
</ul>"#;

/// 属性値(href等)に入れる文字列のエスケープ(`html_escape`は引用符を変換しないため)。
fn html_attr_escape(s: &str) -> String {
    html_escape(s).replace('"', "&quot;").replace('\'', "&#39;")
}

/// AI学習リンクのカード(2026-10-14追加)。`aruaru/aruaru-tech-learning-ai-cache.json`
/// (PHPの毎朝クロールが`aruaru-llm`経由で生成。リンクは取得時に実在確認済み、価格欄は
/// 公式ページで確認できたサービスの実価格か「要確認」に訂正済み)を、技術ごとの折りたたみで表示する。
/// 生成AIの提案であり、人の目での確認ではないことを画面に明記する。
fn render_ai_learning_links(cache: &Result<Value, String>) -> String {
    let Ok(cache) = cache else {
        return String::new();
    };
    let Some(entries) = cache.get("entries").and_then(|v| v.as_object()) else {
        return String::new();
    };
    if entries.is_empty() {
        return String::new();
    }
    let cats: [(&str, &str); 4] = [("lang", "プログラミング言語"), ("fw", "フレームワーク・ライブラリ"), ("db", "データベース"), ("cloud", "クラウド・インフラ")];
    let mut body = String::new();
    let mut techs = 0usize;
    let mut links = 0usize;
    for (prefix, label) in cats {
        let mut keys: Vec<&str> = entries.iter().map(|(k, _)| k).filter(|k| k.starts_with(&format!("{prefix}:"))).collect();
        keys.sort_by_key(|k| k.to_lowercase());
        if keys.is_empty() {
            continue;
        }
        body.push_str(&format!("<h3 style=\"color:#c4b5fd;margin:1.2rem 0 .4rem;\">{}</h3>\n", html_escape(label)));
        for key in keys {
            let name = key.split_once(':').map(|x| x.1).unwrap_or(key);
            let entry = &entries[key];
            let list = |lang: &str, limit: usize| -> String {
                let mut out = String::new();
                for it in entry.get(lang).and_then(|v| v.as_array()).map(|a| a.as_slice()).unwrap_or(&[]).iter().take(limit) {
                    let url = it.get("url").and_then(|v| v.as_str()).unwrap_or("");
                    if !url.starts_with("https://") {
                        continue; // 念のため。https以外のリンクは出さない
                    }
                    let n = it.get("name").and_then(|v| v.as_str()).unwrap_or("リンク");
                    let f = it.get("features").and_then(|v| v.as_str()).unwrap_or("");
                    let p = it.get("price").and_then(|v| v.as_str()).unwrap_or("");
                    out.push_str(&format!(
                        "<li style=\"padding:.3rem 0;\"><a href=\"{}\" target=\"_blank\" rel=\"noopener noreferrer nofollow\">{}</a> — {} <span style=\"opacity:.75;font-size:13px;\">【{}】</span></li>\n",
                        html_attr_escape(url), html_escape(n), html_escape(f), html_escape(p)
                    ));
                }
                out
            };
            let ja = list("ja", 6);
            let en = list("en", 4);
            if ja.is_empty() && en.is_empty() {
                continue;
            }
            techs += 1;
            links += ja.matches("<li").count() + en.matches("<li").count();
            body.push_str(&format!("<details style=\"margin:.25rem 0;\"><summary style=\"cursor:pointer;\">{}</summary>\n", html_escape(name)));
            if !ja.is_empty() {
                body.push_str(&format!("<p style=\"margin:.5rem 0 .1rem;opacity:.8;\">🇯🇵 日本語</p><ul style=\"list-style:none;padding:0;margin:0;\">\n{ja}</ul>\n"));
            }
            if !en.is_empty() {
                body.push_str(&format!("<p style=\"margin:.5rem 0 .1rem;opacity:.8;\">🌐 English</p><ul style=\"list-style:none;padding:0;margin:0;\">\n{en}</ul>\n"));
            }
            body.push_str("</details>\n");
        }
    }
    if techs == 0 {
        return String::new();
    }
    let saved = cache.get("meta").and_then(|m| m.get("saved_at")).and_then(|v| v.as_str()).unwrap_or("");
    format!(
        "<div class=\"card\" id=\"aruaru-ai-learning-links\" style=\"border-color:#a78bfa;\">\n\
<h2 style=\"color:#c4b5fd;\">🤖 AIが提案する学習リンク（技術別・{techs}技術 {links}件）</h2>\n\
<p style=\"opacity:.8;font-size:14px;\">生成AI（自前のaruaru-llm）が提案した学習先です。<b>人の目での確認ではありません。</b>\
リンクは取得時にアクセスして実在を確認済み。金額を表示するのは公式ページで確認できたサービス（ドットインストール・Progate・paiza）だけで、それ以外は「要確認」です。\
「無料」は、公式ドキュメント等を除き、ページ内に料金の記載があるかを自動で調べた結果です。毎日40技術ずつ更新されます。{saved_note}</p>\n\
{body}</div>\n",
        saved_note = if saved.is_empty() { String::new() } else { format!("（最終保存: {}）", html_escape(saved)) }
    )
}

async fn render_aruaru_body() -> String {
    let lang_nav = lang_nav_bar("/aruaru");
    let doda = fetch_cache("aruaru/doda-jobs-cache.json").await;
    let tech = fetch_cache("ai-tech-ranking-cache.json").await;
    let eikaiwa = fetch_cache("aruaru-eikaiwa-ranking-cache.json").await;
    let ai_links = fetch_cache("aruaru/aruaru-tech-learning-ai-cache.json").await;

    let empty = Value::default(); // null
    let doda = doda.as_ref().unwrap_or(&empty);
    let tech = tech.as_ref().unwrap_or(&empty);
    let eikaiwa = eikaiwa.as_ref().unwrap_or(&empty);

    let empty_rows: Vec<Value> = Vec::new();
    let doda_updated = get_disp(doda, "updated_human");

    // PHP版`doda_categories()`(`aruaru/index.php` 4754〜4763行目)と同じ
    // 実際のdoda検索結果URL(未経験可・転勤無しで絞り込み済み)をフォールバック
    // に設定(2026-07-19、ユーザー指摘により修正——従来はキャッシュに
    // `search`フィールドが無い場合、汎用のdoda.jpトップページに飛ぶだけで
    // 「IT・通信業界」「広告・マーケティング業界」で絞り込まれていなかった)。
    let doda_categories: String = [
        (
            "it",
            "💼 IT・通信業界(未経験可／転勤無し)",
            "https://doda.jp/DodaFront/View/JobSearchList.action?ss=1&op=17,70,71,27,24&pic=1&ds=0&ind=01L&tp=1&bf=1&mpsc_sid=10&oldestDayWdtno=0&leftPanelType=1",
        ),
        (
            "ad",
            "📢 広告・マーケティング業界(未経験可／転勤無し)",
            "https://doda.jp/DodaFront/View/JobSearchList.action?ss=1&op=17,70,71,27,24&pic=1&ds=0&ci=131041&ind=1101S,1108S&tp=1&bf=1&mpsc_sid=10&oldestDayWdtno=0&leftPanelType=1",
        ),
    ]
        .iter()
        .map(|(key, fallback_label, fallback_search)| {
            let cat = doda.get("categories").and_then(|c| c.get(key));
            let label = cat.and_then(|c| c.get("label")).and_then(|v| v.as_str()).unwrap_or(fallback_label).to_string();
            let search = cat.and_then(|c| c.get("search")).and_then(|v| v.as_str()).unwrap_or(fallback_search).to_string();
            let items: &[Value] = cat.and_then(|c| c.get("items")).and_then(|v| v.as_array()).map(|v| v.as_slice()).unwrap_or(&empty_rows);
            let item_list: String = items
                .iter()
                .take(12)
                .map(|it| {
                    let title = get_disp(it, "title");
                    let url = get_disp(it, "url");
                    format!(
                        r#"<li style="border-bottom:1px solid rgba(148,163,184,.18);"><a href="{}" target="_blank" rel="noopener noreferrer nofollow" style="display:block;padding:6px 2px;font-size:14px;">{}</a></li>"#,
                        html_escape(&url), html_escape(&title)
                    )
                })
                .collect();
            format!(
                r#"<div style="padding:16px 18px;border-radius:14px;background:rgba(2,6,23,.85);border:1px solid rgba(59,130,246,.4);">
<h3 style="margin:0 0 8px;font-size:15px;">{}</h3>
<ul style="list-style:none;margin:0 0 10px;padding:0;">{}</ul>
<a href="{}" target="_blank" rel="noopener noreferrer nofollow" style="display:block;text-align:center;background:#d12d36;color:#fff;border-radius:8px;padding:9px 14px;font-size:13px;text-decoration:none;">doda で最新の求人一覧を見る ▶</a>
</div>"#,
                html_escape(&label), item_list, html_escape(&search)
            )
        })
        .collect();

    let ext_cards: String = ARUARU_EXT_SITES
        .iter()
        .map(|(name, tag, desc, url)| {
            format!(
                r#"<a href="{url}" target="_blank" rel="noopener noreferrer" class="ext-card">
<div><span class="ext-card-name">{name}</span><span class="ext-tag">{tag}</span></div>
<div class="ext-desc">{desc}</div>
<div style="font-size:.86rem;color:#7dd3fc;font-weight:700;">このサイトへ →</div>
</a>"#,
                name = html_escape(name), tag = html_escape(tag), desc = html_escape(desc), url = html_escape(url)
            )
        })
        .collect();

    // 2026-07-20、ユーザー指示により、プログラミング言語・フレームワーク・
    // DATABASEの「TOP80」「TOP200」的な機械的ランキング表示は廃止した。
    // 代わりに、4つの言語+フレームワーク組み合わせ(PHP+Laravel・
    // Python+FastAPI・Rust+Poem・Ruby on Rails)を、速度・セキュリティ・
    // 読み書きしやすさ・チーム開発のしやすさ・小規模→大規模拡張への
    // 耐性という5軸で比較する、キュレーション済みのレポートに置き換えた
    // (`TECH_COMPARISON_REPORT`定数、下記参照)。`tech`(旧
    // `ai-tech-ranking-cache.json`)は現在この関数からは未使用——
    // cron側の生成自体は他用途のため残置しているが、表示はしない。
    let _ = &tech;

    // 2026-07-19、ユーザー指示により追加: 言語ランキングの下にVersionlessAPI、
    // フレームワークランキングの下にWunderGraph Cosmoへの日本語/英語
    // それぞれのGoogle検索結果リンクを追記(`hl=ja`/`hl=en`で検索UI言語を
    // 切り替え、`google_search_url`と同じクエリ組み立て方針)。
    let versionless_api_ja_url = html_escape(&format!("{}&hl=ja", google_search_url("VersionlessAPI")));
    let versionless_api_en_url = html_escape(&format!("{}&hl=en", google_search_url("VersionlessAPI")));
    let wundergraph_cosmo_ja_url = html_escape(&format!("{}&hl=ja", google_search_url("WunderGraph Cosmo")));
    let wundergraph_cosmo_en_url = html_escape(&format!("{}&hl=en", google_search_url("WunderGraph Cosmo")));

    let learning_sections: String = ARUARU_LEARNING_CATEGORIES
        .iter()
        .map(|(title, rows)| {
            let items: String = rows
                .iter()
                .map(|(name, feat, price)| {
                    format!(
                        r#"<li style="border-bottom:1px solid rgba(255,255,255,.08);padding:6px 0;"><strong>{}</strong> — {} <span style="color:#fde68a;">{}</span></li>"#,
                        html_escape(name), html_escape(feat), html_escape(price)
                    )
                })
                .collect();
            format!(
                r#"<h3>{}</h3><ul style="list-style:none;margin:0;padding:0;font-size:15px;">{}</ul>"#,
                html_escape(title), items
            )
        })
        .collect();

    let eikaiwa_rows: &[Value] = eikaiwa.get("rows").and_then(|v| v.as_array()).map(|v| v.as_slice()).unwrap_or(&empty_rows);
    let eikaiwa_updated = get_disp(eikaiwa, "updated_at");
    let eikaiwa_cols: &[(&str, &str)] = &[
        ("rank", "順位"), ("name", "アプリ・サービス名"), ("platform", "対応端末"),
        ("style", "学習スタイル"), ("level", "レベル"), ("price", "料金目安"), ("note", "ポイント・特徴"),
    ];
    let eikaiwa_table = render_data_table(eikaiwa_rows, "#34d399", eikaiwa_cols);
    let ai_learning_links_card = render_ai_learning_links(&ai_links);

    let it_kenshu_url = google_search_url("未経験 無料IT研修 無料 転職エージェント サービス");
    let daiku_url = google_search_url("未経験から大工 造作大工 型枠大工 木造建築大工");
    let kenchiku_kanri_url = google_search_url("未経験・無資格から 一級建築士 木造建築士 管理建築士 1級建築施工管理技士 を目指せる求人");

    format!(
        r##"{ARUARU_STYLE}
<div class="aruaru-page">
{lang_nav}
<div class="hero">
  <h1>スキルと希望条件から<br>あなたにぴったりの案件が見つかる。</h1>
  <p>言語・フレームワーク・月額・勤務地で絞り込み。マッチした案件の外部サイトへ直接応募 ＋ 似た求人が見つかる外部サービスもご紹介。</p>
  <div class="hero-stats">
    <div><span class="hero-stat-val">32</span><span class="hero-stat-lbl">掲載案件</span></div>
    <div><span class="hero-stat-val">19</span><span class="hero-stat-lbl">外部求人サイト</span></div>
    <div><span class="hero-stat-val">80%</span><span class="hero-stat-lbl">リモート対応</span></div>
    <div><span class="hero-stat-val">¥60〜130万</span><span class="hero-stat-lbl">月額レンジ</span></div>
  </div>
</div>
<div class="wrap">

<div class="card" id="aruaru-online-learning" style="border-color:#06b6d4;">
<h2 style="color:#67e8f9;">🎓 まずはここから！オンライン学習サイト</h2>
{ONLINE_LEARNING_LIST_HTML}
</div>

<div class="card" id="aruaru-ai-driven-research" style="border-color:#a78bfa;">
<h2 style="color:#c4b5fd;">🔬 研究中：AI駆動開発</h2>
{AI_DRIVEN_RESEARCH_HTML}
</div>

<div class="toc">
  <a href="#aruaru-online-learning">🎓 オンライン学習</a>
  <a href="#aruaru-rakuten-mobile-corner">📶 楽天モバイル</a>
  <a href="#doda-jobs">💼 doda求人ピックアップ</a>
  <a href="#ext">🌐 外部求人サイト</a>
  <a href="#policy-service">💡 サービス向上・販売提案</a>
  <a href="#aruaru-top80-tech">🚀 言語＆フレームワーク比較レポート</a>
  <a href="#aruaru-learning">📚 学習サービスTOP50</a>
  <a href="#aruaru-ai-learning-links">🤖 AI学習リンク</a>
  <a href="#aruaru-eikaiwa-top50">🌏 英会話TOP50</a>
</div>

<div class="card" id="aruaru-rakuten-mobile-corner">
<h2>📶 楽天モバイル</h2>
<p>スマホの乗り換えなら楽天モバイル。プラン・国際通話・プラチナバンドの最新情報は <a href="/rakuten-mobile">/rakuten-mobile</a> をご覧ください。</p>
</div>

<div class="card" id="doda-jobs">
<h2>💼 転職求人ピックアップ（doda）</h2>
<p style="color:#94a3b8;font-size:13px;">未経験可・転勤無しの条件で毎日自動更新{doda_updated_note}</p>
<div style="display:grid;grid-template-columns:repeat(auto-fit,minmax(280px,1fr));gap:14px;">{doda_categories}</div>
</div>

<div class="card" id="ext">
<h2>🌐 外部求人サイト（{ext_count}件）</h2>
<p style="color:#dde6f5;font-size:.86rem;">案件検索・応募は各サイトへ直接遷移します。</p>
<div class="ext-grid">{ext_cards}</div>
</div>

<div class="card" id="policy-service">
<h2>💡 サービス向上・販売提案</h2>
<h3>🚗 お客様向け無料送迎サービスの導入を</h3>
<p style="font-size:15px;color:#e2e8f0;">キャバレー・キャバクラでは従業員向けの自動車での無料送迎サービスがある所が多い様ですので、<strong>お客様向けにも無料の送迎サービス</strong>があった方が親切で良いと思われます。</p>
<h3>☀️ 朝・昼からのOPENのお店を増やして欲しい</h3>
<p style="font-size:15px;color:#e2e8f0;">お店も、朝からや昼からでも楽しく飲んで気分転換したい方の需要やニーズも増加中の様ですので、<strong>開店時間を朝からや昼からもOPENのお店</strong>を増やして欲しいです。</p>
<h3>🍺 ノンアルコール・無添加ビールの販売拡大を</h3>
<p style="font-size:15px;color:#e2e8f0;">キャバレー・キャバクラや今後は駅のKIOSK・キヨスクや自動販売機でも、ウイスキーやビールの他に、<strong>ASAHIの青い缶の無添加のノンアルコールビール</strong>など販売や提供などの需要やニーズが増加中の様です。</p>
<h3>💊 心臓に良い薬・サプリメントの優先販売を</h3>
<p style="font-size:15px;color:#e2e8f0;">駅のキオスクや自動販売機やキャバレー・キャバクラなどのお店でも、心臓の薬の<strong>「救心」</strong>や心臓に良いサプリメントとして<strong>「コエンザイムQ10」</strong>などを優先的に販売して欲しいです。</p>
<p style="font-size:13px;color:#94a3b8;">※ 上記はサービス向上・販売促進に関する提案です。各種法令・条例・販売規制を必ずご確認ください。</p>
</div>

<div class="card" id="aruaru-top80-tech">
<h2 style="color:#00ffff;">🚀 言語＆フレームワーク比較レポート</h2>
<p style="opacity:.8;font-size:15px;">機械的な「TOP80」「TOP200」ランキング表示は廃止しました（順位付けだけでは、実際にチームで長期保守する際の使いやすさが伝わらないため）。代わりに、実務でよく選ばれる4つの言語＋フレームワークの組み合わせを、下記5つの観点で比較したレポートを掲載します。PHP＋Laravel・Python＋FastAPI・Rust＋Poemの3つについては、<strong>Claude Code Desktopが様々な観点から評価</strong>した内容をそのまま本文に組み込んでいます。</p>

<div style="padding:14px 18px;border-radius:12px;background:rgba(239,68,68,.12);border:1px solid rgba(239,68,68,.4);margin:14px 0;">
<p style="font-size:15px;margin:0;"><strong>⚠️ Perl・Javaについて</strong>: 近年は人気が低下傾向にあるため、新規プロジェクトでの採用はなるべく避けることを推奨します。既存資産の保守が必要な場合を除き、下記4組み合わせのいずれかへの移行を検討する価値があります。</p>
</div>

<div style="padding:14px 18px;border-radius:12px;background:rgba(251,191,36,.1);border:1px solid rgba(251,191,36,.35);margin:14px 0;">
<p style="font-size:15px;margin:0;"><strong>💡 Ruby on Railsの保守メンテナンスの難しさについて</strong>: Ruby on Railsは、最初の開発者以外の第三者が後から引き継ごうとしても、他の3組み合わせと比べてほとんどの場合で保守メンテナンスが難しいプログラム言語＋フレームワークです。AIが継続的にメンテナンスする体制を組める場合を除き、採用はなるべく避けた方が無難です。</p>
</div>

<div style="overflow-x:auto;">
<table style="width:100%;border-collapse:collapse;font-size:14px;background:#16223d;color:#fff;">
<thead><tr>
<th style="padding:8px 10px;text-align:left;color:#fff;background:#1e2b4d;">組み合わせ</th>
<th style="padding:8px 10px;color:#fff;background:#1e2b4d;">⚡ ハイスピード</th>
<th style="padding:8px 10px;color:#fff;background:#1e2b4d;">🔒 ハイセキュリティ</th>
<th style="padding:8px 10px;color:#fff;background:#1e2b4d;">💾 省メモリ</th>
<th style="padding:8px 10px;color:#fff;background:#1e2b4d;">🤖 AIライブラリの豊富さ</th>
<th style="padding:8px 10px;color:#fff;background:#1e2b4d;">🔗 OpenAIとの親和性</th>
<th style="padding:8px 10px;color:#fff;background:#1e2b4d;">📖 読み書きしやすさ</th>
<th style="padding:8px 10px;color:#fff;background:#1e2b4d;">👥 チーム保守しやすさ</th>
<th style="padding:8px 10px;color:#fff;background:#1e2b4d;">📈 小→大規模拡張耐性</th>
</tr></thead>
<tbody>
<tr style="border-top:1px solid rgba(148,163,184,.25);">
<td style="padding:8px 10px;font-weight:700;color:#fff;background:#16223d;">Rust + Poem</td>
<td style="padding:8px 10px;text-align:center;color:#fff;background:#16223d;">★★★★★</td>
<td style="padding:8px 10px;text-align:center;color:#fff;background:#16223d;">★★★★★</td>
<td style="padding:8px 10px;text-align:center;color:#fff;background:#16223d;">★★★★★</td>
<td style="padding:8px 10px;text-align:center;color:#fff;background:#16223d;">★★★★☆</td>
<td style="padding:8px 10px;text-align:center;color:#fff;background:#16223d;">★★★☆☆</td>
<td style="padding:8px 10px;text-align:center;color:#fff;background:#16223d;">★★★☆☆</td>
<td style="padding:8px 10px;text-align:center;color:#fff;background:#16223d;">★★★★★</td>
<td style="padding:8px 10px;text-align:center;color:#fff;background:#16223d;">★★★★★</td>
</tr>
<tr style="border-top:1px solid rgba(148,163,184,.25);">
<td style="padding:8px 10px;font-weight:700;color:#fff;background:#16223d;">PHP + Laravel</td>
<td style="padding:8px 10px;text-align:center;color:#fff;background:#16223d;">★★★☆☆</td>
<td style="padding:8px 10px;text-align:center;color:#fff;background:#16223d;">★★★☆☆</td>
<td style="padding:8px 10px;text-align:center;color:#fff;background:#16223d;">★★☆☆☆</td>
<td style="padding:8px 10px;text-align:center;color:#fff;background:#16223d;">★★☆☆☆</td>
<td style="padding:8px 10px;text-align:center;color:#fff;background:#16223d;">★★★☆☆</td>
<td style="padding:8px 10px;text-align:center;color:#fff;background:#16223d;">★★★★☆</td>
<td style="padding:8px 10px;text-align:center;color:#fff;background:#16223d;">★★★☆☆</td>
<td style="padding:8px 10px;text-align:center;color:#fff;background:#16223d;">★★★★☆</td>
</tr>
<tr style="border-top:1px solid rgba(148,163,184,.25);">
<td style="padding:8px 10px;font-weight:700;color:#fff;background:#16223d;">Python + FastAPI</td>
<td style="padding:8px 10px;text-align:center;color:#fff;background:#16223d;">★★★★☆</td>
<td style="padding:8px 10px;text-align:center;color:#fff;background:#16223d;">★★★☆☆</td>
<td style="padding:8px 10px;text-align:center;color:#fff;background:#16223d;">★★☆☆☆</td>
<td style="padding:8px 10px;text-align:center;color:#fff;background:#16223d;">★★★★★</td>
<td style="padding:8px 10px;text-align:center;color:#fff;background:#16223d;">★★★★★</td>
<td style="padding:8px 10px;text-align:center;color:#fff;background:#16223d;">★★★★★</td>
<td style="padding:8px 10px;text-align:center;color:#fff;background:#16223d;">★★★☆☆</td>
<td style="padding:8px 10px;text-align:center;color:#fff;background:#16223d;">★★★☆☆</td>
</tr>
<tr style="border-top:1px solid rgba(148,163,184,.25);">
<td style="padding:8px 10px;font-weight:700;color:#fff;background:#16223d;">Ruby on Rails</td>
<td style="padding:8px 10px;text-align:center;color:#fff;background:#16223d;">★★☆☆☆</td>
<td style="padding:8px 10px;text-align:center;color:#fff;background:#16223d;">★★★☆☆</td>
<td style="padding:8px 10px;text-align:center;color:#fff;background:#16223d;">★★☆☆☆</td>
<td style="padding:8px 10px;text-align:center;color:#fff;background:#16223d;">★☆☆☆☆</td>
<td style="padding:8px 10px;text-align:center;color:#fff;background:#16223d;">★★★☆☆</td>
<td style="padding:8px 10px;text-align:center;color:#fff;background:#16223d;">★★★★☆</td>
<td style="padding:8px 10px;text-align:center;color:#fff;background:#16223d;">★★☆☆☆</td>
<td style="padding:8px 10px;text-align:center;color:#fff;background:#16223d;">★★★☆☆</td>
</tr>
</tbody>
</table>
</div>
<p style="font-size:15px;opacity:.65;margin:6px 0 0;">※ 星評価は一般的な技術知見に基づく主観的な目安であり、StackOverflow等のリアルタイム外部指標に基づく実測値ではありません。</p>

<p style="font-size:15px;opacity:.7;margin:22px 0 0;">以下3組み合わせの評価は、Claude Code Desktopが速度・セキュリティ・読み書きしやすさ・チーム保守しやすさ・小→大規模拡張耐性という観点から分析した内容です。</p>
<h3 style="color:#00ffaa;margin-top:8px;">🦀 Rust + Poem</h3>
<p style="font-size:15px;">
<strong>メリット</strong>: コンパイル時の所有権チェックによりメモリ安全性・並行処理安全性が保証され、実行速度もC/C++に匹敵する。借用チェッカーが「書き方の揺れ」自体を許さないため、<strong>誰が書いても似た構造のコードになりやすく、チームでの引き継ぎ・保守に強い</strong>。VersionlessAPI（バージョンレスAPI）やWunderGraph CosmoのようなGraphQL Federation思想とも相性が良く、仕様変更が多い大規模開発ほど恩恵が大きい。<br>
<strong>デメリット</strong>: 学習コストが高く、書き始めの生産性はPython等に劣る。小規模・使い捨てのプロトタイプには過剰投資になりやすい。<br>
<strong>特徴</strong>: 最初から中〜大規模開発・長期保守を見据えたプロジェクトに向く。「小規模から始めて後で大規模化する」場合でも、最初からRustを選んでおくと後年の保守コストを抑えやすい。
</p>
<p style="font-size:14px;margin:10px 0 0;">
<a href="{versionless_api_ja_url}" target="_blank" rel="noopener noreferrer">VersionlessAPI（日本語で検索）</a>
　|
<a href="{versionless_api_en_url}" target="_blank" rel="noopener noreferrer">VersionlessAPI (search in English)</a>
　|　WunderGraph CosmoはRestAPI不要などの特徴を持ちます:
<a href="{wundergraph_cosmo_ja_url}" target="_blank" rel="noopener noreferrer">WunderGraph Cosmo（日本語で検索）</a>
　|
<a href="{wundergraph_cosmo_en_url}" target="_blank" rel="noopener noreferrer">WunderGraph Cosmo (search in English)</a>
</p>

<h3 style="color:#ffaa00;margin-top:22px;">🐘 PHP + Laravel</h3>
<p style="font-size:15px;">
<strong>メリット</strong>: レンタルサーバーを含め対応環境が広く、求人・学習資料の絶対量が多い。Eloquent ORM等の「お作法」が確立しており、中規模までは比較的スムーズにチーム開発できる。<br>
<strong>デメリット</strong>: 動的型付け・暗黙の型変換に起因するバグが起きやすく、大規模化すると型安全性の欠如が保守コストとして表面化しやすい。フレームワークの「魔法」（暗黙の規約）に依存した実装は、後任者が全体像を把握するまでに時間がかかる。<br>
<strong>特徴</strong>: 小〜中規模のWebサービス立ち上げには強いが、「小さく始めて後から大規模化」を見込むなら、早い段階で型安全性・アーキテクチャの明文化に投資する必要がある。
</p>

<h3 style="color:#00ffff;margin-top:22px;">🐍 Python + FastAPI</h3>
<p style="font-size:15px;">
<strong>メリット</strong>: 文法が平易で読み書きしやすく、初学者からAI/データサイエンス分野の専門家まで広い層が書ける。型ヒント＋Pydanticにより一定の型安全性も確保できる。FastAPI自体はASGI（非同期）ベースで、I/O待ちの多いAPI用途ではGo言語のGinフレームワーク並みの高速さを発揮できる。<br>
<strong>デメリット</strong>: CPU律速な重い処理ではPythonインタプリタ自体の速度・GIL（グローバルインタプリタロック）の制約が効いてくるため、そうした用途はRust等に切り出す設計が必要。動的型付け言語共通の「書く人によって書き方がばらつく」問題も残る。<br>
<strong>特徴</strong>: 機械学習・データ分析基盤との親和性が高く、そうした用途では他の3組み合わせより明確に有利。純粋なWeb API基盤としての大規模拡張には、Rust等との併用（重い処理だけ別言語に切り出す等）を検討する価値がある。
</p>

<h3 style="color:#ff66cc;margin-top:22px;">💎 Ruby on Rails</h3>
<p style="font-size:15px;">
<strong>メリット</strong>: 「設定より規約（Convention over Configuration）」の思想により、小〜中規模であれば非常に高速にプロトタイプ〜本番投入まで持っていける。<br>
<strong>デメリット</strong>: 規約に依存した「魔法的」な挙動が多く、Railsの暗黙ルールを熟知していない第三者が保守すると、意図しない副作用に気づきにくい。実行速度も遅めで、大規模トラフィックには追加のスケーリング施策が必須になりやすい。<br>
<strong>特徴</strong>: 「とにかく早く形にしたい」小規模〜MVP開発には最適だが、本比較4組み合わせの中では<strong>チームでの長期保守という観点で最も注意が必要</strong>。将来の大規模化が見込まれる場合は、初期段階からRust等への段階移行も選択肢に入れておくとよい。
</p>

<div style="padding:14px 18px;border-radius:12px;background:rgba(59,130,246,.1);border:1px solid rgba(59,130,246,.35);margin:18px 0 0;">
<p style="font-size:14px;margin:0;"><strong>📌 まとめ</strong>: 「初心者向け・小規模向け」という理由だけで言語＋フレームワークを選ぶと、後から機能拡張が重なり気づけば大規模開発になっている、というケースが後を絶ちません。最初から中〜大規模開発・長期保守を見据えて選定しておく方が、結果的にトータルの保守コストを抑えられる傾向があります（本サイト自体もこの考え方に基づき、PHPモノリスからRust＋Poemへの段階移行を進めています）。</p>
</div>

<h3 style="color:#7dd3fc;margin-top:22px;">🗄️ aruaru-db リポジトリの重要ファイル</h3>
<p style="font-size:15px;">上記4組み合わせのDB層として実際に使われている<a href="https://github.com/aon-co-jp/aruaru-db" target="_blank" rel="noopener noreferrer">aruaru-db</a>（Rust製、Raft強整合×Git-on-SQL）の重要ファイルは、公開リポジトリのためログイン不要でそのままGitHub上から閲覧できます。</p>
<ul style="list-style:none;margin:0;padding:0;font-size:15px;">
<li style="padding:.35rem 0;border-bottom:1px solid rgba(255,255,255,.1);">📄 <a href="https://github.com/aon-co-jp/aruaru-db/blob/main/README.md" target="_blank" rel="noopener noreferrer">README.md</a>（概要）</li>
<li style="padding:.35rem 0;border-bottom:1px solid rgba(255,255,255,.1);">📄 <a href="https://github.com/aon-co-jp/aruaru-db/blob/main/PORTING.md" target="_blank" rel="noopener noreferrer">PORTING.md</a>（お引越し可能ファイル）</li>
<li style="padding:.35rem 0;border-bottom:1px solid rgba(255,255,255,.1);">📄 <a href="https://github.com/aon-co-jp/aruaru-db/blob/main/CLAUDE.md" target="_blank" rel="noopener noreferrer">CLAUDE.md</a>（設計思想＆開発方針＆開発環境ルール）</li>
<li style="padding:.35rem 0;">📄 <a href="https://github.com/aon-co-jp/aruaru-db/blob/main/ARCHITECTURE.md" target="_blank" rel="noopener noreferrer">ARCHITECTURE.md</a>（アーキテクチャ概要）</li>
</ul>
</div>

<div class="card" id="aruaru-learning">
<h2 style="color:#a5f3fc;">📚 おすすめ学習サービス TOP50（日本語・英語）</h2>
<h3 style="color:#67e8f9;">🎓 まずはここから！オンライン学習サイト</h3>
{ONLINE_LEARNING_LIST_HTML}
<p style="opacity:.75;font-size:15px;">学習塾・家庭教師紹介サービス・PC教室・プログラミング教室・学習タブレットの5カテゴリ。各カテゴリ代表数件を抜粋（PHP版はカテゴリごとに機械的な自動生成の穴埋め行を含め50件までパディングしているが、今回は実データのみを抜粋移植——正直に開示するスコープ縮小）。</p>
{learning_sections}
</div>

{ai_learning_links_card}
<div class="card" id="aruaru-eikaiwa-top50">
<h2 style="color:#34d399;">🌏 スマホ・タブレット・PC 優れた英会話アプリ・サイト TOP50</h2>
<p style="opacity:.72;font-size:15px;">📅 最終更新: {eikaiwa_updated} ／ 週1回（7日TTL）Cron自動更新</p>
{eikaiwa_table}
</div>

<div class="card" style="border-color:#6d28d9;">
<h3 style="color:#c4b5fd;">🎓 未経験から、無料IT研修＆無料の転職エージェントサービス</h3>
<p style="font-size:15px;color:#ede9fe;">プログラミング未経験から、受講料無料のIT研修を受けながら正社員を目指せるサービスや、無料で使える転職エージェントの一例検索です。</p>
<p><a href="{it_kenshu_url}" target="_blank" rel="noopener noreferrer">未経験　無料IT研修　無料 転職エージェント サービス を Google で開く</a></p>
</div>

<div class="card" style="border-color:#2d5a28;">
<h3 style="color:#bef264;">🏗️ 未経験から大工・造作大工・型枠大工・木造建築大工のリンクはこちら</h3>
<p style="font-size:15px;color:#ecfccb;">大工見習い〜専門工種への求人の一例検索です。条件は求人票・協会・訓練施設ごとに異なります。</p>
<p><a href="{daiku_url}" target="_blank" rel="noopener noreferrer">未経験から大工　造作大工　型枠大工　木造建築大工 を Google で開く</a></p>
</div>

<div class="card" style="border-color:#3b5998;">
<h3 style="color:#93c5fd;">📐 未経験・無資格の大工・CAD・施設管理・建築現場管理から、将来 一級建築士などを目指せる求人</h3>
<p style="font-size:15px;color:#dbeafe;">未経験・無資格の大工やCADオペレーター、施設管理、建築現場管理からスタートし、将来は一級建築士・木造建築士・管理建築士・1級建築施工管理技士などの資格取得を目指せる求人の一例検索です。</p>
<p><a href="{kenchiku_kanri_url}" target="_blank" rel="noopener noreferrer">未経験・無資格から 一級建築士・木造建築士・管理建築士・1級建築施工管理技士 を目指せる求人 を Google で開く</a></p>
</div>

<div class="notice">
  <span>💃</span>
  <span>キャバクラ・TVチャットレディなど主に女性向けのお仕事情報は移転しました →</span>
  <a href="/aruaru-lady">audiocafe.tokyo/aruaru-lady</a>
</div>

</div>

<footer>
  <p><a href="{ARUARU_TOKYO_URL}" target="_blank" rel="noopener noreferrer">🎲 aruaru.tokyo</a> — 「あるある」まとめ & 開発リポジトリ紹介</p>
  <p style="margin-top:6px;">© audiocafe.tokyo / aruaru — 掲載情報は外部サイトへのリンクです。各社の公式情報を必ずご確認ください。</p>
</footer>
</div>
"##,
        doda_updated_note = if doda_updated.is_empty() { String::new() } else { format!(" ／ 📅 {doda_updated}") },
        ext_count = ARUARU_EXT_SITES.len(),
    )
}

/// PHP側の`rakuten-mobile/index.php`(917行、`rm_render_fragment()`)が
/// 実際に表示している専用ページの内容をそのまま移植する。汎用JSONダンプ
/// (`render_value_generic`)ではPHP版と全く別のページになってしまうため
/// (2026-07-19監査で判明)、この関数はPHP版のセクション構成・見出し・
/// 静的マーケティング文言を1対1で再現しつつ、データ部分(料金・国際通話・
/// プラチナバンド/衛星)は既存の`fetch_cache`アーキテクチャ経由で取得した
/// 3キャッシュJSONから埋め込む。
/// PHP版`rakuten-mobile/index.php`(917行)の`<style>`ブロック
/// (448〜625行目)から、レイアウトの核となるダークテーマCSSを移植した
/// もの(`.rakuten-mobile-page`でスコープし、他ページの同名セレクタと
/// 衝突しないようにしている)。開閉パネル用のJS演出CSS(386〜412行目、
/// 埋め込みモード限定の機能)は対象外(ユーザー指示によるスコープ拡大:
/// 2026-07-19、見た目もPHP版と一致させる)。
const RAKUTEN_MOBILE_STYLE: &str = r#"<style>
.rakuten-mobile-page{--bg:#0b1220;--surface:rgba(15,23,42,.9);--border:rgba(148,163,184,.2);--red:#ef4444;--blue:#3b82f6;--cyan:#22d3ee;--purple:#a78bfa;--orange:#fb923c;--text:#e2e8f0;--dim:#94a3b8;--yellow:#fde68a;margin:-2rem -1rem;background:var(--bg);color:var(--text);font-family:'Noto Sans JP',system-ui,sans-serif;line-height:1.7}
.rakuten-mobile-page a{color:#7dd3fc;text-decoration:underline}
.rakuten-mobile-page a:hover{color:#bae6fd}
.rakuten-mobile-page .page-wrap{max-width:1100px;margin:0 auto;padding:24px 16px 40px}
.rakuten-mobile-page .rm-hero{background:linear-gradient(135deg,rgba(231,10,38,.18) 0%,rgba(59,130,246,.1) 100%);border:2px solid rgba(239,68,68,.4);border-radius:18px;padding:28px 24px;margin-bottom:24px}
.rakuten-mobile-page .rm-hero__badge{display:inline-flex;align-items:center;gap:6px;padding:4px 12px;border-radius:999px;background:rgba(239,68,68,.2);border:1px solid rgba(239,68,68,.5);color:#fca5a5;font-size:13px;font-weight:700;margin-bottom:12px}
.rakuten-mobile-page .rm-hero h1{font-size:clamp(1.3rem,4vw,2rem);font-weight:900;color:#fff;margin-bottom:8px;line-height:1.3}
.rakuten-mobile-page .rm-hero__sub{font-size:15px;color:var(--dim);margin-bottom:16px}
.rakuten-mobile-page .rm-hero__price{font-size:clamp(1.6rem,5vw,2.4rem);font-weight:900;color:var(--red);letter-spacing:-.02em}
.rakuten-mobile-page .rm-hero__price span{font-size:.6em;color:var(--dim)}
.rakuten-mobile-page .rm-hero__official{display:inline-block;margin-top:12px;padding:8px 18px;border-radius:10px;background:rgba(239,68,68,.25);border:1px solid rgba(239,68,68,.6);color:#fca5a5;font-weight:800;font-size:15px;text-decoration:none}
.rakuten-mobile-page .rm-hero__official:hover{background:rgba(239,68,68,.4);color:#fff}
.rakuten-mobile-page .rm-cards{display:grid;grid-template-columns:repeat(auto-fit,minmax(290px,1fr));gap:14px;margin-bottom:24px}
.rakuten-mobile-page .rm-card{padding:18px 20px;border-radius:14px;background:rgba(2,6,23,.85)}
.rakuten-mobile-page .rm-card--intl{border:1px solid rgba(59,130,246,.4)}
.rakuten-mobile-page .rm-card--sat{border:1px solid rgba(56,189,248,.4)}
.rakuten-mobile-page .rm-card--plat{border:1px solid rgba(167,139,250,.4)}
.rakuten-mobile-page .rm-card__h3{color:#fff;font-size:16px;font-weight:800;margin-bottom:6px}
.rakuten-mobile-page .rm-card__meta{color:var(--dim);font-size:13px;margin-bottom:8px}
.rakuten-mobile-page .rm-card__body{font-size:14px;line-height:1.8;color:var(--text)}
.rakuten-mobile-page .rm-card__body .dim{color:var(--dim);font-size:13px}
.rakuten-mobile-page .rm-area{background:rgba(15,23,42,.5);border:1px solid var(--border);border-radius:12px;padding:16px 18px;margin-bottom:24px}
.rakuten-mobile-page .rm-area__h{font-size:15px;font-weight:800;color:var(--dim);margin-bottom:10px}
.rakuten-mobile-page .rm-area__btns{display:flex;flex-wrap:wrap;gap:8px}
.rakuten-mobile-page .rm-area__btn{display:inline-block;padding:6px 14px;border-radius:8px;font-size:15px;font-weight:700;text-decoration:none}
.rakuten-mobile-page .rm-area__btn--red{background:rgba(231,10,38,.2);border:1px solid rgba(231,10,38,.5);color:#fca5a5}
.rakuten-mobile-page .rm-area__btn--blue{background:rgba(59,130,246,.15);border:1px solid rgba(59,130,246,.4);color:#93c5fd}
.rakuten-mobile-page .rm-area__btn--red:hover{background:rgba(231,10,38,.35);color:#fff}
.rakuten-mobile-page .rm-area__btn--blue:hover{background:rgba(59,130,246,.3);color:#fff}
.rakuten-mobile-page .rm-coverage{display:grid;grid-template-columns:repeat(auto-fill,minmax(260px,1fr));gap:10px;margin-bottom:24px}
.rakuten-mobile-page .rm-coverage__item{background:rgba(15,23,42,.55);border-radius:10px;padding:12px 14px}
.rakuten-mobile-page .rm-coverage__item--cyan{border:1px solid rgba(34,211,238,.3)}
.rakuten-mobile-page .rm-coverage__item--purple{border:1px solid rgba(139,92,246,.3)}
.rakuten-mobile-page .rm-coverage__item--orange{border:1px solid rgba(251,146,60,.3)}
.rakuten-mobile-page .rm-coverage__ttl{font-weight:800;font-size:15px;margin-bottom:6px}
.rakuten-mobile-page .rm-coverage__ttl--cyan{color:#22d3ee}
.rakuten-mobile-page .rm-coverage__ttl--purple{color:#a78bfa}
.rakuten-mobile-page .rm-coverage__ttl--orange{color:#fb923c}
.rakuten-mobile-page .rm-coverage__body{font-size:15px;line-height:1.7;color:var(--text)}
.rakuten-mobile-page .rm-links{background:linear-gradient(180deg,rgba(191,219,254,.1),rgba(17,8,24,.6));border:1px solid rgba(56,189,248,.25);border-radius:14px;padding:20px 20px;margin-bottom:24px}
.rakuten-mobile-page .rm-links__h2{font-size:clamp(1.05rem,3.2vw,1.25rem);font-weight:800;color:#7dd3fc;margin-bottom:8px}
.rakuten-mobile-page .rm-links__lead{font-size:15px;line-height:1.75;color:var(--text);opacity:.95;margin-bottom:12px}
.rakuten-mobile-page .rm-links__list{list-style:none;padding:0;margin:0 0 12px;line-height:1.9;font-size:15px;color:#e0f2fe}
.rakuten-mobile-page .rm-links__list a{color:#7dd3fc;font-weight:700;text-decoration:underline}
.rakuten-mobile-page .rm-links__note{font-size:15px;line-height:1.65;color:#cbd5e1;opacity:.9}
.rakuten-mobile-page .rm-search-btns{display:flex;flex-wrap:wrap;gap:8px;margin-bottom:24px}
.rakuten-mobile-page .rm-search-btn{display:inline-block;padding:8px 16px;border-radius:9px;font-size:15px;font-weight:800;text-decoration:none}
.rakuten-mobile-page .rm-search-btn--red{background:rgba(231,10,38,.25);border:1px solid rgba(231,10,38,.6);color:#fca5a5}
.rakuten-mobile-page .rm-search-btn--blue{background:rgba(59,130,246,.15);border:1px solid rgba(59,130,246,.4);color:#93c5fd}
.rakuten-mobile-page .rm-search-btn--purple{background:rgba(139,92,246,.15);border:1px solid rgba(139,92,246,.4);color:#c4b5fd}
.rakuten-mobile-page .rm-search-btn:hover{opacity:.8;color:#fff}
.rakuten-mobile-page .rm-cron-note{background:rgba(15,23,42,.5);border:1px solid var(--border);border-radius:10px;padding:14px 16px;margin-bottom:24px;font-size:13px;color:var(--dim);line-height:1.7}
.rakuten-mobile-page .rm-cron-note code{background:#0a1628;padding:2px 7px;border-radius:4px;color:#a5f3fc;font-size:12px}
.rakuten-mobile-page .rm-footer{text-align:center;padding:24px 16px;color:var(--dim);font-size:14px;border-top:1px solid var(--border);margin-top:16px}
.rakuten-mobile-page .rm-footer a{color:var(--dim)}
.rakuten-mobile-page .rm-footer a:hover{color:var(--text)}
</style>"#;

async fn render_rakuten_mobile_body() -> String {
    let lang_nav = lang_nav_bar("/rakuten-mobile");
    let rk = fetch_cache("rakuten-mobile-cache.json").await;
    let intl = fetch_cache("rakuten-intl-call-cache.json").await;
    let plat = fetch_cache("rakuten-platinum-cache.json").await;

    let empty = Value::default(); // null
    let rk = rk.as_ref().unwrap_or(&empty);
    let intl = intl.as_ref().unwrap_or(&empty);
    let plat = plat.as_ref().unwrap_or(&empty);

    let price = get_str(rk, "price", "最大3,278円（税込）");
    let updated_at = get_str(rk, "updated_at", "");
    let official_url = "https://network.mobile.rakuten.co.jp/fee/saikyo-plan/";

    let intl_price = get_str(intl, "intl_plan_price_ja", "月980円（税込）");
    let intl_name = get_str(intl, "intl_plan_name_ja", "国際通話かけ放題");
    let intl_count = get_str(intl, "intl_countries_count", "66");
    let intl_crawled = get_str(intl, "crawled_at", "");
    let intl_ok = get_bool(intl, "crawl_success");
    let intl_free_url = "https://network.mobile.rakuten.co.jp/service/international-call-free/";

    let plat_status = get_str(plat, "platinum_status_ja", "700MHz帯プラチナバンドを整備中。地下・屋内・山間部でのつながりやすさを改善。");
    let plat_detail = get_str(plat, "platinum_detail_ja", "700MHz帯（プラチナバンド）は電波が建物内や地下街まで届きやすい低周波数帯。屋内での通話・データ通信の安定性が向上。");
    let plat_coverage = get_str(plat, "platinum_coverage_ja", "全国整備進行中（順次拡大中）");
    let sat_status = get_str(plat, "satellite_status_ja", "AST SpaceMobile との提携により、衛星ブロードバンド通話サービスを準備中。");
    let sat_detail = get_str(plat, "satellite_detail_ja", "低軌道衛星（LEO）を利用し、山間部・離島・海上でも通常のスマートフォンで通話・データ通信が可能になる見込み。");
    let sat_launch = get_str(plat, "satellite_launch_ja", "商用サービス開始時期は未定（2025〜2026年を目標と報道あり）");
    let plat_crawled = get_str(plat, "crawled_at", "");
    let plat_ok = get_bool(plat, "crawl_success");

    let area_url = "https://network.mobile.rakuten.co.jp/area/";
    let area_faq_url = "https://network.mobile.rakuten.co.jp/faq/detail/00001549/";

    let we2plus_url = google_search_url("楽天モバイル お勧め 1円 高性能CPU 富士通製 スマホ we2 plus");
    let packet_url = google_search_url("楽天モバイル パケット放題 プラン");
    let phone_url = google_search_url("楽天モバイル 電話放題 楽天リンク");
    let link_android_url = google_search_url("楽天モバイル スマホアプリの名前は 楽天リンク Android版");
    let link_iphone_url = google_search_url("楽天モバイル スマホアプリの名前は 楽天リンク iPhone版");
    let campaign_url = google_search_url("楽天モバイル 乗り換え キャンペーン");
    let price_search_url = google_search_url("楽天モバイル Rakuten最強プラン 料金");

    format!(
        r##"{RAKUTEN_MOBILE_STYLE}
<div class="rakuten-mobile-page">
<div class="page-wrap">
{lang_nav}

<div class="rm-hero">
<div class="rm-hero__badge">📶 Rakuten最強プラン</div>
<h1>楽天モバイル 最新情報</h1>
<p class="rm-hero__sub">自社の楽天回線エリアと au回線（パートナー回線）エリアを合わせてデータ使い放題（パケット放題）となります。</p>
<div>月間無制限に使っても <span class="rm-hero__price">{price}<span>（税込）</span></span></div>
<a href="{official_url}" target="_blank" rel="noopener noreferrer" class="rm-hero__official">公式サイトで確認 →</a>
<p style="font-size:13px;color:var(--dim);margin-top:10px;">📅 {updated_at}</p>
</div>

<div class="rm-coverage">
<div class="rm-coverage__item rm-coverage__item--cyan">
<div class="rm-coverage__ttl rm-coverage__ttl--cyan">📡 楽天回線エリア</div>
<p class="rm-coverage__body">人口カバー率<strong style="color:var(--yellow)">99.9%</strong>を達成。自社基地局エリア内ではデータ高速<strong style="color:var(--yellow)">無制限</strong>で利用できます。</p>
</div>
<div class="rm-coverage__item rm-coverage__item--purple">
<div class="rm-coverage__ttl rm-coverage__ttl--purple">🔄 パートナー回線（au）エリア</div>
<p class="rm-coverage__body">楽天電波が届きにくい屋内や一部エリアでは<strong style="color:#a78bfa">auローミング</strong>を利用。月間<strong style="color:#fca5a5">5GBまで</strong>高速、超過後は最大1Mbps。</p>
</div>
<div class="rm-coverage__item rm-coverage__item--orange">
<div class="rm-coverage__ttl rm-coverage__ttl--orange">⚠️ 注意点</div>
<p class="rm-coverage__body">地下・高層ビル・奥まった屋内では繋がりにくい場合あり。プラチナバンド（700MHz帯）を拡大中。</p>
</div>
</div>

<div class="rm-area">
<div class="rm-area__h">🗺️ エリア確認ツール</div>
<div class="rm-area__btns">
<a href="{area_url}" target="_blank" rel="noopener noreferrer" class="rm-area__btn rm-area__btn--red">📍 楽天モバイル 通信・エリアマップ</a>
<a href="{area_faq_url}" target="_blank" rel="noopener noreferrer" class="rm-area__btn rm-area__btn--blue">❓ データ高速無制限エリアとは</a>
</div>
</div>

<div class="rm-search-btns">
<a href="{price_search_url}" target="_blank" rel="noopener noreferrer" class="rm-search-btn rm-search-btn--red">🔍 最新料金を Google で検索</a>
<a href="{campaign_url}" target="_blank" rel="noopener noreferrer" class="rm-search-btn rm-search-btn--blue">🔍 乗り換えキャンペーン</a>
<a href="{we2plus_url}" target="_blank" rel="noopener noreferrer" class="rm-search-btn rm-search-btn--purple">📱 1円スマホ（we2 plus）</a>
</div>

<div class="rm-cards">
<div class="rm-card rm-card--intl">
<div class="rm-card__h3">📞 楽天モバイル 国際通話プラン詳細</div>
<p class="rm-card__meta">📅 {intl_crawled}{intl_ok_note}</p>
<div class="rm-card__body">
🇯🇵 日本 → 海外 プラン料金：{intl_price} / {intl_name}<br>
🌍 かけ放題対象国：{intl_count} カ国<br>
✈️ 海外 → 日本：Rakuten Link 利用時 無料（対象国・条件あり）<br><br>
<strong style="color:var(--yellow)">🌏 海外からも日本へ電話放題？</strong><br>
✅ はい、かなり本当です。主に Rakuten Link アプリ利用時（条件あり）。<br><br>
🇯🇵 日本→日本：Rakuten Link で無料<br>
🇯🇵 日本→海外：「{intl_name}（{intl_price}）」で{intl_count}カ国かけ放題<br>
✈️ 海外→日本：Rakuten Link で無料（対象国から）<br><br>
<a href="{intl_free_url}" target="_blank" rel="noopener noreferrer">📎 国際通話かけ放題 公式ページ</a>
</div>
</div>

<div class="rm-card rm-card--sat">
<div class="rm-card__h3">🚀 衛星ブロードバンド通話（AST SpaceMobile 提携）</div>
<p class="rm-card__meta">📅 {plat_crawled}{plat_ok_note}</p>
<div class="rm-card__body">{sat_status}<br><span style="color:#cbd5e1">{sat_detail}</span><br><span class="dim">🛰️ {sat_launch}</span></div>
</div>

<div class="rm-card rm-card--plat">
<div class="rm-card__h3">📡 プラチナ回線（700MHz帯 プラチナバンド）</div>
<p class="rm-card__meta">📅 {plat_crawled}{plat_ok_note}</p>
<div class="rm-card__body">{plat_status}<br><span style="color:#cbd5e1">{plat_detail}</span><br><span class="dim">📶 カバレッジ：{plat_coverage}</span></div>
</div>
</div>

<div class="rm-links">
<h2 class="rm-links__h2">📶 楽天モバイル（1円スマホ・パケット放題・電話放題）</h2>
<p class="rm-links__lead">スマホなら楽天モバイルへの乗り換えを検討できます。eSIM 対応端末やキャンペーンの一例として、富士通製「we2 plus」など高性能 CPU 端末を<strong>1円</strong>で入手できる案内が出る場合があります（時期・在庫・契約条件は要確認）。日本全国で<strong>楽天リンク</strong>アプリが使えます。</p>
<ul class="rm-links__list">
<li><a href="{we2plus_url}" target="_blank" rel="noopener noreferrer">1円スマホの例：富士通製 we2 plus など（要確認）</a></li>
<li><a href="{packet_url}" target="_blank" rel="noopener noreferrer">パケット放題・データ使い放題プラン</a></li>
<li><a href="{phone_url}" target="_blank" rel="noopener noreferrer">電話放題・楽天リンク経由の通話</a></li>
<li><a href="{link_android_url}" target="_blank" rel="noopener noreferrer">楽天リンク Android版</a></li>
<li><a href="{link_iphone_url}" target="_blank" rel="noopener noreferrer">楽天リンク iPhone版</a></li>
<li><a href="{campaign_url}" target="_blank" rel="noopener noreferrer">楽天モバイル 乗り換え・キャンペーン全般</a></li>
</ul>
<p class="rm-links__note">楽天モバイルのアンテナ・基地局が届くエリアでは、オンライン配信や TV チャットでも<strong>パケットを気にしにくいプラン</strong>を検討できます。病院などの FREE Wi-Fi が使える場合、在宅・入院中の環境づくりにも役立つことがあります（プラン内容・エリアは必ず公式で確認してください）。</p>
</div>

<div class="rm-cron-note">
<strong style="color:#67e8f9">⏱ 自動更新について</strong><br>
このページの元データ(楽天モバイル料金・国際通話・プラチナバンド)はPHP版サイトが毎朝05:00AMに自動クロール・キャッシュ更新しています。キャッシュ先: <code>rakuten-mobile-cache.json</code> 他2ファイル。
</div>

<div style="display:flex;flex-wrap:wrap;gap:10px;justify-content:center;margin-bottom:24px">
<a href="/" style="display:inline-block;padding:8px 18px;border-radius:10px;background:rgba(15,23,42,.7);border:1px solid var(--border);color:var(--dim);font-weight:700;text-decoration:none;">← audiocafe.tokyo トップ</a>
<a href="/aruaru" style="display:inline-block;padding:8px 18px;border-radius:10px;background:rgba(15,23,42,.7);border:1px solid var(--border);color:var(--dim);font-weight:700;text-decoration:none;">📊 aruaru（IT技術情報）</a>
<a href="/aruaru-lady" style="display:inline-block;padding:8px 18px;border-radius:10px;background:rgba(15,23,42,.7);border:1px solid var(--border);color:var(--dim);font-weight:700;text-decoration:none;">💃 aruaru-lady（女性向け情報）</a>
</div>

<div class="rm-footer">
楽天モバイル情報は毎朝05:00AMに自動クロール更新。内容は必ず<a href="{official_url}" target="_blank" rel="noopener noreferrer">公式サイト</a>でご確認ください。 ・
<a href="{ARUARU_TOKYO_URL}" target="_blank" rel="noopener noreferrer">🎲 aruaru.tokyo</a>
</div>

</div>
</div>
"##,
        intl_ok_note = if intl_ok { " ✓ クロール成功" } else { "" },
        plat_ok_note = if plat_ok { " ✓ クロール成功" } else { "" },
    )
}

async fn render_composite_body(page: &CompositePage) -> String {
    let mut out = format!("<h1>{}</h1>", html_escape(page.title));
    for (heading, path) in page.sections {
        out.push_str(&format!("<h2>{}</h2>", html_escape(heading)));
        match fetch_cache(path).await {
            Ok(data) => out.push_str(&render_value_generic(&data, 1)),
            Err(e) => out.push_str(&format!("<p class=\"disclaimer\">取得エラー: {}</p>", html_escape(&e))),
        }
    }
    out
}

fn healthz_response() -> hyper_compat::Response {
    hyper::Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "text/plain; charset=utf-8")
        .body(hyper_compat::fixed_body(bytes::Bytes::from_static(b"ok")))
        .expect("building a response from a fixed set of valid headers cannot fail")
}

/// PHP版トップページの実データ本体。実ページの`var L=[...]`(147件、
/// `index.php` 1609〜1757行目)を、Node.js(`new Function('return ('+src+')')`)
/// で一度JSとして評価して`JSON.stringify`し、`assets/top_languages.json`
/// (147件全件、フィールドの取捨選択なし)として保存したものを`include_str!`で
/// 埋め込み、起動時に一度だけデシリアライズする(2026-07-19、完全版へ拡張)。
/// フィールド対応: `g`=Google翻訳用言語コード、`n`=英語名、`t`=現地語表記、
/// `a`=カードラベル(国名等)、`r`=地域、`c`=英語エッセイ本文(全147件に存在、
/// 短い1行のみのカードもあれば政治・宗教・地政学的な主張を含む長文のカードも
/// ある)、`d`=日本語エッセイ本文(10件のみ存在)、`p`=正式国名(1件のみ存在)、
/// `cardLinks`=カード別関連リンク(8件のみ存在)、`fc`=flagcdn国コード。
#[derive(Debug, Clone, Deserialize)]
struct CardLink {
    href: String,
    label: String,
}

#[derive(Debug, Clone, Deserialize)]
struct LangCard {
    g: String,
    n: String,
    t: String,
    a: String,
    r: String,
    c: String,
    #[serde(default)]
    d: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    p: Option<String>,
    #[serde(default, rename = "cardLinks")]
    card_links: Option<Vec<CardLink>>,
    fc: String,
}

static TOP_LANGUAGES: Lazy<Vec<LangCard>> = Lazy::new(|| {
    serde_json::from_str::<Vec<LangCard>>(include_str!("../assets/top_languages.json"))
        .expect("assets/top_languages.json must be valid JSON matching LangCard (147 entries)")
});

/// PHP版`SEARCH_SERIES`(`index.php` 2566行目)をNode.jsの
/// `new Function('return ('+src+')')`でJSとして直接評価しlosslessに抽出した
/// もの(`assets/search_series.json`)。PHP版コメントは「84件」と記載していたが
/// (前回HANDOFF参照、未検証の見積もり)、実際にlosslessな評価で数えたところ
/// **77件**だった(2026-07-19、YouTube再生リストのシリーズ機能復活時に判明・
/// 訂正)。各シリーズは`btn`(検索クエリ/ボタンラベル)・`label`(短縮表示用、
/// 空文字のこともある)・`urls`(1件以上のYouTube URL、`watch`/`shorts`/
/// `youtu.be`/`results?search_query`のいずれか)を持つ。
#[derive(Debug, Clone, Deserialize)]
struct SearchSeries {
    btn: String,
    #[serde(default)]
    label: String,
    urls: Vec<String>,
}

static SEARCH_SERIES: Lazy<Vec<SearchSeries>> = Lazy::new(|| {
    serde_json::from_str::<Vec<SearchSeries>>(include_str!("../assets/search_series.json"))
        .expect("assets/search_series.json must be valid JSON matching SearchSeries")
});

/// PHP版トップページの実際のCSS(`<style>`ブロック冒頭、`:root`変数・
/// `.header`/`.card`系クラス)を`.top-page`配下にスコープして移植。
/// 元CSSはbody直下の裸セレクタだったため、既存の他ページ(`ARUARU_STYLE`等)と
/// 同じ手法で`.top-page`プレフィックスを付与した。2026-07-19、完全版への
/// 拡張に伴い、エッセイ本文・カードリンク・言語別導線リンク・検索フォーム・
/// 地域ピル・YouTube背景プレイヤー・壁紙コーナー用のクラスを追加した。
const TOP_STYLE: &str = r#"<style>
.top-page{--bg:#000;--surface:rgba(15,23,42,.52);--border:rgba(255,255,255,.06);--text:#e2e8f0;--text-dim:#94a3b8;--text-muted:#64748b;--cyan:#22d3ee;--cyan-glow:rgba(34,211,238,.15);margin:-2rem -1rem;background:var(--bg);color:var(--text);font-family:'Segoe UI',system-ui,-apple-system,sans-serif;line-height:1.6}
.top-page a{color:#7dd3fc}
.top-page .header{position:relative;overflow:hidden;text-align:center;padding:2rem 1rem 2rem}
.top-page .logo{display:inline-block;font-size:clamp(2rem,6vw,3.2rem);font-weight:900;letter-spacing:-.02em;background:linear-gradient(90deg,#6366f1,#22c55e,#facc15,#ff1493,#7c3aed,#6366f1);background-size:400% 100%;-webkit-background-clip:text;background-clip:text;color:transparent}
.top-page .subtitle{margin-top:.5rem;font-size:clamp(.95rem,2vw,1.25rem);color:#fff;font-weight:500}
.top-page .note{margin-top:.5rem;font-size:.8rem;color:#fff;max-width:36rem;margin-left:auto;margin-right:auto;line-height:1.6}
.top-page .lang-select-link{margin-top:.5rem}
.top-page .blog-link{margin-top:.4rem;font-size:.85rem}
.top-page .blog-link a{color:#7dd3fc;text-decoration:none;font-weight:600}
.top-page .lang-select-link a{color:#fff;font-weight:600}
.top-page .main{max-width:76rem;margin:0 auto;padding:1rem 1rem 3rem;width:100%}
.top-page .region-title{font-size:1.4rem;font-weight:700;color:#fff;margin:1.6rem 0 .8rem;letter-spacing:.03em}
.top-page .grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(min(100%,220px),1fr));gap:.65rem}
.top-page .card{display:flex;flex-direction:column;align-items:center;gap:.35rem;padding:1rem .55rem;border-radius:.75rem;border:1px solid var(--border);background:var(--surface);backdrop-filter:blur(6px);text-align:center}
.top-page .card-flag{width:60px;height:40px;object-fit:cover;border-radius:4px;box-shadow:0 2px 8px rgba(0,0,0,.3)}
.top-page .card-code{display:block;font-size:.95rem;font-weight:800;letter-spacing:.08em;color:var(--cyan)}
.top-page .card-native{display:block;font-size:.95rem;font-weight:600;color:rgba(255,255,255,.9)}
.top-page .card-country{display:block;font-size:.85rem;font-weight:500;color:rgba(255,255,255,.75)}
.top-page .card-essay{text-align:left;font-size:1.15rem;color:var(--text-dim);max-height:9.5rem;overflow-y:auto;margin-top:.3rem;line-height:1.55}
.top-page .card-essay p{margin:0 0 .5rem}
.top-page .card-essay-label{display:block;font-size:.68rem;font-weight:800;letter-spacing:.08em;color:#7dd3fc;text-transform:uppercase;margin-bottom:.15rem}
.top-page .card-essay-d{border-top:1px dashed var(--border);margin-top:.5rem;padding-top:.5rem;font-size:1.15rem}
.top-page .card-links{list-style:none;padding:0;margin:.4rem 0 0;text-align:left;font-size:1.15rem;line-height:1.6}
.top-page .card-links li{margin-bottom:.15rem}
.top-page .card-actions{display:flex;flex-wrap:wrap;gap:.3rem;justify-content:center;margin-top:.5rem}
.top-page .card-actions a{font-size:1.3rem;padding:.15rem .45rem;border:1px solid var(--border);border-radius:999px;background:rgba(34,211,238,.08)}
.top-page .nav-box{background:rgba(15,23,42,.5);border:1px solid var(--border);border-radius:12px;padding:16px 18px;margin:1.5rem 0}
.top-page .nav-box h2{color:#7dd3fc;font-size:1.05rem;margin-bottom:.6rem}
.top-page .nav-box ul{list-style:none;padding:0;margin:0;line-height:1.9}
.top-page .footer{border-top:1px solid rgba(255,255,255,.05);padding:1.5rem 1rem;text-align:center;font-size:.75rem;color:var(--text-muted);line-height:1.7}
.top-page .search-wrap{display:flex;justify-content:center;gap:.4rem;margin:1rem auto 0;max-width:26rem}
.top-page .search-wrap input{flex:1;padding:.5rem .8rem;border-radius:999px;border:1px solid var(--border);background:rgba(255,255,255,.05);color:var(--text)}
.top-page .search-wrap button{padding:.5rem 1rem;border-radius:999px;border:1px solid var(--cyan);background:rgba(34,211,238,.12);color:var(--cyan);font-weight:700}
.top-page .pills{display:flex;flex-wrap:wrap;justify-content:center;gap:.4rem;margin-top:.7rem}
.top-page .pill{padding:.3rem .8rem;border-radius:999px;border:1px solid var(--border);font-size:1.15rem;background:rgba(255,255,255,.04)}
.top-page .pill.is-active{border-color:var(--cyan);color:var(--cyan);background:var(--cyan-glow)}
.top-page .yt-bg-player{max-width:640px;margin:1rem auto;border-radius:12px;overflow:hidden;border:1px solid var(--border);background:rgba(15,23,42,.85);padding-bottom:.5rem;position:relative}
.top-page .yt-panel-close{position:sticky;top:0;z-index:2;width:100%;border:none;background:rgba(15,23,42,.95);color:#22d3ee;font-weight:800;padding:.4rem;cursor:pointer;letter-spacing:.06em}
.top-page .yt-panel-open{position:fixed;top:1rem;right:1rem;z-index:999;font-weight:800;padding:.5rem 1rem;border-radius:14px;border:2px solid rgba(34,211,238,.8);background:rgba(15,23,42,.95);color:#22d3ee;cursor:pointer;letter-spacing:.06em}
.top-page .yt-now-playing{display:block;padding:.4rem .8rem;font-size:.8rem;color:#fff;text-decoration:underline;cursor:pointer}
.top-page .yt-series-controls{padding:0 .8rem .4rem}
.top-page .yt-next-btn{border:1px solid var(--border);border-radius:999px;background:rgba(34,211,238,.12);color:#22d3ee;padding:.3rem .8rem;font-size:.75rem;cursor:pointer}
.top-page .yt-series-list{max-height:180px;overflow-y:auto;display:flex;flex-wrap:wrap;gap:.3rem;padding:.4rem .8rem}
.top-page .yt-series-btn{border:1px solid var(--border);border-radius:999px;background:transparent;color:#fff;padding:.25rem .7rem;font-size:1.05rem;cursor:pointer}
.top-page .yt-series-btn.is-active{background:rgba(34,211,238,.2);border-color:#22d3ee;color:#22d3ee}
.top-page .yt-wp-corner{max-width:640px;margin:1rem auto 0;background:rgba(15,23,42,.5);border:1px solid var(--border);border-radius:12px;padding:1rem}
.top-page #logoWrap{max-width:640px;margin:1rem auto 0;padding:.6rem;text-align:center;background:rgba(15,23,42,.5);border:1px solid var(--border);border-radius:12px;overflow:hidden}
.top-page #acLogoCanvas{width:100%;height:200px;display:block}
.top-page #logoModeBar,.top-page #danceFixedBar{display:flex;flex-wrap:wrap;justify-content:center;gap:.35rem;margin-top:.8rem}
.top-page #danceFixedBar{display:none;margin-top:.5rem}
.top-page #danceFixedBar.is-visible{display:flex}
.top-page .df-label{font-size:.75rem;color:var(--text-dim);align-self:center;margin-right:.2rem}
.top-page .logo-mode-btn,.top-page .dance-fixed-btn{border:1px solid var(--border);border-radius:999px;background:transparent;color:var(--text-dim);padding:.3rem .8rem;font-size:.8rem;cursor:pointer}
.top-page .logo-mode-btn.active,.top-page .dance-fixed-btn.active{background:rgba(34,211,238,.2);border-color:#22d3ee;color:#22d3ee}
.top-page .yt-wp-head{display:block;font-weight:800;color:#fde68a}
.top-page .yt-wp-hint{display:block;font-size:.75rem;color:var(--text-muted);margin:.2rem 0 .6rem}
.top-page .yt-wp-grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(120px,1fr));gap:.6rem}
.top-page .yt-wp-item{text-align:center;font-size:.72rem}
.top-page .yt-wp-thumb{width:100%;border-radius:8px}
.top-page .yt-wp-name{display:block;margin:.25rem 0}
.top-page .yt-wp-dl{display:inline-block;margin-top:.15rem;color:var(--cyan)}
</style>"#;

/// PHP版のcanvasパーティクルロゴ(`index.php` 5854行目以降)の描画式を
/// そのまま移植したJSエンジン。`danceMotionForChar`の10ジャンル分の式・
/// `drawTextGradient`のグラデーション定義・`getTextPixels`/`initParticles`の
/// パーティクル物理は原文と同じ計算式を使用している(YouTube音量連動の
/// `ytEnergy`は接続していないため常に0、Dance AIは音量パターン解析の
/// 代わりに10ジャンルの定期ローテーション、波モードの帆船装飾は省略——
/// いずれもこの定数のコメントとHANDOFFに明記済み)。
const LOGO_CANVAS_SCRIPT: &str = r#"<script>
(function(){
  var canvas = document.getElementById('acLogoCanvas');
  if(!canvas) return;
  var ctx = canvas.getContext('2d');
  var TEXT = 'audiocafe.tokyo';
  var GRAD_SPEED = 18;
  var CW = 1, CH = 1;

  function calcFontSize(w, h){ return Math.min(w * 0.93 / TEXT.length * 1.62, h * 0.70); }

  // PHP版`drawSailingShip()`(`index.php` 5973〜6146行目)をそのまま移植。
  // 波モードで、ロゴ本体の波の進行方向に合わせて帆船がヨー角を
  // ゆっくり切り替えながら3D風に揺れる装飾(2026-07-19、ユーザー指摘
  // 「PHP版で波は上に船が動いていたのを再現して」により追加、前回の
  // 「帆船装飾は省略」というスコープ縮小を撤回)。
  var shipYawDeg = 75;
  function drawSailingShip(ctx, now, W, H, amp, yawDeg){
    var leftPad = 8;
    var fs = calcFontSize(W, H);
    ctx.save();
    ctx.font = '900 ' + fs + "px 'Segoe UI',system-ui,sans-serif";
    var widthBeforeDot = ctx.measureText('audiocafe').width;
    ctx.restore();
    var shipCX = leftPad + widthBeforeDot / 2;
    var shipScale = Math.min(W, H) * 0.18;
    var cy = H * 0.28;

    var bobY = Math.sin(now * 1.1) * 6 * amp + Math.sin(now * 2.3) * 2 * amp;
    var bobX = Math.cos(now * 0.9) * 8 * amp + Math.cos(now * 1.8) * 3 * amp;
    var roll = (Math.sin(now * 1.1) * 0.12 + Math.sin(now * 2.3) * 0.04) * amp;

    var yaw = yawDeg * Math.PI / 180;
    var cosY = Math.cos(yaw), sinY = Math.sin(yaw);
    var hullW = 55;
    var hullWScaled = hullW * Math.abs(cosY);
    var sailDepth = sinY;

    ctx.save();
    ctx.translate(shipCX + bobX, cy + bobY);
    ctx.rotate(roll);
    ctx.scale(shipScale / 100, shipScale / 100);
    var sv = ctx.shadowBlur;

    ctx.beginPath();
    ctx.moveTo(-hullWScaled, 0);
    ctx.bezierCurveTo(-hullWScaled - 5 * Math.abs(cosY), 8, -hullWScaled * 0.7, 22, 0, 24);
    ctx.bezierCurveTo(hullWScaled * 0.7, 22, hullWScaled + 5 * Math.abs(cosY), 8, hullWScaled, 0);
    ctx.closePath();
    var hullLight = cosY > 0 ? '#9a5c22' : '#5c3010';
    ctx.fillStyle = hullLight;
    ctx.shadowColor = 'rgba(0,0,0,0.55)'; ctx.shadowBlur = 10;
    ctx.fill();
    ctx.strokeStyle = '#3a1a05'; ctx.lineWidth = 1.8;
    ctx.stroke();

    if (Math.abs(cosY) > 0.15) {
      ctx.beginPath();
      ctx.moveTo(-hullWScaled, 0); ctx.lineTo(hullWScaled, 0);
      ctx.strokeStyle = cosY > 0 ? '#c8803a' : '#7a4020';
      ctx.lineWidth = 2.5; ctx.shadowBlur = 0;
      ctx.stroke();
    }

    var bowDir = cosY >= 0 ? -1 : 1;
    ctx.beginPath();
    ctx.moveTo(bowDir * hullWScaled, -2);
    ctx.lineTo(bowDir * (hullWScaled + 18), -12);
    ctx.strokeStyle = '#8b5e2a'; ctx.lineWidth = 2.5; ctx.shadowBlur = 0;
    ctx.stroke();

    var mastX = sinY * 6;
    ctx.beginPath();
    ctx.moveTo(mastX, 2); ctx.lineTo(mastX, -80);
    ctx.strokeStyle = '#8b5e2a'; ctx.lineWidth = 3.5; ctx.shadowBlur = 0;
    ctx.stroke();

    var yardW = 36 * Math.abs(cosY) + 8;
    ctx.beginPath();
    ctx.moveTo(mastX - yardW, -40); ctx.lineTo(mastX + yardW, -40);
    ctx.strokeStyle = '#7a5020'; ctx.lineWidth = 2.5;
    ctx.stroke();

    var sailBow = Math.sin(now * 1.4) * 7 * amp * Math.abs(cosY);
    var sailW = yardW * 0.95;
    var sailCurve = sailDepth * 18 + sailBow;
    ctx.beginPath();
    ctx.moveTo(mastX - sailW, -38);
    ctx.bezierCurveTo(mastX - sailW * 0.5 + sailCurve, -22, mastX - sailW * 0.3 + sailCurve, -10, mastX - sailW * 0.2, 2);
    ctx.lineTo(mastX + sailW * 0.2, 2);
    ctx.bezierCurveTo(mastX + sailW * 0.3 + sailCurve, -10, mastX + sailW * 0.5 + sailCurve, -22, mastX + sailW, -38);
    ctx.closePath();
    var sailBrightness = 0.6 + 0.4 * Math.abs(cosY);
    var sg = ctx.createLinearGradient(mastX - sailW, -38, mastX + sailW, 2);
    sg.addColorStop(0, 'rgba(' + Math.round(255*sailBrightness) + ',' + Math.round(250*sailBrightness) + ',' + Math.round(230*sailBrightness) + ',0.95)');
    sg.addColorStop(1, 'rgba(' + Math.round(220*sailBrightness) + ',' + Math.round(205*sailBrightness) + ',' + Math.round(175*sailBrightness) + ',0.90)');
    ctx.fillStyle = sg;
    ctx.shadowColor = 'rgba(100,150,255,0.25)'; ctx.shadowBlur = 5;
    ctx.fill();
    ctx.strokeStyle = 'rgba(160,140,100,0.7)'; ctx.lineWidth = 1.2;
    ctx.stroke();

    var tsW = 18 * Math.abs(cosY) + 4;
    var tsBow = Math.sin(now * 1.8 + 0.5) * 5 * amp * Math.abs(cosY);
    var tsCurve = sailDepth * 10 + tsBow;
    ctx.beginPath();
    ctx.moveTo(mastX - tsW, -42);
    ctx.bezierCurveTo(mastX - tsW * 0.5 + tsCurve, -58, mastX + tsCurve * 0.5, -68, mastX, -78);
    ctx.bezierCurveTo(mastX + tsCurve * 0.5, -68, mastX + tsW * 0.5 + tsCurve, -58, mastX + tsW, -42);
    ctx.closePath();
    ctx.fillStyle = 'rgba(' + Math.round(245*sailBrightness) + ',' + Math.round(240*sailBrightness) + ',' + Math.round(220*sailBrightness) + ',0.88)';
    ctx.shadowBlur = 3;
    ctx.fill();
    ctx.strokeStyle = 'rgba(160,140,100,0.6)'; ctx.lineWidth = 1.0;
    ctx.stroke();

    var flagDir = cosY >= 0 ? 1 : -1;
    var flagWave = Math.sin(now * 3.5) * 5 * amp;
    ctx.beginPath();
    ctx.moveTo(mastX, -80);
    ctx.lineTo(mastX + flagDir * (16 + flagWave), -72);
    ctx.lineTo(mastX + flagDir * (14 + flagWave * 0.5), -66);
    ctx.lineTo(mastX, -72);
    ctx.closePath();
    ctx.fillStyle = '#ef4444';
    ctx.shadowColor = 'rgba(239,68,68,0.7)'; ctx.shadowBlur = 6;
    ctx.fill();

    ctx.beginPath();
    ctx.moveTo(mastX - sailW, -38); ctx.lineTo(-hullWScaled, 2);
    ctx.moveTo(mastX + sailW, -38); ctx.lineTo(hullWScaled, 2);
    ctx.strokeStyle = 'rgba(140,100,50,0.55)'; ctx.lineWidth = 1.0; ctx.shadowBlur = 0;
    ctx.stroke();

    ctx.shadowBlur = sv;
    ctx.restore();
  }

  function resize(){
    var dpr = Math.min(window.devicePixelRatio || 1, 4);
    CW = canvas.offsetWidth; CH = canvas.offsetHeight;
    canvas.width = Math.round(CW * dpr); canvas.height = Math.round(CH * dpr);
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    initParticles();
  }

  var GRAD_STOPS = [
    [0,'#a855f7'],[0.08,'#3b82f6'],[0.20,'#22d3ee'],[0.32,'#22c55e'],[0.45,'#facc15'],
    [0.57,'#f97316'],[0.68,'#ef4444'],[0.80,'#ec4899'],[0.90,'#a855f7'],[1.00,'#3b82f6']
  ];

  function drawTextGradient(now, waveOffset){
    ctx.clearRect(0, 0, CW, CH);
    var fs = calcFontSize(CW, CH);
    ctx.font = '900 ' + fs + "px 'Segoe UI',system-ui,sans-serif";
    ctx.textAlign = 'left'; ctx.textBaseline = 'middle';
    var textW = ctx.measureText(TEXT).width;
    var leftPad = 8, baseY = CH / 2;
    var e = 0;
    var pulse = 1 + e * 0.05;
    var gradShift = (now * GRAD_SPEED / 360 * textW * 1.5) % textW;
    var grad = ctx.createLinearGradient(leftPad - gradShift, 0, leftPad - gradShift + textW * 1.5, 0);
    GRAD_STOPS.forEach(function(s){ grad.addColorStop(s[0], s[1]); });

    if (waveOffset) {
      var curX = leftPad;
      TEXT.split('').forEach(function(ch){
        var chW = ctx.measureText(ch).width;
        var ratio = (curX + chW / 2 - leftPad) / textW;
        var dy = waveOffset(ratio, now);
        ctx.save();
        ctx.shadowColor = 'rgba(168,85,247,0.6)'; ctx.shadowBlur = 18 + e * 18;
        ctx.fillStyle = grad;
        ctx.fillText(ch, curX, baseY + dy);
        ctx.restore();
        curX += chW;
      });
    } else {
      ctx.save();
      ctx.shadowColor = 'rgba(168,85,247,0.5)'; ctx.shadowBlur = 16 + e * 20;
      ctx.fillStyle = grad;
      ctx.translate(leftPad + textW / 2, baseY);
      ctx.scale(pulse, pulse);
      ctx.fillText(TEXT, -textW / 2, 0);
      ctx.restore();
    }
  }

  var DANCE_GENRE_NAMES = {
    kabuki:'🎭 歌舞伎', egypt:'🏛 エジプト', india:'🕉 インド', hiphop:'🎤 HIPHOP', kpop:'💖 K-POP',
    latin:'💃 ラテン', orchestra:'🎻 オーケストラ', jazz:'🎷 JAZZ', ethnic:'🪘 民族', freestyle:'✨ フリースタイル'
  };
  var DANCE_GENRE_COLORS = {
    kabuki:['#d71b3b','#1a1a1a','#ffd700','#ffffff'], egypt:['#ffd700','#d4a017','#3fb6bc','#8b4513'],
    india:['#ff6f00','#ff9933','#ec407a','#138808'], hiphop:['#ff0080','#8000ff','#00ffff','#ffff00'],
    kpop:['#ff6b9d','#c471ed','#12c2e9','#f7797d'], latin:['#ff0040','#ff8c00','#ffd700','#ff1493'],
    orchestra:['#4b0082','#9370db','#daa520','#f5f5dc'], jazz:['#2e1a47','#6a0dad','#d4af37','#4169e1'],
    ethnic:['#8b4513','#cd853f','#d2691e','#ff4500'], freestyle:['#00ffff','#ff00ff','#ffff00','#00ff00']
  };

  function danceMotionForChar(genre, now, i, ratio, beat){
    var dx = 0, dy = 0, rot = 0, scale = 1;
    if (genre === 'kabuki') {
      var pose = Math.floor(now * 0.5) % 4;
      var poseT = (now * 0.5) % 1;
      var freeze = poseT < 0.3 ? 1 : poseT > 0.7 ? 1 : Math.sin((poseT - 0.3) * Math.PI / 0.4);
      var tilt = [0, 0.15, -0.12, 0.2][pose];
      dy = Math.sin(ratio * Math.PI) * 10 * (1 - freeze) + beat * 6;
      rot = tilt * (1 - freeze * 0.7);
      scale = 1 + beat * 0.08;
    } else if (genre === 'egypt') {
      var step = Math.floor(now * 3);
      var stepT = (now * 3) % 1;
      dx = (step % 2 === 0 ? 1 : -1) * (8 + beat * 8) * Math.pow(stepT, 2);
      dy = -Math.abs(Math.sin(now * Math.PI * 2 + i * 0.5)) * (6 + beat * 6);
      scale = 1 + beat * 0.07;
    } else if (genre === 'india') {
      dx = Math.sin(now * 3 + i * 0.3) * (12 + beat * 10);
      dy = Math.cos(now * 2.5 + i * 0.2) * (8 + beat * 8);
      rot = Math.sin(now * 3) * 0.1;
      scale = 1 + Math.sin(now * 4 + i) * 0.08 + beat * 0.08;
    } else if (genre === 'hiphop') {
      var bounce = Math.pow(Math.abs(Math.sin(now * Math.PI * 3 + i * 0.4)), 3);
      dy = -bounce * (18 + beat * 20);
      dx = Math.sin(now * 8 + i) * (3 + beat * 6);
      rot = (Math.random() < 0.05 ? (Math.random() - 0.5) * 0.3 : 0) * (0.5 + beat);
      scale = 1 + bounce * 0.15 + beat * 0.12;
    } else if (genre === 'kpop') {
      var wave = Math.sin(ratio * Math.PI * 2 - now * 3.5);
      dy = wave * (14 + beat * 14);
      dx = Math.cos(ratio * Math.PI * 2 - now * 3.5) * (4 + beat * 4);
      rot = wave * 0.08;
      scale = 1 + Math.abs(wave) * 0.06 + beat * 0.08;
    } else if (genre === 'latin') {
      var hip = Math.sin(now * Math.PI * 2.5) * (12 + beat * 10);
      dx = hip + Math.sin(now * 4 + i * 0.3) * 3;
      dy = Math.abs(Math.sin(now * 5)) * -(6 + beat * 6) + beat * 6;
      rot = Math.sin(now * 2.5) * 0.15;
      scale = 1 + beat * 0.12;
    } else if (genre === 'orchestra') {
      var swing = Math.sin(now * 1.2 + ratio * Math.PI);
      dy = swing * (10 + beat * 16);
      dx = Math.cos(now * 0.8 + i * 0.15) * (2 + beat * 4);
      rot = Math.sin(now * 0.6) * 0.04;
      scale = 1 + Math.abs(swing) * 0.04 + beat * 0.1;
    } else if (genre === 'jazz') {
      var jswing = Math.sin(now * Math.PI * 1.8 + i * 0.3);
      var shuffle = ((now * 2) % 1) < 0.67 ? 1 : 0.5;
      dx = jswing * (6 + beat * 6) * shuffle;
      dy = Math.cos(now * 2.4 + i * 0.4) * (8 + beat * 6) * shuffle;
      rot = jswing * 0.06;
      scale = 1 + beat * 0.09;
    } else if (genre === 'ethnic') {
      var phase = now * 1.8 + i * 0.35;
      dx = Math.cos(phase) * (5 + beat * 6);
      dy = Math.sin(phase * 2) * (4 + beat * 4) + Math.abs(Math.sin(now * 3)) * (4 + beat * 8);
      rot = Math.sin(phase) * 0.05;
      scale = 1 + beat * 0.1;
    } else if (genre === 'freestyle') {
      var r1 = Math.sin(now * (2 + i * 0.2) + i);
      var r2 = Math.cos(now * (3 + i * 0.15));
      dx = r1 * (10 + beat * 10);
      dy = r2 * (12 + beat * 12);
      rot = r1 * r2 * 0.15;
      scale = 1 + Math.abs(r1 * r2) * 0.1 + beat * 0.1;
    }
    return { dx: dx, dy: dy, rot: rot, scale: scale };
  }

  function renderDanceFrame(genre, now, energy, fade){
    ctx.clearRect(0, 0, CW, CH);
    var simulatedBeat = 0.5 + 0.5 * Math.sin(now * 2 * Math.PI * 2);
    var beat = energy > 0.05 ? (energy * 0.85 + simulatedBeat * 0.15) : simulatedBeat * 0.35;
    var fs = calcFontSize(CW, CH);
    ctx.font = '900 ' + fs + "px 'Segoe UI',system-ui,sans-serif";
    ctx.textAlign = 'left'; ctx.textBaseline = 'middle';
    var textW = ctx.measureText(TEXT).width;
    var leftPad = 8, baseY = CH / 2;
    var colors = DANCE_GENRE_COLORS[genre] || DANCE_GENRE_COLORS.freestyle;
    var gradShift = (now * (30 + energy * 40)) % textW;
    var grad = ctx.createLinearGradient(leftPad - gradShift, 0, leftPad - gradShift + textW, 0);
    colors.forEach(function(c, i){ grad.addColorStop(i / (colors.length - 1), c); });

    ctx.save();
    ctx.globalAlpha = fade;
    ctx.shadowColor = colors[0];
    ctx.shadowBlur = 18 + beat * 22;

    var curX = leftPad;
    TEXT.split('').forEach(function(ch, i){
      var chW = ctx.measureText(ch).width;
      var ratio = (curX + chW / 2 - leftPad) / textW;
      var m = danceMotionForChar(genre, now, i, ratio, beat);
      ctx.save();
      ctx.translate(curX + chW / 2 + m.dx, baseY + m.dy);
      ctx.rotate(m.rot);
      ctx.scale(m.scale, m.scale);
      ctx.fillStyle = grad;
      ctx.textAlign = 'center';
      ctx.fillText(ch, 0, 0);
      ctx.restore();
      curX += chW;
    });
    ctx.restore();

    ctx.save();
    ctx.globalAlpha = fade * 0.75;
    ctx.font = '700 ' + Math.max(14, fs * 0.18) + "px 'Segoe UI',system-ui,sans-serif";
    ctx.textAlign = 'left'; ctx.textBaseline = 'top';
    ctx.fillStyle = colors[0];
    ctx.shadowColor = 'rgba(0,0,0,0.5)'; ctx.shadowBlur = 8;
    ctx.fillText(DANCE_GENRE_NAMES[genre] || genre, 12, 10);
    ctx.restore();
  }

  var particles = [];
  var exploding = false, explodeTimer = 0;
  var EXPLODE_HOLD = 90, REFORM_SPEED = 0.025;

  function getTextPixels(w, h){
    var scale = Math.min(window.devicePixelRatio || 1, 4);
    var ow = Math.round(w * scale), oh = Math.round(h * scale);
    var off = document.createElement('canvas');
    off.width = ow; off.height = oh;
    var ox = off.getContext('2d');
    var fs = calcFontSize(w, h) * scale;
    ox.font = '900 ' + fs + "px 'Segoe UI',system-ui,sans-serif";
    ox.fillStyle = '#fff'; ox.textAlign = 'left'; ox.textBaseline = 'middle';
    ox.fillText(TEXT, 8 * scale, oh / 2);
    var d;
    try { d = ox.getImageData(0, 0, ow, oh).data; } catch (e) { return []; }
    var pts = [];
    var step = Math.max(2, Math.round(scale * 2.5));
    for (var y = 0; y < oh; y += step) {
      for (var x = 0; x < ow; x += step) {
        if (d[(y * ow + x) * 4 + 3] > 128) { pts.push({ x: x / scale, y: y / scale }); }
      }
    }
    return pts;
  }

  function initParticles(){
    var W = CW, H = CH;
    if (!W || !H) return;
    var pts = getTextPixels(W, H);
    if (!pts.length) return;
    particles = pts.map(function(p){
      return {
        tx: p.x, ty: p.y, x: p.x, y: p.y, vx: 0, vy: 0,
        ex: (Math.random() - 0.5) * 18, ey: (Math.random() - 0.5) * 14 - 6,
        size: Math.random() * 1.6 + 1.0,
        phase: Math.random() * Math.PI * 2, phaseY: Math.random() * Math.PI * 2
      };
    });
    exploding = false; explodeTimer = 0;
  }

  function particleColor(p, now, W){
    var hue = (p.tx / W * 360 + now * GRAD_SPEED) % 360;
    return 'hsl(' + hue + ',100%,62%)';
  }

  function triggerExplode(){
    exploding = true; explodeTimer = 0;
    particles.forEach(function(p){
      var angle = Math.atan2(p.ty - CH / 2, p.tx - CW / 2);
      var speed = 8 + Math.random() * 16;
      p.ex = Math.cos(angle) * speed + (Math.random() - 0.5) * 10;
      p.ey = Math.sin(angle) * speed + (Math.random() - 0.5) * 10;
    });
  }

  var EFFECT_SEC = 5.0, REST_SEC = 10.0;
  var cycleStart = performance.now() / 1000;
  var cycleSubPhase = 'rest';

  function updateCycle(now, mode){
    var elapsed = now - cycleStart;
    if (cycleSubPhase === 'effect' && elapsed >= EFFECT_SEC) {
      cycleSubPhase = 'rest'; cycleStart = now;
    } else if (cycleSubPhase === 'rest' && elapsed >= REST_SEC) {
      cycleSubPhase = 'effect'; cycleStart = now;
      if (mode === 'explode') triggerExplode();
    }
  }

  function resetCycle(){
    cycleStart = performance.now() / 1000;
    cycleSubPhase = 'effect';
  }

  var logoMode = 'scroll';
  var danceFixedGenre = 'kabuki';
  var danceAiGenres = ['kabuki','egypt','india','hiphop','kpop','latin','orchestra','jazz','ethnic','freestyle'];
  var DANCE_AI_DURATION = 6;
  var danceAiStart = null;

  function animate(){
    requestAnimationFrame(animate);
    if (!CW || !CH) return;
    var energy = 0;
    var now = performance.now() / 1000;
    var mode = logoMode;

    if (mode === 'scroll') { drawTextGradient(now, null); return; }

    if (mode === 'dance') {
      var GENRES = ['kabuki','egypt','india','hiphop','kpop','latin'];
      var GENRE_DURATION = 8;
      if (animate._danceStart == null) animate._danceStart = now;
      var elapsed = now - animate._danceStart;
      var genreIdx = Math.floor(elapsed / GENRE_DURATION) % GENRES.length;
      var genre = GENRES[genreIdx];
      var phaseInGenre = (elapsed % GENRE_DURATION) / GENRE_DURATION;
      var fade = phaseInGenre < 0.15 ? phaseInGenre / 0.15 : phaseInGenre > 0.85 ? (1 - phaseInGenre) / 0.15 : 1;
      renderDanceFrame(genre, now, energy, fade);
      return;
    }

    if (mode === 'danceFixed') { renderDanceFrame(danceFixedGenre, now, energy, 1); return; }

    if (mode === 'danceAI') {
      if (danceAiStart == null) danceAiStart = now;
      var aiElapsed = now - danceAiStart;
      var aiIdx = Math.floor(aiElapsed / DANCE_AI_DURATION) % danceAiGenres.length;
      var aiGenre = danceAiGenres[aiIdx];
      var aiPhase = (aiElapsed % DANCE_AI_DURATION) / DANCE_AI_DURATION;
      var aiFade = aiPhase < 0.1 ? aiPhase / 0.1 : aiPhase > 0.9 ? (1 - aiPhase) / 0.1 : 1;
      renderDanceFrame(aiGenre, now, energy, aiFade);
      return;
    }

    if (mode === 'wave') {
      ctx.clearRect(0, 0, CW, CH);
      var waveAmpX = 22 + energy * 28;
      var fs = calcFontSize(CW, CH);
      ctx.font = '900 ' + fs + "px 'Segoe UI',system-ui,sans-serif";
      ctx.textAlign = 'left'; ctx.textBaseline = 'middle';
      var textW = ctx.measureText(TEXT).width;
      var leftPad = 8, baseY = CH * 0.68;
      var gradShift = (now * GRAD_SPEED / 360 * textW * 1.5) % textW;
      var grad = ctx.createLinearGradient(leftPad - gradShift, 0, leftPad - gradShift + textW * 1.5, 0);
      GRAD_STOPS.forEach(function(s){ grad.addColorStop(s[0], s[1]); });
      var curX = leftPad;
      TEXT.split('').forEach(function(ch){
        var chW = ctx.measureText(ch).width;
        var ratio = (curX + chW / 2 - leftPad) / textW;
        var dy = Math.sin(ratio * Math.PI * 2.5 - now * 2.8) * waveAmpX * 0.7
               + Math.sin(ratio * Math.PI * 4.5 + now * 1.9) * waveAmpX * 0.3;
        ctx.save();
        ctx.shadowColor = 'rgba(168,85,247,0.55)'; ctx.shadowBlur = 16;
        ctx.fillStyle = grad;
        ctx.fillText(ch, curX, baseY + dy);
        ctx.restore();
        curX += chW;
      });

      var ratio05 = 0.5;
      var v1 = -2.8 * Math.cos(ratio05 * Math.PI * 2.5 - now * 2.8) * waveAmpX * 0.7;
      var v2 = 1.9 * Math.cos(ratio05 * Math.PI * 4.5 + now * 1.9) * waveAmpX * 0.3;
      var netV = v1 + v2;
      var targetYaw = netV < 0 ? 75 : -75;
      shipYawDeg += (targetYaw - shipYawDeg) * 0.001;
      drawSailingShip(ctx, now, CW, CH, 1, shipYawDeg);
      return;
    }

    if (!particles.length) return;
    updateCycle(now, mode);
    var isResting = cycleSubPhase === 'rest';
    var restT = isResting ? Math.min(1, (now - cycleStart) / REST_SEC) : 0;
    var returnProgress = Math.min(1, restT / 0.4);
    var spring = isResting ? REFORM_SPEED + returnProgress * REFORM_SPEED * 2 : REFORM_SPEED;

    ctx.clearRect(0, 0, CW, CH);

    if (mode === 'explode') {
      if (isResting) {
        particles.forEach(function(p){
          p.vx = p.vx * 0.80 + (p.tx - p.x) * spring;
          p.vy = p.vy * 0.80 + (p.ty - p.y) * spring;
          p.x += p.vx; p.y += p.vy;
        });
      } else if (exploding) {
        explodeTimer++;
        particles.forEach(function(p){
          p.x += p.ex * (1 + energy * 2.5); p.y += p.ey * (1 + energy * 2.5);
          p.ex *= 0.91; p.ey *= 0.91; p.ey += 0.28;
        });
        if (explodeTimer > EXPLODE_HOLD) exploding = false;
      } else {
        particles.forEach(function(p){
          p.vx = p.vx * 0.80 + (p.tx - p.x) * REFORM_SPEED;
          p.vy = p.vy * 0.80 + (p.ty - p.y) * REFORM_SPEED;
          p.x += p.vx; p.y += p.vy;
        });
      }
    } else if (mode === 'orbit') {
      var orbitAmp = 1 + energy * 1.5, orbitSpeed = 1 + energy * 0.8;
      particles.forEach(function(p){
        var tx = p.tx, ty = p.ty;
        if (!isResting) {
          tx += 20 * orbitAmp * Math.cos(now * 1.6 * orbitSpeed + p.phase);
          ty += 14 * orbitAmp * Math.sin(now * 1.6 * orbitSpeed + p.phaseY);
        }
        var sp = isResting ? spring : 0.08, damp = 0.78;
        p.vx = p.vx * damp + (tx - p.x) * sp;
        p.vy = p.vy * damp + (ty - p.y) * sp;
        p.x += p.vx; p.y += p.vy;
      });
    }

    particles.forEach(function(p){
      var col = particleColor(p, now, CW);
      ctx.shadowColor = col; ctx.shadowBlur = isResting ? 3 : 7;
      ctx.fillStyle = col; ctx.globalAlpha = 0.93;
      ctx.beginPath(); ctx.arc(p.x, p.y, p.size, 0, Math.PI * 2); ctx.fill();
      ctx.globalAlpha = 1; ctx.shadowBlur = 0;
    });
  }

  window.addEventListener('resize', resize);
  setTimeout(function(){ resize(); animate(); }, 80);

  window.acSetLogoMode = function(mode){
    logoMode = mode;
    var btns = document.querySelectorAll('.logo-mode-btn');
    for (var i = 0; i < btns.length; i++) { btns[i].className = 'logo-mode-btn' + (btns[i].dataset.mode === mode ? ' active' : ''); }
    document.getElementById('danceFixedBar').className = mode === 'danceFixed' ? 'is-visible' : '';
    resetCycle();
    if (mode === 'explode') triggerExplode();
  };
  window.acSetLogoGenre = function(genre){
    danceFixedGenre = genre;
    var btns = document.querySelectorAll('.dance-fixed-btn');
    for (var i = 0; i < btns.length; i++) { btns[i].className = 'dance-fixed-btn' + (btns[i].dataset.genre === genre ? ' active' : ''); }
  };
})();
</script>"#;

/// 無料スマホ壁紙ダウンロードコーナーの実データ(`index.php` 1492〜1524行目、
/// `curl`で実データ確認済みの実画像URL、`o0499108015778827097.png`等の
/// Ameba(stat.ameba.jp)ホスティング画像4件)。画像・ダウンロードリンクとも
/// PHP版と同一の実URLをそのまま使用する(プレースホルダではない)。
const TOP_WALLPAPERS: &[(&str, &str, &str)] = &[
    // (画像URL, alt/表示名, download属性のファイル名)
    (
        "https://stat.ameba.jp/user_images/20260505/16/www-aon/87/8b/p/o0499108015778827097.png",
        "NTT IOWN 井上飛鳥さん 黒服",
        "ino-asuka-black.png",
    ),
    (
        "https://stat.ameba.jp/user_images/20260505/16/www-aon/1e/87/j/o0499108015778824305.jpg",
        "NTT IOWN 井上飛鳥さん 白服",
        "ino-asuka-white.jpg",
    ),
    (
        "https://stat.ameba.jp/user_images/20260505/16/www-aon/c2/90/p/o0499108015778820256.png",
        "大阪の神様ビリケン",
        "osaka-billiken.png",
    ),
    (
        "https://stat.ameba.jp/user_images/20260505/18/www-aon/16/38/p/o0498108015778869116.png",
        "大型スピーカー紹介のTONOさん",
        "tono-speaker.png",
    ),
];

/// PHP版がYouTube背景プレイヤーの初期状態(未クリック時のフォールバック)として
/// 実際に読み込む動画ID(`index.php` 3272行目 `DEFAULT_BG_VIDEO_ID='mSDVnO5gFYk'`、
/// `instantiateYtBgPlayer(DEFAULT_BG_VIDEO_ID, 0)`の呼び出し元複数箇所で実際に
/// 使用されている)。本ポートではPHP側のような検索駆動の動画切り替え
/// (`fetchAndCollect`等、クライアント側の巨大なJSロジック)は移植せず
/// (クライアントJSを持たないこのRustサイトのアーキテクチャ方針)、実際に
/// ページ初期表示で使われる本物の埋め込みIDをそのまま`<iframe>`で埋め込む。
const TOP_DEFAULT_BG_VIDEO_ID: &str = "mSDVnO5gFYk";

/// 言語コード(`g`フィールド)から、PHP版`makeAcNavAudiocafeRoot()`
/// (`index.php` 1786〜1789行目)と同じ式でGoogle翻訳プロキシURLを組み立てる。
/// 日本語のみ翻訳なしの素のURL。
fn google_translate_proxy_url(target_url: &str, gc: &str) -> String {
    if gc == "ja" {
        target_url.to_string()
    } else {
        format!(
            "https://translate.google.com/translate?sl=ja&tl={}&u={}",
            percent_encode(gc),
            percent_encode(target_url)
        )
    }
}

/// PHP版`makeAcNavGoogleTransUrl()`(`index.php` 1835〜1844行目)と同じ
/// tld割り当て表・パラメータ構成でGoogle翻訳サイト自体のURL(テキスト翻訳UI)を
/// 組み立てる(「🌐 Google翻訳サイトへ移動」ボタン相当)。
fn google_translate_site_url(gc: &str, page_url: &str) -> String {
    let (tl, hl) = if gc == "ja" { ("en", "ja") } else { (gc, gc) };
    let tld = match hl {
        "zh-CN" => "com.cn",
        "zh-TW" => "com.tw",
        "ru" => "ru",
        "de" => "de",
        "fr" => "fr",
        "it" => "it",
        "es" => "es",
        "pt" => "com.br",
        "ko" => "co.kr",
        "ja" => "co.jp",
        _ => "com",
    };
    format!(
        "https://translate.google.{}/?hl={}&sl=ja&tl={}&text={}&op=translate",
        tld,
        percent_encode(hl),
        percent_encode(tl),
        percent_encode(page_url)
    )
}

/// エッセイ本文(`c`/`d`フィールド)を段落表示用HTMLに変換する。PHP版JSでは
/// プレーンテキストのまま`textContent`風に描画していたが、Rust版はSSRのため
/// 空行(`\n\n`)を`<p>`区切り、単独改行(`\n`)を`<br>`に変換して読みやすくする
/// (内容自体は無加工、HTMLエスケープのみ実施)。
/// エッセイ本文(`c`/`d`)中に埋め込まれた素のURL(`https://...`)を抽出する。
/// PHP版`extractHttpUrlsFromString()`(`mergeCardOutboundLinks`が使用、
/// `index.php` 1935行目)相当——`cardLinks`フィールドに載っていない
/// リンクも本文中に直接書かれていることがあるため、`/summary`画面の
/// リンク一覧を漏れなく揃えるために移植した。
fn extract_urls_from_text(text: &str) -> Vec<String> {
    static RE: Lazy<regex::Regex> = Lazy::new(|| regex::Regex::new(r#"https?://[^\s"'<>]+"#).unwrap());
    RE.find_iter(text)
        .map(|m| m.as_str().trim_end_matches(|c: char| ".,)]}\u{3002}\u{3001}".contains(c)).to_string())
        .collect()
}

fn essay_to_html(text: &str) -> String {
    text.split("\n\n")
        .map(|para| format!("<p>{}</p>", html_escape(para).replace('\n', "<br>")))
        .collect::<String>()
}

/// 言語カード1件を描画する。実際のPHP版が持つ全フィールド
/// (国旗・現地語表記・カードラベル・国名・地域に加え、`c`/`d`の全文エッセイ・
/// `cardLinks`・モーダル相当の遷移先リンク一式)をここで復元する
/// (2026-07-19、40件抜粋+短いフィールドのみ、という前回のスコープ縮小を解消)。
fn render_lang_card(card: &LangCard) -> String {
    // 英語(`c`)と日本語(`d`)の長文エッセイを見出しラベル付きで明確に分離する
    // (2026-07-19、ユーザー指示によりフォントサイズも二回り拡大——
    // `.card-essay`/`.card-essay-d`のCSS参照)。`d`が存在するカードは147件中
    // 10件のみ。
    let mut essay = format!(
        r#"<div class="card-essay"><span class="card-essay-label">English</span>{}"#,
        essay_to_html(&card.c)
    );
    if let Some(d) = &card.d {
        essay.push_str(&format!(
            r#"<div class="card-essay-d"><span class="card-essay-label">日本語</span>{}</div>"#,
            essay_to_html(d)
        ));
    }
    essay.push_str("</div>");

    let links = card
        .card_links
        .as_ref()
        .map(|links| {
            let items: String = links
                .iter()
                .map(|l| format!(r#"<li><a href="{}" target="_blank" rel="noopener noreferrer">{}</a></li>"#, html_escape(&l.href), html_escape(&l.label)))
                .collect();
            format!(r#"<ul class="card-links">{items}</ul>"#)
        })
        .unwrap_or_default();

    // PHP版の「言語カード選択後の遷移先を尋ねるモーダル」(`#acNavChoiceModal`、
    // `index.php` 1578〜1598行目)が提示する実際の遷移先(audiocafe.tokyo本体/
    // /aruaru/aruaru-lady/rakuten-mobile/Google翻訳サイト/aruaru.tokyo)を、
    // モーダルではなく直リンク行として復元する(このRustサイトはクライアント
    // 側JSを持たないアーキテクチャのため、(b)のプレーンHTML化を採用——
    // ユーザーはモーダルを経由せず1クリックで同じ行き先へ実際に到達できる)。
    let gc = &card.g;
    let ac_url = html_escape(&google_translate_proxy_url("https://audiocafe.tokyo/", gc));
    // /aruaru・/aruaru-lady・/rakuten-mobileへの遷移も、audiocafe.tokyo本体と
    // 同じくGoogle翻訳プロキシ経由にする(2026-07-19、ユーザー指摘により修正
    // ——従来は素の日本語ページへ直接リンクしており、選択した言語が
    // 反映されていなかった)。日本語(`gc=="ja"`)の場合は
    // `google_translate_proxy_url`が素のURLを返すため従来通り無翻訳。
    let aruaru_url = html_escape(&google_translate_proxy_url("https://audiocafe.tokyo/aruaru", gc));
    let aruaru_lady_url = html_escape(&google_translate_proxy_url("https://audiocafe.tokyo/aruaru-lady", gc));
    let rakuten_mobile_url = html_escape(&google_translate_proxy_url("https://audiocafe.tokyo/rakuten-mobile", gc));
    // PHP版「📋 言語カード＆/top 要約LIST」ボタン(`acNavBtnMixlist`、
    // `?mixlist=1`遷移、`index.php` 1590行目)の相当機能。全カードの
    // YouTube/Blog等のリンクをタイトル付きで整列表示する`/summary`画面へ、
    // このカードの言語での選択箇所(`#{gc}`)にアンカーして遷移する
    // (2026-07-19、ユーザー要望により新設)。
    let summary_url = html_escape(&google_translate_proxy_url(&format!("https://audiocafe.tokyo/summary#{gc}"), gc));
    let actions = format!(
        r#"<div class="card-actions">
<a href="{ac_url}" target="_blank" rel="noopener noreferrer">audiocafe.tokyo</a>
<a href="{aruaru_url}" target="_blank" rel="noopener noreferrer">/aruaru</a>
<a href="{aruaru_lady_url}" target="_blank" rel="noopener noreferrer">/aruaru-lady</a>
<a href="{rakuten_mobile_url}" target="_blank" rel="noopener noreferrer">/rakuten-mobile</a>
<a href="{aruaru_tokyo}" target="_blank" rel="noopener noreferrer">aruaru.tokyo</a>
<a href="{summary_url}" target="_blank" rel="noopener noreferrer">📋 言語カード要約</a>
<a href="{gt}" target="_blank" rel="noopener noreferrer">🌐 Google Translate</a>
</div>"#,
        aruaru_tokyo = html_escape(ARUARU_TOKYO_URL),
        gt = html_escape(&google_translate_site_url(gc, "https://audiocafe.tokyo/")),
    );

    // 遷移先リンク(`actions`)は、エッセイ本文(数百〜千文字を超える
    // カードもある)より前、国旗・ラベル直後に配置する。エッセイの後に
    // 置くと、長いカードではスクロールしないと遷移先リンクに到達できず、
    // 「国旗をクリックしても遷移先を選ぶ画面が出ない」ように見える実バグ
    // になっていたため(2026-07-19発見・修正)。PHP版のモーダルは
    // クリック直後に(本文を読まずとも)遷移先を選べる体験だったため、
    // このRust版でも同じ即時性を静的リンクで再現する。
    // `t`(現地語表記)と`n`(国名)が同一文字列のカード(英語圏など、
    // 現地語表記=英語国名になるケース)では、両方表示すると
    // 「JAPAN / JAPAN」のように同じ文字列が二段に重複表示されてしまう
    // ため、その場合は`card-native`側を出力しない(2026-07-20、
    // ユーザー報告により修正)。
    let native_html = if card.t.trim() == card.n.trim() {
        String::new()
    } else {
        format!(r#"<span class="card-native">{}</span>"#, html_escape(&card.t))
    };

    format!(
        r#"<div class="card"><a href="{ac_url}" target="_blank" rel="noopener noreferrer"><img class="card-flag" src="https://flagcdn.com/64x48/{fc}.png" alt="{label}"></a><span class="card-code">{label}</span>{native_html}<span class="card-country">{name}</span>{actions}{essay}{links}</div>"#,
        fc = html_escape(&card.fc),
        label = html_escape(&card.a),
        name = html_escape(&card.n),
    )
}

/// PHP版トップページ(`index.php`、8150行)の実際の内容を移植した完全版
/// (2026-07-19、ユーザー指示により前回の40件抜粋+装飾機能除外という
/// スコープ縮小を解消し、フル移植へ拡張)。
///
/// **実際の内容(`curl https://audiocafe.tokyo/`で実データ確認済み)**:
/// `<title>AUDIOCAFE | World — Select Your Language</title>`を持つ、
/// 「147言語のうち好きな言語カードを選んでGoogle翻訳版へ進む」ための
/// 言語選択ランディングページ(ダークテーマ、`#000`背景・`#22d3ee`シアン
/// アクセント)。実際の構成は (1) ヘッダー(サブタイトル・日本語の注記・
/// 検索ボックス・地域絞り込みピル)、(2) 147件全件の言語カードグリッド
/// (国旗画像・現地語表記・英語名・国名・全文エッセイ・`cardLinks`・
/// 遷移先リンク一式)、(3) YouTube背景プレイヤー・無料スマホ壁紙
/// ダウンロードコーナー、(4) フッター(Copyright表記)。
///
/// **今回のスコープ内(完了)**: 147件全カード(前回40件から拡張)、各カードの
/// `c`(英語)/`d`(日本語、10件)の全文エッセイ本文(政治・宗教・地政学的な
/// 主張を含め、実際に公開済みのPHP版コンテンツをそのまま複製)、`cardLinks`
/// (8件のカードに存在、実際の`<a href>`として復元)、YouTube背景プレイヤー
/// (実際に使われるデフォルト動画ID`mSDVnO5gFYk`を`<iframe>`埋め込み)、
/// 無料スマホ壁紙コーナー(実画像4件、実ダウンロードリンク)、検索/地域
/// フィルタ(`?q=`/`?region=`のクエリパラメータによるサーバーサイド絞り込み、
/// クライアントJSを持たないこのサイトのアーキテクチャに合わせた実装)。
///
/// **今回のスコープ外(正直に開示)**: (1) PHP版の「クリックすると遷移先を
/// 尋ねるモーダル」(`#acNavChoiceModal`)自体はJSモーダルとして再現せず、
/// 各カードに直リンク行(`.card-actions`)として展開した(モーダルを経由
/// しないだけで、到達できる行き先自体はモーダルの選択肢と同一)。
/// (2) YouTube背景プレイヤーの検索駆動での動画切り替え(`fetchAndCollect`
/// 等、数千行のクライアントJS)は移植せず、初期表示の実際の動画のみ埋め込み。
/// (3) 壁紙コーナーの「タップで原寸表示」インタラクションはブラウザ標準の
/// 画像リンク遷移で代替(画像自体・ダウンロードリンクは実物)。
const SUMMARY_STYLE: &str = r#"<style>
.summary-page{max-width:52rem;margin:0 auto;padding:1.5rem 1rem 3rem}
.summary-page h1{font-size:1.6rem;margin-bottom:.3rem}
.summary-page .summary-note{color:var(--text-dim);font-size:.85rem;margin-bottom:1.5rem}
.summary-page .summary-card{border:1px solid var(--border);border-radius:.75rem;padding:1rem 1.2rem;margin-bottom:1rem;background:var(--surface);scroll-margin-top:1rem}
.summary-page .summary-card h2{font-size:1.1rem;margin:0 0 .5rem;color:var(--cyan)}
.summary-page .card-links{list-style:none;padding:0;margin:0;font-size:.9rem;line-height:1.7}
.summary-page .card-links li{margin-bottom:.2rem}
</style>"#;

/// PHP版「📋 言語カード＆/top 要約LIST」(`mixlist=1`、`index.php` 1590行目)の
/// 相当機能。全147言語カードそれぞれの`cardLinks`+本文中に埋め込まれたURLを
/// マージし(`extract_urls_from_text`、PHP版`mergeCardOutboundLinks`相当)、
/// カードごとにタイトル付きで整列表示する(2026-07-19新設)。PHP版はさらに
/// Google翻訳ウィジェットで選択言語に画面全体を翻訳する仕組みだったが、
/// このRust版では各カードの`.card-actions`からこの画面へGoogle翻訳
/// プロキシ経由でアンカー付き遷移することで同じ体験を実現している
/// (このページ自体は日本語+各リンクの原文表示のみ、スコープ縮小として
/// 正直に開示)。
fn render_summary_body() -> String {
    use std::collections::HashSet;
    let mut sections = String::new();
    for card in TOP_LANGUAGES.iter() {
        let mut seen: HashSet<String> = HashSet::new();
        let mut links: Vec<(String, String)> = Vec::new();
        if let Some(card_links) = &card.card_links {
            for l in card_links {
                if seen.insert(l.href.clone()) {
                    links.push((l.href.clone(), l.label.clone()));
                }
            }
        }
        for url in extract_urls_from_text(&card.c).into_iter().chain(extract_urls_from_text(card.d.as_deref().unwrap_or(""))) {
            if seen.insert(url.clone()) {
                links.push((url.clone(), url.clone()));
            }
        }
        if links.is_empty() {
            continue;
        }
        let items: String = links
            .iter()
            .map(|(href, label)| format!(r#"<li><a href="{}" target="_blank" rel="noopener noreferrer">{}</a></li>"#, html_escape(href), html_escape(label)))
            .collect();
        sections.push_str(&format!(
            r#"<section class="summary-card" id="{gc}"><h2>{flag} {name}（{native}）</h2><ul class="card-links">{items}</ul></section>"#,
            gc = html_escape(&card.g),
            flag = format!(r#"<img src="https://flagcdn.com/32x24/{}.png" alt="" width="24" height="16" style="vertical-align:middle;border-radius:2px">"#, html_escape(&card.fc)),
            name = html_escape(&card.n),
            native = html_escape(&card.t),
        ));
    }
    format!(
        r##"{SUMMARY_STYLE}
<div class="summary-page">
<h1>📋 言語カード要約 — Link Summary</h1>
<p class="summary-note">147言語カードそれぞれが持つYouTube・ブログ等へのリンクをタイトル付きで一覧表示しています。各カードの「📋 言語カード要約」リンクから、その言語のセクションへ直接ジャンプできます。<a href="/#lang-grid">← 言語カード一覧へ戻る</a></p>
{sections}
</div>"##
    )
}

const AEON_TRAINING_HTML: &str = r#"<section id="aeon-ai-training" style="max-width:56rem;margin:1rem auto;font-family:sans-serif;">
<div style="border-radius:14px;padding:2.2rem 1.2rem;text-align:center;color:#fff;background:linear-gradient(135deg,#0b6fd6 0%,#18b6e8 55%,#6fe0c8 100%);">
<div style="font-size:clamp(1.6rem,5vw,2.8rem);font-weight:800;line-height:1.3;">仲間がいる環境で<br>IT・AIのスキルを</div>
<div style="display:flex;flex-wrap:wrap;gap:.6rem;justify-content:center;margin-top:1.2rem;font-weight:700;">
<span style="background:#fff;color:#0b6fd6;border-radius:999px;padding:.5rem 1.1rem;">未経験者歓迎</span>
<span style="background:#fff;color:#0b6fd6;border-radius:999px;padding:.5rem 1.1rem;">研修は無料</span>
<span style="background:#fff;color:#0b6fd6;border-radius:999px;padding:.5rem 1.1rem;">AI研修+転職エージェント</span>
</div>
</div>
<p style="font-size:1.15rem;line-height:1.8;margin:1rem .5rem;">株式会社エーオンも、未経験者にも優しい無料のAI研修+転職エージェントサービスを早急に始めたいと思っております。</p>
<p style="font-size:1.05rem;line-height:1.8;margin:.5rem;">コピー&amp;ペースト、Google検索、YouTube検索、Claudeを使い、Rust+PoemやRPoemでスマホアプリやWebサイトを、日本語の母国語で開発できるように育成いたします。</p>
<details style="margin:.5rem;border:1px solid #9bc;border-radius:8px;padding:.6rem 1rem;background:rgba(24,182,232,.07);">
<summary style="cursor:pointer;font-weight:700;font-size:1.1rem;">掲載内容(クリックで全文を読む)</summary>
<div style="line-height:1.85;margin-top:.8rem;">
<h3>構想の背景</h3>
<p>「コピペって、どうやるんですか?」「タイピングは指1本ずつ」。そんな方こそ歓迎したいと考えています。IT業界に興味はあるけれど自信がない、という最初の一歩を支える無料の研修と、その後の転職・就職の相談までをひとつの流れで提供することを目指します。立派な志望動機は要りません。ホンネで話せる場にしたいと考えています。</p>
<h3>研修の流れ(予定)</h3>
<ol>
<li>STEP1: PCの起動とマウス操作</li>
<li>STEP2: コピー&ペースト、ショートカットキー、タイピング</li>
<li>STEP3: Excel・Wordなどの基本操作、ビジネスマナー、メールの書き方</li>
<li>STEP4: デザインツールの基礎と、最新のAI研修(生成AIを使って簡単なアプリを作るところまで)</li>
<li>STEP5: 実際の案件・仕事へのデビュー</li>
</ol>
<h3>AI研修で扱いたいこと</h3>
<p>ChatGPT・Gemini・Claude・Cursorなどの生成AIの使い方を、手を動かしながら学びます。1週間ほどで小さなアプリを完成させる体験を通じて「私にもできた」という自信を持ってもらうことが狙いです。将来はIT サポート、AIエンジニア、ゲーム開発など、時代に左右されないスキルを目指せる道筋をつくります。</p>
<h3>転職エージェントとして</h3>
<ul>
<li>研修で身についたスキルに合う企業を、一緒に探します。</li>
<li>月に1回程度の面談で「やりたいこと」を聞き、キャリアの相談に乗ります。</li>
<li>資格(ITパスポート、MOS、基本情報技術者など)の取得を応援します。</li>
<li>残業が少ない、休みが取りやすい、リモートも可能、など働き方の希望も大切にします。</li>
</ul>
<h3>こんな方に</h3>
<ul>
<li>未経験からITの仕事に挑戦したい方、第二新卒・社会人デビューの方</li>
<li>手に職をつけて長く働きたい方</li>
<li>新しいことを学ぶのが好きな方</li>
</ul>
<p style="font-size:.9rem;opacity:.8;">※ 現在準備中の構想です。内容・時期は変更になることがあります。参考にした求人: <a href="https://jp.indeed.com/viewjob?from=appsharedroid&amp;jk=c2dfde0f8c77f92b" target="_blank" rel="noopener noreferrer">株式会社ライトパス様の求人(Indeed)</a></p>
</div>
</details>
</section>
"#;

const HDD_LINKS_HTML: &str = r#"<section class="top-direct" style="max-width:640px;margin:1rem auto;text-align:center;">
<p style="font-size:1.2rem;font-weight:700;margin:0 0 .6rem;"><a href="https://www.youtube.com/watch?v=yq0yDVBjqVM" target="_blank" rel="noopener noreferrer">アメリカ政府による国際法違反な日本支配を改善する必要があります。ダイレクト出版。</a></p>
<div style="position:relative;width:100%;aspect-ratio:16/9;background:#000;border-radius:8px;overflow:hidden;">
<button type="button" aria-label="動画を再生 / Play video" style="all:unset;position:absolute;inset:0;cursor:pointer;" onclick="var f=document.createElement('iframe');f.src='https://www.youtube-nocookie.com/embed/yq0yDVBjqVM?autoplay=1&amp;playsinline=1&amp;rel=0';f.allow='autoplay; encrypted-media; picture-in-picture';f.allowFullscreen=true;f.title='ダイレクト出版';f.style.cssText='position:absolute;inset:0;width:100%;height:100%;border:0';this.replaceWith(f)"><img src="https://i.ytimg.com/vi/yq0yDVBjqVM/hqdefault.jpg" alt="アメリカ政府による国際法違反な日本支配を改善する必要があります。ダイレクト出版。" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block;"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);width:64px;height:64px;border-radius:50%;background:rgba(0,0,0,.65);color:#fff;font-size:30px;line-height:64px;text-align:center;">▶</span></button>
</div>
</section>
<section class="top-fuku" style="max-width:360px;margin:1rem auto;text-align:center;">
<p style="font-size:1.2rem;font-weight:700;margin:0 0 .6rem;"><a href="https://ameblo.jp/www-aon/entry-12984404414.html" target="_blank" rel="noopener noreferrer">捨てられそうな松の木が宝に！中国の驚きの菌類栽培 #茯苓（ぶくりょう） #養殖 #漢方</a></p>
<div style="position:relative;width:100%;aspect-ratio:9/16;background:#000;border-radius:8px;overflow:hidden;">
<button type="button" aria-label="動画を再生 / Play video" style="all:unset;position:absolute;inset:0;cursor:pointer;" onclick="var f=document.createElement('iframe');f.src='https://www.youtube-nocookie.com/embed/wcpa7Suy034?autoplay=1&amp;playsinline=1&amp;rel=0';f.allow='autoplay; encrypted-media; picture-in-picture';f.allowFullscreen=true;f.title='茯苓';f.style.cssText='position:absolute;inset:0;width:100%;height:100%;border:0';this.replaceWith(f)"><img src="https://i.ytimg.com/vi/wcpa7Suy034/hqdefault.jpg" alt="捨てられそうな松の木が宝に！中国の驚きの菌類栽培 #茯苓（ぶくりょう） #養殖 #漢方" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block;"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);width:64px;height:64px;border-radius:50%;background:rgba(0,0,0,.65);color:#fff;font-size:30px;line-height:64px;text-align:center;">▶</span></button>
</div>
<p style="margin:.6rem 0 0;"><a href="https://www.youtube.com/shorts/wcpa7Suy034" target="_blank" rel="noopener noreferrer">YouTube</a> ・ <a href="https://www.facebook.com/reel/1845761623108295?locale=ja_JP" target="_blank" rel="noopener noreferrer">Facebook リール</a></p>
</section>
<section class="top-fuku-note" style="max-width:42rem;margin:1rem auto;padding:.8rem 1rem;border:1px solid rgba(128,128,128,.5);border-radius:10px;text-align:left;line-height:1.8;font-size:1.05rem">
<p style="margin:0 0 .4rem;font-weight:700">茯苓（ぶくりょう）とは</p>
<p style="margin:0 0 .5rem">アカマツやクロマツの枯れ木の根に寄生するキノコ（マツホド）の菌核を使う生薬です。余分な水分の巡りを整える、気持ちを落ち着かせる、胃腸の調子を整える、といった働きで知られ、五苓散・桂枝茯苓丸・苓桂朮甘湯・茯苓飲など、多くの漢方処方に配合されています。</p>
<p style="margin:0 0 .5rem"><a href="https://www.tsumura.co.jp/kampo-view/know/syouyaku/ha013.html" target="_blank" rel="noopener noreferrer">詳しくはツムラの生薬辞典（茯苓）へ</a></p>
<p style="margin:0;font-size:.9rem;opacity:.8">※ 一般的な紹介であり、医療上の助言ではありません。服用は医師・薬剤師にご相談ください。</p>
</section>
<div class="mynavi-job" style="max-width:42rem;margin:1rem auto;padding:.8rem 1rem;border:1px solid rgba(128,128,128,.5);border-radius:10px;text-align:left;line-height:1.8;font-size:1.1rem">
<p style="margin:0 0 .4rem;font-weight:700">8年連続ホワイト企業で【プロジェクト管理】平均年収636万円</p>
<p style="margin:0 0 .4rem">アイアール株式会社</p>
<p style="margin:0 0 .6rem">＼8割以上が未経験スタート／年間休日125日★東証プライム企業G</p>
<p style="margin:0"><a href="https://tenshoku.mynavi.jp/jobinfo-303689-1-61-1/adv25/?ty=eer" target="_blank" rel="noopener noreferrer">マイナビ転職の求人ページ（写真・詳細はこちら）</a></p><p style="margin:.6rem 0 0"><a href="https://tenshoku.mynavi.jp/jobinfo-303689-1-61-1/adv25/?ty=eer" target="_blank" rel="noopener noreferrer"><img src="https://aruaru.tokyo/img/ai-r-recruit.jpg" alt="アイアール株式会社の求人（マイナビ転職掲載）" loading="lazy" style="width:100%;height:auto;display:block;border-radius:8px"></a></p>
</div>
<details class="hdd-corner" style="max-width:42rem;margin:.6rem auto;text-align:center"><summary style="cursor:pointer;font-size:1.4rem;font-weight:700">HDDコーナー</summary><p class="blog-link" style="font-size:1.4rem;text-align:center;margin:.6rem 0"><a href="https://kakaku.com/item/K0001653816/" target="_blank" rel="noopener noreferrer">HDDコーナー　2TB</a></p><p class="blog-link" style="font-size:1.4rem;text-align:center;margin:.6rem 0"><a href="https://kakaku.com/item/K0001651227/" target="_blank" rel="noopener noreferrer">HDDコーナー　4TB</a></p><p class="blog-link" style="font-size:1.4rem;text-align:center;margin:.6rem 0"><a href="https://kakaku.com/item/K0001651228/" target="_blank" rel="noopener noreferrer">HDDコーナー　6TB</a></p><p class="blog-link" style="font-size:1.4rem;text-align:center;margin:.6rem 0"><a href="https://kakaku.com/item/K0001654550/" target="_blank" rel="noopener noreferrer">HDDコーナー　8TB</a></p></details>
<p style="margin:0;line-height:1.2">&nbsp;</p>
<section class="digital-art-museum" style="max-width:42rem;margin:1rem auto;padding:.8rem 1rem;border:1px solid rgba(128,128,128,.5);border-radius:10px;text-align:left;line-height:1.8;font-size:1.1rem">
<h2 style="font-size:1.35rem;border-bottom:none;margin:0 0 .6rem;line-height:1.5">Digital Art Museum （デジタル美術館のアイデア）＆　保育園＆幼稚園＆小中高校＆　小学生と中学生向け学童保育　＆　スーパー銭湯　＆JAZZカフェレストラン（学校給食＆社食）＆　３WAYカーオーディオは、フロントのピラーに（スーパー）ツィーター＆スコーカー埋め込み対応をイエローハット＆オートバックスのお店も一緒に経営して行なうサービス＆不動産と建設と工務店と新車と中古車の自動車の売買と保険の融合店のアイデア</h2>
<p style="margin:0 0 .8rem"><a href="https://www.youtube.com/results?search_query=%E3%82%B4%E3%83%83%E3%83%9B+%E3%83%87%E3%82%B8%E3%82%BF%E3%83%AB%E3%82%A2%E3%83%BC%E3%83%88" target="_blank" rel="noopener noreferrer">ゴッホ　デジタルアートのYouTube検索結果</a></p>
<p style="margin:0 0 .8rem">絵画やバッハ、ベートーベン、モーツァルトなどの音楽家やMANGA（漫画）、アニメなどのAUDIO&amp;VISUAL（IMAX＆4DAXシアター対応）デジタルコンテンツをプロジェクションマッピングなどで体験出来る美術館の様な物を世界中に建設したり、</p>
<p style="margin:0 0 .8rem">コミケ　コスプレ　アニソン、<br><a href="https://www.youtube.com/results?search_query=%E3%82%B3%E3%83%9F%E3%82%B1%E3%80%80%E3%82%B3%E3%82%B9%E3%83%97%E3%83%AC%E3%80%80%E3%82%A2%E3%83%8B%E3%82%BD%E3%83%B3" target="_blank" rel="noopener noreferrer">アニメソングイベント</a></p>
<p style="margin:0 0 .8rem">や</p>
<p style="margin:0 0 1.4rem"><a href="https://www.youtube.com/results?search_query=%E3%82%B8%E3%83%96%E3%83%AA%E3%80%80%E3%83%8A%E3%82%A6%E3%82%B7%E3%82%AB%E3%80%80%E3%83%88%E3%83%88%E3%83%AD%E3%80%80%E3%82%A4%E3%82%AE%E3%83%AA%E3%82%B9%E3%80%80%E6%97%A5%E6%9C%AC%E3%80%80%E3%82%82%E3%81%AE%E3%81%AE%E3%81%91%E5%A7%AB%E3%80%80%E3%83%AF%E3%83%B3%E3%83%94%E3%83%BC%E3%82%B9%E3%80%80%E6%BC%94%E5%8A%87%E3%80%80%E6%AD%8C%E8%88%9E%E4%BC%8E" target="_blank" rel="noopener noreferrer">ジブリ　ナウシカ　トトロ　イギリス　日本　もののけ姫　ワンピース　演劇　歌舞伎</a></p>
<p style="margin:0 0 .8rem"><a href="https://www.youtube.com/results?search_query=%E3%82%AB%E3%82%A6%E3%83%9C%E3%83%BC%E3%82%A4%E3%83%93%E3%83%90%E3%83%83%E3%83%97+%7C+Cowboy+Bebop+Opening+%E2%80%9CTank!%E2%80%9D++Netflix+Japan" target="_blank" rel="noopener noreferrer">カウボーイビバップ | Cowboy Bebop Opening “Tank!”  Netflix Japan</a></p>
<p style="margin:0"><a href="https://www.youtube.com/results?search_query=%E3%82%BC%E3%83%AB%E3%83%80%E3%81%AE%E4%BC%9D%E8%AA%AC+%E3%82%AA%E3%83%BC%E3%82%B1%E3%82%B9%E3%83%88%E3%83%A9" target="_blank" rel="noopener noreferrer">ゼルダの伝説 オーケストラ</a></p>
<p style="margin:1.4rem 0 0">などを経営して売上を出し経費や正社員給料の月給やボーナスや退職金を引いても利益を出し毎年法人税をしっかりと納税する法人を目指す。</p>
</section>
<p style="margin:0;line-height:1.2">&nbsp;</p>
<section class="voice-training" style="max-width:42rem;margin:1rem auto;padding:.8rem 1rem;border:1px solid rgba(128,128,128,.5);border-radius:10px;text-align:left;line-height:1.8;font-size:1.1rem">
<h2 style="font-size:1.35rem;border-bottom:none;margin:0 0 .6rem;line-height:1.5">ボイストレーニング教室</h2>
<p style="margin:0 0 .8rem"><a href="https://www.youtube.com/results?search_query=%E3%83%9C%E3%82%A4%E3%82%B9%E3%83%88%E3%83%AC%E3%83%BC%E3%83%8B%E3%83%B3%E3%82%B0+%E3%83%93%E3%83%96%E3%83%A9%E3%83%BC%E3%83%88" target="_blank" rel="noopener noreferrer">ビブラート　ボイストレーニング</a></p>
<p style="margin:0 0 .6rem;font-weight:700"><a href="https://www.youtube.com/watch?v=nD62Ojw8E24" target="_blank" rel="noopener noreferrer">【ビブラート】ボイストレーナーが教える練習法</a></p>
<div style="position:relative;width:100%;aspect-ratio:16/9;margin:0 0 1.2rem;background:#000;border-radius:8px;overflow:hidden;">
<button type="button" aria-label="動画を再生 / Play video" style="all:unset;position:absolute;inset:0;cursor:pointer;" onclick="var f=document.createElement('iframe');f.src='https://www.youtube-nocookie.com/embed/nD62Ojw8E24?autoplay=1&amp;playsinline=1&amp;rel=0';f.allow='autoplay; encrypted-media; picture-in-picture';f.allowFullscreen=true;f.title='【ビブラート】ボイストレーナーが教える練習法';f.style.cssText='position:absolute;inset:0;width:100%;height:100%;border:0';this.replaceWith(f)"><img src="https://i.ytimg.com/vi/nD62Ojw8E24/hqdefault.jpg" alt="【ビブラート】ボイストレーナーが教える練習法" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block;"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);width:64px;height:64px;border-radius:50%;background:rgba(0,0,0,.65);color:#fff;font-size:30px;line-height:64px;text-align:center;">▶</span></button>
</div>
<p style="margin:0 0 .6rem;font-weight:700"><a href="https://www.youtube.com/watch?v=JbB6w-vflI0" target="_blank" rel="noopener noreferrer">【ボイトレ】絶対カラオケに役立つ！7組のアーティストビブラート分析！！</a></p>
<div style="position:relative;width:100%;aspect-ratio:16/9;margin:0 0 1.2rem;background:#000;border-radius:8px;overflow:hidden;">
<button type="button" aria-label="動画を再生 / Play video" style="all:unset;position:absolute;inset:0;cursor:pointer;" onclick="var f=document.createElement('iframe');f.src='https://www.youtube-nocookie.com/embed/JbB6w-vflI0?autoplay=1&amp;playsinline=1&amp;rel=0';f.allow='autoplay; encrypted-media; picture-in-picture';f.allowFullscreen=true;f.title='【ボイトレ】絶対カラオケに役立つ！7組のアーティストビブラート分析！！';f.style.cssText='position:absolute;inset:0;width:100%;height:100%;border:0';this.replaceWith(f)"><img src="https://i.ytimg.com/vi/JbB6w-vflI0/hqdefault.jpg" alt="【ボイトレ】絶対カラオケに役立つ！7組のアーティストビブラート分析！！" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block;"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);width:64px;height:64px;border-radius:50%;background:rgba(0,0,0,.65);color:#fff;font-size:30px;line-height:64px;text-align:center;">▶</span></button>
</div>
<p style="margin:0 0 .8rem"><a href="https://www.youtube.com/results?search_query=%E3%83%9C%E3%82%A4%E3%82%B9%E3%83%88%E3%83%AC%E3%83%BC%E3%83%8B%E3%83%B3%E3%82%B0+%E3%82%A6%E3%82%A4%E3%82%B9%E3%83%91%E3%83%BC%E3%83%9C%E3%82%A4%E3%82%B9" target="_blank" rel="noopener noreferrer">ボイストレーニング ウイスパーボイス</a></p>
<p style="margin:0 0 .8rem"><a href="https://www.youtube.com/results?search_query=%E3%83%96%E3%83%AC%E3%82%B9%E3%82%A2%E3%82%A6%E3%83%88%E3%80%80%E3%83%9C%E3%82%A4%E3%82%B9%E3%83%88%E3%83%AC%E3%83%BC%E3%83%8B%E3%83%B3%E3%82%B0" target="_blank" rel="noopener noreferrer">ブレスアウト　ボイストレーニング</a></p>
<p style="margin:0 0 .8rem"><a href="https://www.youtube.com/results?search_query=%E3%82%B0%E3%83%AB%E3%83%BC%E3%83%B4%E6%84%9F%E3%80%80%E3%83%9C%E3%82%A4%E3%82%B9%E3%83%88%E3%83%AC%E3%83%BC%E3%83%8B%E3%83%B3%E3%82%B0" target="_blank" rel="noopener noreferrer">グルーヴ感　ボイストレーニング</a></p>
<p style="margin:0 0 .6rem;font-weight:700"><a href="https://www.youtube.com/watch?v=srYeyGa8kAo" target="_blank" rel="noopener noreferrer">一流歌手が持つ”グルーヴ感”を身につける方法7step。リズムを鍛える。あなたはどこまでできる？</a></p>
<div style="position:relative;width:100%;aspect-ratio:16/9;margin:0 0 1.2rem;background:#000;border-radius:8px;overflow:hidden;">
<button type="button" aria-label="動画を再生 / Play video" style="all:unset;position:absolute;inset:0;cursor:pointer;" onclick="var f=document.createElement('iframe');f.src='https://www.youtube-nocookie.com/embed/srYeyGa8kAo?autoplay=1&amp;playsinline=1&amp;rel=0';f.allow='autoplay; encrypted-media; picture-in-picture';f.allowFullscreen=true;f.title='一流歌手が持つ”グルーヴ感”を身につける方法7step。リズムを鍛える。あなたはどこまでできる？';f.style.cssText='position:absolute;inset:0;width:100%;height:100%;border:0';this.replaceWith(f)"><img src="https://i.ytimg.com/vi/srYeyGa8kAo/hqdefault.jpg" alt="一流歌手が持つ”グルーヴ感”を身につける方法7step。リズムを鍛える。あなたはどこまでできる？" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block;"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);width:64px;height:64px;border-radius:50%;background:rgba(0,0,0,.65);color:#fff;font-size:30px;line-height:64px;text-align:center;">▶</span></button>
</div>
<p style="margin:0 0 .6rem;font-weight:700"><a href="https://www.youtube.com/watch?v=5w-475aoO8s" target="_blank" rel="noopener noreferrer">【ボイトレ】グルーヴ感を身につける方法！【グルーヴ感欲しくない人は見ないで】</a></p>
<div style="position:relative;width:100%;aspect-ratio:16/9;margin:0 0 1.2rem;background:#000;border-radius:8px;overflow:hidden;">
<button type="button" aria-label="動画を再生 / Play video" style="all:unset;position:absolute;inset:0;cursor:pointer;" onclick="var f=document.createElement('iframe');f.src='https://www.youtube-nocookie.com/embed/5w-475aoO8s?autoplay=1&amp;playsinline=1&amp;rel=0';f.allow='autoplay; encrypted-media; picture-in-picture';f.allowFullscreen=true;f.title='【ボイトレ】グルーヴ感を身につける方法！【グルーヴ感欲しくない人は見ないで】';f.style.cssText='position:absolute;inset:0;width:100%;height:100%;border:0';this.replaceWith(f)"><img src="https://i.ytimg.com/vi/5w-475aoO8s/hqdefault.jpg" alt="【ボイトレ】グルーヴ感を身につける方法！【グルーヴ感欲しくない人は見ないで】" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block;"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);width:64px;height:64px;border-radius:50%;background:rgba(0,0,0,.65);color:#fff;font-size:30px;line-height:64px;text-align:center;">▶</span></button>
</div>
<p style="margin:0 0 .8rem"><a href="https://www.youtube.com/results?search_query=%E5%80%8D%E9%9F%B3%E3%80%80%E3%83%9C%E3%82%A4%E3%82%B9%E3%83%88%E3%83%AC%E3%83%BC%E3%83%8B%E3%83%B3%E3%82%B0" target="_blank" rel="noopener noreferrer">倍音　ボイストレーニング</a></p>
<p style="margin:0 0 .6rem;font-weight:700"><a href="https://www.youtube.com/watch?v=sp-VAEoINSY" target="_blank" rel="noopener noreferrer">【知らないと損】『いい歌声』の正体はコレ。「倍音と音色」で 場の雰囲気を一変させる歌声の作り方【ボイトレ】</a></p>
<div style="position:relative;width:100%;aspect-ratio:16/9;margin:0 0 1.2rem;background:#000;border-radius:8px;overflow:hidden;">
<button type="button" aria-label="動画を再生 / Play video" style="all:unset;position:absolute;inset:0;cursor:pointer;" onclick="var f=document.createElement('iframe');f.src='https://www.youtube-nocookie.com/embed/sp-VAEoINSY?autoplay=1&amp;playsinline=1&amp;rel=0';f.allow='autoplay; encrypted-media; picture-in-picture';f.allowFullscreen=true;f.title='【知らないと損】『いい歌声』の正体はコレ。「倍音と音色」で 場の雰囲気を一変させる歌声の作り方【ボイトレ】';f.style.cssText='position:absolute;inset:0;width:100%;height:100%;border:0';this.replaceWith(f)"><img src="https://i.ytimg.com/vi/sp-VAEoINSY/hqdefault.jpg" alt="【知らないと損】『いい歌声』の正体はコレ。「倍音と音色」で 場の雰囲気を一変させる歌声の作り方【ボイトレ】" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block;"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);width:64px;height:64px;border-radius:50%;background:rgba(0,0,0,.65);color:#fff;font-size:30px;line-height:64px;text-align:center;">▶</span></button>
</div>
<p style="margin:1.4rem 0 0">ピアノ教室　バイオリン、チェロ、コントラバス教室</p>
</section>
<p style="margin:0;line-height:1.2">&nbsp;</p>
<section class="hakusho-corner" style="max-width:42rem;margin:1rem auto;padding:.8rem 1rem;border:1px solid rgba(128,128,128,.5);border-radius:10px;text-align:left;line-height:1.8;font-size:1.1rem">
<h2 style="font-size:1.35rem;border-bottom:none;margin:0 0 .6rem;line-height:1.5">〇〇白書から始まる、小説作家、漫画家、アニメ・ドラマ脚本家、コンテンツ作家育成コーナー。</h2>
<h3 style="font-size:1.15rem;margin:.8rem 0 .4rem">◯◯白書とは？</h3>
<p style="margin:0 0 .8rem">普通の日常生活の中で、子供から大人まで、〇〇君、〇〇ちゃん、良く気づいたね！良く成長したね！お兄さんも、お姉さんも、お父さんも、お母さんも、サンタクロースもトナカイさんも、七福神もびっくり仰天！などと、驚きながら、褒（ほ）めて差し上げて下さい。</p>
<p style="margin:0 0 .8rem">サンプルとして、<br>有名な、ヒカルの碁では、普通の小学六年生の進藤ヒカル君は、おじいさんの蔵の中でシミの付いた碁盤をガールフレンドと一緒に見つけて、その碁番に憑いていた藤原佐為と言う囲碁の達人に再び現世で代わりに囲碁を打たせてあげる為にその幽霊に取り憑いて良い許可をして、その幽霊が取り憑いていなくても囲碁の達人として囲碁のプロを目指すべく成長していくストーリーです。<br>日常の生活＋囲碁の達人の幽霊。</p>
<p style="margin:0 0 .8rem">他には、普通の男子高校生＋エイリアン彼女で、<br>うる星やつら　と言うアニメにもなった漫画も有名ですね。<br>と言った御紹介です。</p>
<p style="margin:0">〇〇白書の日常生活＋＠で新しいコンテンツ作品が誕生するのです。<br>と言った御紹介でした。</p>
<p style="margin:1.4rem 0 0">小説や漫画やアニメやドラマや映画やGAMEなどでは、〇〇白書以外にも、官能小説、学園恋愛小説、SF小説、ミステリー推理小説、スパイ小説、ホラー小説、歴史・時代小説、ライトノベル、児童文学、ヤングアダルトなどが御座います。</p>
</section>
<div class="jobinfo-guard" style="max-width:42rem;margin:1rem auto;padding:.8rem 1rem;border:1px solid rgba(128,128,128,.5);border-radius:10px;text-align:left;line-height:1.8;font-size:1.1rem"><p style="margin:0 0 .6rem">秋川駅周辺や近隣エリア（立川・羽村・福生など）では、日払いに対応し、即日入寮が可能な警備会社の求人が複数あります。 <a href="https://www.baitoru.com/kanto/jlist/2186akigawaeki/guard/mrt18/" target="_blank" rel="noopener noreferrer">[1]</a> <a href="https://www.google.com/searchviewer/10?svid=CCYSESoPCg0vZy8xMWJ6eXB0c18zGAo" target="_blank" rel="noopener noreferrer">[2]</a></p><p style="margin:0 0 .4rem;font-weight:700">主な警備会社・求人情報</p><p style="margin:0 0 .4rem"><a href="https://www.google.com/search?kgmid=%2Fg%2F11bzypts_3&amp;q=%E5%A4%A7%E7%9C%9F%E7%B6%9C%E5%90%88%E8%AD%A6%E5%82%99%E4%BF%9D%E9%9A%9C+%E6%A0%AA%E5%BC%8F%E4%BC%9A%E7%A4%BE" target="_blank" rel="noopener noreferrer">大真綜合警備保障 株式会社</a><br>4.3(122)　警備業<br>営業時間中 · 柴崎町<br>電話：042-548-1657　24時間営業<br><a href="https://www.google.com/maps/dir//%E3%80%92190-0023+%E6%9D%B1%E4%BA%AC%E9%83%BD%E7%AB%8B%E5%B7%9D%E5%B8%82%E6%9F%B4%E5%B4%8E%E7%94%BA%EF%BC%92%E4%B8%81%E7%9B%AE%EF%BC%97%E2%88%92%EF%BC%96+%E3%81%95%E3%81%8B%E3%81%88%E3%83%93%E3%83%AB/data=!4m6!4m5!1m1!4e2!1m2!1m1!1s0x6018e1782c995555:0xf08281ff691daec2?entry=s&amp;ved=1t:269504&amp;ictx=111" target="_blank" rel="noopener noreferrer">経路</a>　<a href="https://daisin-keibi.com/" target="_blank" rel="noopener noreferrer">ウェブサイト</a></p><ul style="margin:0;padding-left:1.4rem"><li>特徴: 面接当日に合否を伝え、希望者はその日のうちに単身用1Rの完全個室寮へ案内（初期費用・保証人不要）。</li><li>あきる野支店: JR五日市線 秋川駅 南口から徒歩4分。 <a href="https://www.google.com/maps/place/35%C2%B043'34.7%22N+139%C2%B017'07.5%22E/@35.72631,139.2828451,17z/data=!3m1!4b1!4m4!3m3!8m2!3d35.72631!4d139.28542" target="_blank" rel="noopener noreferrer">地図</a></li><li>給与例: 交通誘導警備など（日給11,360円〜など）。</li><li>公式情報: <a href="https://www.google.com/search?kgmid=%2Fg%2F11bzypts_3&amp;q=%E5%A4%A7%E7%9C%9F%E7%B6%9C%E5%90%88%E8%AD%A6%E5%82%99%E4%BF%9D%E9%9A%9C+%E6%A0%AA%E5%BC%8F%E4%BC%9A%E7%A4%BE" target="_blank" rel="noopener noreferrer">大真綜合警備保障 株式会社</a> <a href="https://www.baitoru.com/kanto/jlist/2186akigawaeki/guard/mrt18/" target="_blank" rel="noopener noreferrer">[1]</a> <a href="https://www.google.com/searchviewer/10?svid=CCYSESoPCg0vZy8xMWJ6eXB0c18zGAo" target="_blank" rel="noopener noreferrer">[2]</a></li></ul></div><p style="font-size:1.2rem;font-weight:700;margin:0 0 .6rem;"><a href="https://www.youtube.com/shorts/2MeGiDzY2i0" target="_blank" rel="noopener noreferrer">上海がゴーストタウンに！中国の崩壊が始まった？</a></p>
<div style="position:relative;width:100%;max-width:360px;aspect-ratio:9/16;margin:0 auto 1.2rem;background:#000;border-radius:8px;overflow:hidden;">
<button type="button" aria-label="動画を再生 / Play video" style="all:unset;position:absolute;inset:0;cursor:pointer;" onclick="var f=document.createElement('iframe');f.src='https://www.youtube-nocookie.com/embed/2MeGiDzY2i0?autoplay=1&amp;playsinline=1&amp;rel=0';f.allow='autoplay; encrypted-media; picture-in-picture';f.allowFullscreen=true;f.title='上海がゴーストタウンに！中国の崩壊が始まった？';f.style.cssText='position:absolute;inset:0;width:100%;height:100%;border:0';this.replaceWith(f)"><img src="https://i.ytimg.com/vi/2MeGiDzY2i0/hqdefault.jpg" alt="上海がゴーストタウンに！中国の崩壊が始まった？" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block;"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);width:64px;height:64px;border-radius:50%;background:rgba(0,0,0,.65);color:#fff;font-size:30px;line-height:64px;text-align:center;">▶</span></button>
</div>
<p style="font-size:1.2rem;font-weight:700;margin:0 0 .6rem;"><a href="https://www.youtube.com/shorts/oPGV1n8niXw" target="_blank" rel="noopener noreferrer">今の上海は経済崩壊でシャッター街なの？</a></p>
<div style="position:relative;width:100%;max-width:360px;aspect-ratio:9/16;margin:0 auto 1.2rem;background:#000;border-radius:8px;overflow:hidden;">
<button type="button" aria-label="動画を再生 / Play video" style="all:unset;position:absolute;inset:0;cursor:pointer;" onclick="var f=document.createElement('iframe');f.src='https://www.youtube-nocookie.com/embed/oPGV1n8niXw?autoplay=1&amp;playsinline=1&amp;rel=0';f.allow='autoplay; encrypted-media; picture-in-picture';f.allowFullscreen=true;f.title='今の上海は経済崩壊でシャッター街なの？';f.style.cssText='position:absolute;inset:0;width:100%;height:100%;border:0';this.replaceWith(f)"><img src="https://i.ytimg.com/vi/oPGV1n8niXw/hqdefault.jpg" alt="今の上海は経済崩壊でシャッター街なの？" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block;"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);width:64px;height:64px;border-radius:50%;background:rgba(0,0,0,.65);color:#fff;font-size:30px;line-height:64px;text-align:center;">▶</span></button>
</div>
<div style="position:relative;max-width:560px;aspect-ratio:16/9;cursor:pointer;background:#000;margin:.8rem 0 0" title="クリックで再生" onclick="this.innerHTML='&lt;iframe src=&quot;https://www.youtube-nocookie.com/embed/Hw5tkXmHbDY?autoplay=1&quot; style=&quot;width:100%;height:100%;border:0&quot; allow=&quot;autoplay; encrypted-media; fullscreen&quot; allowfullscreen&gt;&lt;/iframe&gt;'"><img src="https://i.ytimg.com/vi/Hw5tkXmHbDY/hqdefault.jpg" alt="【短縮版】中国経済が崩壊寸前…" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);font-size:3rem;color:#fff;text-shadow:0 0 8px #000">▶</span></div><p style="margin:.3rem 0 0"><a href="https://www.youtube.com/watch?v=Hw5tkXmHbDY" target="_blank" rel="noopener noreferrer">▶️ 【短縮版】中国経済が崩壊寸前…富裕層も軍のトップも一斉に逃げ出しています！</a></p>
<p style="font-size:1.2rem;font-weight:700;margin:1rem 0 .6rem;"><a href="https://www.youtube.com/watch?v=0zFcPiy6K10" target="_blank" rel="noopener noreferrer">【金融シリーズ①】これを見逃すとエライことになります。日本国債＋リスクヘッジで7％の利息受取？</a></p>
<div style="position:relative;width:100%;max-width:560px;aspect-ratio:16/9;margin:0 auto 1.2rem;background:#000;border-radius:8px;overflow:hidden;">
<button type="button" aria-label="動画を再生 / Play video" style="all:unset;position:absolute;inset:0;cursor:pointer;" onclick="var f=document.createElement('iframe');f.src='https://www.youtube-nocookie.com/embed/0zFcPiy6K10?autoplay=1&amp;playsinline=1&amp;rel=0';f.allow='autoplay; encrypted-media; picture-in-picture';f.allowFullscreen=true;f.title='【金融シリーズ①】これを見逃すとエライことになります。日本国債＋リスクヘッジで7％の利息受取？';f.style.cssText='position:absolute;inset:0;width:100%;height:100%;border:0';this.replaceWith(f)"><img src="https://i.ytimg.com/vi/0zFcPiy6K10/hqdefault.jpg" alt="【金融シリーズ①】これを見逃すとエライことになります。日本国債＋リスクヘッジで7％の利息受取？" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block;"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);width:64px;height:64px;border-radius:50%;background:rgba(0,0,0,.65);color:#fff;font-size:30px;line-height:64px;text-align:center;">▶</span></button>
</div>
<p style="margin:.2rem 0 .6rem;"><a href="https://www.facebook.com/reel/1589704945870968?locale=ja_JP" target="_blank" rel="noopener noreferrer">Facebook(予備 / backup)</a></p>
<video controls preload="none" poster="https://i.ytimg.com/vi/0zFcPiy6K10/hqdefault.jpg" style="width:100%;max-width:560px;aspect-ratio:16/9;height:auto;background:#000;border-radius:6px;object-fit:contain;display:block;margin:0 auto .6rem;">
<source src="https://aruaru.tokyo/video/NIHON-KOKUSAI.mp4" type="video/mp4">
</video>
<p style="margin:0 0 1rem;"><a href="https://aruaru.tokyo/video/7-percent.mp4" target="_blank" rel="noopener noreferrer">動画(mp4、予備 / backup)</a></p>
<p style="font-size:1.2rem;font-weight:700;margin:1rem 0 .6rem;"><a href="https://www.youtube.com/watch?v=rH_qa9-QauE" target="_blank" rel="noopener noreferrer">【債券投資】日本国債に大きな追い風！世界一の運用会社・ブラックロックの買いが来る？利回り６％の仕組みを解説。今後に期待大！</a></p>
<div style="position:relative;width:100%;max-width:560px;aspect-ratio:16/9;margin:0 auto 1.2rem;background:#000;border-radius:8px;overflow:hidden;">
<button type="button" aria-label="動画を再生 / Play video" style="all:unset;position:absolute;inset:0;cursor:pointer;" onclick="var f=document.createElement('iframe');f.src='https://www.youtube-nocookie.com/embed/rH_qa9-QauE?autoplay=1&amp;playsinline=1&amp;rel=0';f.allow='autoplay; encrypted-media; picture-in-picture';f.allowFullscreen=true;f.title='【債券投資】日本国債に大きな追い風！世界一の運用会社・ブラックロックの買いが来る？利回り６％の仕組みを解説。今後に期待大！';f.style.cssText='position:absolute;inset:0;width:100%;height:100%;border:0';this.replaceWith(f)"><img src="https://i.ytimg.com/vi/rH_qa9-QauE/hqdefault.jpg" alt="【債券投資】日本国債に大きな追い風！世界一の運用会社・ブラックロックの買いが来る？利回り６％の仕組みを解説。今後に期待大！" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block;"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);width:64px;height:64px;border-radius:50%;background:rgba(0,0,0,.65);color:#fff;font-size:30px;line-height:64px;text-align:center;">▶</span></button>
</div>
<p style="font-size:1.2rem;font-weight:700;margin:1rem 0 .6rem;"><a href="https://www.youtube.com/shorts/1PsaVQcPL84" target="_blank" rel="noopener noreferrer">竹中平蔵💢郵政民営化でゆうちょ370兆円をアメリカに渡して見返りに4兆円を貰う</a></p>
<div style="position:relative;width:100%;max-width:360px;aspect-ratio:9/16;margin:0 auto 1.2rem;background:#000;border-radius:8px;overflow:hidden;">
<button type="button" aria-label="動画を再生 / Play video" style="all:unset;position:absolute;inset:0;cursor:pointer;" onclick="var f=document.createElement('iframe');f.src='https://www.youtube-nocookie.com/embed/1PsaVQcPL84?autoplay=1&amp;playsinline=1&amp;rel=0';f.allow='autoplay; encrypted-media; picture-in-picture';f.allowFullscreen=true;f.title='竹中平蔵💢郵政民営化でゆうちょ370兆円をアメリカに渡して見返りに4兆円を貰う';f.style.cssText='position:absolute;inset:0;width:100%;height:100%;border:0';this.replaceWith(f)"><img src="https://i.ytimg.com/vi/1PsaVQcPL84/hqdefault.jpg" alt="竹中平蔵💢郵政民営化でゆうちょ370兆円をアメリカに渡して見返りに4兆円を貰う" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block;"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);width:64px;height:64px;border-radius:50%;background:rgba(0,0,0,.65);color:#fff;font-size:30px;line-height:64px;text-align:center;">▶</span></button>
</div>
<p style="font-size:1.2rem;font-weight:700;margin:1rem 0 .6rem;"><a href="https://www.youtube.com/watch?v=AZ0H9Ch7Ka4" target="_blank" rel="noopener noreferrer">今、シニア富裕層が米ドル債券への投資を急ぐ「本当の理由」3選！！老後は債券で利息生活！年金に上乗せ！今すぐ放っておいても安心安全の最強運用！！【1219】</a></p>
<div style="position:relative;width:100%;max-width:560px;aspect-ratio:16/9;margin:0 auto 1.2rem;background:#000;border-radius:8px;overflow:hidden;">
<button type="button" aria-label="動画を再生 / Play video" style="all:unset;position:absolute;inset:0;cursor:pointer;" onclick="var f=document.createElement('iframe');f.src='https://www.youtube-nocookie.com/embed/AZ0H9Ch7Ka4?autoplay=1&amp;playsinline=1&amp;rel=0';f.allow='autoplay; encrypted-media; picture-in-picture';f.allowFullscreen=true;f.title='今、シニア富裕層が米ドル債券への投資を急ぐ「本当の理由」3選！！老後は債券で利息生活！年金に上乗せ！今すぐ放っておいても安心安全の最強運用！！【1219】';f.style.cssText='position:absolute;inset:0;width:100%;height:100%;border:0';this.replaceWith(f)"><img src="https://i.ytimg.com/vi/AZ0H9Ch7Ka4/hqdefault.jpg" alt="今、シニア富裕層が米ドル債券への投資を急ぐ「本当の理由」3選！！老後は債券で利息生活！年金に上乗せ！今すぐ放っておいても安心安全の最強運用！！【1219】" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block;"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);width:64px;height:64px;border-radius:50%;background:rgba(0,0,0,.65);color:#fff;font-size:30px;line-height:64px;text-align:center;">▶</span></button>
</div>
<p style="font-size:1.2rem;font-weight:700;margin:1rem 0 .6rem;"><a href="https://www.youtube.com/watch?v=5bWEtmNw488" target="_blank" rel="noopener noreferrer">自然由来の素材　セルロースナノファイバーで雑草を抑制する実験　富士市と日本製紙が共同実施　静岡</a></p>
<div style="position:relative;width:100%;max-width:560px;aspect-ratio:16/9;margin:0 auto 1.2rem;background:#000;border-radius:8px;overflow:hidden;">
<button type="button" aria-label="動画を再生 / Play video" style="all:unset;position:absolute;inset:0;cursor:pointer;" onclick="var f=document.createElement('iframe');f.src='https://www.youtube-nocookie.com/embed/5bWEtmNw488?autoplay=1&amp;playsinline=1&amp;rel=0';f.allow='autoplay; encrypted-media; picture-in-picture';f.allowFullscreen=true;f.title='自然由来の素材　セルロースナノファイバーで雑草を抑制する実験　富士市と日本製紙が共同実施　静岡';f.style.cssText='position:absolute;inset:0;width:100%;height:100%;border:0';this.replaceWith(f)"><img src="https://i.ytimg.com/vi/5bWEtmNw488/hqdefault.jpg" alt="自然由来の素材　セルロースナノファイバーで雑草を抑制する実験　富士市と日本製紙が共同実施　静岡" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block;"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);width:64px;height:64px;border-radius:50%;background:rgba(0,0,0,.65);color:#fff;font-size:30px;line-height:64px;text-align:center;">▶</span></button>
</div><p style="font-size:1.1rem;margin:.2rem 0 1rem;"><a href="https://www.youtube.com/results?search_query=%E7%86%B1%E6%B9%AF+%E9%99%A4%E8%8D%89+%E3%82%B1%E3%83%AB%E3%83%92%E3%83%A3%E3%83%BC" target="_blank" rel="noopener noreferrer">🔎 熱湯 除草 ケルヒャー（YouTube検索結果）</a></p>
<div style="position:relative;max-width:560px;aspect-ratio:16/9;cursor:pointer;background:#000;margin:1rem 0 0" title="クリックで再生" onclick="this.innerHTML='&lt;iframe src=&quot;https://www.youtube-nocookie.com/embed/TwEdf7aJ6yk?autoplay=1&quot; style=&quot;width:100%;height:100%;border:0&quot; allow=&quot;autoplay; encrypted-media; fullscreen&quot; allowfullscreen&gt;&lt;/iframe&gt;'"><img src="https://i.ytimg.com/vi/TwEdf7aJ6yk/hqdefault.jpg" alt="【海外の反応】「日本人は音楽の英才教育を…" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);font-size:3rem;color:#fff;text-shadow:0 0 8px #000">▶</span></div><p style="margin:.3rem 0 0"><a href="https://www.youtube.com/watch?v=TwEdf7aJ6yk" target="_blank" rel="noopener noreferrer">▶️ 【海外の反応】「日本人は音楽の英才教育を受けてるのか？」日本の音楽が今世界を席巻している理由に外国人が熱狂</a></p>
<div style="max-width:640px;margin:0 auto 1.4rem;">
<p style="font-size:1.2rem;font-weight:700;margin:0 0 .6rem;"><a href="https://www.youtube.com/watch?v=c7pEIqhxj20" target="_blank" rel="noopener noreferrer">【海外の反応】塩水で発電を日本が実用化へ！世界中の電力会社が諦めた開発に世界中が大注目の理由</a><br><small lang="en" style="font-weight:400;">[Reactions from overseas] Japan moves to put salt-water power generation into practical use! Why the world is watching a development that power companies worldwide gave up on</small></p>
<div style="position:relative;width:100%;aspect-ratio:16/9;background:#000;border-radius:8px;overflow:hidden;">
<button type="button" aria-label="動画を再生 / Play video" style="all:unset;position:absolute;inset:0;cursor:pointer;" onclick="var f=document.createElement('iframe');f.src='https://www.youtube-nocookie.com/embed/c7pEIqhxj20?autoplay=1&amp;playsinline=1&amp;rel=0';f.allow='autoplay; encrypted-media; picture-in-picture';f.allowFullscreen=true;f.title='【海外の反応】塩水で発電を日本が実用化へ！世界中の電力会社が諦めた開発に世界中が大注目の理由';f.style.cssText='position:absolute;inset:0;width:100%;height:100%;border:0';this.replaceWith(f)"><img src="https://i.ytimg.com/vi/c7pEIqhxj20/hqdefault.jpg" alt="【海外の反応】塩水で発電を日本が実用化へ！世界中の電力会社が諦めた開発に世界中が大注目の理由" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block;"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);width:64px;height:64px;border-radius:50%;background:rgba(0,0,0,.65);color:#fff;font-size:30px;line-height:64px;text-align:center;">▶</span></button>
</div>
</div>
<div style="max-width:360px;margin:0 auto 1.4rem;">
<p style="font-size:1.2rem;font-weight:700;margin:0 0 .6rem;"><a href="https://youtube.com/shorts/atA0yTg6-YE" target="_blank" rel="noopener noreferrer">Googleを落ちた高卒の23歳が世界を変えた実話</a><br><small lang="en" style="font-weight:400;">The true story of a 23-year-old high-school graduate who failed Google and changed the world</small></p>
<div style="position:relative;width:100%;aspect-ratio:9/16;background:#000;border-radius:8px;overflow:hidden;">
<button type="button" aria-label="動画を再生 / Play video" style="all:unset;position:absolute;inset:0;cursor:pointer;" onclick="var f=document.createElement('iframe');f.src='https://www.youtube-nocookie.com/embed/atA0yTg6-YE?autoplay=1&amp;playsinline=1&amp;rel=0';f.allow='autoplay; encrypted-media; picture-in-picture';f.allowFullscreen=true;f.title='Googleを落ちた高卒の23歳が世界を変えた実話';f.style.cssText='position:absolute;inset:0;width:100%;height:100%;border:0';this.replaceWith(f)"><img src="https://i.ytimg.com/vi/atA0yTg6-YE/oar2.jpg" onerror="this.onerror=null;this.src='https://i.ytimg.com/vi/atA0yTg6-YE/hqdefault.jpg'" alt="Googleを落ちた高卒の23歳が世界を変えた実話" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block;"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);width:64px;height:64px;border-radius:50%;background:rgba(0,0,0,.65);color:#fff;font-size:30px;line-height:64px;text-align:center;">▶</span></button>
</div>
<p style="text-align:left;line-height:1.7;margin:.6rem 0 0;">2026年10月現在、TVや新聞や雑誌の広告からYouTubeやSNSなどのWEBに広告はシフトしているのですから、宣伝の仕方も変えていかないとね！<br>株式会社エーオン 代表取締役社長の石　塚　正　浩としても、広告代理店業を早期に立ち上げたいと思っております。</p>
<p lang="en" style="text-align:left;line-height:1.6;font-size:.9rem;opacity:.85;margin:.3rem 0 0;">As of October 2026, advertising has shifted from TV, newspapers and magazines to the web, such as YouTube and social media, so the way we promote things has to change too! As the President and CEO of AON Co., Ltd., Masahiro Ishizuka, I would like to launch an advertising agency business as soon as possible.</p>
</div>
<div lang="en" style="display:flex;flex-wrap:wrap;gap:1rem;justify-content:center;margin:0 auto 1.4rem;">
<div style="flex:1 1 280px;max-width:340px;">
<p style="font-size:1.05rem;font-weight:700;margin:0 0 .5rem;"><a href="https://www.youtube.com/shorts/FCQPLN6zndQ" target="_blank" rel="noopener noreferrer">His Company Worth Millions After He Was Fired - PART1</a></p>
<div style="position:relative;width:100%;aspect-ratio:9/16;background:#000;border-radius:8px;overflow:hidden;">
<button type="button" aria-label="動画を再生 / Play video" style="all:unset;position:absolute;inset:0;cursor:pointer;" onclick="var f=document.createElement('iframe');f.src='https://www.youtube-nocookie.com/embed/FCQPLN6zndQ?autoplay=1&amp;playsinline=1&amp;rel=0';f.allow='autoplay; encrypted-media; picture-in-picture';f.allowFullscreen=true;f.title='His Company Worth Millions After He Was Fired - PART1';f.style.cssText='position:absolute;inset:0;width:100%;height:100%;border:0';this.replaceWith(f)"><img src="https://i.ytimg.com/vi/FCQPLN6zndQ/oar2.jpg" onerror="this.onerror=null;this.src='https://i.ytimg.com/vi/FCQPLN6zndQ/hqdefault.jpg'" alt="His Company Worth Millions After He Was Fired - PART1" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block;"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);width:64px;height:64px;border-radius:50%;background:rgba(0,0,0,.65);color:#fff;font-size:30px;line-height:64px;text-align:center;">▶</span></button>
</div>
</div>
<div style="flex:1 1 280px;max-width:340px;">
<p style="font-size:1.05rem;font-weight:700;margin:0 0 .5rem;"><a href="https://www.youtube.com/shorts/7aLHquRI5oY" target="_blank" rel="noopener noreferrer">His Company Worth Millions After He Was Fired - PART2</a></p>
<div style="position:relative;width:100%;aspect-ratio:9/16;background:#000;border-radius:8px;overflow:hidden;">
<button type="button" aria-label="動画を再生 / Play video" style="all:unset;position:absolute;inset:0;cursor:pointer;" onclick="var f=document.createElement('iframe');f.src='https://www.youtube-nocookie.com/embed/7aLHquRI5oY?autoplay=1&amp;playsinline=1&amp;rel=0';f.allow='autoplay; encrypted-media; picture-in-picture';f.allowFullscreen=true;f.title='His Company Worth Millions After He Was Fired - PART2';f.style.cssText='position:absolute;inset:0;width:100%;height:100%;border:0';this.replaceWith(f)"><img src="https://i.ytimg.com/vi/7aLHquRI5oY/oar2.jpg" onerror="this.onerror=null;this.src='https://i.ytimg.com/vi/7aLHquRI5oY/hqdefault.jpg'" alt="His Company Worth Millions After He Was Fired - PART2" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block;"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);width:64px;height:64px;border-radius:50%;background:rgba(0,0,0,.65);color:#fff;font-size:30px;line-height:64px;text-align:center;">▶</span></button>
</div>
</div>
</div>
<p style="font-size:1.2rem;font-weight:700;margin:0 0 .6rem;"><a href="https://www.youtube.com/watch?v=TooMBNl0lAw" target="_blank" rel="noopener noreferrer">日本が「石油」を超えた！世界を変える新発明の正体</a></p>
<div style="position:relative;width:100%;aspect-ratio:16/9;margin:0 auto 1.2rem;background:#000;border-radius:8px;overflow:hidden;">
<button type="button" aria-label="動画を再生 / Play video" style="all:unset;position:absolute;inset:0;cursor:pointer;" onclick="var f=document.createElement('iframe');f.src='https://www.youtube-nocookie.com/embed/TooMBNl0lAw?autoplay=1&amp;playsinline=1&amp;rel=0';f.allow='autoplay; encrypted-media; picture-in-picture';f.allowFullscreen=true;f.title='日本が「石油」を超えた！世界を変える新発明の正体';f.style.cssText='position:absolute;inset:0;width:100%;height:100%;border:0';this.replaceWith(f)"><img src="https://i.ytimg.com/vi/TooMBNl0lAw/hqdefault.jpg" alt="日本が「石油」を超えた！世界を変える新発明の正体" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block;"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);width:64px;height:64px;border-radius:50%;background:rgba(0,0,0,.65);color:#fff;font-size:30px;line-height:64px;text-align:center;">▶</span></button>
</div>
<p style="font-size:1.2rem;font-weight:700;margin:0 0 .6rem;"><a href="https://www.youtube.com/watch?v=10UgbgnsAcE" target="_blank" rel="noopener noreferrer">日本の新たな発見、コンクリートに取って代わる可能性</a></p>
<div style="position:relative;width:100%;aspect-ratio:16/9;margin:0 auto 1.2rem;background:#000;border-radius:8px;overflow:hidden;">
<button type="button" aria-label="動画を再生 / Play video" style="all:unset;position:absolute;inset:0;cursor:pointer;" onclick="var f=document.createElement('iframe');f.src='https://www.youtube-nocookie.com/embed/10UgbgnsAcE?autoplay=1&amp;playsinline=1&amp;rel=0';f.allow='autoplay; encrypted-media; picture-in-picture';f.allowFullscreen=true;f.title='日本の新たな発見、コンクリートに取って代わる可能性';f.style.cssText='position:absolute;inset:0;width:100%;height:100%;border:0';this.replaceWith(f)"><img src="https://i.ytimg.com/vi/10UgbgnsAcE/hqdefault.jpg" alt="日本の新たな発見、コンクリートに取って代わる可能性" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block;"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);width:64px;height:64px;border-radius:50%;background:rgba(0,0,0,.65);color:#fff;font-size:30px;line-height:64px;text-align:center;">▶</span></button>
</div>
<p style="font-size:1.2rem;font-weight:700;margin:0 0 .6rem;"><a href="https://www.youtube.com/watch?v=noL551LBY7Y" target="_blank" rel="noopener noreferrer">【衝撃】セメントの時代が終了？瓦礫と空気から作る「魔法のコンクリート」が凄すぎる</a></p>
<div style="position:relative;width:100%;aspect-ratio:16/9;margin:0 auto 1.2rem;background:#000;border-radius:8px;overflow:hidden;">
<button type="button" aria-label="動画を再生 / Play video" style="all:unset;position:absolute;inset:0;cursor:pointer;" onclick="var f=document.createElement('iframe');f.src='https://www.youtube-nocookie.com/embed/noL551LBY7Y?autoplay=1&amp;playsinline=1&amp;rel=0';f.allow='autoplay; encrypted-media; picture-in-picture';f.allowFullscreen=true;f.title='【衝撃】セメントの時代が終了？瓦礫と空気から作る「魔法のコンクリート」が凄すぎる';f.style.cssText='position:absolute;inset:0;width:100%;height:100%;border:0';this.replaceWith(f)"><img src="https://i.ytimg.com/vi/noL551LBY7Y/hqdefault.jpg" alt="【衝撃】セメントの時代が終了？瓦礫と空気から作る「魔法のコンクリート」が凄すぎる" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block;"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);width:64px;height:64px;border-radius:50%;background:rgba(0,0,0,.65);color:#fff;font-size:30px;line-height:64px;text-align:center;">▶</span></button>
</div>
<p style="font-size:1.2rem;font-weight:700;margin:0 0 .6rem;"><a href="https://www.youtube.com/watch?v=4jQRmHMk9hU" target="_blank" rel="noopener noreferrer">米国の新発明が銅を完全に置き換える？ 中国が危機感を強める理由</a></p>
<div style="position:relative;width:100%;aspect-ratio:16/9;margin:0 auto 1.2rem;background:#000;border-radius:8px;overflow:hidden;">
<button type="button" aria-label="動画を再生 / Play video" style="all:unset;position:absolute;inset:0;cursor:pointer;" onclick="var f=document.createElement('iframe');f.src='https://www.youtube-nocookie.com/embed/4jQRmHMk9hU?autoplay=1&amp;playsinline=1&amp;rel=0';f.allow='autoplay; encrypted-media; picture-in-picture';f.allowFullscreen=true;f.title='米国の新発明が銅を完全に置き換える？ 中国が危機感を強める理由';f.style.cssText='position:absolute;inset:0;width:100%;height:100%;border:0';this.replaceWith(f)"><img src="https://i.ytimg.com/vi/4jQRmHMk9hU/hqdefault.jpg" alt="米国の新発明が銅を完全に置き換える？ 中国が危機感を強める理由" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block;"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);width:64px;height:64px;border-radius:50%;background:rgba(0,0,0,.65);color:#fff;font-size:30px;line-height:64px;text-align:center;">▶</span></button>
</div>
<p style="font-size:1.2rem;font-weight:700;margin:0 0 .6rem;"><a href="https://www.youtube.com/watch?v=OGRP5KomO2U" target="_blank" rel="noopener noreferrer">【世界初の快挙！！】152年の紙の会社が選んだ...2ナノの道（CNF半導体）</a></p>
<div style="position:relative;width:100%;aspect-ratio:16/9;margin:0 auto 1.2rem;background:#000;border-radius:8px;overflow:hidden;">
<button type="button" aria-label="動画を再生 / Play video" style="all:unset;position:absolute;inset:0;cursor:pointer;" onclick="var f=document.createElement('iframe');f.src='https://www.youtube-nocookie.com/embed/OGRP5KomO2U?autoplay=1&amp;playsinline=1&amp;rel=0';f.allow='autoplay; encrypted-media; picture-in-picture';f.allowFullscreen=true;f.title='【世界初の快挙！！】152年の紙の会社が選んだ...2ナノの道（CNF半導体）';f.style.cssText='position:absolute;inset:0;width:100%;height:100%;border:0';this.replaceWith(f)"><img src="https://i.ytimg.com/vi/OGRP5KomO2U/hqdefault.jpg" alt="【世界初の快挙！！】152年の紙の会社が選んだ...2ナノの道（CNF半導体）" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block;"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);width:64px;height:64px;border-radius:50%;background:rgba(0,0,0,.65);color:#fff;font-size:30px;line-height:64px;text-align:center;">▶</span></button>
</div>
<p style="font-size:1.2rem;font-weight:700;margin:0 0 .6rem;"><a href="https://www.youtube.com/watch?v=xzZuyvt6Jn8" target="_blank" rel="noopener noreferrer">日本製紙が新型蓄電池を開発！2025年の実用化を目指し、木材由来の素材であるセルロースナノファイバー（CNF）に注目が集まる！</a></p>
<div style="position:relative;width:100%;aspect-ratio:16/9;margin:0 auto 1.2rem;background:#000;border-radius:8px;overflow:hidden;">
<button type="button" aria-label="動画を再生 / Play video" style="all:unset;position:absolute;inset:0;cursor:pointer;" onclick="var f=document.createElement('iframe');f.src='https://www.youtube-nocookie.com/embed/xzZuyvt6Jn8?autoplay=1&amp;playsinline=1&amp;rel=0';f.allow='autoplay; encrypted-media; picture-in-picture';f.allowFullscreen=true;f.title='日本製紙が新型蓄電池を開発！2025年の実用化を目指し、木材由来の素材であるセルロースナノファイバー（CNF）に注目が集まる！';f.style.cssText='position:absolute;inset:0;width:100%;height:100%;border:0';this.replaceWith(f)"><img src="https://i.ytimg.com/vi/xzZuyvt6Jn8/hqdefault.jpg" alt="日本製紙が新型蓄電池を開発！2025年の実用化を目指し、木材由来の素材であるセルロースナノファイバー（CNF）に注目が集まる！" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block;"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);width:64px;height:64px;border-radius:50%;background:rgba(0,0,0,.65);color:#fff;font-size:30px;line-height:64px;text-align:center;">▶</span></button>
</div>
<p style="font-size:1.2rem;font-weight:700;margin:0 0 .6rem;"><a href="https://www.youtube.com/watch?v=ovUeKAGipyA" target="_blank" rel="noopener noreferrer">日本の技術が「炭素繊維」を超えた❗世界を変える新発明の正体 CNF（セルロースナノファイバー）とは？</a></p>
<div style="position:relative;width:100%;aspect-ratio:16/9;margin:0 auto 1.2rem;background:#000;border-radius:8px;overflow:hidden;">
<button type="button" aria-label="動画を再生 / Play video" style="all:unset;position:absolute;inset:0;cursor:pointer;" onclick="var f=document.createElement('iframe');f.src='https://www.youtube-nocookie.com/embed/ovUeKAGipyA?autoplay=1&amp;playsinline=1&amp;rel=0';f.allow='autoplay; encrypted-media; picture-in-picture';f.allowFullscreen=true;f.title='日本の技術が「炭素繊維」を超えた❗世界を変える新発明の正体 CNF（セルロースナノファイバー）とは？';f.style.cssText='position:absolute;inset:0;width:100%;height:100%;border:0';this.replaceWith(f)"><img src="https://i.ytimg.com/vi/ovUeKAGipyA/hqdefault.jpg" alt="日本の技術が「炭素繊維」を超えた❗世界を変える新発明の正体 CNF（セルロースナノファイバー）とは？" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block;"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);width:64px;height:64px;border-radius:50%;background:rgba(0,0,0,.65);color:#fff;font-size:30px;line-height:64px;text-align:center;">▶</span></button>
</div>
<p style="font-size:1.2rem;font-weight:700;margin:0 0 .6rem;"><a href="https://www.youtube.com/watch?v=xIyiGPwkyg8" target="_blank" rel="noopener noreferrer">【新電池材料】ボールペンの技術で『カーボンナノチューブ分散液』を開発【三菱鉛筆】Mitsubishi Pencil develops &quot;CNT dispersion liquid&quot;</a></p>
<div style="position:relative;width:100%;aspect-ratio:16/9;margin:0 auto 1.2rem;background:#000;border-radius:8px;overflow:hidden;">
<button type="button" aria-label="動画を再生 / Play video" style="all:unset;position:absolute;inset:0;cursor:pointer;" onclick="var f=document.createElement('iframe');f.src='https://www.youtube-nocookie.com/embed/xIyiGPwkyg8?autoplay=1&amp;playsinline=1&amp;rel=0';f.allow='autoplay; encrypted-media; picture-in-picture';f.allowFullscreen=true;f.title='【新電池材料】ボールペンの技術で『カーボンナノチューブ分散液』を開発【三菱鉛筆】Mitsubishi Pencil develops &quot;CNT dispersion liquid&quot;';f.style.cssText='position:absolute;inset:0;width:100%;height:100%;border:0';this.replaceWith(f)"><img src="https://i.ytimg.com/vi/xIyiGPwkyg8/hqdefault.jpg" alt="【新電池材料】ボールペンの技術で『カーボンナノチューブ分散液』を開発【三菱鉛筆】Mitsubishi Pencil develops &quot;CNT dispersion liquid&quot;" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block;"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);width:64px;height:64px;border-radius:50%;background:rgba(0,0,0,.65);color:#fff;font-size:30px;line-height:64px;text-align:center;">▶</span></button>
</div>
<p style="font-size:1.2rem;font-weight:700;margin:0 0 .6rem;"><a href="https://www.youtube.com/watch?v=gxGx2emqn_w" target="_blank" rel="noopener noreferrer">【ゆっくり解説】中国が握るEV電池市場に異変！日本の黒い素材が世界を変える…ゼオンが単層カーボンナノチューブを数十倍増産、147億円投資でEV200万台分へ</a></p>
<div style="position:relative;width:100%;aspect-ratio:16/9;margin:0 auto 1.2rem;background:#000;border-radius:8px;overflow:hidden;">
<button type="button" aria-label="動画を再生 / Play video" style="all:unset;position:absolute;inset:0;cursor:pointer;" onclick="var f=document.createElement('iframe');f.src='https://www.youtube-nocookie.com/embed/gxGx2emqn_w?autoplay=1&amp;playsinline=1&amp;rel=0';f.allow='autoplay; encrypted-media; picture-in-picture';f.allowFullscreen=true;f.title='【ゆっくり解説】中国が握るEV電池市場に異変！日本の黒い素材が世界を変える…ゼオンが単層カーボンナノチューブを数十倍増産、147億円投資でEV200万台分へ';f.style.cssText='position:absolute;inset:0;width:100%;height:100%;border:0';this.replaceWith(f)"><img src="https://i.ytimg.com/vi/gxGx2emqn_w/hqdefault.jpg" alt="【ゆっくり解説】中国が握るEV電池市場に異変！日本の黒い素材が世界を変える…ゼオンが単層カーボンナノチューブを数十倍増産、147億円投資でEV200万台分へ" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block;"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);width:64px;height:64px;border-radius:50%;background:rgba(0,0,0,.65);color:#fff;font-size:30px;line-height:64px;text-align:center;">▶</span></button>
</div>
"#;

fn render_top_body(query: &std::collections::HashMap<String, String>) -> String {
    // PHP版`REGIONS`配列(`index.php` 1762行目)と同じ並び順・
    // `REGION_LABELS`(1763〜1770行目)と同じ英語見出しを踏襲、日本語は
    // 括弧書きで併記する(2026-07-19、ユーザー指示によりデフォルトを
    // 英語+(日本語)表記に変更)。
    let region_order = ["Asia", "Europe", "Americas", "Middle East", "Africa", "Pacific"];
    let region_label = |r: &str| -> &'static str {
        match r {
            "Asia" => "Asia (アジア)",
            "Europe" => "Europe (ヨーロッパ)",
            "Americas" => "Americas (南北アメリカ)",
            "Middle East" => "Middle East & Central Asia (中東・中央アジア)",
            "Africa" => "Africa (アフリカ)",
            "Pacific" => "Pacific (太平洋)",
            _ => "Other (その他)",
        }
    };

    let q = query.get("q").map(|s| s.to_lowercase()).unwrap_or_default();
    let region_filter = query.get("region").filter(|s| !s.is_empty());

    let matches = |card: &LangCard| -> bool {
        if let Some(rf) = region_filter {
            if card.r != *rf {
                return false;
            }
        }
        if q.is_empty() {
            return true;
        }
        card.n.to_lowercase().contains(&q) || card.t.to_lowercase().contains(&q) || card.a.to_lowercase().contains(&q)
    };

    // PHP版`render()`のロジックに合わせる(`index.php` 1992〜2047行目):
    // 地域フィルタ未指定(`activeRegion==="all"`)の場合はL配列の元の並び順の
    // ままフラットに1セクションで表示し、地域ごとのグループ化は行わない
    // (グループ化は特定の地域ピルを選択した時のみ、`region-title`見出し付きで
    // 表示する)。旧実装は常に6地域へグループ化していたため、デフォルト表示の
    // 先頭が「イラン→日本→…」というPHP版の実際の並び順と異なっていた
    // (2026-07-19、ユーザー指摘により修正)。
    let matched: Vec<&LangCard> = TOP_LANGUAGES.iter().filter(|c| matches(c)).collect();
    let shown = matched.len();
    let sections = if shown == 0 {
        r#"<p class="note">条件に一致する言語カードが見つかりません。</p>"#.to_string()
    } else if let Some(rf) = region_filter {
        let cards: String = matched.iter().map(|c| render_lang_card(c)).collect();
        format!(
            r#"<section class="region-section"><h2 class="region-title">{}</h2><div class="grid">{cards}</div></section>"#,
            region_label(rf)
        )
    } else {
        let cards: String = matched.iter().map(|c| render_lang_card(c)).collect();
        format!(r#"<section class="region-section"><div class="grid">{cards}</div></section>"#)
    };

    // 地域絞り込みピル(PHP版`#pills`相当、JSでのDOM生成をサーバーサイドの
    // クエリパラメータリンクへ置き換え——クリックのみで動作し、JS不要)。
    let mut pills = format!(
        r#"<a class="pill{}" href="/{}">All ({})</a>"#,
        if region_filter.is_none() { " is-active" } else { "" },
        if q.is_empty() { String::new() } else { format!("?q={}", percent_encode(query.get("q").unwrap())) },
        TOP_LANGUAGES.len(),
    );
    for region in region_order {
        let count = TOP_LANGUAGES.iter().filter(|c| c.r == region).count();
        if count == 0 {
            continue;
        }
        let is_active = region_filter.map(|r| r == region).unwrap_or(false);
        let qs = if q.is_empty() {
            format!("?region={}", percent_encode(region))
        } else {
            format!("?region={}&q={}", percent_encode(region), percent_encode(query.get("q").unwrap()))
        };
        pills.push_str(&format!(
            r#"<a class="pill{}" href="/{qs}">{} ({count})</a>"#,
            if is_active { " is-active" } else { "" },
            region_label(region),
        ));
    }

    let q_value = query.get("q").map(|s| html_escape(s)).unwrap_or_default();
    let region_hidden = region_filter
        .map(|r| format!(r#"<input type="hidden" name="region" value="{}">"#, html_escape(r)))
        .unwrap_or_default();

    let wallpapers: String = TOP_WALLPAPERS
        .iter()
        .map(|(url, name, filename)| {
            format!(
                r#"<div class="yt-wp-item"><a href="{url}" target="_blank" rel="noopener noreferrer"><img class="yt-wp-thumb" loading="lazy" src="{url}?caw=200" alt="{name} スマホ壁紙"></a><span class="yt-wp-name">{name}</span><a class="yt-wp-dl" href="{url}" download="{filename}" target="_blank" rel="noopener noreferrer">⬇ 保存</a></div>"#,
                url = html_escape(url),
                name = html_escape(name),
                filename = html_escape(filename),
            )
        })
        .collect();

    // YouTube再生リストのシリーズ機能(PHP版`SEARCH_SERIES`、77件)の復活。
    // 各シリーズの`urls`から再生可能な動画ID(`scraper::extract_yt_id`、
    // `/discover`のサムネイル抽出と同じロジックを再利用)を抽出し、1件も
    // 再生可能な動画が無いシリーズは、そのシリーズが実際に持っている
    // 最初のURL(`/results?search_query=...`・Google検索/画像検索・
    // 各社公式サイト等、`urls[0]`)へそのまま遷移させる(新規タブ)。
    //
    // **2026-07-29修正(実バグ)**: 以前はここで`urls`の中身を一切見ず、
    // `s.btn`(説明文丸ごとのことすらある)から常に新規のYouTube検索
    // クエリを合成していたため、Google画像検索・Google検索・各社公式
    // サイト(SPEC/Pass Labs/Accuphase/LUXMAN等、動画が1本も無い全ての
    // シリーズ)がことごとく「YouTube検索」に化けてしまう実障害があった
    // (ユーザー実機での動作確認で発覚)。`urls`が空の場合のみ、従来通り
    // ラベルからのYouTube検索合成へ後方互換フォールバックする。
    let series_payload: Vec<serde_json::Value> = SEARCH_SERIES
        .iter()
        .map(|s| {
            let ids: Vec<String> = s.urls.iter().filter_map(|u| scraper::extract_yt_id(u)).collect();
            let label = if s.label.is_empty() { s.btn.clone() } else { s.label.clone() };
            let search_url = match s.urls.first() {
                Some(url) => url.clone(),
                None => format!("https://www.youtube.com/results?search_query={}", percent_encode(&s.btn)),
            };
            serde_json::json!({
                "label": label,
                "ids": ids,
                "searchUrl": search_url,
            })
        })
        .collect();
    let series_json = serde_json::to_string(&series_payload).unwrap_or_else(|_| "[]".to_string());
    let series_buttons: String = SEARCH_SERIES
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let label = if s.label.is_empty() { &s.btn } else { &s.label };
            format!(
                r#"<button type="button" class="yt-series-btn{}" data-idx="{i}" onclick="acPlaySeries({i})">{}</button>"#,
                if i == 0 { " is-active" } else { "" },
                html_escape(label),
            )
        })
        .collect();
    // PHP版コメント(`index.php` 195〜197行目・4625〜4633行目)の通り、
    // 起動時はSEARCH_SERIES[0]の先頭URLを再生する(`TOP_DEFAULT_BG_VIDEO_ID`は
    // PHP側でも「再生候補が全て失敗した場合のみ使用」するフォールバック専用
    // 定数であり、実際の初期表示動画ではなかった——2026-07-19訂正)。
    let default_series_label = SEARCH_SERIES
        .first()
        .map(|s| if s.label.is_empty() { s.btn.clone() } else { s.label.clone() })
        .unwrap_or_default();
    let default_id = SEARCH_SERIES
        .first()
        .and_then(|s| s.urls.iter().find_map(|u| scraper::extract_yt_id(u)))
        .unwrap_or_else(|| TOP_DEFAULT_BG_VIDEO_ID.to_string());
    let default_series_label_esc = html_escape(&default_series_label);
    // PHP版`#ytNowPlaying`(`index.php` 1459〜1463行目、「タップ→動画/
    // ページを開く」)相当。「今再生中」表示をクリック可能なリンクにし、
    // 実際のYouTube視聴ページ(`watch?v=`)へ遷移できるようにする
    // (2026-07-19、ユーザー指摘により復活——旧実装は非クリック可能な
    // ただのテキストだった)。
    let default_now_url = html_escape(&format!("https://www.youtube.com/watch?v={default_id}"));
    let blog_title_ja_esc = html_escape(BLOG_POST_TITLE_JA);
    let blog_title_en_esc = html_escape(BLOG_POST_TITLE_EN);
    let blog_title2_ja_esc = html_escape(BLOG_POST2_TITLE_JA);

    let list: String = RANKINGS
        .iter()
        .map(|(slug, _, label)| format!(r#"<li><a href="/ranking/{slug}">{}</a></li>"#, html_escape(label)))
        .collect();
    let composite_list: String = COMPOSITE_PAGES
        .iter()
        .map(|p| format!(r#"<li><a href="/page/{}">{}</a></li>"#, p.slug, html_escape(p.title)))
        .collect();

    format!(
        r##"{TOP_STYLE}
<div class="top-page">
<div class="header">
<span class="logo">audiocafe.tokyo</span>
<p class="subtitle">Please select your native language.</p>
<p class="note" style="font-size:1.2rem">あなたの母国語を選択してください。2回目の選択時はブラウザを閉じて再度開いてください。<br>動画を視聴するには日本語を選択してください。</p>
<p class="lang-select-link" style="font-size:1.2rem"><a href="#lang-grid">🌐 Select your language / 言語を選択する（世界中の言語から選べます） →</a></p>
<p class="note" style="font-size:1.2rem">オーディオ入門用として、100円ショップでも、SONY WALKMANやDAP（デジタルオーディオプレーヤー）、スマートフォン・タブレット・PCと接続する前提で、消費税込み110円から、高くても数千円くらいで、Bluetooth接続の物や、USB Type-C＆3.5mmステレオコネクター接続の物、500円からスマホを充電しながらイヤフォンやヘッドフォンを接続して音楽を聴いたりする為の、有線の3.5mmコネクタなども販売されております。<br><br>As an introduction to audio, there are also stores, such as 100-yen shops, that sell items intended for connecting to Sony Walkman players, DAPs (digital audio players), smartphones, tablets, and PCs, priced from 110 yen including consumption tax up to a few thousand yen at most: Bluetooth models, USB Type-C and 3.5mm stereo connector models, and, from 500 yen, wired 3.5mm connectors that let you connect earphones or headphones and listen to music while charging your smartphone.</p>
<p class="note" style="font-size:1.2rem;text-align:center"><img src="/images/daiso-usbc-35mm-charging-adapter.jpg" alt="DAISO 充電しながら使えるUSB Type-C変換ケーブル（DACチップ搭載・4極ミニジャック対応・500円/税込550円） / DAISO USB Type-C conversion cable with charging port (DAC chip, 4-pole mini jack, 500 yen / 550 yen incl. tax)" style="max-width:100%;height:auto;max-height:560px"></p>
<p class="note" style="font-size:1.2rem">Bluetoothよりも更にWiFiなどの無線では、対応していればWiFiの方が音質が良いです。さらに、有線の方がノイズが乗りにくいので音質が良く、3.5mmや4.4mmやXLR接続のイヤホンやヘッドホンの中には、ベッドの枕に横になっても使える、クリップ式（耳掛け式）の物も御座います。<br><br>Among wireless connections, Wi-Fi sounds better than Bluetooth if the equipment supports it. Furthermore, wired connections are less prone to picking up noise and therefore sound better, and among earphones and headphones with 3.5mm, 4.4mm, or XLR connections, there are also clip-on (ear-hook) types that can be used even while lying on a pillow in bed.</p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://www.soundhouse.co.jp/search/index?s_maker_cd=&amp;s_category_cd=1446&amp;s_mid_category_cd=&amp;s_large_category_cd=&amp;s_product_cd=&amp;search_all=&amp;sSeriesCd=&amp;sPriceFrom=0&amp;sPriceTo=9999999&amp;s_spec=&amp;s_zaiko=&amp;i_type=c&amp;i_sub_type=&amp;i_page=1&amp;i_sort=&amp;i_page_size=32&amp;i_ListType=type2&amp;tag=" target="_blank" rel="noopener noreferrer">📝 SOUNDHOUSE</a></p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://www.amazon.co.jp/dp/B076XZNKV8" target="_blank" rel="noopener noreferrer">📝 【国内正規品】KOSS(コス) KSC75 耳掛け式ヘッドホン(オープン型) 有線 ・ケーブル長/約1.2m・コネクター:L型3.5㎜プラグ・43g</a></p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://www.amazon.co.jp/dp/B0002ERI22" target="_blank" rel="noopener noreferrer">📝 オーディオテクニカ ATH-EW9 ヘッドホン 有線 耳掛け オンイヤー ウッドハウジング ブラウン 小型 | ウッドハウジング 小型軽量 【国内正規品】</a></p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://www.e-earphone.jp/products/668330" target="_blank" rel="noopener noreferrer">📝 aune audio/有線ヘッドホン AC55</a></p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://www.amazon.co.jp/dp/B0FXW5GZJ4" target="_blank" rel="noopener noreferrer">📝 Amazon: MADOO MDO-T930-TI Typ930</a></p>
<p class="note" style="font-size:1.2rem">5年保証を付けたい方は、<a href="https://www.e-earphone.jp/products/647635" target="_blank" rel="noopener noreferrer">e-earphone.jp</a> こちらへどうぞ！<br><br>If you would like a 5-year warranty, please visit <a href="https://www.e-earphone.jp/products/647635" target="_blank" rel="noopener noreferrer">e-earphone.jp</a>!</p>
<p class="note" style="font-size:1.2rem">また、Rakuten Mobile（楽天モバイル）は、日本国内ではRakuten Mobileのアンテナが近くに立っていれば、データ通信が使い放題（パケット放題）となるエリアが広いようで、YouTubeやU-NEXT、音楽アプリやサイトなどを聴いたり視聴したりする方も増えているようでございます。<br><br>In addition, Rakuten Mobile appears to offer unlimited data coverage across a wide area of Japan wherever a Rakuten Mobile antenna is nearby, and it seems that a growing number of people are listening to or watching YouTube, U-NEXT, music apps, and music sites.</p>
<p class="note" style="font-size:1.2rem">DDCのHDMI(I2S)出力からのI2S入力端子付きで、安くて音質が良いとされる2026/09/20現在のUSB-DACの普及モデルとして、次をオススメ致します。<br><br>As a popular USB DAC model that has an I2S input terminal for I2S from a DDC's HDMI (I2S) output and is considered inexpensive with good sound quality as of September 20, 2026, I recommend the following.</p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://www.amazon.co.jp/dp/B0C5X91CSQ" target="_blank" rel="noopener noreferrer">📝 LOXJIE D40 PRO DAコンバーター &amp; ヘッドホンアンプ 一体型 フラッグシップDAC「ES9039MSPRO」搭載/Bluetooth 5.0対応/MQA・ハイレゾ・DSD音源全対応/バランス・アンバランス・高性能ヘッドホンアンプ付き 光 同軸 OTG USB DAC (グレー)</a>　<a href="https://search.kakaku.com/LOXJIE%20D40%20PRO/" target="_blank" rel="noopener noreferrer">［価格.comで見る］</a></p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://www.amazon.co.jp/dp/B0CSP4ZXJR" target="_blank" rel="noopener noreferrer">📝 Shanling L8 シャンリン I2Sデジタルケーブル高純度 無酸素銅線 ノイズレスシールド設計Hi-Fiグレードデジタル伝送 【国内正規品】</a></p>
<p class="note" style="font-size:1.2rem">GUSTARD U18 DDCと、I2S(hdmi)でそのままつなげます。接続は未確認ながらI2S接続なら、MUSICIAN Pisces IIIと言う製品の方が少し高いですが、こちらの方がDDCとしては、音質は良いと思います。念の為ですが、DDC単体では音は出ません。PCやスマホなどとUSBでDDCを接続して、DDCとI2S(hdmi)で、I2S入力端子が付いているUSB-DACを購入したら、そのUSB-DACと接続して、ヘッドフォンなら3.5mmより、4.4mmバランス接続の方が音質が良いとされており、更に、モノラルのXLRを2個使用してのステレオ接続が一番音質が良いとされております。アンプ経由でスピーカーと接続されるのでしたらRCA同軸よりXLR接続の方が音質が良いとされており、更に音質の良いコネクターやケーブルを変更して音質の違いを楽しむ趣味の方々もいらっしゃいます。スピーカーケーブルのコネクターのバナナプラグにスピーカーケーブルを接続するには、2倍の長さを剥いて、2倍の太さにして折りたたんで使用致します。個人的には、バナナプラグの代わりにXLRバランス・コネクターをアンプとスピーカーもバイワイヤリング（バイアンプ）も対応させる改造まで請け負える業者さんがいたら、その接続方法がしっかりロックも出来て、しっかり抜き差しも出来て、やっと安心して良い音で音楽を聴けるのです。</p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://amzn.asia/d/0hxopV5I" target="_blank" rel="noopener noreferrer">📝 S.M.S.L DL400 フルバランス Bluetooth DAC ヘッドホンアンプ プリアンプ 一体型 フラッグシップDAC「ES9039MSPRO」搭載／MQA・ハイレゾ・DSD音源対応／LDAC・aptX・aptX HD・AAC・SBC対応　入力端子：光、デジタル同軸、I2S、OTG USB</a></p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://www.amazon.co.jp/dp/B0CH9Y9CS3" target="_blank" rel="noopener noreferrer">📝 VMV SMSL D2R D/Aコンバーター フラッグシップDAC「BD34301EKV」搭載 オーディオ DAC Bluetooth USB 光 同軸 DAC</a></p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://www.amazon.co.jp/dp/B0H118K2MP" target="_blank" rel="noopener noreferrer">📝 VMV SMSL D2R MK2 D/Aコンバーター ROHM新型フラッグシップDAC「BD34302EKV」搭載 オーディオ DAC Bluetooth USB光 同軸DAC</a></p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://www.amazon.co.jp/dp/B0GT8XSKCC" target="_blank" rel="noopener noreferrer">📝 VMV SMSL D3R D/Aコンバーター ROHM新型フラッグシップDAC「BD34302EKV」x2搭載 オーディオ DAC Bluetooth USB 光 同軸 DAC</a></p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://www.amazon.com/dp/B0C6B5Y39G" target="_blank" rel="noopener noreferrer">📝 LOXJIE D40 Pro, Decoder Headphone Amplifier, ES 9039MS Pro Chip, HiFi, MQA Full Decoding, OPA1612, Support Transmission DSD, XU-316, Bluetooth 5.0, with Remote Control (Gray)</a></p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://store.shanling.com/L8-I2S-to-I2S-Cable-1M-pd153678738.html" target="_blank" rel="noopener noreferrer">📝 Shanling L8-I2S to I2S Cable (1M) — Shanling Official Store</a></p>
<p class="note" style="font-size:1.2rem">The GUSTARD U18 DDC can be connected directly over I2S (HDMI). I have not confirmed this connection myself. For I2S, a product called the MUSICIAN Pisces III is a little more expensive, but I think it is the better DDC in terms of sound quality.<br><br>Just to be clear, a DDC on its own produces no sound. You connect the DDC to a PC, smartphone or similar over USB, buy a USB DAC that has an I2S input, and connect the DDC to that USB DAC over I2S (HDMI).<br><br>For headphones, a 4.4 mm balanced connection is said to sound better than 3.5 mm, and a stereo connection using two mono XLR connectors is said to sound best of all. When connecting to speakers through an amplifier, an XLR connection is said to sound better than RCA coaxial. Some hobbyists also enjoy the differences in sound they get by swapping in higher-quality connectors and cables.<br><br>To attach a speaker cable to a banana plug, strip twice the usual length of insulation, double the strands over to make the conductor twice as thick, and fold it back before inserting it.<br><br>Personally, if there were a company willing to modify the amplifier and the speakers to accept XLR balanced connectors instead of banana plugs, including bi-wiring (bi-amping) support, that connection would lock firmly and plug and unplug securely, and only then could I finally listen to music in good sound with peace of mind.</p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://www.amazon.com/dp/B0DK2THD14" target="_blank" rel="noopener noreferrer">📝 S.M.S.L DL400 fully balanced Bluetooth DAC, headphone amplifier and preamplifier all-in-one, equipped with the flagship DAC "ES9039MSPRO"; supports MQA, Hi-Res and DSD; supports LDAC, aptX, aptX HD, AAC and SBC. Inputs: optical, digital coaxial, I2S, OTG USB</a></p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://www.amazon.com/dp/B0CN6YM1B4" target="_blank" rel="noopener noreferrer">📝 SMSL VMV D2R Audio DAC MQA MQA-CD Decoder BD34301EKV Chip Bluetooth 5.1 DSD512 XU316 PCM Balance USB Decoder with Remote Control</a></p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://www.amazon.com/dp/B0H111VSWD" target="_blank" rel="noopener noreferrer">📝 SMSL VMV D2R MK2 DAC ROHM BD34302EKV Decoding Chip CK-04 Clock | Bluetooth 5.4 LDAC/APTX/HD、SBC、AAC for HiFi System/PC/Home Audio USB/Optical/Coaxial /I2S/ AES Input XLR/RCA Output</a></p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://www.amazon.com/dp/B0GT8PXNVF" target="_blank" rel="noopener noreferrer">📝 SMSL VMV D3R Hi-Res DAC 2x ROHM BD34302EKV Chip Clock Input Bluetooth5.4 MQA CD DAC True Preamp Built-in 3ppb 10MHZ OCXO DSD512 PCM 768kHz USB/Opt/Coax/BT/I²S/AES Input RCA/XLR Output Balanced Decoder</a></p>
<p class="note" style="font-size:1.2rem">熱心なAUDIOファンで音質の良い、MUSICIAN AUDIOの<a href="https://www.amazon.co.jp/Pisces-II-%E3%83%87%E3%82%B8%E3%82%BF%E3%83%AB%E3%82%A4%E3%83%B3%E3%82%BF%E3%83%BC%E3%83%95%E3%82%A7%E3%83%BC%E3%82%B9-USB%E3%82%A4%E3%83%B3%E3%82%BF%E3%83%BC%E3%83%95%E3%82%A7%E3%83%BC%E3%82%B9-%E5%90%8C%E8%BB%B8%E5%87%BA%E5%8A%9BDDC%E3%83%87%E3%82%B8%E3%82%BF%E3%83%AB%E3%82%A2%E3%82%A4%E3%82%BD%E3%83%AC%E3%83%BC%E3%82%B7%E3%83%A7%E3%83%B3%E4%BB%98%E3%81%8D/dp/B0CT26YXR6" target="_blank" rel="noopener noreferrer">PISCES III</a>と言うDDC（GustardのDDCのU18よりロスレスのI2Sなので音質が良いとされております。2026年9月16日現在、Amazon USAではMUSICIAN Pisces II（2）しか販売されておらず、Amazon JapanではMUSICIAN Pisces III（3）を販売中で御座います。）＋USB-DAC次期製品を希望している者と致しまして、<br>As an avid AUDIO fan hoping for a good-sounding next-generation DDC + USB-DAC — specifically MUSICIAN AUDIO's <a href="https://www.amazon.com/Pisces-II-%E3%83%87%E3%82%B8%E3%82%BF%E3%83%AB%E3%82%A4%E3%83%B3%E3%82%BF%E3%83%BC%E3%83%95%E3%82%A7%E3%83%BC%E3%82%B9-USB%E3%82%A4%E3%83%B3%E3%82%BF%E3%83%BC%E3%83%95%E3%82%A7%E3%83%BC%E3%82%B9-%E5%90%8C%E8%BB%B8%E5%87%BA%E5%8A%9BDDC%E3%83%87%E3%82%B8%E3%82%BF%E3%83%AB%E3%82%A2%E3%82%A4%E3%82%BD%E3%83%AC%E3%83%BC%E3%82%B7%E3%83%A7%E3%83%B3%E4%BB%98%E3%81%8D/dp/B0CT26YXR6" target="_blank" rel="noopener noreferrer">PISCES II (２)</a> DDC (said to sound better than Gustard's U18 DDC thanks to its lossless I2S; note: as of September 16, 2026, only the MUSICIAN Pisces II was available on Amazon USA, while the MUSICIAN <a href="https://www.amazon.co.jp/Pisces-II-%E3%83%87%E3%82%B8%E3%82%BF%E3%83%AB%E3%82%A4%E3%83%B3%E3%82%BF%E3%83%BC%E3%83%95%E3%82%A7%E3%83%BC%E3%82%B9-USB%E3%82%A4%E3%83%B3%E3%82%BF%E3%83%BC%E3%83%95%E3%82%A7%E3%83%BC%E3%82%B9-%E5%90%8C%E8%BB%B8%E5%87%BA%E5%8A%9BDDC%E3%83%87%E3%82%B8%E3%82%BF%E3%83%AB%E3%82%A2%E3%82%A4%E3%82%BD%E3%83%AC%E3%83%BC%E3%82%B7%E3%83%A7%E3%83%B3%E4%BB%98%E3%81%8D/dp/B0CT26YXR6" target="_blank" rel="noopener noreferrer">Pisces Ⅲ（３）</a> is being sold on Amazon Japan) — my request is as follows:</p>
<p class="note" style="font-size:1.2rem">日本語:<br>Musician Audio「<a href="https://www.amazon.co.jp/Pegasus-III-DAC%E3%80%81DSD1024-PCM1536kHz-%E5%87%BA%E5%8A%9B%E3%80%81%E3%83%90%E3%83%A9%E3%83%B3%E3%82%B9%E5%9E%8B%E3%83%87%E3%82%B8%E3%82%BF%E3%83%AB-%E3%82%A2%E3%83%8A%E3%83%AD%E3%82%B0/dp/B0GM6D99V1" target="_blank" rel="noopener noreferrer">USB DAC PEGASUSⅢ</a>（３）」シリーズの後継のUSB DAC PEGASUS Ⅳ（4）のアイデアとして、PEGASUSⅢ（３）で採用されている、110Vから240V自動対応と、ロスレスI2S(HDMI)は、そのまま引き続き採用して、後継機種は、R2R方式のUSB-DACではなく、ローム（ROHM）製フラッグシップD/AコンバーターIC「BD34302EKV」を4基（クアッド構成）搭載し、これに旭化成エレクトロニクス（AKM）製のデジタル信号処理・デルタシグマ変調用チップ「AK4191」を組み合わせた、2系統のDAC構成のモデルの発売を希望します。<br><br>あわせて、低ノイズで音質に優れた真空管も搭載し、真空管回路は、スイッチのONとOFFで切り替えが可能な仕様にしていただきたいです。<br><br>また、Musician Audioの一般的なUSB-DACと同様に、110Vから240Vまでの入力電圧自動切り替え機能を、SMSLやGustardといった他社のUSB-DACにも搭載していただきたいです。日本のオーディオメーカーであるAccuphase（アキュフェーズ）、LUXMAN（ラックスマン）、UESUGI（上杉研究所）などのアンプやUSB-DACにも、同じく110Vから240Vへの自動切り替え機能を搭載していただきたいです。<br><br>オーディオ製品は、日本以外への輸出モデルや外国製品、また日本国内向け製品でも、220V～230Vに切り替えられる物の方が音質が良いとされております。できれば、昇圧トランスを使用してでも220V～230Vに切り替えて使用することを、メーカーが保証してくれるとなお良いと思います。<br><br>さらに、東京電力が、200Vへの切り替えだけでなく、220V～230Vへの昇圧トランス機能を搭載し、100Vから110V、200V、220V～230Vまで対応するオーディオ専用分電盤を、新規に設計・開発して、販売やレンタルをしてくれると良いと思います。また、都市ガスでもプロパンガスでも対応できるガス発電システムを、ノイズの少ない電源を好むオーディオファンやマニア向けに、同じく販売やレンタルで提供してくれると良いと思います。これらのアイデアを、提案としてここに書かせていただきます。<br><br>電源は外付けのリニアDC電源とし、搭載するトランスは、手巻きの職人によるオーダーメイド・ハンドメイド品を特別注文で選べるようにしていただきたいです。<br><br>さらに、IEC 60320 C19/C20規格（欧州を含む国際標準の大電流用インレット。定格は16A/250V、北米規格では20A）に対応した、音質に優れた特殊な電源ケーブルにも対応するオプションを用意していただきたいです。このような次期製品をぜひ発売してください。<br><br>English:<br>As the next product in the Musician Audio "<a href="https://www.amazon.com/Pegasus-III-DAC%E3%80%81DSD1024-PCM1536kHz-%E5%87%BA%E5%8A%9B%E3%80%81%E3%83%90%E3%83%A9%E3%83%B3%E3%82%B9%E5%9E%8B%E3%83%87%E3%82%B8%E3%82%BF%E3%83%AB-%E3%82%A2%E3%83%8A%E3%83%AD%E3%82%B0/dp/B0GM6D99V1?th=1" target="_blank" rel="noopener noreferrer">USB DAC PEGASUSⅢ</a> (3)" series, as an idea for the USB DAC PEGASUS IV (4), I would like the new model to carry over PEGASUS III (3)'s automatic 110V–240V input voltage switching and lossless I2S (HDMI) support as-is. Rather than an R2R design, I would like the successor model to feature four (quad configuration) of ROHM's flagship D/A converter ICs, the "BD34302EKV," combined with Asahi Kasei Microdevices' (AKM) "AK4191," a chip dedicated to digital signal processing and delta-sigma modulation, for a dual-DAC configuration.<br><br>I would also like it to include a low-noise, high-fidelity vacuum tube stage, with the tube circuit switchable by an ON/OFF switch.<br><br>Furthermore, just like Musician Audio's standard USB DACs, I would like automatic 110V–240V input voltage switching to be built into USB DACs from other manufacturers such as SMSL and Gustard. I would also like the same 110V–240V automatic switching in the amplifiers and USB DACs of Japanese audio manufacturers such as Accuphase, LUXMAN, and UESUGI.<br><br>It is said that audio products sound better when they can be switched to 220V–230V, whether they are export models for markets outside Japan, foreign-made products, or even products intended for the Japanese domestic market. Ideally, I would also like manufacturers to guarantee the use of such equipment switched to 220V–230V, even when a step-up transformer is used.<br><br>Furthermore, I would like Tokyo Electric Power Company to newly design, develop, and offer for sale or rental a dedicated audio distribution board that goes beyond 200V switching: one with a step-up transformer function for 220V–230V and support for 100V, 110V, 200V, and 220V–230V. I would also like a gas power generation system that runs on either city gas or propane gas, offered in the same way, for audio enthusiasts and hobbyists who prefer low-noise power. I am writing down these ideas here as a proposal.<br><br>For power, I would like an external linear DC power supply. I would also like a special-order option to install custom, handmade transformers wound by hand by skilled artisans.<br><br>In addition, I would like an option that supports special, high-sound-quality power cables conforming to the IEC 60320 C19/C20 standard (the international high-current inlet standard used in Europe and elsewhere, rated 16 A at 250 V, and 20 A under North American ratings). I sincerely hope you will release such a next-generation product.</p>
<p class="note" style="font-size:1.2rem">日本語:<br>LOXJIE D40 PRO のヘッドホンアンプ機能とゲイン調整機能がD3Rには無いです。AIT DAC AK4191/BD34302 (2026ver) に+ヘッドホンアンプ機能とゲイン調整機能とロスレスI2Sと電源は100Vから240V自動対応を追加して欲しい。<br><br>English:<br>The SMSL D3R does not have the headphone amplifier function or the gain adjustment function that the LOXJIE D40 PRO has. I would like the AIT DAC AK4191/BD34302 (2026ver) to additionally have a headphone amplifier function, a gain adjustment function, lossless I2S, and a power supply that automatically supports 100V to 240V.</p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://ameblo.jp/www-aon/entry-12981669650.html" target="_blank" rel="noopener noreferrer">📝 LOXJIE D40 PRO のヘッドホンアンプ機能とゲイン調整機能がD3Rには無いです / The D3R lacks the LOXJIE D40 PRO's headphone amplifier and gain adjustment</a></p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://ameblo.jp/www-aon/entry-12979252794.html" target="_blank" rel="noopener noreferrer">📝 BD34302とAK4191の組み合わせDACについて</a></p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://aonceo.blogspot.com/2026/09/ait-dac-ak4191bd34302-2026ver-20260531.html" target="_blank" rel="noopener noreferrer">📝 AIT DAC AK4191/BD34302 (2026ver)</a></p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://ameblo.jp/www-aon/entry-12979252794.html" target="_blank" rel="noopener noreferrer">📝 On a DAC combining the BD34302 and AK4191</a></p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://aonceo.blogspot.com/2026/09/ait-dac-ak4191bd34302-2026ver-20260531.html" target="_blank" rel="noopener noreferrer">📝 AIT DAC AK4191/BD34302 (2026ver)</a></p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://ameblo.jp/www-aon/entry-12978878261.html" target="_blank" rel="noopener noreferrer">📝 プリアンプやUSB-DAC用の真空管の大量生産依頼先はどこが良いかClaudeに聞いて見ました。</a></p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://ameblo.jp/www-aon/entry-12978908274.html" target="_blank" rel="noopener noreferrer">📝 I asked Claude which manufacturer would be best for mass-producing vacuum tubes for preamps and USB-DACs.</a></p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://www.amazon.co.jp/AOSHIDA-Andromeda-Silver-%E3%82%AF%E3%83%A9%E3%82%B9A%E3%83%90%E3%83%A9%E3%83%B3%E3%82%B9%E3%83%98%E3%83%83%E3%83%89%E3%83%95%E3%82%A9%E3%83%B3%E3%82%A2%E3%83%B3%E3%83%97%E3%83%97%E3%83%AA%E3%82%A2%E3%83%B3%E3%83%97%E3%80%813%E3%83%94%E3%83%B3XLR%E3%81%8B%E3%82%896-35mm-4%E3%83%94%E3%83%B3XLR%E3%83%98%E3%83%83%E3%83%89%E3%83%95%E3%82%A9%E3%83%B3%E3%82%A2%E3%83%B3%E3%83%97%E3%80%813800mW%C3%973800mW%E3%83%98%E3%83%83%E3%83%89%E3%83%95%E3%82%A9%E3%83%B3%E3%82%A2%E3%83%B3%E3%83%97%E3%80%813%E3%83%94%E3%83%B3XLR%E3%83%97%E3%83%AA%E3%82%A2%E3%82%A6%E3%83%88%E3%83%97%E3%83%83%E3%83%88%E3%83%97%E3%83%AA%E3%82%A2%E3%83%B3%E3%83%97%EF%BC%88%E9%8A%80%EF%BC%89/dp/B08Y5Q88RB?th=1" target="_blank" rel="noopener noreferrer">📝 MUSICIAN Andromeda クラスAバランスヘッドフォンアンププリアンプ、3ピンXLRから6.35mm/4ピンXLRヘッドフォンアンプ、3800mW×3800mWヘッドフォンアンプ、3ピンXLRプリアウトプットプリアンプ（銀）もしくは（黒）</a></p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://www.amazon.com/AOSHIDA-Andromeda-Silver-%E3%82%AF%E3%83%A9%E3%82%B9A%E3%83%90%E3%83%A9%E3%83%B3%E3%82%B9%E3%83%98%E3%83%83%E3%83%89%E3%83%95%E3%82%A9%E3%83%B3%E3%82%A2%E3%83%B3%E3%83%97%E3%83%97%E3%83%AA%E3%82%A2%E3%83%B3%E3%83%97%E3%80%813%E3%83%94%E3%83%B3XLR%E3%81%8B%E3%82%896-35mm-4%E3%83%94%E3%83%B3XLR%E3%83%98%E3%83%83%E3%83%89%E3%83%95%E3%82%A9%E3%83%B3%E3%82%A2%E3%83%B3%E3%83%97%E3%80%813800mW%C3%973800mW%E3%83%98%E3%83%83%E3%83%89%E3%83%95%E3%82%A9%E3%83%B3%E3%82%A2%E3%83%B3%E3%83%97%E3%80%813%E3%83%94%E3%83%B3XLR%E3%83%97%E3%83%AA%E3%82%A2%E3%82%A6%E3%83%88%E3%83%97%E3%83%83%E3%83%88%E3%83%97%E3%83%AA%E3%82%A2%E3%83%B3%E3%83%97%EF%BC%88%E9%8A%80%EF%BC%89/dp/B08Y5Q88RB?th=1" target="_blank" rel="noopener noreferrer">📝 AOSHIDA Musician Andromeda Class A Balanced Headphone Amp 3800mW×3800mW Preamp 3-Pin XLR to 6.35mm/4-Pin XLR Headphone Amps 3-Pin XLR Preoutput Preamplifiers (Silver)</a></p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://kakaku.com/item/K0001658256/" target="_blank" rel="noopener noreferrer">📝 日本のLUXMAN の高級ヘッドフォンアンプ　P-100 CENTENNIAL</a></p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://kakaku.com/item/K0001658256/" target="_blank" rel="noopener noreferrer">📝 Japanese high-end headphone amplifier, LUXMAN P-100 CENTENNIAL.</a></p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://av.watch.impress.co.jp/docs/topic/special/1651296.html" target="_blank" rel="noopener noreferrer">📝 ラックスマン史上最強ヘッドフォンアンプ「P-100 CENTENNIAL」。驚愕の2台駆動も聴く</a></p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://av.watch.impress.co.jp/docs/topic/special/1651296.html" target="_blank" rel="noopener noreferrer">📝 Luxman's strongest headphone amplifier ever, the "P-100 CENTENNIAL": Listening to its astonishing dual-unit drive setup.</a></p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://av.watch.impress.co.jp/docs/news/1626602.html" target="_blank" rel="noopener noreferrer">📝 ラックスマン100周年記念、“あらゆるヘッドフォンを縦横無尽にドライブするアンプ”「P-100 CENTENNIAL」99万円</a></p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://av.watch.impress.co.jp/docs/news/1626602.html" target="_blank" rel="noopener noreferrer">📝 Commemorating Luxman's 100th anniversary, the P-100 CENTENNIAL is an amplifier designed to effortlessly drive any headphone, priced at 990,000 yen.</a></p>
<p class="note" style="font-size:1.2rem">■ご予約受付■　FOCAL(フォーカル) / UTOPIA NP 開放型ヘッドホン （※代引き不可） 1大特典セット</p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://www.mh-friends.com/form.shtml" target="_blank" rel="noopener noreferrer">📝 問い合わせ</a></p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://www.mh-friends.com/shop/index.cgi?No=1059899" target="_blank" rel="noopener noreferrer">📝 商品の通信販売サイト</a></p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://www.fujiya-avic.co.jp/shop/g/g240004022564/" target="_blank" rel="noopener noreferrer">📝 高級ヘッドフォン　UTOPIA　NP　中古本体の初期型＋XLR-4Pケーブルとキャンリングケース</a></p>
<p class="note" style="font-size:1.2rem">FOCAL UTOPIAの初期型は約50万円で、一番壊れやすいですが、解像度が高く音楽的に生々しい音質が最高なので、壊れても修理に出して、治してまた聴くを繰り返すヘッドフォンマニアのオタクさんがいっぱい、いらっしゃるそうで御座います。<br><br>Even though the first-generation Focal Utopia costs around 500,000 yen and is notoriously prone to breaking, many die-hard headphone audiophiles swear by it. Its high resolution and remarkably lifelike, musical sound quality are so captivating that they don't mind repeatedly repairing it just to keep enjoying its sound.</p>
<p class="note" style="font-size:1.2rem">一般的に<a href="https://www.accuphase.co.jp/" target="_blank" rel="noopener noreferrer">Accuphaseのアンプ</a>の音は、一般的に解像度が高く、音質が良く、高級ヘッドフォンのFOCAL UTOPIAの初期型に一番近い音質のアンプと言われる場合が多い様です。ただし解像度が高いので、人によっては飽きやすい場合もある様なので、音質の好みは、人によりけりな様で御座います。そう言う意味で、聴いていて飽きにくい音質と言う意味では、<a href="https://www.youtube.com/results?search_query=uesgi+%E4%B8%8A%E6%9D%89+%E3%82%A2%E3%83%B3%E3%83%97" target="_blank" rel="noopener noreferrer">UESUGI　上杉　のアンプ</a>や<a href="https://www.youtube.com/results?search_query=LUXMAN+%E3%82%A2%E3%83%B3%E3%83%97" target="_blank" rel="noopener noreferrer">LUXMANのアンプ</a>や<a href="https://www.youtube.com/results?search_query=Macintosh+%E3%82%A2%E3%83%B3%E3%83%97" target="_blank" rel="noopener noreferrer">Macintosh アンプ</a>や<a href="https://www.youtube.com/results?search_query=Pass+labs%E3%80%80%E3%82%A2%E3%83%B3%E3%83%97" target="_blank" rel="noopener noreferrer">Pass labs　アンプ</a>や<a href="https://www.youtube.com/results?search_query=SPEC%E3%80%80%E3%82%A2%E3%83%B3%E3%83%97" target="_blank" rel="noopener noreferrer">SPEC　AMP(アンプ）</a>などの方が好みの方もいらっしゃる様で人によって好みは分かれる様で御座います。<br><br>Generally speaking, the sound of <a href="https://www.accuphase.co.jp/" target="_blank" rel="noopener noreferrer">Accuphase amplifiers</a> is often described as having high resolution and excellent sound quality, said to be the closest match to the sound of the high-end FOCAL UTOPIA (first generation). That said, because of its high resolution, some listeners find it can grow tiresome over time, so sound preference really comes down to the individual. In that sense — when it comes to sound that doesn't tire the ear even after long listening — some people instead prefer <a href="https://www.youtube.com/results?search_query=uesgi+%E4%B8%8A%E6%9D%89+%E3%82%A2%E3%83%B3%E3%83%97" target="_blank" rel="noopener noreferrer">UESUGI amplifiers</a>, <a href="https://www.youtube.com/results?search_query=LUXMAN+%E3%82%A2%E3%83%B3%E3%83%97" target="_blank" rel="noopener noreferrer">LUXMAN amplifiers</a>, <a href="https://www.youtube.com/results?search_query=Macintosh+%E3%82%A2%E3%83%B3%E3%83%97" target="_blank" rel="noopener noreferrer">Macintosh amplifiers</a>, or <a href="https://www.youtube.com/results?search_query=Pass+labs%E3%80%80%E3%82%A2%E3%83%B3%E3%83%97" target="_blank" rel="noopener noreferrer">Pass Labs amplifiers</a>, or <a href="https://www.youtube.com/results?search_query=SPEC%E3%80%80%E3%82%A2%E3%83%B3%E3%83%97" target="_blank" rel="noopener noreferrer">SPEC amplifiers</a> — it really seems that preferences differ from person to person.</p>
<p class="note" style="font-size:1.2rem">秋葉原イヤフォン＆ヘッドフォン専門店<br>AKIHABARA　Earphone &amp; HEADPHONE</p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://www.e-earphone.jp/collections/5?filter.p.m.custom.is_on_sale=%E8%B2%A9%E5%A3%B2%E4%B8%AD&sort_by=price-descending" target="_blank" rel="noopener noreferrer">📝 高級ヘッドフォン / HIEND HEADPHONE</a></p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://www.e-earphone.jp/collections/3?filter.p.m.custom.is_on_sale=%E8%B2%A9%E5%A3%B2%E4%B8%AD&sort_by=price-descending" target="_blank" rel="noopener noreferrer">📝 高級有線イヤフォン / HIEND CABLE EARPHONE</a></p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://www.e-earphone.jp/collections/1?filter.p.m.custom.is_on_sale=%E8%B2%A9%E5%A3%B2%E4%B8%AD&sort_by=price-descending" target="_blank" rel="noopener noreferrer">📝 高級・完全ワイヤレス・イヤフォン / HIEND true wireless earbuds</a></p>
<p class="note" style="font-size:1.2rem">最近の無線接続のEarphones(イヤフォン)やHeadphones(ヘッドフォン)は、BluetoothよりもWifiの方が音質が良く、3.5mmより4.4mm Balanced(バランス)接続の方が更に、Headphone Amplifier(ヘッドフォンアンプ)が対応していれば、XLRのSTEREO(ステレオ)より、XLRのMONONAURAL(モノラル)　×　2(DUAL) で、LUXMANのP-100 CENTENNIAL　×　2（DUAL) に接続した方が音質が良いとされております。<br><br>For today's wireless Earphones and Headphones, Wi-Fi offers better sound quality than Bluetooth, and a 4.4mm Balanced connection is said to sound better than 3.5mm — and further, if the Headphone Amplifier supports it, connecting via two (DUAL) XLR MONAURAL cables to two (DUAL) LUXMAN P-100 CENTENNIAL units is said to sound even better than an XLR STEREO connection.</p>
<p class="note" style="font-size:1.2rem">寝室などで枕に寝転びながら音楽を聴く時に耳が痛くなりにくい耳掛け式ヘッドホンとしては最適。<br><br>The ideal pain-free clip-on headphones for listening to music in bed.</p>
<p class="note" style="font-size:1.2rem">コストパフォーマンスが良い商品その１。<br><br>Cost-effective clip-on headphones　PART1.</p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://www.amazon.co.jp/KSC75-%E8%80%B3%E6%8E%9B%E3%81%91%E5%BC%8F%E3%83%98%E3%83%83%E3%83%89%E3%83%9B%E3%83%B3-%E3%82%AA%E3%83%BC%E3%83%97%E3%83%B3%E5%9E%8B-%E3%83%BB%E3%82%B1%E3%83%BC%E3%83%96%E3%83%AB%E9%95%B7-%E7%B4%841-2m%E3%83%BB%E3%82%B3%E3%83%8D%E3%82%AF%E3%82%BF%E3%83%BC/dp/B076XZNKV8" target="_blank" rel="noopener noreferrer">📝 Amazon Japan「【国内正規品】KOSS(コス) KSC75 耳掛け式ヘッドホン(オープン型) 有線 ・ケーブル長/約1.2m・コネクター:L型3.5㎜プラグ・43g」</a></p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://www.amazon.com/Koss-KSC75-Portable-Stereophone-Headphones/dp/B0006B486K?th=1" target="_blank" rel="noopener noreferrer">📝 Amazon USA "KOSS KSC75"</a></p>
<p class="note" style="font-size:1.2rem">コストパフォーマンスが良い商品その２。<br><br>Cost-effective clip-on headphones　PART２.</p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://www.amazon.co.jp/audio-technica-W-%E5%AF%86%E9%96%89%E5%9E%8B%E3%82%AA%E3%83%B3%E3%82%A4%E3%83%A4%E3%83%BC%E3%83%98%E3%83%83%E3%83%89%E3%83%9B%E3%83%B3-%E8%80%B3%E6%8E%9B%E3%81%91%E5%BC%8F-ATH-EW9/dp/B0002ERI22" target="_blank" rel="noopener noreferrer">📝 Amazon Japan「オーディオテクニカ ATH-EW9 ヘッドホン 有線 耳掛け オンイヤー ウッドハウジング ブラウン 小型」</a></p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://www.amazon.com/audio-technica-W-%E5%AF%86%E9%96%89%E5%9E%8B%E3%82%AA%E3%83%B3%E3%82%A4%E3%83%A4%E3%83%BC%E3%83%98%E3%83%83%E3%83%89%E3%83%9B%E3%83%B3-%E8%80%B3%E6%8E%9B%E3%81%91%E5%BC%8F-ATH-EW9/dp/B0002ERI22" target="_blank" rel="noopener noreferrer">📝 Amazon USA "audio-technica ATH-EW9"</a></p>
<p class="note" style="font-size:1.2rem;text-align:center"><img src="/images/fyne-audio-supertrax.jpg" alt="FYNE AUDIO “SuperTrax”(スーパートラックス)スーパートゥイーター" style="max-width:100%;height:auto;max-height:560px"></p>
<p class="note" style="font-size:1.2rem">FYNE AUDIO “SuperTrax”(スーパートラックス)スーパートゥイーター<br><br>色は次の二種類。<br><br>WALNUT (WN)<br>＆<br>PIANO GROSS BLACK (PGB)<br><br><a href="http://www.axiss.co.jp/brand/fyne-audio/super-tweeter-series/supertrax/" target="_blank" rel="noopener noreferrer">http://www.axiss.co.jp/brand/fyne-audio/super-tweeter-series/supertrax/</a></p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://www.amazon.co.jp/dp/B0H2BJYGRQ" target="_blank" rel="noopener noreferrer">📝 SONY SUBWOOFER SA-SW9 BRAVIA Theatre Sub 9 600W</a></p>
<p class="note" style="font-size:1.2rem;text-align:center"><a href="https://www.amazon.co.jp/dp/B0H2BJYGRQ" target="_blank" rel="noopener noreferrer"><img src="/images/sony-sa-sw9-bravia-theatre-sub9-centered2.jpg" alt="SONY SUBWOOFER SA-SW9 BRAVIA Theatre Sub 9 600W" style="max-width:100%;height:auto;max-height:560px"></a></p>
<p class="note" style="font-size:1.2rem">プリメインアンプのみならず、USB-DAC付きヘッドフォンアンプなどにもサブウーファー端子を付けて、ヘッドフォンやイヤフォンと同時にサブウーファーで低音を補いながら音楽を聴いたりビデオなどをご視聴になりたい方もいらっしゃるのではないでしょうか？出来ればサブウーファー端子はRCAとXLR（もしくは4.4mmバランス）の両方をご用意頂きたい。<br><br>As a comment: not only integrated amplifiers, but also headphone amplifiers with a built-in USB DAC could be given a subwoofer output. There may well be people who would like to supplement the bass with a subwoofer while listening to music or watching videos through headphones or earphones at the same time. Ideally, I would like both RCA and XLR (or 4.4mm balanced) subwoofer outputs to be provided.</p>
<p class="note" style="font-size:1.2rem;text-align:center"><a href="https://www.google.com/search?q=fyne%20audio%20f500s%20%E3%83%AC%E3%83%93%E3%83%A5%E3%83%BC" target="_blank" rel="noopener noreferrer"><img src="/images/fyne-audio-f500s-pgb.jpg" alt="FYNE AUDIO F500S-PGB(ペア) PIANO GROSS BLACK ファイン オーディオ 2way ブックシェルフ スピーカー" style="max-width:100%;height:auto;max-height:560px"></a></p>
<p class="note" style="font-size:1.2rem"><a href="https://www.google.com/search?q=fyne%20audio%20f500s%20%E3%83%AC%E3%83%93%E3%83%A5%E3%83%BC" target="_blank" rel="noopener noreferrer">FYNE AUDIO F500S-PGB(ペア) PIANO GROSS BLACK ファイン オーディオ 2way ブックシェルフ スピーカー</a></p>
<p class="note" style="font-size:1.2rem;text-align:center"><img src="/images/monitor-audio-bronze2-square.jpg" alt="MONITOR AUDIO Bronze2" style="max-width:100%;height:auto;max-height:560px"></p>
<p class="note" style="font-size:1.2rem">MONITOR AUDIO Bronze2には、ウーファーがフラットに近い物と真ん中が突き出ている物の2種類ある様で御座います。<br><br>現状はほとんど中古のみで御座います。<br>ご参考まで、<a href="https://auctions.yahoo.co.jp/closedsearch/closedsearch?p=monitor+audio+bronze2&amp;fixed=0&amp;auccat=23764&amp;b=1&amp;n=50&amp;select=6&amp;mode=3" target="_blank" rel="noopener noreferrer">ヤフオク</a><br><br>It seems that the MONITOR AUDIO Bronze2 comes in two types: one with a nearly flat woofer cone and one with a protruding center.<br><br>At present, almost only used units are available.<br>For reference, here is <a href="https://auctions.yahoo.co.jp/closedsearch/closedsearch?p=monitor+audio+bronze2&amp;fixed=0&amp;auccat=23764&amp;b=1&amp;n=50&amp;select=6&amp;mode=3" target="_blank" rel="noopener noreferrer">Yahoo! Auctions (Japan)</a>.</p>
<p class="note" style="font-size:1.2rem"><strong>DENON PMA2000RE</strong><br>DENONのサブウーファー端子付きプリメインアンプの中では音質が良い方で御座います。<br>高級品でもサブウーファー端子の付いていない物も沢山御座いますので御注意<br><br>Among DENON's integrated amplifiers with a subwoofer output, it is one of the better-sounding ones.<br>Please note that there are also many high-end models that do not have a subwoofer output.</p>
<p class="note" style="font-size:1.2rem">DENON PMA2000RE　<a href="https://auctions.yahoo.co.jp/closedsearch/closedsearch?p=DENON+PMA2000RE&amp;auccat=23764&amp;b=1&amp;n=50&amp;select=6&amp;mode=3&amp;dest_pref_code=13" target="_blank" rel="noopener noreferrer">ヤフオクはこちら</a><br><br>DENON PMA2000RE　<a href="https://auctions.yahoo.co.jp/search/search?p=denon+pma2000re&amp;fixed=0&amp;b=1&amp;is_postage_mode=1&amp;n=50&amp;select=33&amp;mode=3" target="_blank" rel="noopener noreferrer">Yahoo! Auctions (Japan) is here</a></p>
<p class="note" style="font-size:1.2rem">The EXT.PRE on the PMA-2500NE is not for a subwoofer. According to what I found in searches, there appears to be no dedicated pre-out to which a subwoofer can be connected. The owner's manual (rear panel and specifications) also lists no pre-out. (Sources: <a href="https://manuals.denon.com/PMA2500NE/EU/EN/OBAOSYegrydhhu.php" target="_blank" rel="noopener noreferrer">Rear panel</a> / <a href="https://manuals.denon.com/PMA2500NE/EU/EN/OBAOSYxljoeltq.php" target="_blank" rel="noopener noreferrer">Specifications</a> / <a href="https://www.stereonet.com/forums/topic/349943-denon-pma-2500ne-pma-a110/" target="_blank" rel="noopener noreferrer">StereoNET</a>)<br><br>EXT.PRE is an input terminal. It is a terminal for feeding in the signal of an external preamplifier so that the unit can be used as a power amplifier; it is not a terminal that outputs a signal. A subwoofer cannot be connected here. The Japanese and English Denon owner's manuals state that the volume in this case is adjusted on the connected device (<a href="https://manuals.denon.com/PMA2500NE/EU/EN/OBAOSYegrydhhu.php" target="_blank" rel="noopener noreferrer">owner's manual: rear panel</a> / <a href="https://manuals.denon.com/PMA2500NE/EU/EN/OBAOSYqogowrrs.php" target="_blank" rel="noopener noreferrer">connecting a pre-amplifier</a>).<br><br>The output appears to stop at REC OUT. On a forum, someone wrote that it is a fixed-volume REC output and not suited to other equipment (<a href="https://www.stereonet.com/forums/topic/112089-denon-pma-2500ne/" target="_blank" rel="noopener noreferrer">StereoNET</a>).<br><br>Difference from the PMA-2000RE: the PRE OUT of the PMA-2000RE is a terminal whose output follows the volume, and it can also be used for a subwoofer. This point is confirmed by the PHILE WEB article, which mentions a pre-out terminal convenient for connecting a subwoofer. (<a href="https://www.phileweb.com/news/audio/201209/19/12496.html" target="_blank" rel="noopener noreferrer">PHILE WEB (Japanese)</a>)<br><br>PMA-2500NE の EXT.PRE はサブウーファー用ではありません。 検索で見つかった情報では、サブウーファーを接続できる専用のプリアウトは無いようです。取扱説明書(リアパネル・主な仕様)にも、プリアウトの記載はありません。（出典: <a href="https://manuals.denon.com/PMA2500NE/JP/JA/OBAOSYegrydhhu.php" target="_blank" rel="noopener noreferrer">リアパネル</a> / <a href="https://manuals.denon.com/PMA2500NE/JP/JA/OBAOSYxljoeltq.php" target="_blank" rel="noopener noreferrer">主な仕様</a> / <a href="https://www.stereonet.com/forums/topic/349943-denon-pma-2500ne-pma-a110/" target="_blank" rel="noopener noreferrer">StereoNET(英語)</a>）<br><br>EXT.PRE は入力端子です。 外付けプリアンプの信号を入れ、本機をパワーアンプとして使うための端子で、信号を出す端子ではありません。ここにサブウーファーはつなげません。Denon の日本語版・英語版の取扱説明書では、このときの音量は接続した機器側で調整する、とされています(<a href="https://manuals.denon.com/PMA2500NE/JP/JA/OBAOSYegrydhhu.php" target="_blank" rel="noopener noreferrer">取扱説明書:リアパネル</a> / <a href="https://manuals.denon.com/PMA2500NE/JP/JA/OBAOSYqogowrrs.php" target="_blank" rel="noopener noreferrer">プリアンプを接続する</a>)。<br><br>出力は REC OUT 止まりのようです。 フォーラムでは、音量固定のREC出力で、他の機器には向かないという書き込みがありました(<a href="https://www.stereonet.com/forums/topic/112089-denon-pma-2500ne/" target="_blank" rel="noopener noreferrer">StereoNET(英語)</a>)。<br><br>PMA-2000RE との違い: PMA-2000RE の PRE OUT は、音量に連動する出力としてサブウーファーにも使える端子です。この点は、PHILE WEBの記事に「サブウーファーとの接続による便利なプリアウト端子」と記載があり、裏付けが取れています。（参考: <a href="https://www.phileweb.com/news/audio/201209/19/12496.html" target="_blank" rel="noopener noreferrer">PHILE WEB</a>）</p>
<p class="note" style="font-size:1.2rem"><strong>Title</strong><br>With a fully balanced design, going mono makes it easy to add a subwoofer output.<br><br>For a fully balanced integrated amplifier, wouldn't it be easy to design, manufacture and sell a mono subwoofer output on both XLR and RCA, as part of the pre-out? Please also describe the benefits of making the subwoofer output mono. I put this question to a paid AI called Claude.<br><br>Yes, it is technically quite easy. The extra parts are only one or two op-amps, some resistors and XLR/RCA jacks, so the cost is small. However, there are a few points to get right if you want a clean result.<br><br><strong>Design points</strong><br>・Mixing position: If you sum L+R after the volume control, the subwoofer level follows the main volume.<br>・Balanced summing: Sum the Hot signals together and the Cold signals together, each through its own resistors, and receive them with a buffer. This keeps the circuit fully balanced and gives a mono XLR output.<br>・RCA output: Feed it through a differential-to-single-ended conversion circuit. Simply taking only the Hot signal is easy, but it is worse for noise.<br>・Channel isolation: Use summing resistors of roughly a few kΩ to 10 kΩ. This prevents the main L/R outputs from interfering with each other and prevents crosstalk from getting worse.<br>・Buffer insertion: Make sure the subwoofer's input impedance does not load the main pre-outs.<br>・Gain adjustment: Highly correlated bass rises by about +6 dB when summed, so compensate the gain with the resistor values.<br>・Options: A Stereo/Mono switch, a simple low-pass filter (about 80–120 Hz) and a phase-invert switch would make the product more attractive.<br><br><strong>Benefits of a mono subwoofer output</strong><br>・Little effect on imaging: Bass below about 100 Hz has almost no sense of direction, so mono causes virtually no disadvantage in listening.<br>・Simple connection: One cable is enough. It connects directly to subwoofers that have only a mono input, and no Y-splitter cable is needed.<br>・Good match with the source: In mixing, bass is generally placed in the center, and vinyl records also have their low frequencies made mono because of cutting limitations.<br>・Better S/N ratio: Summing L+R raises the signal level, which is advantageous against noise on the subwoofer side.<br>・Easy multi-unit use: When two or more subwoofers are placed at different positions in the room to even out room modes (standing waves), sending the same mono signal to all of them is the basic approach.<br>・Compatibility with balanced transmission: The cable run to the subwoofer tends to be long, and XLR is resistant to hum and ground loops, so it shows its real value here.<br><br><strong>Cautions (drawbacks)</strong><br>・Out-of-phase bass (some live recordings, synthesizer sources, etc.) cancels when summed and sounds thin.<br>・For people who want stereo low-frequency imaging (spatial spread) with two subwoofers, it is safest to include a switch.<br>・On the manufacturing and sales side, the circuit is small and can be handled by adding a board or mounting it on an existing board. For a mid-price or higher fully balanced amplifier, it is a strong enough point of differentiation.</p>
<p class="note" style="font-size:1.2rem"><strong>タイトル</strong><br>完全バランス設計なら、モノラル化すれば、サブウーファー端子が簡単に付けられます。<br><br>完全バランス設計のプリメインアンプは、プリアウトのXLRとRCAのサブウーファー端子は、モノラルなら、簡単に設計、製造、販売出来るのではありませんか？サブウーファー端子をモノラル化するメリットも記載して。CLAUDEと言う、有料のAIに質問して見ました。<br><br>はい、技術的にはかなり簡単です。追加部品はオペアンプ1〜2個と抵抗、XLR/RCA端子程度で、コストも小さく済みます。ただし、きれいに作るにはいくつか押さえるべき点があります。<br><br><strong>設計のポイント</strong><br>・ミックス位置：ボリューム回路の後で L+R を加算すると、サブウーファーの音量がメイン音量に連動します。<br>・バランス加算：Hot同士、Cold同士を別々に抵抗で加算し、バッファで受けます。これで完全バランス構成のまま、モノラルのXLR出力が得られます。<br>・RCA出力：差動からシングルエンドへの変換回路を通して出します。Hotだけを取り出す方式は簡単ですが、ノイズ面では不利です。<br>・チャンネル間の分離：加算抵抗は数kΩ〜10kΩ程度にします。メインのL/R出力同士が干渉したり、クロストークが悪化したりするのを防ぐためです。<br>・バッファの挿入：サブウーファー側の入力インピーダンスが、メインのプリアウトに負荷をかけないようにします。<br>・ゲイン調整：相関の強い低音は加算すると約+6dBになるため、抵抗値でゲインを補正します。<br>・オプション：Stereo/Monoの切替スイッチ、簡易ローパスフィルター（約80〜120Hz）、位相反転スイッチがあると商品力が上がります。<br><br><strong>サブウーファー出力をモノラル化するメリット</strong><br>・定位への影響が小さい：約100Hz以下の低音は方向感がほとんどないため、モノラルでも聴感上の不利はほぼありません。<br>・接続が簡単：ケーブル1本で済みます。モノラル入力しかないサブウーファーにもそのまま繋がり、Y分岐ケーブルも不要です。<br>・ソースとの相性が良い：ミックスでは低音を中央に定位させるのが一般的で、アナログ盤もカッティングの都合で低域をモノラル化しています。<br>・S/N比の向上：L+Rを加算すると信号レベルが上がるため、サブウーファー側のノイズに対して有利になります。<br>・複数台の運用がしやすい：2台以上を部屋の別々の位置に置いてルームモード（定在波）を均す使い方では、全台に同じモノラル信号を送るのが基本です。<br>・バランス伝送との相性：サブウーファーまでの配線は長くなりがちですが、XLRならハムやグランドループに強く、ここで真価を発揮します。<br><br><strong>注意点（デメリット）</strong><br>・逆相成分の低音（一部のライブ録音やシンセ音源など）は、加算すると打ち消し合って痩せます。<br>・2台のサブウーファーで低域のステレオ感（空間の広がり）まで狙う層向けに、切替スイッチを付けておくのが無難です。<br>・製造・販売面では、回路は小規模で、基板の追加か既存基板への実装で対応できます。中価格帯以上の完全バランスアンプなら差別化要素として十分に成立します。</p>
<p class="note" style="font-size:1.2rem"><strong>サブウーファー専用端子の回路図 (VER1〜VER7) / Subwoofer terminal schematics</strong><br><a href="https://github.com/aon-co-jp/open-network-amp/tree/main/VER1" target="_blank" rel="noopener noreferrer">VER1</a> · <a href="https://github.com/aon-co-jp/open-network-amp/tree/main/VER2" target="_blank" rel="noopener noreferrer">VER2</a> · <a href="https://github.com/aon-co-jp/open-network-amp/tree/main/VER3" target="_blank" rel="noopener noreferrer">VER3</a> · <a href="https://github.com/aon-co-jp/open-network-amp/tree/main/VER4" target="_blank" rel="noopener noreferrer">VER4</a> · <a href="https://github.com/aon-co-jp/open-network-amp/tree/main/VER5" target="_blank" rel="noopener noreferrer">VER5</a> · <a href="https://github.com/aon-co-jp/open-network-amp/tree/main/VER6" target="_blank" rel="noopener noreferrer">VER6</a> · <a href="https://github.com/aon-co-jp/open-network-amp/tree/main/VER7" target="_blank" rel="noopener noreferrer">VER7</a></p>
<p class="note" style="font-size:1.2rem;text-align:center"><img src="/images/denon-pma2000re-rear.jpg" alt="DENON PMA2000RE 背面 / rear panel" style="max-width:100%;height:auto;max-height:560px"></p>
<p class="note" style="font-size:1.2rem;text-align:center"><img src="/images/denon-pma2000re-front.jpg" alt="DENON PMA2000RE 正面 / front" style="max-width:100%;height:auto;max-height:560px"></p>
<p class="note" style="font-size:1.2rem">サブウーファーはSUB WOOFERかPRE OUT端子につなげます。<br><br>A subwoofer is connected to the SUB WOOFER or PRE OUT terminal.</p>
<p class="note" style="font-size:1.2rem"><a href="https://kakaku.com/item/K0000418759/" target="_blank" rel="noopener noreferrer">kakaku.com レビュー</a><br><a href="https://kakaku.com/item/K0000418759/" target="_blank" rel="noopener noreferrer">https://kakaku.com/item/K0000418759/</a></p>
<p class="note" style="font-size:1.2rem"><strong>DENON PMA600NE</strong><br><a href="https://kakaku.com/item/K0001182074/" target="_blank" rel="noopener noreferrer">https://kakaku.com/item/K0001182074/</a><br>PMA600NEにはPRE OUT端子（サブウーファー接続用）が付いております。<br>The PMA600NE has a PRE OUT terminal (for connecting a subwoofer).</p>
<p class="note" style="font-size:1.2rem"><strong>DENON PMA-900HNE</strong><br><a href="https://kakaku.com/item/K0001442122/" target="_blank" rel="noopener noreferrer">https://kakaku.com/item/K0001442122/</a><br>PMA-900HNEにはPRE OUT端子（サブウーファー接続用）が付いております。<br>The PMA-900HNE has a PRE OUT terminal (for connecting a subwoofer).</p>
<p class="note" style="font-size:1.2rem">深く香り立つ東亜樹の歌声――他の歌手たちの歌声がどこか平面的に聞こえるのに対し、アキちゃんの歌は立体的である。まるで地上の陸軍と天空の空軍が同時に展開する合同作戦のように、幾重もの声の層が響き合う。<br><br>東亜紀　奥飛騨慕情　昭島温泉 湯楽の里の2階の休憩室で壁コンセント経由でクリップ式ヘッドフォンのKOSSと富士通のスマホarrows We2 Plus M06 12GBと真空管のUSB-DAC　DAC HiFiGo MUSEHiFi M3 Ultra ポータブルで聴いたら素晴らしい臨場感で素晴らしい音質でした。<br><br>オススメUSB-C　HUB　UGREEN Revodok USB-C ハブ 6in1 100W USB PD対応 4K 60Hz 10Gbps 高速データ転送</p>
<div class="yt-click-play" data-vid="PUph4FgQBk8" style="font-size:1.2rem;margin:.8rem auto;max-width:640px;text-align:center">
<p class="blog-link" style="font-size:1.2rem;margin:0 0 .5rem"><a href="https://www.youtube.com/watch?v=PUph4FgQBk8" onclick="return acClickPlay(this)">▶ Enjoy HIMARI’s brilliant performance of Bruch Violin concerto with New Japan Phil</a><br><small style="font-size:.8rem"><a href="https://www.facebook.com/reel/2448903922580973?locale=ja_JP" target="_blank" rel="noopener noreferrer">Facebook(予備 / backup)</a></small></p>
<a class="yt-thumb" href="https://www.youtube.com/watch?v=PUph4FgQBk8" onclick="return acClickPlay(this)" style="display:block;position:relative"><img src="https://i.ytimg.com/vi/PUph4FgQBk8/hqdefault.jpg" alt="Enjoy HIMARI’s brilliant performance of Bruch Violin concerto with New Japan Phil" style="width:100%;height:auto;display:block;border-radius:8px"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);font-size:3rem;color:#fff;background:rgba(0,0,0,.6);border-radius:50%;width:5rem;height:5rem;line-height:5rem;text-align:center">▶</span></a>
</div>
<div class="yt-click-play" data-vid="R8-C8ObhA5c" style="font-size:1.2rem;margin:.8rem auto;max-width:640px;text-align:center">
<p class="blog-link" style="font-size:1.2rem;margin:0 0 .5rem"><a href="https://www.youtube.com/watch?v=R8-C8ObhA5c" onclick="return acClickPlay(this)">▶ The KLIPSCHORN in Wintertime! 4K 60fps HD Audio</a><br><small style="font-size:.8rem"><a href="https://www.facebook.com/reel/932413525160019?locale=ja_JP" target="_blank" rel="noopener noreferrer">Facebook(予備 / backup)</a></small></p>
<a class="yt-thumb" href="https://www.youtube.com/watch?v=R8-C8ObhA5c" onclick="return acClickPlay(this)" style="display:block;position:relative"><img src="https://i.ytimg.com/vi/R8-C8ObhA5c/hqdefault.jpg" alt="The KLIPSCHORN in Wintertime! 4K 60fps HD Audio" style="width:100%;height:auto;display:block;border-radius:8px"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);font-size:3rem;color:#fff;background:rgba(0,0,0,.6);border-radius:50%;width:5rem;height:5rem;line-height:5rem;text-align:center">▶</span></a>
</div>
<div class="yt-click-play" data-vid="9yqTQ-yl7_A" style="font-size:1.2rem;margin:.8rem auto;max-width:640px;text-align:center">
<p class="blog-link" style="font-size:1.2rem;margin:0 0 .5rem"><a href="https://www.youtube.com/watch?v=9yqTQ-yl7_A" onclick="return acClickPlay(this)">▶ 高橋洋子　残酷な天使のテーゼ　＆　魂のルフラン</a></p>
<a class="yt-thumb" href="https://www.youtube.com/watch?v=9yqTQ-yl7_A" onclick="return acClickPlay(this)" style="display:block;position:relative"><img src="https://i.ytimg.com/vi/9yqTQ-yl7_A/hqdefault.jpg" alt="高橋洋子　残酷な天使のテーゼ　＆　魂のルフラン" style="width:100%;height:auto;display:block;border-radius:8px"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);font-size:3rem;color:#fff;background:rgba(0,0,0,.6);border-radius:50%;width:5rem;height:5rem;line-height:5rem;text-align:center">▶</span></a>
</div>
<p class="note" style="font-size:1.4rem;font-weight:700;margin:1.6rem 0 .4rem">福山雅治　ビデオ集</p>
<p class="note" style="font-size:1.2rem;font-weight:700"><a href="https://www.youtube.com/watch?v=fDAToBerhPE" target="_blank" rel="noopener noreferrer">福山雅治 - 道標 2022〈Another Story of Promise for the Future『裸の音』〉(Short ver.)</a></p>
<div style="max-width:640px;margin:0 auto 1.2rem">
<div style="position:relative;width:100%;aspect-ratio:16/9;margin:1rem 0;background:#000;border-radius:6px;overflow:hidden;">
<button type="button" aria-label="動画を再生 / Play video" style="all:unset;position:absolute;inset:0;cursor:pointer;" onclick="var f=document.createElement('iframe');f.src='https://www.youtube-nocookie.com/embed/fDAToBerhPE?autoplay=1&amp;playsinline=1&amp;rel=0';f.allow='autoplay; encrypted-media; picture-in-picture';f.allowFullscreen=true;f.title='福山雅治 - 道標 2022〈Another Story of Promise for the Future『裸の音』〉(Short ver.)';f.style.cssText='position:absolute;inset:0;width:100%;height:100%;border:0';this.replaceWith(f)"><img src="https://i.ytimg.com/vi/fDAToBerhPE/hqdefault.jpg" alt="福山雅治 - 道標 2022〈Another Story of Promise for the Future『裸の音』〉(Short ver.)" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block;"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);width:64px;height:64px;border-radius:50%;background:rgba(0,0,0,.65);color:#fff;font-size:30px;line-height:64px;text-align:center;">▶</span></button>
</div>
</div>
<p style="font-size: 0.85rem; color: var(--muted, #94a3b8);">YouTubeのリンクが切れたら次のURLでもご視聴になれます。 / If the YouTube link breaks, you can also watch it via the following URL:<br><a href="https://www.facebook.com/reel/314453634730386?locale=ja_JP" target="_blank" rel="noopener noreferrer">Facebook(予備 / backup)</a><br><a href="/video/fukuyama/fukuyama-michishirube-2022.mp4" target="_blank" rel="noopener noreferrer" onclick="var v=document.createElement('video');v.src=this.href;v.controls=true;v.autoplay=true;v.poster='https://i.ytimg.com/vi/fDAToBerhPE/hqdefault.jpg';v.playsInline=true;v.preload='auto';v.style.cssText='display:block;width:100%;max-width:640px;margin:.6rem auto;background:#000;border-radius:6px';this.parentNode.insertBefore(v,this.nextSibling);this.style.display='none';return false">MP4(クリックで再生 / click to play)</a></p>
<p class="note" style="font-size:1.2rem;font-weight:700"><a href="https://www.youtube.com/watch?v=GJMAjdd2amY" target="_blank" rel="noopener noreferrer">福山雅治 - あの夏も 海も 空も（FUKUYAMA MASAHARU DOME LIVE 2018 -暗闇の中で飛べ-）</a></p>
<div style="max-width:640px;margin:0 auto 1.2rem">
<div style="position:relative;width:100%;aspect-ratio:16/9;margin:1rem 0;background:#000;border-radius:6px;overflow:hidden;">
<button type="button" aria-label="動画を再生 / Play video" style="all:unset;position:absolute;inset:0;cursor:pointer;" onclick="var f=document.createElement('iframe');f.src='https://www.youtube-nocookie.com/embed/GJMAjdd2amY?autoplay=1&amp;playsinline=1&amp;rel=0';f.allow='autoplay; encrypted-media; picture-in-picture';f.allowFullscreen=true;f.title='福山雅治 - あの夏も 海も 空も（FUKUYAMA MASAHARU DOME LIVE 2018 -暗闇の中で飛べ-）';f.style.cssText='position:absolute;inset:0;width:100%;height:100%;border:0';this.replaceWith(f)"><img src="https://i.ytimg.com/vi/GJMAjdd2amY/hqdefault.jpg" alt="福山雅治 - あの夏も 海も 空も（FUKUYAMA MASAHARU DOME LIVE 2018 -暗闇の中で飛べ-）" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block;"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);width:64px;height:64px;border-radius:50%;background:rgba(0,0,0,.65);color:#fff;font-size:30px;line-height:64px;text-align:center;">▶</span></button>
</div>
</div>
<p style="font-size: 0.85rem; color: var(--muted, #94a3b8);">YouTubeのリンクが切れたら次のURLでもご視聴になれます。 / If the YouTube link breaks, you can also watch it via the following URL:<br><a href="https://www.facebook.com/reel/1049901451439463?locale=ja_JP" target="_blank" rel="noopener noreferrer">Facebook(予備 / backup)</a><br><a href="/video/fukuyama/fukuyama-ano-natsu-mo-umi-mo-sora-mo.mp4" target="_blank" rel="noopener noreferrer" onclick="var v=document.createElement('video');v.src=this.href;v.controls=true;v.autoplay=true;v.poster='https://i.ytimg.com/vi/GJMAjdd2amY/hqdefault.jpg';v.playsInline=true;v.preload='auto';v.style.cssText='display:block;width:100%;max-width:640px;margin:.6rem auto;background:#000;border-radius:6px';this.parentNode.insertBefore(v,this.nextSibling);this.style.display='none';return false">MP4(クリックで再生 / click to play)</a></p>
<p class="note" style="font-size:1.2rem;font-weight:700"><a href="https://www.youtube.com/watch?v=yjlsyMGJlvM" target="_blank" rel="noopener noreferrer">福山雅治 - ヒトツボシ（WE'RE BROS. TOUR 2022 “光” -HIKARI-）</a></p>
<div style="max-width:640px;margin:0 auto 1.2rem">
<div style="position:relative;width:100%;aspect-ratio:16/9;margin:1rem 0;background:#000;border-radius:6px;overflow:hidden;">
<button type="button" aria-label="動画を再生 / Play video" style="all:unset;position:absolute;inset:0;cursor:pointer;" onclick="var f=document.createElement('iframe');f.src='https://www.youtube-nocookie.com/embed/yjlsyMGJlvM?autoplay=1&amp;playsinline=1&amp;rel=0';f.allow='autoplay; encrypted-media; picture-in-picture';f.allowFullscreen=true;f.title='福山雅治 - ヒトツボシ（WE'RE BROS. TOUR 2022 “光” -HIKARI-）';f.style.cssText='position:absolute;inset:0;width:100%;height:100%;border:0';this.replaceWith(f)"><img src="https://i.ytimg.com/vi/yjlsyMGJlvM/hqdefault.jpg" alt="福山雅治 - ヒトツボシ（WE'RE BROS. TOUR 2022 “光” -HIKARI-）" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block;"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);width:64px;height:64px;border-radius:50%;background:rgba(0,0,0,.65);color:#fff;font-size:30px;line-height:64px;text-align:center;">▶</span></button>
</div>
</div>
<p style="font-size: 0.85rem; color: var(--muted, #94a3b8);">YouTubeのリンクが切れたら次のURLでもご視聴になれます。 / If the YouTube link breaks, you can also watch it via the following URL:<br><a href="https://www.facebook.com/reel/2346373692826504?locale=ja_JP" target="_blank" rel="noopener noreferrer">Facebook(予備 / backup)</a><br><a href="/video/fukuyama/fukuyama-hitotsuboshi.mp4" target="_blank" rel="noopener noreferrer" onclick="var v=document.createElement('video');v.src=this.href;v.controls=true;v.autoplay=true;v.poster='https://i.ytimg.com/vi/yjlsyMGJlvM/hqdefault.jpg';v.playsInline=true;v.preload='auto';v.style.cssText='display:block;width:100%;max-width:640px;margin:.6rem auto;background:#000;border-radius:6px';this.parentNode.insertBefore(v,this.nextSibling);this.style.display='none';return false">MP4(クリックで再生 / click to play)</a></p>
<p class="note" style="font-size:1.2rem;font-weight:700"><a href="https://www.youtube.com/watch?v=dA3kYLJ57NQ" target="_blank" rel="noopener noreferrer">福山雅治 - 蛍（福山☆夏の大創業祭 2015 稲佐山）</a></p>
<div style="max-width:640px;margin:0 auto 1.2rem">
<div style="position:relative;width:100%;aspect-ratio:16/9;margin:1rem 0;background:#000;border-radius:6px;overflow:hidden;">
<button type="button" aria-label="動画を再生 / Play video" style="all:unset;position:absolute;inset:0;cursor:pointer;" onclick="var f=document.createElement('iframe');f.src='https://www.youtube-nocookie.com/embed/dA3kYLJ57NQ?autoplay=1&amp;playsinline=1&amp;rel=0';f.allow='autoplay; encrypted-media; picture-in-picture';f.allowFullscreen=true;f.title='福山雅治 - 蛍（福山☆夏の大創業祭 2015 稲佐山）';f.style.cssText='position:absolute;inset:0;width:100%;height:100%;border:0';this.replaceWith(f)"><img src="https://i.ytimg.com/vi/dA3kYLJ57NQ/hqdefault.jpg" alt="福山雅治 - 蛍（福山☆夏の大創業祭 2015 稲佐山）" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block;"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);width:64px;height:64px;border-radius:50%;background:rgba(0,0,0,.65);color:#fff;font-size:30px;line-height:64px;text-align:center;">▶</span></button>
</div>
</div>
<p style="font-size: 0.85rem; color: var(--muted, #94a3b8);">YouTubeのリンクが切れたら次のURLでもご視聴になれます。 / If the YouTube link breaks, you can also watch it via the following URL:<br><a href="https://www.facebook.com/reel/2168438207439523?locale=ja_JP" target="_blank" rel="noopener noreferrer">Facebook(予備 / backup)</a><br><a href="/video/fukuyama/fukuyama-hotaru.mp4" target="_blank" rel="noopener noreferrer" onclick="var v=document.createElement('video');v.src=this.href;v.controls=true;v.autoplay=true;v.poster='https://i.ytimg.com/vi/dA3kYLJ57NQ/hqdefault.jpg';v.playsInline=true;v.preload='auto';v.style.cssText='display:block;width:100%;max-width:640px;margin:.6rem auto;background:#000;border-radius:6px';this.parentNode.insertBefore(v,this.nextSibling);this.style.display='none';return false">MP4(クリックで再生 / click to play)</a></p>
<p class="note" style="font-size:1.2rem;font-weight:700"><a href="https://www.youtube.com/watch?v=my4Dxmyx1n8" target="_blank" rel="noopener noreferrer">(再投稿) 福山雅治 桜坂 2005</a></p>
<div style="max-width:640px;margin:0 auto 1.2rem">
<div style="position:relative;width:100%;aspect-ratio:16/9;margin:1rem 0;background:#000;border-radius:6px;overflow:hidden;">
<button type="button" aria-label="動画を再生 / Play video" style="all:unset;position:absolute;inset:0;cursor:pointer;" onclick="var f=document.createElement('iframe');f.src='https://www.youtube-nocookie.com/embed/my4Dxmyx1n8?autoplay=1&amp;playsinline=1&amp;rel=0';f.allow='autoplay; encrypted-media; picture-in-picture';f.allowFullscreen=true;f.title='(再投稿) 福山雅治 桜坂 2005';f.style.cssText='position:absolute;inset:0;width:100%;height:100%;border:0';this.replaceWith(f)"><img src="https://i.ytimg.com/vi/my4Dxmyx1n8/hqdefault.jpg" alt="(再投稿) 福山雅治 桜坂 2005" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block;"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);width:64px;height:64px;border-radius:50%;background:rgba(0,0,0,.65);color:#fff;font-size:30px;line-height:64px;text-align:center;">▶</span></button>
</div>
</div>
<p style="font-size: 0.85rem; color: var(--muted, #94a3b8);">YouTubeのリンクが切れたら次のURLでもご視聴になれます。 / If the YouTube link breaks, you can also watch it via the following URL:<br><a href="https://www.facebook.com/reel/1629015382578865?locale=ja_JP" target="_blank" rel="noopener noreferrer">Facebook(予備 / backup)</a><br><a href="/video/fukuyama/fukuyama-sakurazaka-2005.mp4" target="_blank" rel="noopener noreferrer" onclick="var v=document.createElement('video');v.src=this.href;v.controls=true;v.autoplay=true;v.poster='https://i.ytimg.com/vi/my4Dxmyx1n8/hqdefault.jpg';v.playsInline=true;v.preload='auto';v.style.cssText='display:block;width:100%;max-width:640px;margin:.6rem auto;background:#000;border-radius:6px';this.parentNode.insertBefore(v,this.nextSibling);this.style.display='none';return false">MP4(クリックで再生 / click to play)</a></p>
<p class="note" style="font-size:1.2rem;font-weight:700"><a href="https://www.youtube.com/watch?v=LG9zEl7ASVk" target="_blank" rel="noopener noreferrer">『桜坂』 EARTH THE KO Opening Ceremony at 嚴島神社 | 柴咲コウ【期間限定】</a></p>
<div style="max-width:640px;margin:0 auto 1.2rem">
<div style="position:relative;width:100%;aspect-ratio:16/9;margin:1rem 0;background:#000;border-radius:6px;overflow:hidden;">
<button type="button" aria-label="動画を再生 / Play video" style="all:unset;position:absolute;inset:0;cursor:pointer;" onclick="var f=document.createElement('iframe');f.src='https://www.youtube-nocookie.com/embed/LG9zEl7ASVk?autoplay=1&amp;playsinline=1&amp;rel=0';f.allow='autoplay; encrypted-media; picture-in-picture';f.allowFullscreen=true;f.title='『桜坂』 EARTH THE KO Opening Ceremony at 嚴島神社 | 柴咲コウ【期間限定】';f.style.cssText='position:absolute;inset:0;width:100%;height:100%;border:0';this.replaceWith(f)"><img src="https://i.ytimg.com/vi/LG9zEl7ASVk/hqdefault.jpg" alt="『桜坂』 EARTH THE KO Opening Ceremony at 嚴島神社 | 柴咲コウ【期間限定】" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block;"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);width:64px;height:64px;border-radius:50%;background:rgba(0,0,0,.65);color:#fff;font-size:30px;line-height:64px;text-align:center;">▶</span></button>
</div>
</div>
<p style="font-size: 0.85rem; color: var(--muted, #94a3b8);">YouTubeのリンクが切れたら次のURLでもご視聴になれます。 / If the YouTube link breaks, you can also watch it via the following URL:<br><a href="https://www.facebook.com/reel/2044662199570657?locale=ja_JP" target="_blank" rel="noopener noreferrer">Facebook(予備 / backup)</a><br><a href="/video/fukuyama/shibasaki-sakurazaka-itsukushima.mp4" target="_blank" rel="noopener noreferrer" onclick="var v=document.createElement('video');v.src=this.href;v.controls=true;v.autoplay=true;v.poster='https://i.ytimg.com/vi/LG9zEl7ASVk/hqdefault.jpg';v.playsInline=true;v.preload='auto';v.style.cssText='display:block;width:100%;max-width:640px;margin:.6rem auto;background:#000;border-radius:6px';this.parentNode.insertBefore(v,this.nextSibling);this.style.display='none';return false">MP4(クリックで再生 / click to play)</a></p>
<p class="note" style="font-size:1.2rem;font-weight:700;margin-top:1.6rem"><strong>東亜希　奥飛騨慕情 (AKI AZUMA OKUHIDABOJYOU)</strong></p>
<p class="note" style="font-size:1.2rem"><video controls preload="metadata" playsinline style="max-width:100%;height:auto" src="/video/AKI-AZUMA-OKUHIDABOJYOU.mp4"></video></p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://youtu.be/EVZ0aClCWTk?si=rqEv77BWqlgcwMys" target="_blank" rel="noopener noreferrer">📝 YouTube「東亜希 奥飛騨慕情 AKI AZUMA OKUHIDABOJYOU」</a></p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://www.facebook.com/reel/1101027912297527?locale=ja_JP" target="_blank" rel="noopener noreferrer">📝 Facebook リール「東亜希 奥飛騨慕情」</a></p>
<div class="yt-click-play" data-vid="Dyw3un2sRVQ" style="font-size:1.2rem;margin:.8rem auto;max-width:640px;text-align:center">
<p class="blog-link" style="font-size:1.2rem;margin:0 0 .5rem"><a href="https://www.youtube.com/watch?v=Dyw3un2sRVQ" onclick="return acClickPlay(this)">▶ 아즈마아키(東亜樹)X진해성(ジンヘソン) - 아름다운 스위스 아가씨(美しいスイスのお嬢さん)｜한일톱텐쇼 59회</a></p>
<a class="yt-thumb" href="https://www.youtube.com/watch?v=Dyw3un2sRVQ" onclick="return acClickPlay(this)" style="display:block;position:relative"><img src="https://i.ytimg.com/vi/Dyw3un2sRVQ/hqdefault.jpg" alt="아즈마아키(東亜樹)X진해성(ジンヘソン) - 아름다운 스위스 아가씨(美しいスイスのお嬢さん)" style="width:100%;height:auto;display:block;border-radius:8px"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);font-size:3rem;color:#fff;background:rgba(0,0,0,.6);border-radius:50%;width:5rem;height:5rem;line-height:5rem;text-align:center">▶</span></a>
</div>
<script>function acClickPlay(a){{var b=a.closest('.yt-click-play');var t=b.querySelector('.yt-thumb');if(t){{var f=document.createElement('iframe');f.src='https://www.youtube.com/embed/'+b.dataset.vid+'?autoplay=1&rel=0';f.allow='autoplay; encrypted-media; picture-in-picture';f.allowFullscreen=true;f.style.cssText='width:100%;aspect-ratio:16/9;border:0;border-radius:8px';t.replaceWith(f);}}return false;}}</script>
<div class="yt-click-play" data-vid="FNUKj6qMQZw" style="font-size:1.2rem;margin:.8rem auto;max-width:640px;text-align:center">
<p class="blog-link" style="font-size:1.2rem;margin:0 0 .5rem"><a href="https://www.youtube.com/watch?v=FNUKj6qMQZw" onclick="return acClickPlay(this)">▶ 아즈마아키(東亜樹) - 와라비가미(童神)｜한일가왕전 4회</a><br><small style="font-size:.8rem"><a href="https://www.facebook.com/reel/2017593788903131?locale=ja_JP" target="_blank" rel="noopener noreferrer">Facebook(予備 / backup)</a></small></p>
<a class="yt-thumb" href="https://www.youtube.com/watch?v=FNUKj6qMQZw" onclick="return acClickPlay(this)" style="display:block;position:relative"><img src="https://i.ytimg.com/vi/FNUKj6qMQZw/hqdefault.jpg" alt="아즈마아키(東亜樹) - 와라비가미(童神)｜한일가왕전 4회" style="width:100%;height:auto;display:block;border-radius:8px"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);font-size:3rem;color:#fff;background:rgba(0,0,0,.6);border-radius:50%;width:5rem;height:5rem;line-height:5rem;text-align:center">▶</span></a>
</div>
<div class="yt-click-play" data-vid="szpO2_1ysC8" style="font-size:1.2rem;margin:.8rem auto;max-width:640px;text-align:center">
<p class="blog-link" style="font-size:1.2rem;margin:0 0 .5rem"><a href="https://www.youtube.com/watch?v=szpO2_1ysC8" onclick="return acClickPlay(this)">▶ 東亜樹 いつも何度でも　スタジオ版</a><br><small style="font-size:.8rem"><a href="https://www.facebook.com/reel/1170914635495600?locale=ja_JP" target="_blank" rel="noopener noreferrer">Facebook(予備 / backup)</a></small></p>
<a class="yt-thumb" href="https://www.youtube.com/watch?v=szpO2_1ysC8" onclick="return acClickPlay(this)" style="display:block;position:relative"><img src="https://i.ytimg.com/vi/szpO2_1ysC8/hqdefault.jpg" alt="東亜樹 いつも何度でも　スタジオ版" style="width:100%;height:auto;display:block;border-radius:8px"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);font-size:3rem;color:#fff;background:rgba(0,0,0,.6);border-radius:50%;width:5rem;height:5rem;line-height:5rem;text-align:center">▶</span></a>
</div>
<div class="yt-click-play" data-vid="dRQl07pSfpU" style="font-size:1.2rem;margin:.8rem auto;max-width:640px;text-align:center">
<p class="blog-link" style="font-size:1.2rem;margin:0 0 .5rem"><a href="https://www.youtube.com/watch?v=dRQl07pSfpU" onclick="return acClickPlay(this)">▶ 東亜樹　いつも何度でも</a></p>
<a class="yt-thumb" href="https://www.youtube.com/watch?v=dRQl07pSfpU" onclick="return acClickPlay(this)" style="display:block;position:relative"><img src="https://i.ytimg.com/vi/dRQl07pSfpU/hqdefault.jpg" alt="東亜樹　いつも何度でも" style="width:100%;height:auto;display:block;border-radius:8px"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);font-size:3rem;color:#fff;background:rgba(0,0,0,.6);border-radius:50%;width:5rem;height:5rem;line-height:5rem;text-align:center">▶</span></a>
</div>
<div class="yt-click-play" data-vid="ebTaPQ-KvRw" style="font-size:1.2rem;margin:.8rem auto;max-width:640px;text-align:center">
<p class="blog-link" style="font-size:1.2rem;margin:0 0 .5rem"><a href="https://www.youtube.com/watch?v=ebTaPQ-KvRw" onclick="return acClickPlay(this)">▶ 아즈마아키(東亜樹) - Volare｜한일톱텐쇼 60회</a><br><small style="font-size:.8rem"><a href="https://www.facebook.com/reel/1623842208598207?locale=ja_JP" target="_blank" rel="noopener noreferrer">Facebook(予備 / backup)</a></small></p>
<a class="yt-thumb" href="https://www.youtube.com/watch?v=ebTaPQ-KvRw" onclick="return acClickPlay(this)" style="display:block;position:relative"><img src="https://i.ytimg.com/vi/ebTaPQ-KvRw/hqdefault.jpg" alt="아즈마아키(東亜樹) - Volare｜한일톱텐쇼 60회" style="width:100%;height:auto;display:block;border-radius:8px"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);font-size:3rem;color:#fff;background:rgba(0,0,0,.6);border-radius:50%;width:5rem;height:5rem;line-height:5rem;text-align:center">▶</span></a>
</div>
<div class="yt-click-play" data-vid="9uRNlZENWbA" style="font-size:1.2rem;margin:.8rem auto;max-width:640px;text-align:center">
<p class="blog-link" style="font-size:1.2rem;margin:0 0 .5rem"><a href="https://www.youtube.com/watch?v=9uRNlZENWbA" onclick="return acClickPlay(this)">▶ [공연실황] 아즈마아키(東亜樹)-Que Sera Sera(ケセラセラ) I 스테이지엠 영화음악 콘서트</a><br><small style="font-size:.8rem"><a href="https://www.facebook.com/reel/29467881546163845?locale=ja_JP" target="_blank" rel="noopener noreferrer">Facebook(予備 / backup)</a></small></p>
<a class="yt-thumb" href="https://www.youtube.com/watch?v=9uRNlZENWbA" onclick="return acClickPlay(this)" style="display:block;position:relative"><img src="https://i.ytimg.com/vi/9uRNlZENWbA/hqdefault.jpg" alt="[공연실황] 아즈마아키(東亜樹)-Que Sera Sera(ケセラセラ) I 스테이지엠 영화음악 콘서트" style="width:100%;height:auto;display:block;border-radius:8px"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);font-size:3rem;color:#fff;background:rgba(0,0,0,.6);border-radius:50%;width:5rem;height:5rem;line-height:5rem;text-align:center">▶</span></a>
</div>
<div class="yt-click-play" data-vid="vPGm2q79px4" style="font-size:1.2rem;margin:.8rem auto;max-width:640px;text-align:center">
<p class="blog-link" style="font-size:1.2rem;margin:0 0 .5rem"><a href="https://www.youtube.com/watch?v=vPGm2q79px4" onclick="return acClickPlay(this)">▶ 歌心りえ たしかなこと / Utagokoro Rie - Tashikana Koto</a><br><small style="font-size:.8rem"><a href="https://www.facebook.com/reel/1339787288201605?locale=ja_JP" target="_blank" rel="noopener noreferrer">Facebook(予備 / backup)</a></small></p>
<a class="yt-thumb" href="https://www.youtube.com/watch?v=vPGm2q79px4" onclick="return acClickPlay(this)" style="display:block;position:relative"><img src="https://i.ytimg.com/vi/vPGm2q79px4/hqdefault.jpg" alt="歌心りえ たしかなこと / Utagokoro Rie - Tashikana Koto" style="width:100%;height:auto;display:block;border-radius:8px"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);font-size:3rem;color:#fff;background:rgba(0,0,0,.6);border-radius:50%;width:5rem;height:5rem;line-height:5rem;text-align:center">▶</span></a>
</div>
<div class="yt-click-play" data-vid="yDG3MuRUPgU" style="font-size:1.2rem;margin:.8rem auto;max-width:640px;text-align:center">
<p class="blog-link" style="font-size:1.2rem;margin:0 0 .5rem"><a href="https://www.youtube.com/watch?v=yDG3MuRUPgU" onclick="return acClickPlay(this)">▶ 우타고코로리에(歌心りえ) - 고이비토요(恋人よ)｜한일톱텐쇼 13회</a><br><small style="font-size:.8rem"><a href="https://www.facebook.com/reel/1091133880354089?locale=ja_JP" target="_blank" rel="noopener noreferrer">Facebook(予備 / backup)</a></small></p>
<a class="yt-thumb" href="https://www.youtube.com/watch?v=yDG3MuRUPgU" onclick="return acClickPlay(this)" style="display:block;position:relative"><img src="https://i.ytimg.com/vi/yDG3MuRUPgU/hqdefault.jpg" alt="우타고코로리에(歌心りえ) - 고이비토요(恋人よ)｜한일톱텐쇼 13회" style="width:100%;height:auto;display:block;border-radius:8px"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);font-size:3rem;color:#fff;background:rgba(0,0,0,.6);border-radius:50%;width:5rem;height:5rem;line-height:5rem;text-align:center">▶</span></a>
</div>
<div class="yt-click-play" data-vid="qsYPw-VNekQ" style="font-size:1.2rem;margin:.8rem auto;max-width:640px;text-align:center">
<p class="blog-link" style="font-size:1.2rem;margin:0 0 .5rem"><a href="https://www.youtube.com/watch?v=qsYPw-VNekQ" onclick="return acClickPlay(this)">▶ ＜DayDay.＞日韓歌姫 チョ・ジョンミン＆歌心りえ「恋人よ」SPデュエット【アーティストLIVE】</a><br><small style="font-size:.8rem"><a href="https://www.facebook.com/reel/2287559882094384?locale=ja_JP" target="_blank" rel="noopener noreferrer">Facebook(予備 / backup)</a></small></p>
<a class="yt-thumb" href="https://www.youtube.com/watch?v=qsYPw-VNekQ" onclick="return acClickPlay(this)" style="display:block;position:relative"><img src="https://i.ytimg.com/vi/qsYPw-VNekQ/hqdefault.jpg" alt="＜DayDay.＞日韓歌姫 チョ・ジョンミン＆歌心りえ「恋人よ」SPデュエット【アーティストLIVE】" style="width:100%;height:auto;display:block;border-radius:8px"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);font-size:3rem;color:#fff;background:rgba(0,0,0,.6);border-radius:50%;width:5rem;height:5rem;line-height:5rem;text-align:center">▶</span></a>
</div>
<div class="yt-click-play" data-vid="3DrVc07KdyQ" style="font-size:1.2rem;margin:.8rem auto;max-width:640px;text-align:center">
<p class="blog-link" style="font-size:1.2rem;margin:0 0 .5rem"><a href="https://www.youtube.com/watch?v=3DrVc07KdyQ" onclick="return acClickPlay(this)">▶ 歌心りえ　雪の華</a><br><small style="font-size:.8rem"><a href="https://www.facebook.com/reel/1138017215474814?locale=ja_JP" target="_blank" rel="noopener noreferrer">Facebook(予備 / backup)</a></small></p>
<a class="yt-thumb" href="https://www.youtube.com/watch?v=3DrVc07KdyQ" onclick="return acClickPlay(this)" style="display:block;position:relative"><img src="https://i.ytimg.com/vi/3DrVc07KdyQ/hqdefault.jpg" alt="歌心りえ　雪の華" style="width:100%;height:auto;display:block;border-radius:8px"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);font-size:3rem;color:#fff;background:rgba(0,0,0,.6);border-radius:50%;width:5rem;height:5rem;line-height:5rem;text-align:center">▶</span></a>
</div>
<div class="yt-click-play" data-vid="sRh9t3q2R2M" style="font-size:1.2rem;margin:.8rem auto;max-width:640px;text-align:center">
<p class="blog-link" style="font-size:1.2rem;margin:0 0 .5rem"><a href="https://www.youtube.com/watch?v=sRh9t3q2R2M" onclick="return acClickPlay(this)">▶ 歌心りえ　雪の華　ライブ#TVCHOSUNMUSIC (TV CHOSUN 251231 방송)</a><br><small style="font-size:.8rem"><a href="https://www.facebook.com/reel/1442808854663565?locale=ja_JP" target="_blank" rel="noopener noreferrer">Facebook(予備 / backup)</a></small></p>
<a class="yt-thumb" href="https://www.youtube.com/watch?v=sRh9t3q2R2M" onclick="return acClickPlay(this)" style="display:block;position:relative"><img src="https://i.ytimg.com/vi/sRh9t3q2R2M/hqdefault.jpg" alt="歌心りえ　雪の華　ライブ#TVCHOSUNMUSIC (TV CHOSUN 251231 방송)" style="width:100%;height:auto;display:block;border-radius:8px"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);font-size:3rem;color:#fff;background:rgba(0,0,0,.6);border-radius:50%;width:5rem;height:5rem;line-height:5rem;text-align:center">▶</span></a>
</div>
<div class="yt-click-play" data-vid="Tl7EbkDnj7Y" style="font-size:1.2rem;margin:.8rem auto;max-width:640px;text-align:center">
<p class="blog-link" style="font-size:1.2rem;margin:0 0 .5rem"><a href="https://www.youtube.com/watch?v=Tl7EbkDnj7Y" onclick="return acClickPlay(this)">▶ 木蘭の涙（MOKUREN　NO　NAMIDA）　歌心りえ（RIE　UTAGOKORO）　人生、歌がある　今週のイチオシ　歌心りえ (2025.11.15)</a></p>
<a class="yt-thumb" href="https://www.youtube.com/watch?v=Tl7EbkDnj7Y" onclick="return acClickPlay(this)" style="display:block;position:relative"><img src="https://i.ytimg.com/vi/Tl7EbkDnj7Y/hqdefault.jpg" alt="木蘭の涙（MOKUREN　NO　NAMIDA）　歌心りえ（RIE　UTAGOKORO）　人生、歌がある　今週のイチオシ　歌心りえ (2025.11.15)" style="width:100%;height:auto;display:block;border-radius:8px"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);font-size:3rem;color:#fff;background:rgba(0,0,0,.6);border-radius:50%;width:5rem;height:5rem;line-height:5rem;text-align:center">▶</span></a>
</div>
<div class="yt-click-play" data-vid="DKpwG6E5rok" style="font-size:1.2rem;margin:.8rem auto;max-width:640px;text-align:center">
<p class="blog-link" style="font-size:1.2rem;margin:0 0 .5rem"><a href="https://www.youtube.com/watch?v=DKpwG6E5rok" onclick="return acClickPlay(this)">▶ 歌心りえ　X　東亜樹　童神</a><br><small style="font-size:.8rem"><a href="https://www.facebook.com/reel/1129537502877613?locale=ja_JP" target="_blank" rel="noopener noreferrer">Facebook(予備 / backup)</a></small></p>
<a class="yt-thumb" href="https://www.youtube.com/watch?v=DKpwG6E5rok" onclick="return acClickPlay(this)" style="display:block;position:relative"><img src="https://i.ytimg.com/vi/DKpwG6E5rok/hqdefault.jpg" alt="歌心りえ　X　東亜樹　童神" style="width:100%;height:auto;display:block;border-radius:8px"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);font-size:3rem;color:#fff;background:rgba(0,0,0,.6);border-radius:50%;width:5rem;height:5rem;line-height:5rem;text-align:center">▶</span></a>
</div>
<div class="yt-click-play" data-vid="qVymStzDEjU"style="font-size:1.2rem;margin:.8rem auto;max-width:640px;text-align:center">
<p class="blog-link" style="font-size:1.2rem;margin:0 0 .5rem"><a href="https://www.youtube.com/watch?v=qVymStzDEjU" onclick="return acClickPlay(this)">▶ 歌心りえ　最初から今まで　冬のソナタ　日本語版</a></p>
<a class="yt-thumb" href="https://www.youtube.com/watch?v=qVymStzDEjU" onclick="return acClickPlay(this)" style="display:block;position:relative"><img src="https://i.ytimg.com/vi/qVymStzDEjU/hqdefault.jpg" alt="歌心りえ　最初から今まで　冬のソナタ　日本語版" style="width:100%;height:auto;display:block;border-radius:8px"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);font-size:3rem;color:#fff;background:rgba(0,0,0,.6);border-radius:50%;width:5rem;height:5rem;line-height:5rem;text-align:center">▶</span></a>
</div>
<div class="yt-click-play" data-vid="wCM7St1MPx4" style="font-size:1.2rem;margin:.8rem auto;max-width:640px;text-align:center">
<p class="blog-link" style="font-size:1.2rem;margin:0 0 .5rem"><a href="https://www.youtube.com/watch?v=wCM7St1MPx4" onclick="return acClickPlay(this)">▶ 歌心りえ　最初から今まで　音質改善版</a><br><small style="font-size:.8rem"><a href="https://www.facebook.com/reel/2298448570940009?locale=ja_JP" target="_blank" rel="noopener noreferrer">Facebook(予備 / backup)</a></small></p>
<a class="yt-thumb" href="https://www.youtube.com/watch?v=wCM7St1MPx4" onclick="return acClickPlay(this)" style="display:block;position:relative"><img src="https://i.ytimg.com/vi/wCM7St1MPx4/hqdefault.jpg" alt="歌心りえ　最初から今まで　音質改善版" style="width:100%;height:auto;display:block;border-radius:8px"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);font-size:3rem;color:#fff;background:rgba(0,0,0,.6);border-radius:50%;width:5rem;height:5rem;line-height:5rem;text-align:center">▶</span></a>
</div>
<div class="yt-click-play" data-vid="iC1Y-1pwbm0" style="font-size:1.2rem;margin:.8rem auto;max-width:640px;text-align:center">
<p class="blog-link" style="font-size:1.2rem;margin:0 0 .5rem"><a href="https://www.youtube.com/watch?v=iC1Y-1pwbm0" onclick="return acClickPlay(this)">▶ 歌心りえ 　道化師（ピエロ）のソネット</a></p>
<a class="yt-thumb" href="https://www.youtube.com/watch?v=iC1Y-1pwbm0" onclick="return acClickPlay(this)" style="display:block;position:relative"><img src="https://i.ytimg.com/vi/iC1Y-1pwbm0/hqdefault.jpg" alt="歌心りえ 　道化師（ピエロ）のソネット" style="width:100%;height:auto;display:block;border-radius:8px"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);font-size:3rem;color:#fff;background:rgba(0,0,0,.6);border-radius:50%;width:5rem;height:5rem;line-height:5rem;text-align:center">▶</span></a>
</div>
<div class="yt-click-play" data-vid="nr97b5xnzZ0" style="font-size:1.2rem;margin:.8rem auto;max-width:640px;text-align:center">
<p class="blog-link" style="font-size:1.2rem;margin:0 0 .5rem"><a href="https://www.youtube.com/watch?v=nr97b5xnzZ0" onclick="return acClickPlay(this)">▶ 歌心りえ 道化師のソネット　ライブ</a><br><small style="font-size:.8rem">미소로 행복을 전하는🤗💕 우타고코로 리에 ‘어릿광대의 소네트’♪🤡 TV CHOSUN 251231 방송 | [한일슈퍼콘서트 2회] | TV조선</small><br><small style="font-size:.8rem"><a href="https://www.facebook.com/reel/1862839491831174?locale=ja_JP" target="_blank" rel="noopener noreferrer">Facebook(予備 / backup)</a></small></p>
<a class="yt-thumb" href="https://www.youtube.com/watch?v=nr97b5xnzZ0" onclick="return acClickPlay(this)" style="display:block;position:relative"><img src="https://i.ytimg.com/vi/nr97b5xnzZ0/hqdefault.jpg" alt="歌心りえ 道化師のソネット　ライブ" style="width:100%;height:auto;display:block;border-radius:8px"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);font-size:3rem;color:#fff;background:rgba(0,0,0,.6);border-radius:50%;width:5rem;height:5rem;line-height:5rem;text-align:center">▶</span></a>
</div>
<div class="yt-click-play" data-vid="vVzQVcnib1w" style="font-size:1.2rem;margin:.8rem auto;max-width:640px;text-align:center">
<p class="blog-link" style="font-size:1.2rem;margin:0 0 .5rem"><a href="https://www.youtube.com/watch?v=vVzQVcnib1w" onclick="return acClickPlay(this)">▶ 歌心りえ　シン·スンテ　川の流れのように</a><br><small style="font-size:.8rem"><a href="https://www.facebook.com/reel/1599732914645635?locale=ja_JP" target="_blank" rel="noopener noreferrer">Facebook(予備 / backup)</a></small></p>
<a class="yt-thumb" href="https://www.youtube.com/watch?v=vVzQVcnib1w" onclick="return acClickPlay(this)" style="display:block;position:relative"><img src="https://i.ytimg.com/vi/vVzQVcnib1w/hqdefault.jpg" alt="歌心りえ　シン·スンテ　川の流れのように" style="width:100%;height:auto;display:block;border-radius:8px"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);font-size:3rem;color:#fff;background:rgba(0,0,0,.6);border-radius:50%;width:5rem;height:5rem;line-height:5rem;text-align:center">▶</span></a>
</div>
<div class="yt-click-play" data-vid="O1fY5NJlwCY" style="font-size:1.2rem;margin:.8rem auto;max-width:640px;text-align:center">
<p class="blog-link" style="font-size:1.2rem;margin:0 0 .5rem"><a href="https://www.youtube.com/watch?v=O1fY5NJlwCY" onclick="return acClickPlay(this)">▶ 歌心りえ　PIECE OF MY WISH 　TV CHOSUN 260220</a><br><small style="font-size:.8rem"><a href="https://www.facebook.com/reel/2772549453190596?locale=ja_JP" target="_blank" rel="noopener noreferrer">Facebook(予備 / backup)</a></small></p>
<a class="yt-thumb" href="https://www.youtube.com/watch?v=O1fY5NJlwCY" onclick="return acClickPlay(this)" style="display:block;position:relative"><img src="https://i.ytimg.com/vi/O1fY5NJlwCY/hqdefault.jpg" alt="歌心りえ　PIECE OF MY WISH 　TV CHOSUN 260220" style="width:100%;height:auto;display:block;border-radius:8px"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);font-size:3rem;color:#fff;background:rgba(0,0,0,.6);border-radius:50%;width:5rem;height:5rem;line-height:5rem;text-align:center">▶</span></a>
</div>
<div class="yt-click-play" data-vid="v52UNMJH7tg" style="font-size:1.2rem;margin:.8rem auto;max-width:640px;text-align:center">
<p class="blog-link" style="font-size:1.2rem;margin:0 0 .5rem"><a href="https://www.youtube.com/watch?v=v52UNMJH7tg" onclick="return acClickPlay(this)">▶ 歌心りえ　エノク　カサブランカ</a><br><small style="font-size:.8rem"><a href="https://www.facebook.com/reel/1132188675859844?locale=ja_JP" target="_blank" rel="noopener noreferrer">Facebook(予備 / backup)</a></small></p>
<a class="yt-thumb" href="https://www.youtube.com/watch?v=v52UNMJH7tg" onclick="return acClickPlay(this)" style="display:block;position:relative"><img src="https://i.ytimg.com/vi/v52UNMJH7tg/hqdefault.jpg" alt="歌心りえ　エノク　カサブランカ" style="width:100%;height:auto;display:block;border-radius:8px"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);font-size:3rem;color:#fff;background:rgba(0,0,0,.6);border-radius:50%;width:5rem;height:5rem;line-height:5rem;text-align:center">▶</span></a>
</div>
<div class="yt-click-play" data-vid="p70qdkpf6jw" style="font-size:1.2rem;margin:.8rem auto;max-width:640px;text-align:center">
<p class="blog-link" style="font-size:1.2rem;margin:0 0 .5rem"><a href="https://www.youtube.com/watch?v=p70qdkpf6jw" onclick="return acClickPlay(this)">▶ 歌心りえ　ソンテジン　Rainy Blue</a><br><small style="font-size:.8rem"><a href="https://www.facebook.com/reel/3352339781624239?locale=ja_JP" target="_blank" rel="noopener noreferrer">Facebook(予備 / backup)</a></small></p>
<a class="yt-thumb" href="https://www.youtube.com/watch?v=p70qdkpf6jw" onclick="return acClickPlay(this)" style="display:block;position:relative"><img src="https://i.ytimg.com/vi/p70qdkpf6jw/hqdefault.jpg" alt="歌心りえ　ソンテジン　Rainy Blue" style="width:100%;height:auto;display:block;border-radius:8px"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);font-size:3rem;color:#fff;background:rgba(0,0,0,.6);border-radius:50%;width:5rem;height:5rem;line-height:5rem;text-align:center">▶</span></a>
</div>
<div class="yt-click-play" data-vid="u2-aAlOPzIY" style="font-size:1.2rem;margin:.8rem auto;max-width:640px;text-align:center">
<p class="blog-link" style="font-size:1.2rem;margin:0 0 .5rem"><a href="https://www.youtube.com/watch?v=u2-aAlOPzIY" onclick="return acClickPlay(this)">▶ 歌心りえの家族　瑠璃色の地球</a><br><small style="font-size:.8rem"><a href="https://www.facebook.com/reel/854589809869402?locale=ja_JP" target="_blank" rel="noopener noreferrer">Facebook(予備 / backup)</a></small></p>
<a class="yt-thumb" href="https://www.youtube.com/watch?v=u2-aAlOPzIY" onclick="return acClickPlay(this)" style="display:block;position:relative"><img src="https://i.ytimg.com/vi/u2-aAlOPzIY/hqdefault.jpg" alt="歌心りえの家族　瑠璃色の地球" style="width:100%;height:auto;display:block;border-radius:8px"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);font-size:3rem;color:#fff;background:rgba(0,0,0,.6);border-radius:50%;width:5rem;height:5rem;line-height:5rem;text-align:center">▶</span></a>
</div>
<div class="yt-click-play" data-vid="1ycWIhuXqZ8"style="font-size:1.2rem;margin:.8rem auto;max-width:640px;text-align:center">
<p class="blog-link" style="font-size:1.2rem;margin:0 0 .5rem"><a href="https://youtu.be/1ycWIhuXqZ8" onclick="return acClickPlay(this)">▶ 【7.83Hz × 純正律】シューマン共振に完全に調律されたカノン｜睡眠・癒し・ヒーリングBGM</a></p>
<a class="yt-thumb" href="https://youtu.be/1ycWIhuXqZ8" onclick="return acClickPlay(this)" style="display:block;position:relative"><img src="https://i.ytimg.com/vi/1ycWIhuXqZ8/hqdefault.jpg" alt="【7.83Hz × 純正律】シューマン共振に完全に調律されたカノン｜睡眠・癒し・ヒーリングBGM" style="width:100%;height:auto;display:block;border-radius:8px"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);font-size:3rem;color:#fff;background:rgba(0,0,0,.6);border-radius:50%;width:5rem;height:5rem;line-height:5rem;text-align:center">▶</span></a>
</div>
<div class="yt-click-play" data-vid="ug-X9JlcUfs" style="font-size:1.2rem;margin:.8rem auto;max-width:640px;text-align:center">
<p class="blog-link" style="font-size:1.2rem;margin:0 0 .5rem"><a href="https://youtu.be/ug-X9JlcUfs" onclick="return acClickPlay(this)">▶ シューマン共振 | 脳波同期のための純粋周波数 7.83Hz — 瞑想用</a></p>
<a class="yt-thumb" href="https://youtu.be/ug-X9JlcUfs" onclick="return acClickPlay(this)" style="display:block;position:relative"><img src="https://i.ytimg.com/vi/ug-X9JlcUfs/hqdefault.jpg" alt="シューマン共振 | 脳波同期のための純粋周波数 7.83Hz — 瞑想用" style="width:100%;height:auto;display:block;border-radius:8px"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);font-size:3rem;color:#fff;background:rgba(0,0,0,.6);border-radius:50%;width:5rem;height:5rem;line-height:5rem;text-align:center">▶</span></a>
</div>
<div class="yt-click-play" data-vid="RuZsQ-8yntM" style="font-size:1.2rem;margin:.8rem auto;max-width:640px;text-align:center">
<p class="blog-link" style="font-size:1.2rem;margin:0 0 .5rem"><a href="https://www.youtube.com/watch?v=RuZsQ-8yntM" onclick="return acClickPlay(this)">▶ フラクタル DMT 共鳴 |サブハーモニック松果体グリッド (風強化)</a></p>
<a class="yt-thumb" href="https://www.youtube.com/watch?v=RuZsQ-8yntM" onclick="return acClickPlay(this)" style="display:block;position:relative"><img src="https://i.ytimg.com/vi/RuZsQ-8yntM/hqdefault.jpg" alt="フラクタル DMT 共鳴 |サブハーモニック松果体グリッド (風強化)" style="width:100%;height:auto;display:block;border-radius:8px"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);font-size:3rem;color:#fff;background:rgba(0,0,0,.6);border-radius:50%;width:5rem;height:5rem;line-height:5rem;text-align:center">▶</span></a>
</div>
<div class="yt-click-play" data-vid="pTh2CPoHZhM" style="font-size:1.2rem;margin:.8rem auto;max-width:640px;text-align:center">
<p class="blog-link" style="font-size:1.2rem;margin:0 0 .5rem"><a href="https://www.youtube.com/watch?v=pTh2CPoHZhM" onclick="return acClickPlay(this)">▶ 【神回】プロなら駅ピアノで「月光第3楽章/ベートーヴェン」を突然リクエストされても弾けるんですよね？【ストリートピアノ】</a></p>
<a class="yt-thumb" href="https://www.youtube.com/watch?v=pTh2CPoHZhM" onclick="return acClickPlay(this)" style="display:block;position:relative"><img src="https://i.ytimg.com/vi/pTh2CPoHZhM/hqdefault.jpg" alt="【神回】プロなら駅ピアノで「月光第3楽章/ベートーヴェン」を突然リクエストされても弾けるんですよね？【ストリートピアノ】" style="width:100%;height:auto;display:block;border-radius:8px"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);font-size:3rem;color:#fff;background:rgba(0,0,0,.6);border-radius:50%;width:5rem;height:5rem;line-height:5rem;text-align:center">▶</span></a>
</div>
<div class="yt-click-play" data-vid="wKV6ibqZChQ" style="font-size:1.2rem;margin:.8rem auto;max-width:640px;text-align:center">
<p class="blog-link" style="font-size:1.2rem;margin:0 0 .5rem"><a href="https://www.youtube.com/watch?v=wKV6ibqZChQ" onclick="return acClickPlay(this)">▶ 本場のプロのリクエストに応えてラフマニノフの神曲弾いたらとんでもないリアクションが！？【ストリートピアノ】Rachmaninoff Medley/Рахманинов Попурри</a></p>
<a class="yt-thumb" href="https://www.youtube.com/watch?v=wKV6ibqZChQ" onclick="return acClickPlay(this)" style="display:block;position:relative"><img src="https://i.ytimg.com/vi/wKV6ibqZChQ/hqdefault.jpg" alt="本場のプロのリクエストに応えてラフマニノフの神曲弾いたらとんでもないリアクションが！？【ストリートピアノ】Rachmaninoff Medley/Рахманинов Попурри" style="width:100%;height:auto;display:block;border-radius:8px"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);font-size:3rem;color:#fff;background:rgba(0,0,0,.6);border-radius:50%;width:5rem;height:5rem;line-height:5rem;text-align:center">▶</span></a>
</div>
<div class="yt-click-play" data-vid="z9Mw5H-YRaI" style="font-size:1.2rem;margin:.8rem auto;max-width:640px;text-align:center">
<p class="blog-link" style="font-size:1.2rem;margin:0 0 .5rem"><a href="https://www.youtube.com/watch?v=z9Mw5H-YRaI" onclick="return acClickPlay(this)">▶ Audiophile stereo system.</a></p>
<a class="yt-thumb" href="https://www.youtube.com/watch?v=z9Mw5H-YRaI" onclick="return acClickPlay(this)" style="display:block;position:relative"><img src="https://i.ytimg.com/vi/z9Mw5H-YRaI/hqdefault.jpg" alt="Audiophile stereo system." style="width:100%;height:auto;display:block;border-radius:8px"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);font-size:3rem;color:#fff;background:rgba(0,0,0,.6);border-radius:50%;width:5rem;height:5rem;line-height:5rem;text-align:center">▶</span></a>
</div>
<p class="blog-link" style="font-size:1.2rem"><a href="https://www.facebook.com/reel/1770240353999972" target="_blank" rel="noopener noreferrer">📝 Facebook リール「Audiophile stereo system.」</a></p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://www.electori.co.jp/pass/" target="_blank" rel="noopener noreferrer">📝 PASS Labs　Japan</a></p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://www.passlabs.com/" target="_blank" rel="noopener noreferrer">📝 PASS Labs USA</a></p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://www.electori.co.jp/mcintosh/products/" target="_blank" rel="noopener noreferrer">📝 Macintosh　マッキントッシュ　Japan</a></p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://www.accuphase.co.jp/" target="_blank" rel="noopener noreferrer">📝 Accuphase</a></p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://www.luxman.co.jp/" target="_blank" rel="noopener noreferrer">📝 LUXMAN</a></p>
<p class="blog-link" style="font-size:1.2rem"><a href="https://jp.yamaha.com/" target="_blank" rel="noopener noreferrer">📝 YAMAHA</a></p>
<p class="blog-link" style="font-size:1.2rem"><a href="{BLOG_POST_URL}" target="_blank" rel="noopener noreferrer">📝 {blog_title_ja_esc}</a> / <a href="{BLOG_POST_URL}" target="_blank" rel="noopener noreferrer">{blog_title_en_esc}</a></p>
<p class="blog-link" style="font-size:1.2rem"><a href="{BLOG_POST2_URL}" target="_blank" rel="noopener noreferrer">📝 {blog_title2_ja_esc}</a></p>
<div class="yt-bg-player" id="ytBgPlayer">
<button type="button" class="yt-panel-close" id="ytPanelClose" onclick="acToggleYtPanel(false)">✕ CLOSE</button>
<div class="yt-now-playing">
<a id="ytNowPlaying" href="{default_now_url}" target="_blank" rel="noopener noreferrer" title="タップ → 本物のYouTubeで最初から再生">▶ <span id="ytNowTitle">{default_series_label_esc}</span></a>
<a id="ytNowUrl" href="{default_now_url}" target="_blank" rel="noopener noreferrer" title="タップ → 本物のYouTubeで最初から再生">{default_now_url}</a>
</div>
<iframe id="ytBgIframe" width="100%" height="220" src="https://www.youtube.com/embed/{default_id}?autoplay=1&mute=1&rel=0" title="AUDIOCAFE background video" frameborder="0" allow="autoplay; encrypted-media" allowfullscreen loading="lazy"></iframe>
<div class="yt-series-controls"><button type="button" class="yt-next-btn" onclick="acNextVideo()">⏭ NEXT</button></div>
<div class="yt-series-list">{series_buttons}</div>
</div>
<button type="button" class="yt-panel-open" id="ytPanelOpen" style="display:none" onclick="acToggleYtPanel(true)">▶ OPEN</button>
<script type="application/json" id="ytSeriesData">{series_json}</script>
<script>
(function(){{
  var data = JSON.parse(document.getElementById('ytSeriesData').textContent);
  var cur = 0, idx = 0;
  var iframe = document.getElementById('ytBgIframe');
  var nowPlaying = document.getElementById('ytNowPlaying');
  var nowTitle = document.getElementById('ytNowTitle');
  var nowUrl = document.getElementById('ytNowUrl');
  function embedUrl(id) {{ return 'https://www.youtube.com/embed/' + id + '?autoplay=1&mute=1&rel=0'; }}
  function watchUrl(id) {{ return 'https://www.youtube.com/watch?v=' + id; }}
  function updateActive() {{
    var btns = document.querySelectorAll('.yt-series-btn');
    for (var i = 0; i < btns.length; i++) {{ btns[i].className = 'yt-series-btn' + (i === cur ? ' is-active' : ''); }}
  }}
  // 動画の上のタイトル+URL表示(PHP版`#ytNowPlaying`「タップ→動画/
  // ページを開く」相当)。タイトル・URLのどちらをタップしても、実際の
  // YouTube視聴ページ(タイムスタンプ無し=常に最初から再生)へ新規タブで
  // 遷移する(2026-07-19、ユーザー指摘により動画の上に移動+URL表示も
  // クリック可能化)。
  function setNowPlaying(url, label) {{
    nowPlaying.href = url; nowUrl.href = url;
    nowTitle.textContent = label; nowUrl.textContent = url;
  }}
  window.acPlaySeries = function(i) {{
    var s = data[i];
    if (!s) return;
    cur = i; idx = 0;
    if (s.ids.length > 0) {{
      iframe.src = embedUrl(s.ids[0]);
      setNowPlaying(watchUrl(s.ids[0]), s.label);
    }} else {{
      window.open(s.searchUrl, '_blank', 'noopener');
      setNowPlaying(s.searchUrl, s.label + '（リンク先へ）');
    }}
    updateActive();
  }};
  window.acNextVideo = function() {{
    var s = data[cur];
    if (!s) return;
    if (s.ids.length === 0) {{ window.open(s.searchUrl, '_blank', 'noopener'); return; }}
    if (idx + 1 >= s.ids.length) {{
      // シリーズ内の全動画を再生し終えたら、次のシリーズへ自動的に進む
      // (2026-09-04、ユーザー指示: アルニコ4351を10種類聞いたら次の
      // 岡野昭仁の動画へ移る、という挙動をシリーズ切替の一般ルールとして実装)。
      // 次のシリーズが無い(配列末尾)場合のみ、従来通り同シリーズ内でループする。
      if (cur + 1 < data.length) {{ acPlaySeries(cur + 1); return; }}
      idx = 0;
    }} else {{
      idx = idx + 1;
    }}
    iframe.src = embedUrl(s.ids[idx]);
    setNowPlaying(watchUrl(s.ids[idx]), s.label);
  }};
  window.acToggleYtPanel = function(open) {{
    document.getElementById('ytBgPlayer').style.display = open ? '' : 'none';
    document.getElementById('ytPanelOpen').style.display = open ? 'none' : 'block';
  }};
}})();
</script>
<!-- PHP版のcanvasパーティクルロゴ+YouTube音楽連動(`index.php` 5854行目以降、
     1000行超)の移植。文字の描画式(`danceMotionForChar`の10ジャンル分の
     dx/dy/rot/scale計算式、波モードの二重sin波、爆発/オービットの
     パーティクル物理)はPHP版のものをそのままJSへ移植し、実際にcanvas上で
     文字が動く——CSSキーフレームでの簡略化(旧版)をやめた(2026-07-19、
     ユーザー指摘により再実装)。省略したのはYouTube音量連動(`ytEnergy`、
     このRust版はYouTube Player APIと接続していないため常に0扱い=PHP版の
     「無音時のBPM模擬フォールバック」相当の動きになる)と、波モード専用の
     帆船装飾(`drawSailingShip`、ロゴ本体の動きとは別の付随演出)、Dance AI
     モードの音量パターン解析(`detectGenreFromEnergy`、実音声エネルギー
     入力が無いため10ジャンルを一定間隔でローテーションする方式に簡略化)の
     3点のみ——正直に開示。 -->
<div id="logoWrap"><canvas id="acLogoCanvas"></canvas></div>
<div id="logoModeBar">
<button type="button" class="logo-mode-btn active" data-mode="scroll" onclick="acSetLogoMode('scroll')">🌈 スクロール</button>
<button type="button" class="logo-mode-btn" data-mode="dance" onclick="acSetLogoMode('dance')">💃 Dance</button>
<button type="button" class="logo-mode-btn" data-mode="danceFixed" onclick="acSetLogoMode('danceFixed')">🎯 Dance Fixed</button>
<button type="button" class="logo-mode-btn" data-mode="danceAI" onclick="acSetLogoMode('danceAI')">🤖 Dance AI</button>
<button type="button" class="logo-mode-btn" data-mode="wave" onclick="acSetLogoMode('wave')">🌊 波</button>
<button type="button" class="logo-mode-btn" data-mode="explode" onclick="acSetLogoMode('explode')">💥 爆発</button>
<button type="button" class="logo-mode-btn" data-mode="orbit" onclick="acSetLogoMode('orbit')">🌀 オービット</button>
</div>
<div id="danceFixedBar">
<span class="df-label">Genre →</span>
<button type="button" class="dance-fixed-btn active" data-genre="kabuki" onclick="acSetLogoGenre('kabuki')">🎭 歌舞伎</button>
<button type="button" class="dance-fixed-btn" data-genre="egypt" onclick="acSetLogoGenre('egypt')">🏛 エジプト</button>
<button type="button" class="dance-fixed-btn" data-genre="india" onclick="acSetLogoGenre('india')">🕉 インド</button>
<button type="button" class="dance-fixed-btn" data-genre="hiphop" onclick="acSetLogoGenre('hiphop')">🎤 HIPHOP</button>
<button type="button" class="dance-fixed-btn" data-genre="kpop" onclick="acSetLogoGenre('kpop')">💖 K-POP</button>
<button type="button" class="dance-fixed-btn" data-genre="latin" onclick="acSetLogoGenre('latin')">💃 ラテン</button>
<button type="button" class="dance-fixed-btn" data-genre="orchestra" onclick="acSetLogoGenre('orchestra')">🎻 オーケストラ</button>
<button type="button" class="dance-fixed-btn" data-genre="jazz" onclick="acSetLogoGenre('jazz')">🎷 JAZZ</button>
<button type="button" class="dance-fixed-btn" data-genre="ethnic" onclick="acSetLogoGenre('ethnic')">🪘 民族</button>
<button type="button" class="dance-fixed-btn" data-genre="freestyle" onclick="acSetLogoGenre('freestyle')">✨ フリースタイル</button>
</div>
{LOGO_CANVAS_SCRIPT}
<div class="yt-wp-corner">
<span class="yt-wp-head">🎁 無料 スマホ壁紙コーナー</span>
<span class="yt-wp-hint">画像をタップで原寸表示 →「⬇ 保存」または画像長押しで端末に保存できます。</span>
<div class="yt-wp-grid">{wallpapers}</div>
</div>
<form class="search-wrap" method="get" action="/">
<input type="text" name="q" value="{q_value}" placeholder="Search languages...">
{region_hidden}
<button type="submit">Search</button>
</form>
<div class="pills">{pills}</div>
</div>
<div class="main" id="lang-grid">
{sections}
<div class="nav-box">
<h2>総合ページ(既存PHP側の/aruaru・/aruaru-lady・/rakuten-mobileに相当)</h2>
<ul>{composite_list}</ul>
</div>
<div class="nav-box">
<h2>個別ランキング(Rust版独自の内部ナビゲーション)</h2>
<ul>{list}</ul>
</div>
</div>
<div class="footer">
<p>Copyright &copy; 2025 <a href="/">audiocafe.tokyo</a>, Akiru Akiruno-City Tokyo Japan. All Rights Reserved.</p>
<p>Powered by Google Translate</p>
</div>
</div>
"##
    )
}

async fn ranking_page(params: Params) -> hyper_compat::Response {
    let slug = params.get("slug").unwrap_or("");
    let Some((_, filename, label)) = RANKINGS.iter().find(|(s, _, _)| *s == slug) else {
        return hyper_compat::html_response(
            StatusCode::NOT_FOUND,
            page_shell("見つかりません", "<h1>404</h1><p>未対応のランキングです。</p>"),
        );
    };
    match fetch_cache(filename).await {
        Ok(data) => hyper_compat::html_response(StatusCode::OK, page_shell(label, &render_ranking_body(label, &data))),
        Err(e) => hyper_compat::html_response(
            StatusCode::OK,
            page_shell("エラー", &format!("<h1>取得エラー</h1><p>{}</p>", html_escape(&e))),
        ),
    }
}

async fn composite_page_by_slug(slug: &str) -> hyper_compat::Response {
    // `/rakuten-mobile`はPHP版の専用ページ(917行、独自の見出し・マーケティング
    // 文言・カード構成を持つ)であり、他の複合ページと違い汎用JSONダンプ
    // (`render_value_generic`)では全く別のページになってしまうことが
    // 2026-07-19の監査で判明した。そのためこのslugだけは専用の
    // `render_rakuten_mobile_body`でPHP版の実際の内容を再現する
    // (データ取得自体は既存の`fetch_cache`アーキテクチャのまま)。
    if slug == "rakuten-mobile" {
        return hyper_compat::html_response(
            StatusCode::OK,
            page_shell("📶 楽天モバイル情報 — audiocafe.tokyo", &render_rakuten_mobile_body().await),
        );
    }
    // `/aruaru-lady`もPHP版が独自の見出し・マーケティング文言・CSS装飾
    // (ダークテーマ、`.hero`/`.card`/`.toc`等)を持つ専用ページであり、
    // 汎用JSONダンプでは全く別のページになってしまう(2026-07-19監査、
    // `/rakuten-mobile`と同種の問題)。専用レンダラーで再現する。
    if slug == "aruaru-lady" {
        return hyper_compat::html_response(
            StatusCode::OK,
            page_shell("💃 女性向けお仕事情報｜キャバレー・キャバクラ・TVチャットレディ | audiocafe.tokyo", &render_aruaru_lady_body().await),
        );
    }
    // `/aruaru`もPHP版が独自の見出し・マーケティング文言・CSS装飾
    // (ダークテーマ、doda求人ピックアップ・技術TOP80ランキング・学習サービス
    // TOP50・英会話TOP50等)を持つ専用ページであり、汎用JSONダンプ
    // (旧`COMPOSITE_PAGES`の`aruaru`エントリはキャバクラ/熟女キャバランキング
    // を列挙していたが、これらは実際には`/aruaru-lady`へ移転済みで
    // `/aruaru`側にはもう無い)では全く別のページになってしまう
    // (2026-07-19監査、`/rakuten-mobile`・`/aruaru-lady`と同種の問題)。
    // 専用レンダラーで再現する。
    if slug == "aruaru" {
        return hyper_compat::html_response(
            StatusCode::OK,
            page_shell("aruaru | ITエンジニア案件・求人マッチング | audiocafe.tokyo", &render_aruaru_body().await),
        );
    }
    let Some(page) = COMPOSITE_PAGES.iter().find(|p| p.slug == slug) else {
        return hyper_compat::html_response(
            StatusCode::NOT_FOUND,
            page_shell("見つかりません", "<h1>404</h1><p>未対応のページです。</p>"),
        );
    };
    hyper_compat::html_response(StatusCode::OK, page_shell(page.title, &render_composite_body(page).await))
}

async fn composite_page(params: Params) -> hyper_compat::Response {
    composite_page_by_slug(params.get("slug").unwrap_or("")).await
}

/// PHP側の`build_lists($SEED_URLS)`(テキストリンク・動画リンク・写真の
/// 収集アルゴリズム)を移植した`scraper::build_lists`を呼び出して表示する。
/// 元のPHPはトップページ内で条件付きに埋め込んでいたが、Rust側では
/// 独立したページ`/discover`として切り出した。
async fn discover_page() -> hyper_compat::Response {
    let lists = scraper::build_lists(seed_urls::SEED_URLS).await;

    let video_items: String = lists
        .video_links
        .iter()
        .map(|v| {
            let thumb = scraper::extract_yt_id(&v.url)
                .map(|id| format!(r#"<img src="https://i.ytimg.com/vi/{id}/default.jpg" alt="" style="vertical-align:middle;margin-right:0.5rem;">"#))
                .unwrap_or_default();
            format!(
                r#"<li><a href="{}" target="_blank" rel="noopener noreferrer">{}▶️ [{}] {}</a></li>"#,
                html_escape(&v.url),
                thumb,
                html_escape(scraper::source_name(&v.url)),
                html_escape(&v.title)
            )
        })
        .collect();

    let text_items: String = lists
        .text_links
        .iter()
        .map(|t| format!(r#"<li><a href="{}" target="_blank" rel="noopener noreferrer">{}</a></li>"#, html_escape(&t.url), html_escape(&t.title)))
        .collect();

    let photo_items: String = lists
        .photos
        .iter()
        .map(|p| format!(r#"<li><a href="{}" target="_blank" rel="noopener noreferrer">🖼️ {}</a></li>"#, html_escape(&p.src), html_escape(&p.alt)))
        .collect();

    let body = format!(
        r#"<h1>Discover</h1>
<p>登録済みの記事URL群から、動画リンク・テキストリンク・写真を自動収集しています
(PHP側の<code>build_lists()</code>アルゴリズムのRust移植版、1日キャッシュ)。</p>

<h2>動画リンク ({video_count}件)</h2>
<ul class="linklist">{video_items}</ul>

<h2>記事リンク ({text_count}件)</h2>
<ul class="linklist">{text_items}</ul>

<h2>写真 ({photo_count}件)</h2>
<ul class="linklist">{photo_items}</ul>
"#,
        video_count = lists.video_links.len(),
        text_count = lists.text_links.len(),
        photo_count = lists.photos.len(),
    );
    hyper_compat::html_response(StatusCode::OK, page_shell("Discover | audiocafe.tokyo", &body))
}

fn help_body() -> String {
    let body = r#"<h1>困った時は</h1>

<h2>Google Chromeで「保護されていない通信」と出る場合</h2>
<p>Edge(Windowsの証明書ストアを使用)では正常なのに対し、Chromeは独自の
「Chrome Root Store」という、Windowsとは別の信頼済みルート証明書リストを
持っています。新しいLet's Encryptのルート証明書がまだお使いのChromeの
バージョンに反映されていない可能性があります。</p>
<p><strong>対処法:</strong> Chromeを<code>chrome://settings/help</code>から更新し、
再起動(タスクマネージャーでプロセスが残っていないか確認)してから再度アクセスしてください。</p>

<h2>サイトが表示されない場合(DNS_PROBE_FINISHED_NXDOMAIN等)</h2>
<p>お使いのDNS(特にCloudflareの1.1.1.1)が、ドメインの権威サーバーに
一時的に到達できないことがあります。Google(8.8.8.8)・Quad9(9.9.9.9)
など別のDNSでは問題なく解決できることが多いです。</p>
<p><strong>対処法:</strong> スマホのモバイル回線(Wi-Fiオフ)で試すか、
Windowsの設定(ネットワークとインターネット → プロパティ → DNSサーバーの
割り当てを「手動」)でDNSサーバーを変更してください。
<strong>優先DNS</strong>欄に<code>8.8.8.8</code>のみ、<strong>代替DNS</strong>欄に
<code>8.8.4.4</code>をそれぞれ別々に入力してください
(1つの欄に<code>8.8.8.8 / 8.8.4.4</code>とまとめて入力すると
「無効なエントリ」エラーになります)。「HTTPS経由のDNS」が
「オン(手動テンプレート)」の場合はまず「オフ」にしてから保存を試してください。
スマホでWi-Fi経由の場合は、静的IP化不要で「プライベートDNS」設定だけ
変更できます(設定 → ネットワークとインターネット、機種によっては
Wi-Fi → 詳細設定の中にある場合も → 「プライベートDNS」→
「プロバイダのホスト名」を選択 → <code>dns.google</code> と入力して保存)。
それでも解決しない場合は、単純にDNSの反映待ち(通常数分〜1時間程度)
であることも多いです。</p>
"#;
    page_shell("困った時は", body)
}

#[tokio::main]
async fn main() -> Result<(), std::io::Error> {
    tracing_subscriber::fmt::init();

    // PHP版`--cron-all`(aruaru/index.php 7564〜7649行目)相当。
    // CLI引数で起動された場合はcron処理のみ実行してサーバーは立ち上げず終了する
    // (PHP側は`aruaru_is_cron_request()`でCGI/CLI両対応していたが、Rustバイナリは
    // 常にCLI起動のみを想定すればよいため`std::env::args()`の単純チェックで足りる)。
    if std::env::args().any(|a| a == "--cron-all") {
        cron::run_cron_all().await;
        return Ok(());
    }

    let router = hyper_compat::Router::new()
        .route(
            Method::GET,
            "/",
            Arc::new(|req, _params| {
                Box::pin(async move {
                    let query = hyper_compat::query_params(&req);
                    hyper_compat::html_response(StatusCode::OK, page_shell("AUDIOCAFE | World — Select Your Language", &render_top_body(&query)).replacen("<body>
", &format!("<body>
{}{}{}", HDD_LINKS_HTML, AEON_TRAINING_HTML, TOP_AMP_HTML), 1))
                })
            }),
        )
        .route(Method::GET, "/healthz", Arc::new(|_req, _params| Box::pin(async move { healthz_response() })))
        .route(
            Method::GET,
            "/help",
            Arc::new(|_req, _params| Box::pin(async move { hyper_compat::html_response(StatusCode::OK, help_body()) })),
        )
        .route(Method::GET, "/discover", Arc::new(|_req, _params| Box::pin(async move { discover_page().await })))
        .route(
            Method::GET,
            "/ranking/:slug",
            Arc::new(|_req, params: Params| Box::pin(async move { ranking_page(params).await })),
        )
        .route(
            Method::GET,
            "/page/:slug",
            Arc::new(|_req, params: Params| Box::pin(async move { composite_page(params).await })),
        )
        // PHP版と完全に同じURLパス(`/aruaru/`・`/aruaru-lady/`・
        // `/rakuten-mobile/`)への別名ルート。aruaru.tokyo側のnginxが
        // `Host: audiocafe.tokyo`指定でこれらのパスへ直接プロキシして
        // いるため(`location /aruaru/`等)、`/page/:slug`だけでは
        // 本番切り替え時にaruaru.tokyoが壊れる(404)ことが判明したため
        // 追加した(2026-07-19)。内容は`/page/aruaru`等と同一。
        .route(Method::GET, "/aruaru", Arc::new(|_req, _params| Box::pin(async move { composite_page_by_slug("aruaru").await })))
        .route(Method::GET, "/aruaru-lady", Arc::new(|_req, _params| Box::pin(async move { composite_page_by_slug("aruaru-lady").await })))
        .route(Method::GET, "/rakuten-mobile", Arc::new(|_req, _params| Box::pin(async move { composite_page_by_slug("rakuten-mobile").await })))
        // PHP版の「言語カード＆/top 要約LIST」(`mixlist=1`、`index.php`
        // 1590行目)相当。全カードの主要リンク(YouTube/Blog等)をタイトル付きで
        // 整列表示する専用画面(2026-07-19、ユーザー要望により新設)。
        .route(Method::GET, "/summary", Arc::new(|_req, _params| Box::pin(async move { hyper_compat::html_response(StatusCode::OK, page_shell("AUDIOCAFE | 言語カード要約 — Link Summary", &render_summary_body())) })));

    tracing::info!("audiocafe-tokyo-server listening on 127.0.0.1:4400");
    let (_, handle) = hyper_compat::serve(router, "127.0.0.1:4400".parse().unwrap()).await?;
    handle.await.map_err(|e| std::io::Error::other(e))?;
    Ok(())
}
