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
use serde_json::Value;
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

const TOP_VIDEO_HTML: &str = r#"<section class="top-news" style="max-width:640px;margin:1rem auto;text-align:center;">
<div style="max-width:960px;margin:0 auto 1.6rem;padding:.8rem 1rem;border:1px solid rgba(128,128,128,.5);border-radius:10px;text-align:left;line-height:1.8;">
<p style="margin:0 0 .7rem;font-size:1.2rem;">私、石　塚　正　浩は、ウグイス鳴くよ（794年）平安京で学校の教科書に出てくる和気清麻呂と言う人物の末裔で、秦氏の末裔でもあり、中国人ではなく、古代シュメールやユダヤ人代表として、古代中国を支配牛耳って秦の始皇帝としても君臨して、その先祖はエジプトでファラオでもあり、その証拠としてテレパシストなのです。その先祖は、アブラハムやノアのファミリーです。</p>
<p lang="en" style="margin:0;font-size:1.15rem;opacity:.9;">I, Masahiro Ishizuka, am a descendant of Wake no Kiyomaro, the person who appears in school textbooks about Heian-kyo, founded in 794 (the "Uguisu nakuyo" mnemonic for 794), and also a descendant of the Hata clan. I am not Chinese; as a representative of the ancient Sumerians and the Jewish people, I ruled and controlled ancient China and reigned as Qin Shi Huang, the first emperor of Qin, and my ancestors were also pharaohs in Egypt. As proof of this, I am a telepathist. My ancestors are the families of Abraham and Noah.</p>
<p style="margin:.9rem 0 .7rem;font-size:1.2rem;">そして、旧宮家代表として日本と世界の天皇として、世界の大統領として、アメリカの次の大統領として、イスラエルの次の首相として、中国の次の首席として、韓国の次の大統領として、北朝鮮の最高指導者として、ロシアの次の大統領として、ウクライナの次の大統領として、イランの次の大統領として、日本の次の首相として兼任する為には、世界的、国際的な信頼を得てこそ実現するものだと思われます。</p>
<p lang="en" style="margin:0;font-size:1.15rem;opacity:.9;">And in order to serve concurrently as a representative of the former imperial branch families, as the Emperor of Japan and of the world, as the world's president, as the next President of the United States, the next Prime Minister of Israel, the next Chairman of China, the next President of South Korea, the Supreme Leader of North Korea, the next President of Russia, the next President of Ukraine, the next President of Iran and the next Prime Minister of Japan, I think this can only be realized by earning worldwide, international trust.</p>
</div>
<details open style="max-width:960px;margin:0 auto 1.6rem;padding:.8rem 1rem;border:1px solid rgba(128,128,128,.5);border-radius:10px;text-align:left;line-height:1.8;">
<summary style="font-size:1.3rem;font-weight:700;cursor:pointer;">カレーとラーメンの作り方 / How to Make Curry and Ramen</summary>
<div style="font-size:1.1rem;">
<h3 style="font-size:1.25rem;margin:1.4rem 0 .5rem;">カツカレー・カキフライカレー・コロッケカレー・チャーシューラーメンの作り方</h3>
<p style="margin:0 0 .7rem;">サクサクの揚げ物を乗せた3種のカレーと、ホロホロに仕込んだ自家製チャーシューラーメンの作り方です。特にチャーシューの仕込みと、揚げ物のサクサク感を出すコツを紹介します。</p>
<h3 style="font-size:1.25rem;margin:1.4rem 0 .5rem;">1. カレールウ(2パターン)</h3>
<p style="margin:.9rem 0 .4rem;font-weight:700;">【Aパターン】スパイスから作る本格カレー(4人分)</p>
<ul style="margin:0 0 .8rem;padding-left:1.6rem;"><li>スパイス:クミン(大さじ1)、コリアンダー(大さじ1)、ターメリック(小さじ1)、チリペッパー(小さじ1/2〜お好みで)</li><li>鍋に油とみじん切りの生姜・にんにく(各1片分)を入れ、弱火で炒めて香りを立たせます。</li><li>みじん切りの玉ねぎ(2個)を加え、強めの火で飴色になるまでしっかり炒めます(甘みとコクを出します)。</li><li>トマト缶(200g)を加え、水分が抜けてペースト状になるまで炒めます。</li><li>火を極弱火にし、スパイスと塩(小さじ1.5)を加えて1分ほど炒め合わせます。</li><li>水(600ml)、隠し味に醤油(小さじ1)とウスターソース(大さじ1)を加え、弱火で20分ほど煮込んで完成です。</li></ul>
<p style="margin:.9rem 0 .4rem;font-weight:700;">【Bパターン】市販のルウを格上げする裏ワザ</p>
<ul style="margin:0 0 .8rem;padding-left:1.6rem;"><li>玉ねぎの「時間差」炒め:半分は飴色になるまで炒めて「コク」を出し、もう半分はザク切りにして中盤で加え「食感と甘み」を残します。</li><li>トリプル隠し味:ルウを溶かした後、インスタントコーヒー(小さじ1/2)、ウスターソース(大さじ1)、バター(10g)を加えると、一晩寝かせたような深みが出ます。</li></ul>
<p style="margin:.9rem 0 .4rem;font-weight:700;">香りと辛さを際立たせるプロ技</p>
<ul style="margin:0 0 .8rem;padding-left:1.6rem;"><li>テンパリング:仕上げの直前に、小さなフライパンで油(またはバター)大さじ1を熱し、クミンシードやカレーパウダーを弱火でシュワシュワと炒め、香りを移した油をカレーにジュッと加えます。</li><li>チリペッパーは、最初に玉ねぎと炒めると「じんわりした辛さ」、火を止める直前に加えると「シャープな辛さ」になります。辛党なら最初と最後の二段階で。</li><li>仕上げにガラムマサラ(小さじ1)とレモン汁(小さじ1)を加えると、スパイスの輪郭がはっきりします。</li></ul>
<h3 style="font-size:1.25rem;margin:1.4rem 0 .5rem;">1-2. ブロックのカレールウから作る基本のカレー(4人分)</h3>
<p style="margin:0 0 .7rem;">市販のブロック(固形)のカレールウを使った、基本のカレーの作り方です。箱の表示(水の量・ルウの量)を優先してください。</p>
<p style="margin:.9rem 0 .4rem;font-weight:700;">材料</p>
<ul style="margin:0 0 .8rem;padding-left:1.6rem;"><li>肉(豚こま切れ・鶏もも・牛のいずれか)300g、玉ねぎ2個、にんじん1本、じゃがいも2個</li><li>油 大さじ1、水 約700ml(箱の表示に従う)</li><li>ブロックのカレールウ 1箱(表示の4〜5皿分)</li></ul>
<p style="margin:.9rem 0 .4rem;font-weight:700;">作り方</p>
<ol style="margin:0 0 .8rem;padding-left:1.6rem;"><li>玉ねぎはくし切り、にんじんは乱切り、じゃがいもは大きめに切り、水にさらします。肉は食べやすい大きさに切ります。</li><li>鍋に油を熱し、肉を炒めます。色が変わったら、玉ねぎ、にんじん、じゃがいもの順に加えて炒めます(玉ねぎは時間をかけて炒めるほど甘みとコクが出ます)。</li><li>水を加え、沸騰したらアクを取り、弱火で15〜20分、野菜がやわらかくなるまで煮ます。</li><li>火を止めて、ルウを割り入れてよく溶かします(火を止めてから入れると、ダマになりません)。</li><li>弱火に戻し、焦げないように混ぜながら5〜10分、とろみがつくまで煮て完成です。</li></ol>
<p style="margin:0 0 .7rem;">コツ:隠し味(インスタントコーヒー、ウスターソース、バター)や、一晩寝かせると深みが出ます(上の「Bパターン」を参照)。</p>
<h3 style="font-size:1.25rem;margin:1.4rem 0 .5rem;">2. 揚げ物をサクサクに仕上げる3大鉄則</h3>
<ul style="margin:0 0 .8rem;padding-left:1.6rem;"><li>バッター液:卵1個・小麦粉大さじ3・水大さじ1を混ぜたもの。衣がよく密着し、剥がれにくく、水分が抜けてサクサクになります。</li><li>生パン粉:乾燥パン粉より水分があり、軽くザクザクした食感になります。</li><li>二度揚げ:160℃の低温で中まで火を通し、一度引き上げて3分余熱で休ませ、仕上げに180〜190℃で30秒〜1分一気に揚げます。冷めてもサクサクが続きます。</li></ul>
<p style="margin:.9rem 0 .4rem;font-weight:700;">① カツカレー</p>
<ul style="margin:0 0 .8rem;padding-left:1.6rem;"><li>豚ロース肉の赤身と脂身の境目の筋に包丁の先で切れ目を入れ(縮み防止)、塩コショウを振ります。</li><li>バッター液、生パン粉の順につけ、160℃で3〜4分揚げて3分休ませ、180℃(または190℃)で仕上げ揚げします。</li><li>ご飯にカレーをかけ、切ったカツを上に乗せます(衣にルーをかけすぎないのがサクサクを保つコツ)。</li></ul>
<p style="margin:.9rem 0 .4rem;font-weight:700;">② カキフライカレー</p>
<ul style="margin:0 0 .8rem;padding-left:1.6rem;"><li>加熱用のカキを薄い塩水か片栗粉でもみ洗いして流水で流し、ペーパータオルで水分を完全に拭き取ります(水気が残るとサクサクになりません)。</li><li>小麦粉、溶き卵、パン粉の順に薄く均一につけ、180℃の油で1分半〜2分、キツネ色になるまで一気に揚げます(火の通しすぎに注意)。</li><li>揚げたてをカレーライスに乗せます。ウスターソースを少し垂らしても絶品です。</li></ul>
<p style="margin:.9rem 0 .4rem;font-weight:700;">③ コロッケカレー</p>
<ul style="margin:0 0 .8rem;padding-left:1.6rem;"><li>茹でて潰したじゃがいもに、炒めたひき肉と玉ねぎを混ぜます。</li><li>タネは冷蔵庫で完全に冷まして固めてから成形します(温かいままだと油の中で破裂します)。</li><li>小麦粉、溶き卵、パン粉を隙間なくつけ、180℃の油で衣を固めるように2〜3分、きつね色になったらすぐに引き上げます。</li></ul>
<p style="margin:.9rem 0 .4rem;font-weight:700;">油切れを良くして胃もたれを防ぐコツ</p>
<ul style="margin:0 0 .8rem;padding-left:1.6rem;"><li>引き上げる直前の10秒は油の温度を180〜190℃に上げ、内部の水蒸気で衣に染み込もうとする油を弾き飛ばします。</li><li>引き上げる時はトングや箸で「縦」に持ち、油の上で4〜5回優しく上下に振って空気に触れさせます。</li><li>キッチンペーパーに寝かせず、網(バット)の上に少し重ねて「立てて」並べます(油戻りを防ぎます)。</li><li>油は米油(またはキャノーラ油)にすると、衣が軽く仕上がり、胃もたれが軽くなります。</li><li>同じ油で揚げる順番は ①コロッケ → ②トンカツ → ③カキフライ。カキは油に磯の香りと水分が移りやすいので最後にします。</li></ul>
<h3 style="font-size:1.25rem;margin:1.4rem 0 .5rem;">3. チャーシュー(圧力鍋で時短・約40分)</h3>
<p style="margin:0 0 .7rem;">材料:豚バラブロック500g、長ネギの青い部分、生姜スライス3枚。煮汁(カエシ):醤油150ml、みりん80ml、酒80ml、砂糖大さじ3、にんにく1片。</p>
<ol style="margin:0 0 .8rem;padding-left:1.6rem;"><li>焼き付け:フライパンを強火で熱し、豚ブロックの表面全体に焼き色をつけます(旨味を閉じ込め、香ばしさが増します)。</li><li>加圧(下茹で):圧力鍋に肉、ネギ、生姜、肉が被るくらいの水を入れ、圧力がかかったら弱火にして15分加圧します。火を止め、圧力が完全に抜けるまで放置します。</li><li>味付け:肉を取り出し(下茹で汁はラーメンスープに使うので取っておく)、別の鍋かフライパンで煮汁の材料を沸騰させます。</li><li>絡め煮:肉を入れ、スプーンで煮汁をかけながら、弱火で10分ほど転がして煮詰めます。</li><li>急冷(重要):ジッパー付き袋に肉と煮汁を入れ、氷水か冷蔵庫で急冷します。温度が下がる時に味が中まで染み込み、肉が引き締まって薄切りしやすくなります。</li></ol>
<h3 style="font-size:1.25rem;margin:1.4rem 0 .5rem;">4. ラーメンスープ(2つのアプローチ)</h3>
<p style="margin:0 0 .7rem;">圧力鍋に残った下茹で汁(豚の旨味が溶け出した豚白湯ベース)を使い、お好みのスタイルに仕上げます。圧力鍋は洗わず、そのままスープに使えます。</p>
<p style="margin:.9rem 0 .4rem;font-weight:700;">【A:醤油清湯(あっさり)】</p>
<ol style="margin:0 0 .8rem;padding-left:1.6rem;"><li>下茹で汁を一度冷まし、表面に固まった白いラード(脂)を8割ほどお玉ですくい取ります。</li><li>スープ(約300〜400ml)に、水100ml、昆布・かつおだし(または和風だしの素小さじ1)、鶏ガラスープの素小さじ1を加え、沸騰させずに弱火で温めます(濁らせないコツ)。</li><li>どんぶりにチャーシューの煮汁(カエシ)大さじ2を入れ、スープを注ぎます。</li></ol>
<p style="margin:.9rem 0 .4rem;font-weight:700;">【B:魚介豚骨風(濃厚)】</p>
<ol style="margin:0 0 .8rem;padding-left:1.6rem;"><li>下茹で汁を、表面の脂ごと強火で5分ほどガンガン沸騰させてかき混ぜ、乳化させます(とろみのある豚白湯になります)。</li><li>魚粉(かつお粉)大さじ1(またはサバ節・煮干しの粉末)と鶏ガラスープの素小さじ1を加えます。</li><li>どんぶりにカエシ大さじ2.5〜3を入れ、スープを注ぎます。お好みでおろしにんにくを少し添えるとさらにパンチが増します。</li></ol>
<p style="margin:0 0 .7rem;">仕上げ:茹でた麺を入れ、急冷して切ったチャーシュー、ネギ、メンマ、煮卵を乗せて完成です。</p>
<h3 style="font-size:1.25rem;margin:1.4rem 0 .5rem;">4-2. 豚骨ラーメン 3種類のレシピ(博多風・熊本風・豚骨醤油風)</h3>
<p style="margin:0 0 .7rem;">3種類とも、同じ「豚骨スープ」を土台にして、タレ・香味油・麺・トッピングで味を変えます。分量と時間は目安なので、お好みで調整してください(実際に作って確かめたものではありません)。</p>
<p style="margin:.9rem 0 .4rem;font-weight:700;">【共通】豚骨スープの土台(圧力鍋・約4杯分)</p>
<ol style="margin:0 0 .8rem;padding-left:1.6rem;"><li>豚ゲンコツ(または背骨)約1kgを、沸騰した湯で5分ほど下茹でします。流水で血やぬめりをよく洗い流します(臭みを抜く大切な工程)。</li><li>圧力鍋に骨、長ねぎの青い部分、生姜、にんにく、玉ねぎ1/2個を入れ、水(約2L。鍋の最大量の2/3以下)を加えて60分加圧し、圧力が自然に抜けるまで待ちます。</li><li>蓋を開けて強火にし、10〜20分ほど沸騰させてかき混ぜ、脂と水分を乳化させて白く濁らせます。骨をつぶして旨味を出し、ザルで漉します。</li><li>注意:豚骨は泡立ちやすいので、必ず圧力鍋の取扱説明書の最大量・使用できる食材を守り、泡が弁を詰まらせないようにしてください。圧力鍋を使わない場合は、強火で沸騰を保ちながら4〜6時間煮出します(水が減ったら熱湯を足します)。</li></ol>
<p style="margin:.9rem 0 .4rem;font-weight:700;">① 博多風(あっさり白濁・細麺)</p>
<ul style="margin:0 0 .8rem;padding-left:1.6rem;"><li>カエシ:どんぶり1杯につき、塩小さじ1/2、醤油小さじ1、鶏ガラスープの素ひとつまみ、白コショウ少々。</li><li>スープ300〜350mlを温めてカエシと合わせ、細いストレート麺を固めに茹でて入れます。</li><li>トッピング:チャーシュー、きくらげ、ねぎ、紅しょうが、辛子高菜、ごま。</li><li>替え玉:麺だけを別に茹でて追加します。スープが薄まらないよう、カエシ少々を足します。</li></ul>
<p style="margin:.9rem 0 .4rem;font-weight:700;">② 熊本風(マー油・ニンニクの香り)</p>
<ul style="margin:0 0 .8rem;padding-left:1.6rem;"><li>マー油:ごま油大さじ3とみじん切りのにんにく2片を小鍋に入れ、弱火でじっくり加熱し、茶色〜黒っぽくなったら火を止めます(焦がしすぎると苦くなるので注意)。</li><li>カエシ:どんぶり1杯につき、醤油大さじ1、塩ひとつまみ、鶏ガラスープの素小さじ1/2。</li><li>スープ300ml を合わせ、中太のストレート麺を入れます。マー油を小さじ1〜2垂らし、揚げにんにく(チップ)を散らします。</li><li>トッピング:チャーシュー、煮卵、きくらげ、ねぎ。</li></ul>
<p style="margin:.9rem 0 .4rem;font-weight:700;">③ 豚骨醤油風(濃厚・家系風)</p>
<ul style="margin:0 0 .8rem;padding-left:1.6rem;"><li>鶏油(チーユ):鶏皮をフライパンで弱火でじっくり炒めて出た脂を使います(ない場合はラードやごま油で代用)。</li><li>カエシ:どんぶり1杯につき、醤油大さじ2、みりん大さじ1/2、砂糖小さじ1/2、鶏油大さじ1。</li><li>スープ300mlを温めて注ぎ、太めのストレート麺(やわらかめ〜普通)を入れます。</li><li>トッピング:チャーシュー、ほうれん草、海苔3枚、ねぎ、うずらの卵。お好みで「麺かため・味濃いめ・油多め」に調整できます。</li></ul>
<h3 style="font-size:1.25rem;margin:1.4rem 0 .5rem;">4-3. 魚介豚骨つけ麺(つけラーメン)2種類のレシピ</h3>
<p style="margin:0 0 .7rem;">豚骨スープ(4-2の土台)に魚介だしを合わせた「魚介豚骨」のつけ麺を、濃厚タイプとあっさりタイプの2種類で紹介します。分量と時間は目安なので、お好みで調整してください(実際に作って確かめたものではありません)。</p>
<p style="margin:.9rem 0 .4rem;font-weight:700;">【共通】魚介だし(1〜2人分)</p>
<ol style="margin:0 0 .8rem;padding-left:1.6rem;"><li>水300mlに、煮干し10g(頭とワタを取る)、サバ節10g、昆布5cmを入れ、30分ほど浸けます。</li><li>弱火にかけ、沸騰直前に昆布を取り出し、煮干しとサバ節を5分ほど煮ます。ザルで漉して魚介だし(約250ml)にします。</li></ol>
<p style="margin:.9rem 0 .4rem;font-weight:700;">【共通】麺の準備(つけ麺は水で締める)</p>
<ul style="margin:0 0 .8rem;padding-left:1.6rem;"><li>太めの麺200〜250gを表示どおりに茹で、冷水でしっかり洗って締め、ぬめりを取って水気をよく切ります(締めるほどコシが出ます)。お好みで少量のごま油を和えると麺がくっつきにくくなります。</li></ul>
<p style="margin:.9rem 0 .4rem;font-weight:700;">① 濃厚 魚介豚骨つけ麺(ダブルスープ)</p>
<ul style="margin:0 0 .8rem;padding-left:1.6rem;"><li>つけ汁(1人分):豚骨スープ100ml、魚介だし100ml、カエシ(醤油大さじ1.5、みりん大さじ1/2、砂糖小さじ1/2)、魚粉小さじ2、チャーシューの煮汁大さじ1/2。小鍋で5分ほど煮詰めて濃度を出します。</li><li>冷たい麺を、熱いつけ汁にくぐらせて食べます。つけ汁が冷めやすいので、小鍋のまま温めながら、または熱したどんぶりに入れて出します。</li><li>トッピング:チャーシュー、メンマ、煮卵、刻みねぎ、海苔、お好みで刻み玉ねぎ。</li><li>割りスープ:食べ終わりに、温めた魚介だし(または豚骨スープ)を残ったつけ汁に注いで薄め、飲み干します。</li></ul>
<p style="margin:.9rem 0 .4rem;font-weight:700;">② あっさり 魚介つけ麺(酸味とラー油)</p>
<ul style="margin:0 0 .8rem;padding-left:1.6rem;"><li>つけ汁(1人分):魚介だし150ml、豚骨スープ50ml、カエシ(醤油大さじ2、みりん大さじ1、砂糖大さじ1/2)を温め、火を止めて酢大さじ1とラー油少々を加えます。</li><li>中太の麺を冷水で締め、そのまま添えます。冷たいつけ汁でも、温かいつけ汁でも作れます。</li><li>トッピング:鶏チャーシュー(または豚チャーシュー)、メンマ、刻みねぎ、白ごま、柚子皮またはレモン。</li><li>さっぱり食べたい時や、夏の冷やしつけ麺におすすめです。</li></ul>
<h3 style="font-size:1.25rem;margin:1.4rem 0 .5rem;">5. 絶品味付け玉子(煮卵)とメンマ</h3>
<p style="margin:.9rem 0 .4rem;font-weight:700;">味付け玉子(チャーシューの煮汁で)</p>
<ol style="margin:0 0 .8rem;padding-left:1.6rem;"><li>冷蔵庫から出したての卵(M〜L)を、沸騰した湯にお玉で静かに入れます。とろっとした半熟は6分30秒、ねっとりした半熟は7分15秒(最初の2分間、箸で転がすと黄身が真ん中に来ます)。</li><li>すぐに氷水で完全に冷やし、殻全体に細かくひびを入れ、水の中で剥きます。</li><li>ジッパー袋に卵と冷ましたチャーシューの煮汁をひたひたに入れます(浮く場合はキッチンペーパーを被せます)。</li><li>冷蔵庫で最低4時間、ベストは一晩(約12時間)寝かせます。ラーメンにも、カレーの横にも合います。</li></ol>
<p style="margin:.9rem 0 .4rem;font-weight:700;">極上メンマ</p>
<ol style="margin:0 0 .8rem;padding-left:1.6rem;"><li>水煮メンマ(約100〜150g)をサッと湯通しして、水煮臭さを抜きます。</li><li>ごま油大さじ1/2(お好みで輪切り唐辛子少々)でメンマを強火で炒め、水分を飛ばします。</li><li>鶏ガラスープの素小さじ1/2、水50ml、酒大さじ1、みりん大さじ1/2、チャーシューの煮汁大さじ1を加え、弱火で汁気がなくなるまで煮詰め、白コショウを振ります。</li></ol>
<h3 style="font-size:1.25rem;margin:1.4rem 0 .5rem;">6. 効率のよい全体の流れ(コンロ3口)</h3>
<p style="margin:0 0 .7rem;">器具の目安:圧力鍋3.0〜4.5L、揚げ物鍋(または深めのフライパン)直径20〜24cm、カレー鍋20cm程度。</p>
<ol style="margin:0 0 .8rem;padding-left:1.6rem;"><li>【最初に】豚バラ肉を焼き、圧力鍋で15分加圧(下茹で)。</li><li>【加圧中】カレーの玉ねぎを飴色になるまで炒め、カレー(スパイスまたはルウ)を煮込んでおく。</li><li>【圧が抜けたら】チャーシューを取り出して煮汁で10分絡め煮し、ジッパー袋に入れて氷水で急冷する。圧力鍋の下茹で汁は、そのままラーメンスープに仕上げる。</li><li>【食べる直前】揚げ物を ①コロッケ → ②トンカツ → ③カキフライ の順に揚げる。</li><li>【仕上げ】カレーにテンパリングしたスパイスを加え、ご飯に盛って揚げ物を乗せる。ラーメンは麺を茹で、スープ、カエシ、冷えて切りやすくなったチャーシューを乗せる。</li></ol>
<h3 style="font-size:1.25rem;margin:1.4rem 0 .5rem;">7. 福神漬けの代わりに:紫玉ねぎとパプリカのアチャール(インド風の即席漬け)</h3>
<p style="margin:0 0 .7rem;">スパイスカレーの専門店で添えられる付け合わせです。シャキシャキした食感と爽やかな酸味が、カツや揚げ物の油っぽさを切り替えてくれます。作り方の流れ(薄切り → 塩もみ・水さらし → 調味料と和える)は下記の参考ページを基にしています。分量は目安なので、お好みで調整してください。</p>
<p style="margin:.9rem 0 .4rem;font-weight:700;">材料(作りやすい量)</p>
<ul style="margin:0 0 .8rem;padding-left:1.6rem;"><li>紫玉ねぎ 1/2個、パプリカ(赤または黄)1/2個</li><li>塩 小さじ1/2(塩もみ用)</li><li>サラダ油 大さじ1(あればマスタードオイル)</li><li>レモン汁 大さじ1、砂糖 ひとつまみ</li><li>クミンパウダー 小さじ1/2、ターメリック 小さじ1/4、チリパウダー(またはパプリカパウダー)小さじ1/4(辛くしたくない場合はパプリカパウダー)</li></ul>
<p style="margin:.9rem 0 .4rem;font-weight:700;">作り方</p>
<ol style="margin:0 0 .8rem;padding-left:1.6rem;"><li>紫玉ねぎとパプリカを3〜5mmの薄切りにします。</li><li>塩をふってよくもみ、5分ほどおいて、水にさらして辛みを抜き、水気をしっかり絞ります(パプリカは水にさらすと、紫玉ねぎの色が移りにくくなります)。</li><li>小さなフライパンで油を温め、クミン・ターメリック・チリパウダーを弱火で香りが立つまで10〜20秒炒めます(焦がさないように)。</li><li>ボウルに野菜、3の油ごとスパイス、レモン汁、砂糖を入れて和えます。</li><li>30分以上冷蔵庫で味をなじませて完成です(冷蔵で2〜3日が目安)。</li></ol>
<p style="margin:0 0 .7rem;font-size:.9rem;">参考:<a href="https://macaro-ni.jp/164460" target="_blank" rel="noopener noreferrer">macaroni</a>、<a href="https://www.city.shinjuku.lg.jp/kenkou/kenko01_000001_00031.html" target="_blank" rel="noopener noreferrer">新宿区</a>、<a href="https://cookpad.com/recipe/4990071" target="_blank" rel="noopener noreferrer">Cookpad</a></p>
<h3 style="font-size:1.25rem;margin:1.4rem 0 .5rem;">8. 油とお肉・食材のこだわり</h3>
<p style="margin:0 0 .7rem;">フライで使う油は、圧搾式の米油が最高で、キャノーラ油などは、身体に悪いと言われております。</p>
<p style="margin:0 0 .7rem;">日本に輸入されている調理用のお肉も、アメリカン・ビーフやオージー・ビーフなどほとんどの外国産のお肉は、ホルモン剤と抗生物質を大量に使用して家畜を育てているケースが多く、食べると乳がんや子宮がんなどの原因になるのではないかと心配されていますので、なるべくお肉やお魚やお野菜などは日本産を使ったり、それらを家庭菜園や家畜も地産地消で自分で食べる分と乳牛を飼いながら、大学に行かなくても、講座を受講してTESTに合格すれば取得出来る、人工授精師の資格を取得して、和牛の受精卵を和牛より体格が良くて身体が大きいので子牛を出産しやすい乳牛の子宮に移植して乳牛に和牛と乳牛の子牛を出産して出荷して、乳製品と両方出荷出来るので、酪農家がやっと黒字に成れるのです。</p>
<p style="margin:0 0 .7rem;">北海道の様な広大な土地ならデコボコな土地を平らにフラットに出来ればゴルフ場を経営出来たり、牛や馬などを放牧しているとそのフンが大量に地面に落ちて、冬の雪が溶ける時に混ざって片づけるのが大変なので、下水溝をしっかりと作り、家畜のフンや砂利や石や土や泥などを流し込んでも畑の肥料などとしてリサイクル出来る施設をしっかりと建設するべきだと思われます。</p>
<p style="margin:0 0 .7rem;">なお、広大な土地があれば、空港を建設したり、半導体工場やAIデーターセンターと発電所は2026年ではSETで建設されたり、ハウスメーカーが集客用モデルルームとして大型AUDIO　CAFE(Jazz喫茶）を建設して中は、ショールームとなり、中で不動産会社や建設会社や工務店や航空機や新車と中古車の自動車売買やPCやタブレットやスマホの販売やレンタル、リースやAFLACやNISSAY(日本生命）やSONY損保などの保険も扱うケースが増えている様です。</p>
<p style="margin:0 0 .7rem;">航空機と自動車とバイクなどの板金修理も同時に経営するべきだと思います。スーパーカーのブガッティの例を上げますと、中古のブガッティを5千万円で購入直後に原価数百円のパーツの交換の修理代金が5千万円とぼったくられるのも良くある話の一つだそうです。長期的で国際的な信頼を得るには、世界中の航空機メーカーやBMWやベンツやスーパーカーを日本の財閥財団で買収して、Made in Japan 航空機やスーパーカーや高級車や高級バイクをLEXUS・ブランドとして、もしくはNISSANの高級ブランドのINFINITIやHONDAやMAZDAも高級ブランドを立ち上げて一緒に販売やレンタルやリースなどをしていくべきだと思われます。</p>
<p style="margin:0 0 .7rem;">更に追加する話として、YouTubeにて、エジプトのピラミッドの地下から巨大なUFOと同時にその操作マニュアルや設計書が日本語の石板で発見されたニュースがあり、航空機や自動車やバイクや船舶やクルーザーなどのエンジンのあり方にも改革が起きている最中のようですので、そちらにも、国策としても財閥財団の予算としての配分も考慮しておくべきだと思います。</p>
<div lang="en" style="margin-top:1.6rem;padding-top:1rem;border-top:1px solid rgba(128,128,128,.4);">
<h3 style="font-size:1.25rem;margin:1.4rem 0 .5rem;">How to Make Katsu Curry, Fried Oyster Curry, Croquette Curry and Chashu Ramen</h3>
<p style="margin:0 0 .7rem;">Three curries topped with crispy fried food, and a ramen with fall-apart homemade chashu. The focus is on preparing the chashu and on tips for making fried food crispy.</p>
<h3 style="font-size:1.25rem;margin:1.4rem 0 .5rem;">1. Curry Roux (Two Ways)</h3>
<p style="margin:.9rem 0 .4rem;font-weight:700;">[Pattern A] Authentic curry from spices (serves 4)</p>
<ul style="margin:0 0 .8rem;padding-left:1.6rem;"><li>Spices: cumin (1 tbsp), coriander (1 tbsp), turmeric (1 tsp), chili pepper (1/2 tsp, or to taste).</li><li>Put oil and minced ginger and garlic (1 clove each) in a pot and stir-fry over low heat until fragrant.</li><li>Add minced onions (2) and fry over fairly high heat until caramelized (this builds sweetness and body).</li><li>Add canned tomatoes (200 g) and fry until the moisture is gone and it becomes a paste.</li><li>Turn the heat to very low, add the spices and salt (1.5 tsp), and fry together for about 1 minute.</li><li>Add water (600 ml) and, as hidden flavors, soy sauce (1 tsp) and Worcestershire sauce (1 tbsp). Simmer over low heat for about 20 minutes.</li></ul>
<p style="margin:.9rem 0 .4rem;font-weight:700;">[Pattern B] Tricks to upgrade store-bought roux</p>
<ul style="margin:0 0 .8rem;padding-left:1.6rem;"><li>Staggered onion frying: fry half of the onions until caramelized for body, and add the other half, roughly chopped, midway to keep texture and sweetness.</li><li>Triple hidden flavors: after dissolving the roux, add instant coffee (1/2 tsp), Worcestershire sauce (1 tbsp) and butter (10 g). It tastes as deep as if it had rested overnight.</li></ul>
<p style="margin:.9rem 0 .4rem;font-weight:700;">Professional techniques for aroma and heat</p>
<ul style="margin:0 0 .8rem;padding-left:1.6rem;"><li>Tempering: just before finishing, heat 1 tbsp of oil (or butter) in a small frying pan, fry cumin seeds or curry powder over low heat until they sizzle, and pour the aromatic oil into the curry.</li><li>Chili pepper fried with the onions at the start gives a deep, slowly building heat; added just before turning off the heat it gives a sharp heat. If you like it hot, add it at both times.</li><li>At the end, add garam masala (1 tsp) and lemon juice (1 tsp) so the outline of the spices stands out.</li></ul>
<h3 style="font-size:1.25rem;margin:1.4rem 0 .5rem;">1-2. Basic Curry Made from Block Curry Roux (Serves 4)</h3>
<p style="margin:0 0 .7rem;">A basic curry made with commercial block (solid) curry roux. Follow the amounts of water and roux shown on the box first.</p>
<p style="margin:.9rem 0 .4rem;font-weight:700;">Ingredients</p>
<ul style="margin:0 0 .8rem;padding-left:1.6rem;"><li>300 g meat (pork pieces, chicken thigh or beef), 2 onions, 1 carrot, 2 potatoes</li><li>1 tbsp oil, about 700 ml water (follow the box)</li><li>1 box of block curry roux (the 4–5 servings shown on the box)</li></ul>
<p style="margin:.9rem 0 .4rem;font-weight:700;">How to make it</p>
<ol style="margin:0 0 .8rem;padding-left:1.6rem;"><li>Cut the onions into wedges, the carrot into chunks and the potatoes into large pieces, and soak the potatoes in water. Cut the meat into bite-size pieces.</li><li>Heat the oil in a pot and fry the meat. When it changes color, add the onions, carrot and potatoes in that order and fry (the longer you fry the onions, the more sweetness and body they give).</li><li>Add the water, skim the scum when it boils, and simmer over low heat for 15–20 minutes until the vegetables are soft.</li><li>Turn off the heat, break in the roux and dissolve it well (adding it after turning off the heat prevents lumps).</li><li>Return to low heat and simmer for 5–10 minutes, stirring so it does not burn, until it thickens. Done.</li></ol>
<p style="margin:0 0 .7rem;">Tip: hidden flavors (instant coffee, Worcestershire sauce, butter) or resting it overnight add depth (see "Pattern B" above).</p>
<h3 style="font-size:1.25rem;margin:1.4rem 0 .5rem;">2. The 3 Golden Rules for Crispy Fried Food</h3>
<ul style="margin:0 0 .8rem;padding-left:1.6rem;"><li>Batter liquid: 1 egg, 3 tbsp flour and 1 tbsp water mixed together. The coating sticks well, does not peel off, and moisture escapes so it turns out crispy.</li><li>Fresh breadcrumbs: they contain more moisture than dried ones and give a lighter, crunchier texture.</li><li>Double frying: cook through at a low 160°C, take it out and rest it for 3 minutes, then finish at 180–190°C for 30 seconds to 1 minute. The crispness lasts even after it cools.</li></ul>
<p style="margin:.9rem 0 .4rem;font-weight:700;">① Katsu Curry</p>
<ul style="margin:0 0 .8rem;padding-left:1.6rem;"><li>Cut the tendon between the lean meat and fat of the pork loin with the tip of a knife (to prevent shrinking) and season with salt and pepper.</li><li>Coat with batter liquid, then fresh breadcrumbs. Fry at 160°C for 3–4 minutes, rest for 3 minutes, then finish at 180°C (or 190°C).</li><li>Put curry on the rice and place the cut cutlet on top (not pouring too much sauce over the coating keeps it crispy).</li></ul>
<p style="margin:.9rem 0 .4rem;font-weight:700;">② Fried Oyster Curry</p>
<ul style="margin:0 0 .8rem;padding-left:1.6rem;"><li>Rub-wash oysters (for cooking) in light salt water or with potato starch, rinse under running water, and wipe off all moisture with paper towels (they will not be crispy if moisture remains).</li><li>Coat thinly and evenly with flour, beaten egg and breadcrumbs, and fry in 180°C oil for 1.5–2 minutes until golden (do not overcook).</li><li>Put them freshly fried on the curry rice. A little Worcestershire sauce is also excellent.</li></ul>
<p style="margin:.9rem 0 .4rem;font-weight:700;">③ Croquette Curry</p>
<ul style="margin:0 0 .8rem;padding-left:1.6rem;"><li>Mix boiled, mashed potatoes with stir-fried ground meat and onion.</li><li>Cool the filling completely in the refrigerator before shaping (if warm, it bursts in the oil).</li><li>Coat with flour, beaten egg and breadcrumbs without gaps, and fry in 180°C oil for 2–3 minutes just to set the coating; take it out as soon as it is golden.</li></ul>
<p style="margin:.9rem 0 .4rem;font-weight:700;">Tips for draining oil and avoiding a heavy stomach</p>
<ul style="margin:0 0 .8rem;padding-left:1.6rem;"><li>For the last 10 seconds before lifting out, raise the oil to 180–190°C so steam from inside blows off the oil trying to soak into the coating.</li><li>Lift the food out held vertically with tongs or chopsticks, and shake it gently up and down 4–5 times above the oil to expose it to air.</li><li>Do not lay it flat on kitchen paper; stand it on a rack, slightly overlapping (this prevents oil from soaking back in).</li><li>Using rice bran oil (or canola oil) makes the coating lighter and eases heaviness in the stomach.</li><li>Frying order in the same oil: ① croquettes → ② pork cutlets → ③ fried oysters. Oysters transfer sea smell and moisture to the oil, so they go last.</li></ul>
<h3 style="font-size:1.25rem;margin:1.4rem 0 .5rem;">3. Chashu (Quick Pressure-Cooker Method, About 40 Minutes)</h3>
<p style="margin:0 0 .7rem;">Ingredients: 500 g pork belly block, the green part of a green onion, 3 slices of ginger. Braising liquid (kaeshi): 150 ml soy sauce, 80 ml mirin, 80 ml sake, 3 tbsp sugar, 1 clove of garlic.</p>
<ol style="margin:0 0 .8rem;padding-left:1.6rem;"><li>Searing: heat a frying pan over high heat and brown the whole surface of the pork block (locks in flavor and adds aroma).</li><li>Pressure cooking (parboiling): put the meat, green onion, ginger and enough water to cover the meat in the pressure cooker. Once under pressure, lower the heat and cook for 15 minutes. Turn off the heat and leave it until the pressure is completely released.</li><li>Seasoning: take out the meat (keep the parboiling liquid for the ramen soup) and bring the braising-liquid ingredients to a boil in another pot or frying pan.</li><li>Glazing: add the meat and, spooning the liquid over it, roll it over low heat for about 10 minutes until the liquid reduces.</li><li>Rapid cooling (important): put the meat and liquid in a zip-top bag and cool quickly in ice water or the refrigerator. As the temperature drops the flavor soaks deep inside, and the meat firms up and slices thinly.</li></ol>
<h3 style="font-size:1.25rem;margin:1.4rem 0 .5rem;">4. Ramen Soup (Two Approaches)</h3>
<p style="margin:0 0 .7rem;">Use the parboiling liquid left in the pressure cooker (a pork white-broth base with the pork flavor dissolved in it) and finish it in the style you like. There is no need to wash the pressure cooker; use it for the soup as it is.</p>
<p style="margin:.9rem 0 .4rem;font-weight:700;">[A: Shoyu clear soup (light)]</p>
<ol style="margin:0 0 .8rem;padding-left:1.6rem;"><li>Let the parboiling liquid cool once and scoop off about 80% of the white lard that has solidified on the surface with a ladle.</li><li>To the soup (about 300–400 ml), add 100 ml water, kombu–bonito stock (or 1 tsp Japanese soup-stock granules) and 1 tsp chicken-bouillon granules, and warm it over low heat without boiling (so it does not turn cloudy).</li><li>Put 2 tbsp of the chashu braising liquid (kaeshi) in the bowl and pour in the soup.</li></ol>
<p style="margin:.9rem 0 .4rem;font-weight:700;">[B: Seafood-tonkotsu style (rich)]</p>
<ol style="margin:0 0 .8rem;padding-left:1.6rem;"><li>Boil the parboiling liquid, together with the fat on its surface, vigorously over high heat for about 5 minutes while stirring, to emulsify it into a thick pork white soup.</li><li>Add 1 tbsp of bonito powder (or powdered mackerel flakes or dried sardines) and 1 tsp of chicken-bouillon granules.</li><li>Put 2.5–3 tbsp of kaeshi in the bowl and pour in the soup. A little grated garlic adds even more punch.</li></ol>
<p style="margin:0 0 .7rem;">Finish: add the boiled noodles and top with the rapidly cooled, sliced chashu, green onion, menma and seasoned egg.</p>
<h3 style="font-size:1.25rem;margin:1.4rem 0 .5rem;">4-2. Three Tonkotsu Ramen Recipes (Hakata Style, Kumamoto Style, Tonkotsu-Shoyu Style)</h3>
<p style="margin:0 0 .7rem;">All three share the same tonkotsu (pork-bone) soup as a base, and differ in the seasoning base, aroma oil, noodles and toppings. The amounts and times are a guide, so adjust to taste (they have not been tested by cooking them).</p>
<p style="margin:.9rem 0 .4rem;font-weight:700;">[Common] Tonkotsu soup base (pressure cooker, about 4 bowls)</p>
<ol style="margin:0 0 .8rem;padding-left:1.6rem;"><li>Parboil about 1 kg of pork femur bones (or backbones) in boiling water for about 5 minutes, then rinse thoroughly under running water to remove blood and sliminess (an important step to remove odor).</li><li>Put the bones, the green part of a green onion, ginger, garlic and 1/2 onion in the pressure cooker, add water (about 2 L, no more than 2/3 of the cooker's maximum), pressure-cook for 60 minutes and wait until the pressure releases naturally.</li><li>Open the lid, turn the heat to high and boil for 10–20 minutes while stirring so the fat and water emulsify and turn white and cloudy. Crush the bones to release flavor and strain the soup through a colander.</li><li>Caution: pork bones foam easily, so always follow your pressure cooker's manual for maximum amount and usable ingredients, and keep the foam from clogging the valve. Without a pressure cooker, simmer for 4–6 hours at a steady boil over high heat (add hot water when it reduces).</li></ol>
<p style="margin:.9rem 0 .4rem;font-weight:700;">① Hakata style (light and cloudy, thin noodles)</p>
<ul style="margin:0 0 .8rem;padding-left:1.6rem;"><li>Seasoning base (kaeshi) per bowl: 1/2 tsp salt, 1 tsp soy sauce, a pinch of chicken-bouillon granules and a little white pepper.</li><li>Heat 300–350 ml of soup, combine it with the kaeshi, and add thin straight noodles boiled firm.</li><li>Toppings: chashu, wood-ear mushrooms, green onion, red pickled ginger, spicy pickled mustard greens (takana), sesame.</li><li>Kaedama (extra noodles): boil just the noodles separately and add them; add a little kaeshi so the soup does not get diluted.</li></ul>
<p style="margin:.9rem 0 .4rem;font-weight:700;">② Kumamoto style (mayu black garlic oil)</p>
<ul style="margin:0 0 .8rem;padding-left:1.6rem;"><li>Mayu: put 3 tbsp sesame oil and 2 cloves of minced garlic in a small pot and heat slowly over low heat; when it turns brown to blackish, turn off the heat (do not burn it, or it becomes bitter).</li><li>Kaeshi per bowl: 1 tbsp soy sauce, a pinch of salt and 1/2 tsp chicken-bouillon granules.</li><li>Combine with 300 ml of soup and add medium-thick straight noodles. Drizzle 1–2 tsp of mayu and sprinkle on fried garlic chips.</li><li>Toppings: chashu, seasoned egg, wood-ear mushrooms, green onion.</li></ul>
<p style="margin:.9rem 0 .4rem;font-weight:700;">③ Tonkotsu-shoyu style (rich, Iekei style)</p>
<ul style="margin:0 0 .8rem;padding-left:1.6rem;"><li>Chicken oil (chiyu): use the fat rendered by frying chicken skin slowly in a frying pan (substitute lard or sesame oil if you have none).</li><li>Kaeshi per bowl: 2 tbsp soy sauce, 1/2 tbsp mirin, 1/2 tsp sugar and 1 tbsp chicken oil.</li><li>Pour in 300 ml of hot soup and add thick straight noodles (soft to regular).</li><li>Toppings: chashu, spinach, 3 sheets of nori, green onion, quail eggs. You can adjust it to "firm noodles, strong flavor, extra oil" as you like.</li></ul>
<h3 style="font-size:1.25rem;margin:1.4rem 0 .5rem;">4-3. Two Seafood-Tonkotsu Tsukemen (Dipping Ramen) Recipes</h3>
<p style="margin:0 0 .7rem;">Two tsukemen (dipping noodles) made by combining the tonkotsu soup (the base in 4-2) with seafood stock: a rich type and a light type. The amounts and times are a guide, so adjust to taste (they have not been tested by cooking them).</p>
<p style="margin:.9rem 0 .4rem;font-weight:700;">[Common] Seafood stock (for 1–2 servings)</p>
<ol style="margin:0 0 .8rem;padding-left:1.6rem;"><li>Put 10 g dried sardines (heads and guts removed), 10 g mackerel flakes and a 5 cm piece of kombu in 300 ml of water and soak for about 30 minutes.</li><li>Heat over low heat, take out the kombu just before it boils, and simmer the sardines and mackerel flakes for about 5 minutes. Strain through a colander to get about 250 ml of seafood stock.</li></ol>
<p style="margin:.9rem 0 .4rem;font-weight:700;">[Common] Preparing the noodles (tsukemen noodles are rinsed in cold water)</p>
<ul style="margin:0 0 .8rem;padding-left:1.6rem;"><li>Boil 200–250 g of thick noodles as directed, rinse them well in cold water to firm them up and remove the sliminess, and drain thoroughly (the more they are rinsed, the chewier they get). Tossing them with a little sesame oil keeps them from sticking.</li></ul>
<p style="margin:.9rem 0 .4rem;font-weight:700;">① Rich seafood-tonkotsu tsukemen (double soup)</p>
<ul style="margin:0 0 .8rem;padding-left:1.6rem;"><li>Dipping soup (per serving): 100 ml tonkotsu soup, 100 ml seafood stock, kaeshi (1.5 tbsp soy sauce, 1/2 tbsp mirin, 1/2 tsp sugar), 2 tsp fish powder and 1/2 tbsp chashu braising liquid. Reduce it in a small pot for about 5 minutes to thicken it.</li><li>Dip the cold noodles in the hot dipping soup as you eat. The soup cools quickly, so keep it warm in the small pot or serve it in a preheated bowl.</li><li>Toppings: chashu, menma, seasoned egg, chopped green onion, nori and, if you like, chopped onion.</li><li>Wari-soup: at the end, pour warmed seafood stock (or tonkotsu soup) into the remaining dipping soup to dilute it and drink it up.</li></ul>
<p style="margin:.9rem 0 .4rem;font-weight:700;">② Light seafood tsukemen (with vinegar and chili oil)</p>
<ul style="margin:0 0 .8rem;padding-left:1.6rem;"><li>Dipping soup (per serving): warm 150 ml seafood stock, 50 ml tonkotsu soup and kaeshi (2 tbsp soy sauce, 1 tbsp mirin, 1/2 tbsp sugar), then turn off the heat and add 1 tbsp vinegar and a little chili oil.</li><li>Rinse medium-thick noodles in cold water and serve them as they are. You can make it with either a cold or a warm dipping soup.</li><li>Toppings: chicken chashu (or pork chashu), menma, chopped green onion, white sesame, and yuzu peel or lemon.</li><li>Recommended when you want something refreshing, or as cold tsukemen in summer.</li></ul>
<h3 style="font-size:1.25rem;margin:1.4rem 0 .5rem;">5. Superb Seasoned Eggs (Ajitama) and Menma</h3>
<p style="margin:.9rem 0 .4rem;font-weight:700;">Seasoned eggs (using the chashu braising liquid)</p>
<ol style="margin:0 0 .8rem;padding-left:1.6rem;"><li>Gently lower eggs (M–L size) straight from the refrigerator into boiling water with a ladle. Boil 6 minutes 30 seconds for a runny soft yolk, or 7 minutes 15 seconds for a slightly stickier one (rolling them gently with chopsticks for the first 2 minutes keeps the yolk in the center).</li><li>Cool immediately in ice water, crack the whole shell finely and peel under water.</li><li>Put the eggs in a zip-top bag with cooled chashu braising liquid to just cover (if they float, cover with kitchen paper).</li><li>Refrigerate for at least 4 hours, ideally overnight (about 12 hours). They go well on ramen and beside curry.</li></ol>
<p style="margin:.9rem 0 .4rem;font-weight:700;">Superb menma (seasoned bamboo shoots)</p>
<ol style="margin:0 0 .8rem;padding-left:1.6rem;"><li>Quickly blanch boiled menma (about 100–150 g) to remove the water-packed smell.</li><li>Fry in 1/2 tbsp sesame oil (and a little sliced chili pepper, if you like) over high heat so the moisture evaporates.</li><li>Add 1/2 tsp chicken-bouillon granules, 50 ml water, 1 tbsp sake, 1/2 tbsp mirin and 1 tbsp chashu braising liquid; simmer over low heat until the liquid is gone and finish with white pepper.</li></ol>
<h3 style="font-size:1.25rem;margin:1.4rem 0 .5rem;">6. An Efficient Order for the Whole Process (3 Burners)</h3>
<p style="margin:0 0 .7rem;">Cookware guide: pressure cooker 3.0–4.5 L, frying pot (or deep frying pan) 20–24 cm in diameter, curry pot about 20 cm.</p>
<ol style="margin:0 0 .8rem;padding-left:1.6rem;"><li>[First] Sear the pork belly and pressure-cook (parboil) it for 15 minutes.</li><li>[While under pressure] Fry the curry onions until caramelized and simmer the curry (spice or roux).</li><li>[When the pressure is released] Take out the chashu, glaze it in the braising liquid for 10 minutes, then cool it quickly in a zip-top bag in ice water. Finish the parboiling liquid left in the pressure cooker as the ramen soup.</li><li>[Just before eating] Fry in the order ① croquettes → ② pork cutlets → ③ fried oysters.</li><li>[Finishing] Add the tempered spices to the curry, serve it over rice and top with the fried food. For the ramen, boil the noodles and add the soup, kaeshi and the chashu, now cold and easy to slice.</li></ol>
<h3 style="font-size:1.25rem;margin:1.4rem 0 .5rem;">7. Instead of Fukujinzuke: Red Onion and Bell Pepper Achar (Indian-Style Quick Pickle)</h3>
<p style="margin:0 0 .7rem;">This is the side dish served at spice-curry specialty shops. Its crisp texture and refreshing acidity reset the greasiness of katsu and other fried food. The flow (slice → salt-rub and rinse → toss with seasonings) is based on the reference pages below; the amounts are a guide, so adjust them to taste.</p>
<p style="margin:.9rem 0 .4rem;font-weight:700;">Ingredients (an easy-to-make amount)</p>
<ul style="margin:0 0 .8rem;padding-left:1.6rem;"><li>1/2 red onion, 1/2 red or yellow bell pepper</li><li>1/2 tsp salt (for rubbing)</li><li>1 tbsp vegetable oil (mustard oil if you have it)</li><li>1 tbsp lemon juice, a pinch of sugar</li><li>1/2 tsp cumin powder, 1/4 tsp turmeric, 1/4 tsp chili powder (use paprika powder if you do not want it hot)</li></ul>
<p style="margin:.9rem 0 .4rem;font-weight:700;">How to make it</p>
<ol style="margin:0 0 .8rem;padding-left:1.6rem;"><li>Slice the red onion and bell pepper thinly, 3–5 mm.</li><li>Sprinkle with salt, rub well, leave for about 5 minutes, then rinse in water to remove the harshness and squeeze out the water well (rinsing the bell pepper in water keeps the red onion's color from tinting it).</li><li>Heat the oil in a small frying pan and fry the cumin, turmeric and chili powder over low heat for 10–20 seconds until fragrant (do not burn them).</li><li>Toss the vegetables with the spiced oil from step 3, the lemon juice and the sugar in a bowl.</li><li>Let it rest in the refrigerator for at least 30 minutes. It keeps about 2–3 days refrigerated.</li></ol>
<p style="margin:0 0 .7rem;font-size:.9rem;">References: <a href="https://macaro-ni.jp/164460" target="_blank" rel="noopener noreferrer">macaroni</a>, <a href="https://www.city.shinjuku.lg.jp/kenkou/kenko01_000001_00031.html" target="_blank" rel="noopener noreferrer">Shinjuku City</a>, <a href="https://cookpad.com/recipe/4990071" target="_blank" rel="noopener noreferrer">Cookpad</a></p>
<h3 style="font-size:1.25rem;margin:1.4rem 0 .5rem;">8. Our Commitment to Oil, Meat and Ingredients</h3>
<p style="margin:0 0 .7rem;">For oil used in frying, expeller-pressed rice bran oil is the best, and it is said that canola oil and the like are bad for the body.</p>
<p style="margin:0 0 .7rem;">As for the cooking meat imported into Japan, most foreign meat, such as American beef and Aussie beef, often comes from livestock raised with large amounts of hormones and antibiotics, and there is concern that eating it may cause breast cancer, uterine cancer and other diseases. So, as much as possible, use Japanese-produced meat, fish and vegetables, and grow them yourself in a home garden, with livestock too, eating what you produce yourself (local production for local consumption). Even without going to university, you can take a course and pass a test to obtain a qualification as an artificial insemination technician. Then, while keeping dairy cows, you can transfer fertilized Wagyu eggs into the wombs of dairy cows, which are larger and sturdier than Wagyu and so calve more easily, so that the dairy cows give birth to Wagyu-and-dairy crossbred calves. You can ship those calves as well as dairy products, which is how dairy farmers can finally turn a profit.</p>
<p style="margin:0 0 .7rem;">On vast land such as Hokkaido, if bumpy land can be leveled flat, a golf course could be operated. And when cattle, horses and other animals are pastured, a large amount of their dung falls on the ground and gets mixed in when the winter snow melts, making it hard to clean up. So I think a proper drainage system should be built, together with a solid facility that can recycle livestock dung, gravel, stones, soil, mud and so on as fertilizer for fields even when they are flushed into it.</p>
<p style="margin:0 0 .7rem;">Also, if there is vast land, airports can be built, and in 2026 semiconductor factories, AI data centers and power plants are being built together as a set. It also seems that cases are increasing in which house builders build large AUDIO CAFEs (jazz cafes) as model homes to attract customers, and the inside becomes a showroom where real estate companies, construction companies and local builders operate, aircraft and new and used cars are bought and sold, PCs, tablets and smartphones are sold, rented and leased, and insurance such as AFLAC, NISSAY (Nippon Life) and SONY Property &amp; Casualty Insurance is also handled.</p>
<p style="margin:0 0 .7rem;">I think sheet-metal and body repair for aircraft, cars and motorcycles should be run at the same time. To take the supercar Bugatti as an example, it is said to be not uncommon for someone who has just bought a used Bugatti for 50 million yen to be overcharged 50 million yen for replacing a part that costs only a few hundred yen to make. To gain long-term, international trust, I think Japanese zaibatsu foundations should acquire aircraft makers around the world, BMW, Mercedes-Benz and supercar makers, and sell, rent and lease Made in Japan aircraft, supercars, luxury cars and luxury motorcycles together, under the LEXUS brand, or under NISSAN's luxury brand INFINITI, or with HONDA and MAZDA also launching luxury brands.</p>
<p style="margin:0 0 .7rem;">As a further point: on YouTube there is news that a giant UFO was found beneath the pyramids of Egypt, together with its operation manual and design documents on stone tablets written in Japanese, and it seems that a reform is now under way in the very nature of the engines of aircraft, cars, motorcycles, ships and cruisers. I think that allocations should also be considered for this area, both as national policy and in the budgets of the zaibatsu foundations.</p>
</div>
</div>
</details>
<p style="font-size:1.4rem;font-weight:700;text-align:center;margin:0 auto 1.2rem;"><a href="https://www.kikkoman.co.jp/homecook/theme/popular/healthy-200calories-recipes.html" target="_blank" rel="noopener noreferrer">低カロリーのヘルシーレシピ！200kcal以下の人気メニュー</a><br><small style="font-size:.85rem;font-weight:400;word-break:break-all;"><a href="https://www.kikkoman.co.jp/homecook/theme/popular/healthy-200calories-recipes.html" target="_blank" rel="noopener noreferrer">https://www.kikkoman.co.jp/homecook/theme/popular/healthy-200calories-recipes.html</a></small><span class="diet-src" data-key="kikkoman" style="display:block;"></span><script src="/diet.js" defer></script></p>
<p style="font-size:1.4rem;font-weight:700;text-align:center;margin:0 auto 1.2rem;"><a href="https://oceans-nadia.com/recipe_list/5060" target="_blank" rel="noopener noreferrer">ダイエットレシピ33選｜簡単で美味しく満腹！</a><br><small style="font-size:.85rem;font-weight:400;word-break:break-all;"><a href="https://oceans-nadia.com/recipe_list/5060" target="_blank" rel="noopener noreferrer">https://oceans-nadia.com/recipe_list/5060</a></small><span class="diet-src" data-key="nadia" style="display:block;"></span></p>
<p style="font-size:1.4rem;font-weight:700;text-align:center;margin:0 auto 1.2rem;"><a href="https://recipe.rakuten.co.jp/category/39-504/" target="_blank" rel="noopener noreferrer">低カロリー・ダイエットのレシピ・作り方 （人気順）</a><br><small style="font-size:.85rem;font-weight:400;word-break:break-all;"><a href="https://recipe.rakuten.co.jp/category/39-504/" target="_blank" rel="noopener noreferrer">https://recipe.rakuten.co.jp/category/39-504/</a></small><span class="diet-src" data-key="rakuten" style="display:block;"></span></p>
<div style="max-width:960px;margin:0 auto 1.4rem;padding:.8rem 1rem;border:1px solid rgba(128,128,128,.5);border-radius:10px;text-align:left;line-height:1.7;">
<p style="font-size:1.3rem;font-weight:700;margin:0 0 .6rem;">「時短 ダイエット レシピ」の検索結果<br><span lang="en" style="font-size:1rem;font-weight:400;">Search results for "quick diet recipes" (<span id="diet-search-date">2026-10-30</span>)</span></p>
<ul id="diet-search-list" style="margin:0;padding-left:1.4rem;">
<li style="margin:0 0 .9rem;"><a href="https://www.city.takasago.lg.jp/material/files/group/19/1234.pdf" target="_blank" rel="noopener noreferrer" style="font-weight:700;">高砂市 健康増進課の資料「時短レシピのコツ」(PDF)</a><br><span lang="en" style="font-weight:700;">Takasago City Health Promotion Division: "Tips for Quick Recipes" (PDF)</span><br><span style="font-size:.95rem;">レンジ調理の「豚肉と大根のうま煮」(1人分152kcal)と、時短のコツ6項目が載っています。</span><br><span lang="en" style="font-size:.95rem;opacity:.85;">Includes a microwave-cooked "simmered pork and daikon" (152 kcal per serving) and 6 tips for saving time.</span></li>
<li style="margin:0 0 .9rem;"><a href="https://maquia.hpplus.jp/life/photo/74343/" target="_blank" rel="noopener noreferrer" style="font-weight:700;">マキア:管理栄養士・中野明海さんの簡単レシピ&amp;おすすめ食材</a><br><span lang="en" style="font-weight:700;">MAQUIA: Easy recipes and recommended ingredients by registered dietitian Akemi Nakano</span><br><span style="font-size:.95rem;">10分程度で作れるメニュー。低カロリーのわかめを使った台湾風の一品やズッキーニパスタなど。</span><br><span lang="en" style="font-size:.95rem;opacity:.85;">Menus made in about 10 minutes, such as a Taiwanese-style dish with low-calorie wakame and zucchini pasta.</span></li>
<li style="margin:0 0 .9rem;"><a href="https://oggi.jp/192764" target="_blank" rel="noopener noreferrer" style="font-weight:700;">Oggi:時短から揚げ</a><br><span lang="en" style="font-weight:700;">Oggi: Quick fried chicken</span><br><span style="font-size:.95rem;">鶏肉を電子レンジで蒸してから揚げ焼きにし、油の処理の手間を省く方法。</span><br><span lang="en" style="font-size:.95rem;opacity:.85;">Steam the chicken in a microwave, then pan-fry in a little oil, saving the trouble of handling frying oil.</span></li>
<li style="margin:0 0 .9rem;"><a href="https://fitmap.jp/magazine/?p=188882" target="_blank" rel="noopener noreferrer" style="font-weight:700;">fitmap:食事制限なしで栄養バランスが取れるレシピ10選</a><br><span lang="en" style="font-weight:700;">fitmap: 10 recipes with balanced nutrition and no dietary restrictions</span><br><span style="font-size:.95rem;">食事制限をせずに続けやすいダイエット向けのレシピを紹介する記事。</span><br><span lang="en" style="font-size:.95rem;opacity:.85;">An article introducing diet-friendly recipes that are easy to keep up without restricting your diet.</span></li>
<li style="margin:0 0 .9rem;"><a href="https://macaro-ni.jp/tag/recipes/4674?page=11" target="_blank" rel="noopener noreferrer" style="font-weight:700;">macaroni:時短レシピの一覧</a><br><span lang="en" style="font-weight:700;">macaroni: List of quick recipes</span><br><span style="font-size:.95rem;">「ダイエット中の人もぜひ」と案内された豚のねぎ塩レモンなどの時短レシピ集。</span><br><span lang="en" style="font-size:.95rem;opacity:.85;">A collection of quick recipes, including pork with green-onion salt and lemon, recommended even for people on a diet.</span></li>
</ul>
</div>
<p style="font-size:1.4rem;font-weight:700;text-align:center;margin:0 auto 1.2rem;"><a href="https://www.youtube.com/results?search_query=100%E5%86%86%E3%80%80%E5%8C%85%E4%B8%81+%E7%A0%94%E3%81%8E%E5%99%A8%E3%80%80%E3%83%8F%E3%82%B5%E3%83%9F%E3%80%80%E9%9B%BB%E5%8B%95" target="_blank" rel="noopener noreferrer">100円　包丁 研ぎ器　ハサミ　電動</a></p>
<div style="max-width:720px;margin:0 auto 1.6rem;text-align:center;">
<p style="font-size:1.2rem;font-weight:700;margin:0 0 .6rem;">立沢君、平野君、次の写真の文章内のCADメーカーを全部買収して</p>
<p lang="en" style="font-size:.9rem;opacity:.85;margin:0 0 .8rem;">Tatsuzawa-kun, Hirano-kun, please acquire all of the CAD makers mentioned in the text in the photo below.</p>
<p style="font-size:1.4rem;font-weight:700;margin:.2rem 0 .6rem;"><a href="https://www.youtube.com/results?search_query=Onshape+by+PTC%E3%80%80%E3%82%88%E3%82%8A%E3%80%80AI%E9%80%A3%E6%90%BA%E3%81%AE%E5%84%AA%E3%82%8C%E3%81%9F%E3%80%80CAD%E3%81%AF%E3%81%82%E3%82%8A%E3%81%BE%E3%81%99%E3%81%8B%3F++%E7%B5%90%E8%AB%96%E3%81%8B%E3%82%89%E8%A8%80%E3%81%86%E3%81%A8%E3%80%81AI%E9%80%A3%E6%90%BA%E3%81%AE%E6%96%B9%E5%90%91%E6%80%A7%E3%81%AB%E3%82%88%E3%81%A3%E3%81%A6%E3%81%AF%E3%80%81Onshape%E3%82%88%E3%82%8A%E5%85%88%E3%81%AB%E9%80%B2%E3%82%93%E3%81%A7%E3%81%84%E3%82%8BCAD%E3%81%8C%E3%81%82%E3%82%8A%E3%81%BE%E3%81%99%E3%80%82%E3%81%9F%E3%81%A0%E3%81%97%E3%81%93%E3%82%8C%E3%81%AF%E7%A7%81%E3%81%AE%E7%9F%A5%E8%AD%98(2026%E5%B9%B46%E6%9C%88%E3%81%BE%E3%81%A7)%E3%81%AB%E5%9F%BA%E3%81%A5%E3%81%8F%E6%95%B4%E7%90%86%E3%81%A7%E3%80%81%E6%A9%9F%E8%83%BD%E3%81%AF%E5%A4%89%E5%8C%96%E3%81%8C%E9%80%9F%E3%81%84%E3%81%AE%E3%81%A7%E3%80%81%E5%B0%8E%E5%85%A5%E5%89%8D%E3%81%AB%E6%9C%80%E6%96%B0%E3%81%AE%E5%85%AC%E5%BC%8F%E6%83%85%E5%A0%B1%E3%81%A7%E7%A2%BA%E8%AA%8D%E3%81%97%E3%81%A6%E3%81%8F%E3%81%A0%E3%81%95%E3%81%84%E3%80%82++%E7%94%A8%E9%80%94%E5%88%A5%E3%81%AE%E5%80%99%E8%A3%9C+%E6%96%B9%E5%90%91%E6%80%A7%09%E5%80%99%E8%A3%9C%09%E7%89%B9%E5%BE%B4+%E3%83%86%E3%82%AD%E3%82%B9%E3%83%88%E3%81%8B%E3%82%89CAD%E7%94%9F%E6%88%90(AI%E3%83%8D%E3%82%A4%E3%83%86%E3%82%A3%E3%83%96)%09Zoo+(%E6%97%A7KittyCAD)%09Text-to-CAD%E3%81%A8API%E3%82%92%E6%8F%90%E4%BE%9B%E3%81%97%E3%80%81%E3%83%96%E3%83%A9%E3%82%A6%E3%82%B6%E3%81%A7%E4%BD%BF%E3%81%88%E3%81%BE%E3%81%99%E3%80%82%E8%87%AA%E7%84%B6%E8%A8%80%E8%AA%9E%E3%81%8B%E3%82%89%E7%B7%A8%E9%9B%86%E5%8F%AF%E8%83%BD%E3%81%AA%E3%83%91%E3%83%A9%E3%83%A1%E3%83%88%E3%83%AA%E3%83%83%E3%82%AF%E3%83%A2%E3%83%87%E3%83%AB%E3%82%92%E4%BD%9C%E3%82%8B%E7%82%B9%E3%81%8C%E5%BC%B7%E3%81%BF%E3%81%A7%E3%81%99%E3%80%82+%E5%90%8C%E4%B8%8A%09Adam+%2F+CADAM(%E3%82%AA%E3%83%BC%E3%83%97%E3%83%B3%E3%82%BD%E3%83%BC%E3%82%B9)%E3%80%81Backflip+AI%09%E6%96%87%E7%AB%A0%E3%82%84%E7%94%BB%E5%83%8F%E3%80%81%E3%82%B9%E3%82%AD%E3%83%A3%E3%83%B3%E3%81%8B%E3%82%89%E3%81%AE%E7%94%9F%E6%88%90%E3%82%92%E8%A9%A6%E3%81%9B%E3%81%BE%E3%81%99%E3%80%82%E5%AE%9F%E5%8B%99%E3%81%AE%E7%B2%BE%E5%BA%A6%E3%81%AF%E7%99%BA%E5%B1%95%E9%80%94%E4%B8%8A%E3%81%A7%E3%81%99%E3%80%82+%E7%B7%8F%E5%90%88CAD%E3%81%AE%E4%B8%AD%E3%81%A7%E3%81%AEAI%09Autodesk+Fusion%09%E3%82%B8%E3%82%A7%E3%83%8D%E3%83%AC%E3%83%BC%E3%83%86%E3%82%A3%E3%83%96%E3%83%87%E3%82%B6%E3%82%A4%E3%83%B3%E3%80%81%E8%87%AA%E5%8B%95%E6%8B%98%E6%9D%9F%E3%80%81AI%E3%82%A2%E3%82%B7%E3%82%B9%E3%82%BF%E3%83%B3%E3%83%88%E3%80%81Autodesk%E3%81%AE%E7%94%9F%E6%88%90%E7%B3%BB%E7%A0%94%E7%A9%B6%E3%82%92%E7%B5%B1%E5%90%88%E3%81%97%E3%81%A6%E3%81%8A%E3%82%8A%E3%80%81AI%E6%A9%9F%E8%83%BD%E3%81%AE%E5%B9%85%E3%81%AF%E5%95%86%E7%94%A8CAD%E3%81%A7%E6%9C%80%E3%82%82%E5%BA%83%E3%81%84%E9%83%A8%E9%A1%9E%E3%81%A7%E3%81%99%E3%80%82+%E5%A4%A7%E4%BC%81%E6%A5%AD%E5%90%91%E3%81%91%E3%81%AE%E3%83%8F%E3%82%A4%E3%82%A8%E3%83%B3%E3%83%89%09Siemens+NX%09AI%E3%81%AB%E3%82%88%E3%82%8B%E3%82%B3%E3%83%9E%E3%83%B3%E3%83%89%E6%8F%90%E6%A1%88%E3%82%84%E8%A8%AD%E8%A8%88%E6%94%AF%E6%8F%B4%E3%80%81Industrial+Copilot%E3%81%A8%E3%81%AE%E9%80%A3%E6%90%BA%E3%81%8C%E3%81%82%E3%82%8A%E3%81%BE%E3%81%99%E3%80%82+%E5%90%8C%E4%B8%8A%09SOLIDWORKS+%2F+3DEXPERIENCE+(Dassault)%09AI%E3%82%A2%E3%82%B7%E3%82%B9%E3%82%BF%E3%83%B3%E3%83%88%E3%82%84%E7%94%9F%E6%88%90%E8%A8%AD%E8%A8%88%E3%80%81%E9%A1%9E%E4%BC%BC%E9%83%A8%E5%93%81%E6%A4%9C%E7%B4%A2%E3%81%8C%E3%81%82%E3%82%8A%E3%81%BE%E3%81%99%E3%80%82+%E8%A7%A3%E6%9E%90%E3%83%BB%E6%9C%80%E9%81%A9%E5%8C%96%09nTop%09AI%E3%81%A8%E3%81%84%E3%81%86%E3%82%88%E3%82%8A%E8%A8%88%E7%AE%97%E8%A8%AD%E8%A8%88%E3%81%A8%E3%83%88%E3%83%9D%E3%83%AD%E3%82%B8%E3%83%BC%E6%9C%80%E9%81%A9%E5%8C%96%E3%81%8C%E5%BE%97%E6%84%8F%E3%81%A7%E3%81%99%E3%80%82+%E8%A8%AD%E8%A8%88%E3%81%AE%E7%9B%B8%E8%AB%87%E5%BD%B9%09Leo+AI" target="_blank" rel="noopener noreferrer">AI　CAD</a></p>
<div style="max-width:960px;margin:1rem auto 0;text-align:left;font-size:1.3rem;line-height:1.7;">
<p style="margin:0 0 .8rem;background:rgba(128,128,128,.15);border-radius:10px;padding:.5rem .9rem;">Onshape by PTC　より　AI連携の優れた　CADはありますか?</p>
<p style="margin:0 0 1rem;">結論から言うと、AI連携の方向性によっては、Onshapeより先に進んでいるCADがあります。<br>ただしこれは私の知識(2026年6月まで)に基づく整理で、機能は変化が速いので、<br>導入前に最新の公式情報で確認してください。</p>
<p style="margin:0 0 .5rem;font-weight:700;">用途別の候補</p>
<div style="overflow-x:auto;"><table style="border-collapse:collapse;width:100%;font-size:inherit;">
<tr><th style="text-align:left;padding:.5rem .6rem;">方向性</th><th style="text-align:left;padding:.5rem .6rem;">候補</th><th style="text-align:left;padding:.5rem .6rem;">特徴</th></tr>
<tr><td style="padding:.5rem .6rem;border-top:1px solid rgba(128,128,128,.4);vertical-align:top;">テキストからCAD生成<br>(AIネイティブ)</td><td style="padding:.5rem .6rem;border-top:1px solid rgba(128,128,128,.4);vertical-align:top;font-weight:700;">Zoo (旧KittyCAD)</td><td style="padding:.5rem .6rem;border-top:1px solid rgba(128,128,128,.4);vertical-align:top;">Text-to-CADとAPIを提供し、ブラウザで使えます。<br>自然言語から編集可能なパラメトリックモデルを作る点が強みです。</td></tr>
<tr><td style="padding:.5rem .6rem;border-top:1px solid rgba(128,128,128,.4);vertical-align:top;">同上</td><td style="padding:.5rem .6rem;border-top:1px solid rgba(128,128,128,.4);vertical-align:top;font-weight:700;">Adam / CADAM(オープンソース)、<br>Backflip AI</td><td style="padding:.5rem .6rem;border-top:1px solid rgba(128,128,128,.4);vertical-align:top;">文章や画像、スキャンからの生成を試せます。<br>実務の精度は発展途上です。</td></tr>
<tr><td style="padding:.5rem .6rem;border-top:1px solid rgba(128,128,128,.4);vertical-align:top;">総合CADの中でのAI</td><td style="padding:.5rem .6rem;border-top:1px solid rgba(128,128,128,.4);vertical-align:top;font-weight:700;">Autodesk Fusion</td><td style="padding:.5rem .6rem;border-top:1px solid rgba(128,128,128,.4);vertical-align:top;">ジェネレーティブデザイン、自動拘束、AIアシスタント、<br>Autodeskの生成系研究を統合しており、<br>AI機能の幅は商用CADで最も広い部類です。</td></tr>
<tr><td style="padding:.5rem .6rem;border-top:1px solid rgba(128,128,128,.4);vertical-align:top;">大企業向けのハイエンド</td><td style="padding:.5rem .6rem;border-top:1px solid rgba(128,128,128,.4);vertical-align:top;font-weight:700;">Siemens NX</td><td style="padding:.5rem .6rem;border-top:1px solid rgba(128,128,128,.4);vertical-align:top;">AIによるコマンド提案や設計支援、<br>Industrial Copilotとの連携があります。</td></tr>
<tr><td style="padding:.5rem .6rem;border-top:1px solid rgba(128,128,128,.4);vertical-align:top;">同上</td><td style="padding:.5rem .6rem;border-top:1px solid rgba(128,128,128,.4);vertical-align:top;font-weight:700;">SOLIDWORKS /<br>3DEXPERIENCE (Dassault)</td><td style="padding:.5rem .6rem;border-top:1px solid rgba(128,128,128,.4);vertical-align:top;">AIアシスタントや生成設計、類似部品検索があります。</td></tr>
<tr><td style="padding:.5rem .6rem;border-top:1px solid rgba(128,128,128,.4);vertical-align:top;">解析・最適化</td><td style="padding:.5rem .6rem;border-top:1px solid rgba(128,128,128,.4);vertical-align:top;font-weight:700;">nTop</td><td style="padding:.5rem .6rem;border-top:1px solid rgba(128,128,128,.4);vertical-align:top;">AIというより計算設計とトポロジー最適化が得意です。</td></tr>
<tr><td style="padding:.5rem .6rem;border-top:1px solid rgba(128,128,128,.4);vertical-align:top;">設計の相談役</td><td style="padding:.5rem .6rem;border-top:1px solid rgba(128,128,128,.4);vertical-align:top;font-weight:700;">Leo AI など</td><td style="padding:.5rem .6rem;border-top:1px solid rgba(128,128,128,.4);vertical-align:top;">既存CADの横で、部品選定や設計の相談に使うコパイロット型です。</td></tr>
</table></div>
<p style="margin:1.2rem 0 .5rem;font-weight:700;">Onshapeとの比較</p>
<ul style="margin:0;padding-left:1.4rem;">
<li><b>Onshapeの強み</b>:クラウドネイティブで、バージョン管理や共同編集が得意です。<br>AI連携は後発で、現状は補助的な機能が中心です。</li>
<li><b>生成の自由度</b>:ゼロから形状を生み出すなら、ZooやFusionのほうが進んでいます。</li>
<li><b>業務での確実さ</b>:AIの出力は、まだ複雑な機械部品で…(写真ではボタンに隠れて読めません)…や拘束を人が検証する前提です。<br>「AIだけで完成」は現実的ではありません。</li>
</ul>
</div>
</div>
<div style="max-width:640px;margin:0 auto 1.4rem;padding:.8rem 1rem;border:1px solid rgba(128,128,128,.5);border-radius:10px;text-align:left;line-height:1.8;">
<p style="margin:0 0 .6rem;font-weight:700;font-size:1.1rem;">消費税とは?</p>
<p style="margin:0 0 .6rem;">法人税が約40%だった時に、将来約20%まで下げる約束で導入された税金で、財源は、法人税だったのです。</p>
<p style="margin:0 0 .6rem;">消費税を全廃、撤廃するには、2026年の今、約20%まで下がっている法人税を元の約40%もしくは、足りなければ、45%くらいまで元に戻し、更に、財源が足りなければ、バラク・オバマ元大統領が言われた様に、我々、裕福層にこそ、所得税などの税金の累進課税率のカーブをUPするべきだと言う、ご提案に賛成で御座います。</p>
<p style="margin:0 0 .6rem;">所得税や社会保険税などの税金は、裕福層への累進課税率のカーブはアップして、逆に低所得層への累進課税率のカーブはダウンさせて、負担率を下げる、しかし一定の金額以下は、全く税金を払わなくて良いと言う考え方は、全く収入が無いに近い場合を除いて、考え直した方が良いと思います。</p>
<p style="margin:0 0 .6rem;">更に、バラク・オバマ元大統領は、次の様にも言われました。政府とは、大きいか？小さいか？ではなく、機能するか？しないか？であると。eガバメント、デジタルガバメントと言う政府やお役所のLINEアプリ化やTVアプリ化やコンビニのロッピーやファミポートなどによって、市区町村役場や都道府県庁の庁舎と司法立法行政府、中央省庁の全公務員を大幅にリストラして少子高齢化予算、専業主婦や働いていない人なども含めて全国民参加の年金を皆保険として、基礎年金+厚生年金+増額年金の年金予算確保、後期高齢者医療制度の低所得層と中間所得層の負担率の大幅なカーブのダウンや失業時だけ、住宅ローンや自動車ローンや携帯電話代やインターネットプロバイダー代金を肩代わりして無償化と失業時だけベーシックインカムや半公務員として生活費を補償する予算などや低所得層の医療費無償化や中間所得層の医療費の累進負担率のカーブも大幅に下げる予算などやイタリア製ミニウェル・プロクサ（お勧めは、左右の視力が異なっていても同じ種類の30cm手元から遠くまでよく見える眼内コンタクトレンズ）や長期間壊れにくくて安心して使える人工心臓や人工関節などを完全に国保対応とする予算確保など。</p>
<div lang="en" style="margin-top:.8rem;font-size:.9rem;opacity:.85;line-height:1.6;">
<p style="margin:0 0 .5rem;font-weight:700;">What is the consumption tax?</p>
<p style="margin:0 0 .6rem;">It was introduced when the corporate tax rate was about 40%, with a promise to lower the corporate tax to about 20% in the future, and its source of revenue was the corporate tax.</p>
<p style="margin:0 0 .6rem;">To abolish the consumption tax completely, in 2026 the corporate tax, which has now fallen to about 20%, should be restored to its original level of about 40%, or to around 45% if that is not enough. If revenue is still short, I agree with the proposal, as former President Barack Obama said, that we, the wealthy, should be the ones to raise the progressive curve of income tax and other taxes.</p>
<p style="margin:0 0 .6rem;">For income tax, social insurance premiums and similar taxes, the progressive curve should be raised for the wealthy and lowered for low-income earners to reduce their burden. However, the idea that people below a certain amount pay no tax at all should be reconsidered, except for those with almost no income.</p>
<p style="margin:0 0 .6rem;">Furthermore, former President Barack Obama also said the following: the question about government is not whether it is big or small, but whether it works or not. With e-government and digital government (government and local office services as LINE apps and TV apps, and at convenience-store terminals such as Loppi and FamiPort), all public employees at city, ward and town offices, prefectural offices, the judicial, legislative and executive branches and the central ministries should be drastically restructured, and the money saved should be used to secure: universal pension insurance covering the whole population, including full-time homemakers and people not working (basic pension + employees' pension + additional pension) to cope with the aging, declining-birth-rate society; a large lowering of the burden-rate curve for low- and middle-income people under the medical care system for people aged 75 and over; a budget for the government to take over the payments of housing loans, car loans, mobile phone bills and internet provider fees, making them free of charge, and a budget to guarantee living expenses through a basic income or semi-public-employee status, only while a person is unemployed; free medical care for low-income people and a greatly lowered progressive burden curve for middle-income people; and a budget to place under full national health insurance coverage the Italian Minuwell Proxa (recommended: an intraocular contact lens that lets people see well from 30 cm at hand to far away with the same type of lens, even when the left and right eyes have different vision), and of durable, reliable artificial hearts and artificial joints.</p>
</div>
</div>
<p style="margin:-.6rem 0 1.4rem;text-align:center;"><a href="https://maid-cafe.tokyo" target="_blank" rel="noopener noreferrer">https://maid-cafe.tokyo</a><br><small>「消費税」を質問すると、この説明を答えます / Ask about the consumption tax (消費税) to get this explanation</small></p>
<div style="max-width:640px;margin:0 auto 1.4rem;padding:.8rem 1rem;border:1px solid rgba(128,128,128,.5);border-radius:10px;text-align:left;line-height:1.8;">
<p style="margin:0 0 .6rem;font-weight:700;">2026/10/28　19:00 から　19:45くらいまで、<br>ヒューレット・パッカード　市ヶ谷本社と八王子と北八王子関係者と日本マイクロソフトと富士通と日立と東芝と品川区のソフトウェーブとバーテックスシステムズとアメリカ政府と日本政府と韓国政府と中国政府と中国公安警察と全人代関係者を名乗る方々が、ヒューレットパッカード様は優秀なのに石　塚　正　浩は馬鹿だから首にしましょうね。と、あきる野市に民泊しながら、大声で、以下の内容と共におっしゃりました。</p>
<p style="margin:0 0 .6rem;">私、石　塚　正　浩が、以前、北八王子のヒューレットパッカード（今は、キーサイト）にお仕事で、HP-UXのOSを10GBしかないHDDへのインストール依頼があったので、インストールと設定は成功したので、報告して帰った後に、社員がアプリを追加でインストールしようとしたら容量不足でインストール出来なかったと言うクレームを頂いた事がありました。</p>
<p style="margin:0 0 .6rem;">ヒューレットパッカード　市ケ谷本社に出向していた期間には、486SXと言うノートパソコンを最新のノートパソコンだから大切に使う様に言われて使いましたが、それで一生懸命お仕事しましたが、仕事が遅いとクレームを受けた事がありました。</p>
<p style="margin:0;">品川のソフトウェーブ経由で、ヒューレットパッカードの仕事で九州へ出張があっても、新幹線代とホテル代も出ませんでした。</p>
<p style="margin:.6rem 0 0;">品川のソフトウェーブの社員だった時は、手取り月給は、最初は、16万円くらいが二ヶ月間で、2回も給料が下がり、最終の手取り月給14万円くらいでした。退職金も1円ももらってません。</p>
<div lang="en" style="margin-top:.8rem;font-size:.9rem;opacity:.85;line-height:1.6;">
<p style="margin:0 0 .5rem;font-weight:700;">From about 19:00 to 19:45 on October 28, 2026, people claiming to be connected with Hewlett-Packard's Ichigaya headquarters, Hachioji and Kita-Hachioji, Microsoft Japan, Fujitsu, Hitachi, Toshiba, Softwave of Shinagawa Ward, Vertex Systems, the U.S. government, the Japanese government, the South Korean government, the Chinese government, the Chinese public security police and the National People's Congress said, &quot;Hewlett-Packard is excellent, but Masahiro Ishizuka is stupid, so let's fire him.&quot; While staying at a private lodging (minpaku) in Akiruno City, they said this in loud voices, together with the following.</p>
<p style="margin:0 0 .5rem;">I, Masahiro Ishizuka, once worked at Hewlett-Packard in Kita-Hachioji (now Keysight). I was asked to install the HP-UX operating system on a hard disk of only 10GB. The installation and configuration succeeded, and I reported back and left. Afterwards, when an employee tried to install additional applications, there was not enough space, and I received a complaint.</p>
<p style="margin:0 0 .5rem;">While I was seconded to Hewlett-Packard's Ichigaya headquarters, I was told to take good care of a 486SX notebook PC because it was the newest model. I worked as hard as I could on it, but I received a complaint that my work was slow.</p>
<p style="margin:0;">Even when I went on a business trip to Kyushu for Hewlett-Packard work through Softwave in Shinagawa, my Shinkansen fare and hotel costs were not paid.</p>
<p style="margin:.5rem 0 0;">When I was an employee of Softwave in Shinagawa, my take-home monthly pay was about 160,000 yen for the first two months, then my salary was cut twice, and my final take-home monthly pay was about 140,000 yen. I did not receive a single yen in retirement allowance.</p>
</div>
</div>
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
</section>
<div class="pixel-intro">
<div class="google-proposal" style="max-width:420px;margin:1rem auto .6rem;padding:.7rem 1rem;border:2px solid rgba(128,128,128,.6);border-radius:10px;text-align:left;line-height:1.6;">
<p style="margin:0 0 .1rem;"><strong>📣 Google様へのご提案！</strong></p>
<p style="margin:0 0 .5rem;">AppleのiPhoneのOS標準の「いつでも拡大縮小」機能を、Androidスマホとタブレットの全ての機種で、OS標準としてアップデートで対応してほしいです。</p>
<p lang="en" style="margin:0 0 .1rem;font-size:.9rem;opacity:.9;"><strong>📣 A proposal to Google!</strong></p>
<p lang="en" style="margin:0;font-size:.9rem;opacity:.9;">We would like the always-available zoom feature built into Apple's iPhone OS to be added as a standard OS feature, via an update, on all Android smartphones and tablets.</p>
</div>
<p style="text-align:center;margin:1rem auto .1rem;max-width:420px;font-size:.95rem;">💡 SMSのワンタイムパスワードが受け取れない場合に、設定から仮想メモリーを4GBから8GBに変更すると受け取れるようになる場合がございますので、お試しください。</p>
<p lang="en" style="text-align:center;margin:0 auto .6rem;max-width:420px;font-size:.88rem;opacity:.85;">💡 If you cannot receive SMS one-time passwords, changing the virtual memory from 4GB to 8GB in Settings may fix it. Please give it a try.</p>
<h2 style="font-size:1.3rem;margin:1rem 0 .2rem;text-align:center;">📱 2026年10月 スマホ紹介(楽天モバイル)<br><small lang="en" style="font-size:.8em;font-weight:600;">October 2026 Smartphone Picks (Rakuten Mobile)</small></h2>
<p style="text-align:center;margin:0 0 .1rem;font-size:.95rem;">8GBメモリの1円スマホはほとんど売り切れ。乗り換えならこの5台がおすすめです。</p>
<p lang="en" style="text-align:center;margin:0 0 .4rem;font-size:.88rem;opacity:.85;">Most 8GB-RAM 1-yen phones are sold out. If you are switching, we recommend these five.</p>
<p style="text-align:center;margin:0 0 .1rem;font-size:.95rem;">💡 スマホでLINEやカカオトークをご利用の方は、定期的にキャッシュをクリアされた方が処理が重くなりにくいのでお勧めです。</p>
<p lang="en" style="text-align:center;margin:0 0 .6rem;font-size:.88rem;opacity:.85;">💡 If you use LINE or KakaoTalk on your phone, clearing the cache regularly helps keep it from slowing down.</p>
<section class="arrows-alpha2" style="max-width:420px;margin:1rem auto;padding:.9rem 1rem;border:1px solid rgba(128,128,128,.5);border-radius:10px;text-align:left;line-height:1.6;border-width:2px;">
<h3 style="font-size:1.2rem;margin:0 0 .4rem;text-align:center;">★ arrows Alpha2 <small>(FCNT製・8月27日発売)</small></h3>
<p style="text-align:center;margin:.3rem 0 0;"><img src="data:image/webp;base64,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" alt="arrows Alpha2 12GB/512GB" style="max-width:100%;height:auto;border-radius:8px"></p>
<p style="text-align:center;margin:.4rem 0 .6rem;font-size:1.25rem;font-weight:800;">ARROWS　ALPHA2　お勧めの12GB　512GB</p>
<p style="text-align:center;margin:.3rem 0 0;font-size:1.3rem;font-weight:800;line-height:1.3;">8GB / 256GB<br>と 12GB / 512GB の2モデル<br><span lang="en" style="font-size:.7em;">8GB/256GB and 12GB/512GB models</span></p>
<p style="text-align:center;margin:.2rem 0 .6rem;font-size:.95rem;">12GBモデル(512GB)は楽天モバイルのWeb限定販売です</p>
<p style="margin:.3rem 0 .2rem;font-size:1.1rem;"><strong>💴 8GBと12GBの価格・分割払いの違い(税込・楽天カード払い)</strong></p>
<table style="width:100%;border-collapse:collapse;font-size:.92rem;text-align:center;">
<tr style="background:rgba(128,128,128,.18);"><th style="padding:.25rem;border:1px solid rgba(128,128,128,.5);"></th><th style="padding:.25rem;border:1px solid rgba(128,128,128,.5);">8GB(RAM)<br>/ 256GB(ROM)</th><th style="padding:.25rem;border:1px solid rgba(128,128,128,.5);">12GB(RAM)<br>/ 512GB(ROM)</th><th style="padding:.25rem;border:1px solid rgba(128,128,128,.5);">差</th></tr>
<tr><td style="padding:.25rem;border:1px solid rgba(128,128,128,.5);">一括払い</td><td style="padding:.25rem;border:1px solid rgba(128,128,128,.5);">85,900円</td><td style="padding:.25rem;border:1px solid rgba(128,128,128,.5);">109,990円</td><td style="padding:.25rem;border:1px solid rgba(128,128,128,.5);">+24,090円</td></tr>
<tr><td style="padding:.25rem;border:1px solid rgba(128,128,128,.5);">48回払い<br>(月額)</td><td style="padding:.25rem;border:1px solid rgba(128,128,128,.5);"><strong>月1,789円</strong><br><small>(1回目1,817円)</small></td><td style="padding:.25rem;border:1px solid rgba(128,128,128,.5);"><strong>月2,291円</strong><br><small>(1回目2,313円※)</small></td><td style="padding:.25rem;border:1px solid rgba(128,128,128,.5);"><strong>+502円/月</strong></td></tr>
<tr><td style="padding:.25rem;border:1px solid rgba(128,128,128,.5);">24回払い<br>(月額)</td><td style="padding:.25rem;border:1px solid rgba(128,128,128,.5);"><strong>月3,579円</strong><br><small>(1回目3,583円)</small></td><td style="padding:.25rem;border:1px solid rgba(128,128,128,.5);"><strong>月4,582円</strong><br><small>(1回目4,604円※)</small></td><td style="padding:.25rem;border:1px solid rgba(128,128,128,.5);"><strong>+1,003円/月</strong></td></tr>
</table>
<p style="margin:.3rem 0 .1rem;"><small>※ 1回目は端数のため少し多くなります(計算値)。48回払いは楽天カードのみ。実質年率0%。512GBの金額は報道(<a href="https://k-tai.watch.impress.co.jp/docs/news/2133368.html" target="_blank" rel="noopener noreferrer">ケータイ Watch</a>)に基づきます。最新は公式で確認してください。</small></p>
<table lang="en" style="width:100%;border-collapse:collapse;font-size:.82rem;text-align:center;opacity:.85;margin-top:.4rem;">
<tr style="background:rgba(128,128,128,.18);"><th style="padding:.2rem;border:1px solid rgba(128,128,128,.5);"></th><th style="padding:.2rem;border:1px solid rgba(128,128,128,.5);">8GB RAM / 256GB</th><th style="padding:.2rem;border:1px solid rgba(128,128,128,.5);">12GB RAM / 512GB</th><th style="padding:.2rem;border:1px solid rgba(128,128,128,.5);">Difference</th></tr>
<tr><td style="padding:.2rem;border:1px solid rgba(128,128,128,.5);">Lump sum</td><td style="padding:.2rem;border:1px solid rgba(128,128,128,.5);">85,900 yen</td><td style="padding:.2rem;border:1px solid rgba(128,128,128,.5);">109,990 yen</td><td style="padding:.2rem;border:1px solid rgba(128,128,128,.5);">+24,090 yen</td></tr>
<tr><td style="padding:.2rem;border:1px solid rgba(128,128,128,.5);">48 payments</td><td style="padding:.2rem;border:1px solid rgba(128,128,128,.5);">1,789 yen/mo</td><td style="padding:.2rem;border:1px solid rgba(128,128,128,.5);">2,291 yen/mo</td><td style="padding:.2rem;border:1px solid rgba(128,128,128,.5);">+502 yen/mo</td></tr>
<tr><td style="padding:.2rem;border:1px solid rgba(128,128,128,.5);">24 payments</td><td style="padding:.2rem;border:1px solid rgba(128,128,128,.5);">3,579 yen/mo</td><td style="padding:.2rem;border:1px solid rgba(128,128,128,.5);">4,582 yen/mo</td><td style="padding:.2rem;border:1px solid rgba(128,128,128,.5);">+1,003 yen/mo</td></tr>
</table>
<p lang="en" style="font-size:.82rem;opacity:.85;margin:.2rem 0 .6rem;">Tax included. The first payment is slightly higher because of rounding. 48 payments: Rakuten Card only (0% APR). The 512GB figures follow press reports (Keitai Watch); please check Rakuten Mobile for the latest.</p>
<ul style="margin:.4rem 0 .1rem;padding-left:1.3rem;">
<li>メモリ <strong>8GB または 12GB</strong>(12GBモデルは仮想メモリで最大12GB追加) / ストレージ <strong>256GB または 512GB</strong>(microSDで最大2TBまで拡張)</li>
<li>約6.4インチ 有機EL(最大144Hz・最大3,000nit)</li>
<li>SoC:Dimensity 8350 Extreme / Android 16 / 5G(Sub6)</li>
<li>カメラ:メイン約5,030万画素 + 超広角約4,990万画素 / 4K 60fps動画 / フロント約4,990万画素</li>
<li>バッテリー5,370mAh(1%→100%を最短約40分)</li>
<li>約155×72×8.6mm・約187g / IPX6・IPX8・IPX9 / IP6Xの防水防塵 / おサイフケータイ対応</li>
<li>ハイレゾオーディオ対応 / <strong>ワイヤレス充電は非対応</strong> / OSアップデート最大3回・セキュリティ更新最長5年</li>
</ul>
<ul lang="en" style="margin:0 0 .6rem;padding-left:1.3rem;font-size:.88rem;opacity:.85;">
<li><strong>8GB or 12GB RAM</strong> (12GB model: up to 12GB more with virtual memory) / <strong>256GB or 512GB</strong> storage (microSD up to 2TB)</li>
<li>About 6.4-inch OLED (up to 144Hz, up to 3,000 nits)</li>
<li>SoC: Dimensity 8350 Extreme / Android 16 / 5G (Sub6)</li>
<li>Camera: about 50.3MP main + 49.9MP ultra-wide, 4K 60fps video, 49.9MP front camera</li>
<li>5,370mAh battery (1% to 100% in about 40 minutes at the fastest)</li>
<li>About 155 x 72 x 8.6 mm, 187 g / IPX6/IPX8/IPX9 and IP6X / Osaifu-Keitai supported</li>
<li>Hi-res audio supported / <strong>no wireless charging</strong> / up to 3 OS updates and up to 5 years of security updates</li>
</ul>
<p style="margin:.3rem 0 0;">他社から初めて乗り換えると最大16,000ポイント還元(事前エントリー必要)。256GBは、このポイントを使うと実質69,900円(下取りなし・公式ページの試算)。</p>
<p lang="en" style="font-size:.88rem;opacity:.85;margin:.1rem 0 .5rem;">Up to 16,000 points back when switching from another carrier for the first time (pre-entry required). The 256GB model works out to 69,900 yen with those points (no trade-in; Rakuten's own estimate).</p>
<p style="margin:.5rem 0 0;text-align:center;"><a href="https://network.mobile.rakuten.co.jp/product/smartphone/arrows-alpha2/" target="_blank" rel="noopener noreferrer">▶️ 楽天モバイル arrows Alpha2 の公式ページ</a></p>
</section>
<section class="pixel-11" style="max-width:420px;margin:1rem auto;padding:.9rem 1rem;border:1px solid rgba(128,128,128,.5);border-radius:10px;text-align:left;line-height:1.6;border-width:2px;">
<h3 style="font-size:1.2rem;margin:0 0 .4rem;text-align:center;">① Google Pixel 11</h3>
<p style="text-align:center;margin:0 0 .5rem;"><img src="data:image/webp;base64,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" alt="Google Pixel 11 本体(正面・背面) / front and back" width="240" height="240" style="max-width:100%;height:auto;"></p>
<p style="text-align:center;margin:.3rem 0 0;font-size:1.6rem;font-weight:800;line-height:1.3;">システムメモリ12GB<br><span lang="en" style="font-size:.8em;">12GB System Memory (RAM)</span></p>
<p style="text-align:center;margin:.2rem 0 0;font-size:.95rem;">8GB機より4GB多く、アプリの切り替えやAI(Gemini)も余裕で動きます</p>
<p lang="en" style="text-align:center;margin:0 0 .6rem;font-size:.88rem;opacity:.85;">4GB more than 8GB models: smoother app switching and AI (Gemini).</p>
<p style="margin:.3rem 0 .2rem;font-size:1.1rem;"><strong>💴 本体48回払い(楽天カード払いのみ)</strong></p>
<ul style="margin:0 0 .1rem;padding-left:1.3rem;">
<li><strong>256GB</strong>：1〜24回目 <strong>月1,990円</strong> / 25〜47回目 月3,839円 / 48回目 3,843円(一括払い139,900円)</li>
<li><strong>512GB(おすすめ)</strong>：1〜24回目 <strong>月2,990円</strong> / 25〜47回目 月3,881円 / 48回目 3,887円(一括払い164,910円)</li>
</ul>
<p style="margin:0 0 .4rem;"><small>25カ月目以降に条件を満たして返却すれば、残りの支払いは不要です。</small></p>
<p lang="en" style="font-size:.88rem;opacity:.85;margin:.1rem 0 .1rem;"><strong>💴 48-installment plan (Rakuten Card only)</strong></p>
<ul lang="en" style="margin:0 0 .1rem;padding-left:1.3rem;font-size:.88rem;opacity:.85;">
<li><strong>256GB</strong>: payments 1-24 <strong>1,990 yen/month</strong> / 25-47 3,839 yen/month / 48th 3,843 yen (lump sum 139,900 yen)</li>
<li><strong>512GB (recommended)</strong>: payments 1-24 <strong>2,990 yen/month</strong> / 25-47 3,881 yen/month / 48th 3,887 yen (lump sum 164,910 yen)</li>
</ul>
<p lang="en" style="font-size:.88rem;opacity:.85;margin:0 0 .6rem;">If you return the device under the conditions from month 25, the remaining payments are not required.</p>
<ul style="margin:.4rem 0 .1rem;padding-left:1.3rem;">
<li><strong>メモリ12GB</strong> / ストレージ256GB または <strong>512GB(おすすめ)</strong></li>
<li>ハイレゾオーディオ対応</li>
<li>約6.3インチ OLEDディスプレイ</li>
<li>カメラ：48MP広角 + 13MP超広角 + 10.8MP望遠</li>
<li>バッテリー4,985mAh / Gemini Intelligenceに最適化</li>
<li>SoC:Tensor G6 / <strong>ワイヤレス充電対応</strong> / 防水IPX8・防塵IP6X / おサイフケータイ・NFC対応 / 約197g</li>
</ul>
<ul lang="en" style="margin:0 0 .6rem;padding-left:1.3rem;font-size:.88rem;opacity:.85;">
<li><strong>12GB RAM</strong> / 256GB or <strong>512GB (recommended)</strong> storage</li>
<li>Hi-res audio supported</li>
<li>About 6.3-inch OLED display</li>
<li>Camera: 48MP wide + 13MP ultra-wide + 10.8MP telephoto</li>
<li>4,985mAh battery / optimized for Gemini Intelligence</li>
<li>SoC: Tensor G6 / <strong>wireless charging supported</strong> / IPX8 and IP6X / Osaifu-Keitai and NFC / about 197g</li>
</ul>
<p style="margin:.3rem 0 0;">他社から初めて乗り換えると最大16,000ポイント還元(事前エントリー必要)。</p>
<p lang="en" style="font-size:.88rem;opacity:.85;margin:.1rem 0 .5rem;">Up to 16,000 points back when switching from another carrier for the first time (pre-entry required).</p>
<p style="margin:.5rem 0 0;text-align:center;"><a href="https://network.mobile.rakuten.co.jp/product/smartphone/google-pixel-11/" target="_blank" rel="noopener noreferrer">▶️ 楽天モバイル Google Pixel 11 / Rakuten Mobile</a></p>
</section>
<section class="pixel-10a" style="max-width:420px;margin:1rem auto;padding:.9rem 1rem;border:1px solid rgba(128,128,128,.5);border-radius:10px;text-align:left;line-height:1.6;">
<h3 style="font-size:1.1rem;margin:0 0 .4rem;text-align:center;">② Google Pixel 10a</h3>
<p style="text-align:center;margin:0 0 .5rem;"><img src="data:image/webp;base64,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" alt="Google Pixel 10a 本体(正面・背面) / front and back" width="200" height="200" style="max-width:100%;height:auto;"></p>
<p style="margin:.3rem 0 0;">メモリ8GBでハイレゾオーディオ対応。もっと安く乗り換えたい人向けです。</p>
<p lang="en" style="font-size:.88rem;opacity:.85;margin:.1rem 0 .5rem;">8GB RAM with hi-res audio. For those who want to switch more cheaply.</p>
<p style="margin:.3rem 0 0;"><strong>💴 本体48回払い：1〜24回目は月990円</strong><br><small>(25回目以降は月2,750円、楽天カード払いのみ)。一括払いは89,760円〜。</small></p>
<p lang="en" style="font-size:.88rem;opacity:.85;margin:.1rem 0 .6rem;"><strong>💴 48-installment plan: 990 yen/month for payments 1-24</strong> (2,750 yen/month from the 25th; Rakuten Card only). Lump sum from 89,760 yen.</p>
<ul style="margin:.4rem 0 .1rem;padding-left:1.3rem;">
<li>メモリ8GB / ストレージ128GB・256GB</li>
<li>ハイレゾオーディオ対応</li>
<li>約6.3インチ OLEDディスプレイ</li>
<li>カメラ：48MP広角 + 13MP超広角</li>
<li>バッテリー5,100mAh / Gemini対応</li>
<li>SoC:Tensor G4 / <strong>ワイヤレス充電対応</strong> / 防水IPX8・防塵IP6X / 約183g</li>
</ul>
<ul lang="en" style="margin:0 0 .6rem;padding-left:1.3rem;font-size:.88rem;opacity:.85;">
<li>8GB RAM / 128GB or 256GB storage</li>
<li>Hi-res audio supported</li>
<li>About 6.3-inch OLED display</li>
<li>Camera: 48MP wide + 13MP ultra-wide</li>
<li>5,100mAh battery / Gemini</li>
<li>SoC: Tensor G4 / <strong>wireless charging supported</strong> / IPX8 and IP6X / about 183g</li>
</ul>
<p style="margin:.3rem 0 0;">他社から初めて乗り換えると最大16,000ポイント還元(事前エントリー必要)。</p>
<p lang="en" style="font-size:.88rem;opacity:.85;margin:.1rem 0 .5rem;">Up to 16,000 points back when switching from another carrier for the first time (pre-entry required).</p>
<p style="margin:.5rem 0 0;text-align:center;"><a href="https://network.mobile.rakuten.co.jp/product/smartphone/google-pixel-10a/" target="_blank" rel="noopener noreferrer">▶️ 楽天モバイル Google Pixel 10a / Rakuten Mobile</a></p>
</section>
<section class="arrows-we2-plus" style="max-width:420px;margin:1rem auto;padding:.9rem 1rem;border:1px solid rgba(128,128,128,.5);border-radius:10px;text-align:left;line-height:1.6;">
<h3 style="font-size:1.1rem;margin:0 0 .4rem;text-align:center;">③ 富士通 arrows We2 Plus(未使用品)</h3>
<p style="text-align:center;margin:0 0 .5rem;"><img src="data:image/webp;base64,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" alt="arrows We2 Plus 本体(正面・背面) / front and back" width="200" height="200" style="max-width:100%;height:auto;"></p>
<p style="margin:.3rem 0 0;"><strong>メモリ8GB版</strong>と<strong>12GB版</strong>があります(ストレージはどちらも256GB)。国産ブランドで、未使用品を探してみるのもおすすめです。</p>
<p lang="en" style="font-size:.88rem;opacity:.85;margin:.1rem 0 .5rem;">Available in <strong>8GB</strong> and <strong>12GB</strong> RAM versions (256GB storage for both). A Japanese-brand phone; looking for an unused (new, unopened) unit is also worth a try.</p>
<ul style="margin:.4rem 0 .1rem;padding-left:1.3rem;">
<li>メモリ8GB版(ドコモ・楽天モバイル版) / <strong>12GB版(SIMフリー版 M06。楽天モバイルの公式スペック表は8GB/256GBのみ)</strong></li>
<li>ストレージ256GB(microSDで最大1TBまで拡張可)</li>
<li>約6.6インチ 有機EL(フルHD+ 2,400×1,080)</li>
<li>カメラ：約5,010万画素広角(F1.8・光学手ブレ補正) + 約800万画素超広角 / 前面約1,610万画素</li>
<li>バッテリー5,000mAh / Snapdragon 7s Gen 2</li>
<li>防水(IPX5/IPX8)・防塵(IP6X)</li>
<li>おサイフケータイ対応 / 指紋認証・顔認証</li>
<li>脈波センサーで自律神経の活性度を測定(We2 Plusのみの機能)</li>
<li>約182g、約162×75×8.5mm</li>
<li><strong>ワイヤレス充電は非対応</strong> / イヤホンジャック対応 / Android 14</li>
</ul>
<ul lang="en" style="margin:0 0 .6rem;padding-left:1.3rem;font-size:.88rem;opacity:.85;">
<li>8GB RAM (Docomo / Rakuten Mobile versions) / <strong>12GB RAM (SIM-free version M06; Rakuten's spec table lists only 8GB/256GB)</strong></li>
<li>256GB storage (expandable up to 1TB with microSD)</li>
<li>About 6.6-inch OLED (Full HD+, 2,400x1,080)</li>
<li>Camera: 50.1MP wide (F1.8, optical image stabilization) + 8MP ultra-wide / 16.1MP front</li>
<li>5,000mAh battery / Snapdragon 7s Gen 2</li>
<li>Water resistance (IPX5/IPX8) and dust resistance (IP6X)</li>
<li>Osaifu-Keitai (mobile wallet) supported / fingerprint and face authentication</li>
<li>Pulse-wave sensor measures autonomic nervous system activity (We2 Plus only)</li>
<li>About 182g, about 162 x 75 x 8.5mm</li>
<li><strong>No wireless charging</strong> / headphone jack / Android 14</li>
</ul>
<p style="margin:.3rem 0 0;"><small>未使用品の価格・在庫は販売店ごとに異なります。最新の情報はご自身でご確認ください。</small></p>
<p lang="en" style="font-size:.88rem;opacity:.85;margin:.1rem 0 .5rem;">Prices and stock of unused units vary by seller. Please check the latest information yourself.</p>
<p style="margin:.5rem 0 0;text-align:center;"><a href="https://www.google.com/search?q=%E5%AF%8C%E5%A3%AB%E9%80%9A+arrows+we2+plus+%E6%9C%AA%E4%BD%BF%E7%94%A8" target="_blank" rel="noopener noreferrer">🔎 「富士通 arrows we2 plus 未使用」を検索 / Search</a><br><a href="https://network.mobile.rakuten.co.jp/product/smartphone/arrows-we2-plus/" target="_blank" rel="noopener noreferrer">▶️ 楽天モバイル arrows We2 Plus(仕様) / Specs</a></p>
</section>
<section class="oppo-reno11-a" style="max-width:420px;margin:1rem auto;padding:.9rem 1rem;border:1px solid rgba(128,128,128,.5);border-radius:10px;text-align:left;line-height:1.6;">
<h3 style="font-size:1.1rem;margin:0 0 .4rem;text-align:center;">④ OPPO Reno11 A(未使用品)</h3>
<p style="text-align:center;margin:0 0 .5rem;"><img src="data:image/webp;base64,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" alt="OPPO Reno11 A 本体(コーラルパープル・ダークグリーン) / Coral Purple and Dark Green" width="280" style="max-width:100%;height:auto;"></p>
<p style="margin:.3rem 0 0;"><strong>メモリ8GB</strong>と、<strong>アウトカメラの解像度が高い(約6,400万画素)</strong>のが特徴です。</p>
<p lang="en" style="font-size:.88rem;opacity:.85;margin:.1rem 0 .5rem;"><strong>8GB RAM</strong> and a <strong>high-resolution rear camera (about 64MP)</strong> are the key features.</p>
<ul style="margin:.4rem 0 .1rem;padding-left:1.3rem;">
<li>メモリ8GB / ストレージ128GB(microSDで最大2TBまで対応)</li>
<li>アウトカメラ：約6,400万画素メイン + 約800万画素超広角(112°) + 約200万画素マクロ / インカメラ約3,200万画素</li>
<li>約6.7インチ 有機EL、120Hz</li>
<li>バッテリー5,000mAh、急速充電対応</li>
<li>防水・防塵 IPX5 / IP6X / おサイフケータイ(FeliCa)対応</li>
<li>MediaTek Dimensity 7050 / 約177g、厚さ約7.6mm</li>
<li><strong>ワイヤレス充電は非対応</strong> / Android 14</li>
<li>カラー：ダークグリーン、コーラルパープル</li>
</ul>
<ul lang="en" style="margin:0 0 .6rem;padding-left:1.3rem;font-size:.88rem;opacity:.85;">
<li>8GB RAM / 128GB storage (microSD up to 2TB)</li>
<li>Rear camera: about 64MP main + about 8MP ultra-wide (112 degrees) + about 2MP macro / 32MP front camera</li>
<li>About 6.7-inch OLED, 120Hz</li>
<li>5,000mAh battery, fast charging supported</li>
<li>IPX5 / IP6X water and dust resistance / Osaifu-Keitai (FeliCa) supported</li>
<li>MediaTek Dimensity 7050 / about 177g, about 7.6mm thick</li>
<li><strong>No wireless charging</strong> / Android 14</li>
<li>Colors: Dark Green, Coral Purple</li>
</ul>
<p style="margin:.3rem 0 0;"><small>未使用品の価格・在庫は販売店ごとに異なります。最新の情報はご自身でご確認ください。</small></p>
<p lang="en" style="font-size:.88rem;opacity:.85;margin:.1rem 0 .5rem;">Prices and stock of unused units vary by seller. Please check the latest information yourself.</p>
<p style="margin:.5rem 0 0;text-align:center;"><a href="https://www.google.com/search?q=OPPO+RENO11A+%E6%9C%AA%E4%BD%BF%E7%94%A8" target="_blank" rel="noopener noreferrer">🔎 「OPPO RENO11A 未使用」を検索 / Search</a><br><a href="https://www.oppo.com/jp/smartphones/series-reno/reno11-a/specs/" target="_blank" rel="noopener noreferrer">▶️ OPPO Reno11 A(仕様) / Specs</a></p>
</section>
<p style="text-align:center;margin:0 0 1rem;"><small>(価格・条件は2026-10-26時点の楽天モバイル公式ページ記載。最新は公式で確認。Pixel 11・10aの分)<br><span lang="en">Prices and terms as listed on the Rakuten Mobile official pages on 2026-10-26 (Pixel 11 and 10a). Please check the official pages for the latest.</span></small></p>
</div>
<section class="top-news" style="max-width:640px;margin:1rem auto;text-align:center;">
<p style="font-size:1.2rem;margin:1rem 0;">面白そうなカフェ　検索ワード<br><a href="https://ameblo.jp/www-aon/entry-12981863547.html" target="_blank" rel="noopener noreferrer">https://ameblo.jp/www-aon/entry-12981863547.html</a></p>
<p style="text-align:center;font-size:1.2rem;margin:1rem 0;"><br>民間ガン治療法<br><br>ボランティア情報<br><br><a href="https://ameblo.jp/www-aon/entry-12981774738.html" target="_blank" rel="noopener noreferrer">https://ameblo.jp/www-aon/entry-12981774738.html</a></p>
</section>
<div class="top-intro" style="margin:1rem auto;padding:.8rem 1rem;max-width:42rem;text-align:left;line-height:1.8;font-size:1.1rem;">
    <p style="margin:0 0 .8rem;">CLAUDEと言うAIをマスターして使いこなしておりますので、<br>様々なプログラム言語とフレームワーク対応可能で、<br>様々なWEBサイトやスマホアプリ開発が可能で御座います。</p>
    <p style="margin:0;"><a href="mailto:norukia.jp@gmail.com">norukia.jp@gmail.com</a><br>
    石　塚　正　浩<br>
    郵便番号　197-0828<br>
    東京都あきる野市秋留5-3-11<br>
    <a href="tel:09075555011">090-7555-5011</a></p>
    <br>
    <p style="text-align:center;margin:1rem 0 .4rem"><img src="/images/falconlab-model-707.jpg" alt="FalconLab Model 707" style="max-width:100%;height:auto;max-height:560px"></p>
    <p style="font-size:1.2rem;margin:.4rem 0 1.4rem"><a href="https://www.google.com/search?ibp=oshop&amp;prds=pvt:hg,pvo:48,imageDocid:10771413835225635290,headlineOfferDocid:3061133778700411862,productDocid:3061133778700411862&amp;q=product&amp;sa=X&amp;ved=2ahUKEwjE6bOfhNiXAxVss1YBHWfiH1wQxa4PegoIAggACAAIGhAF&amp;biw=1144&amp;bih=722&amp;dpr=2" target="_blank" rel="noopener noreferrer">FalconLab Model 707</a><br>ペアで3,850,000円(税込)<br>・特徴: 3ウェイ・5スピーカー構成の無指向性（全指向性）バスレフ型<br>・音響効果: 部屋のどこにいても均一でリアルな立体音響空間（360度）を再現します。<br>公式ページ: <a href="https://www.falconlab.jp/model707.php" target="_blank" rel="noopener noreferrer">https://www.falconlab.jp/model707.php</a></p>
    <p style="text-align:center;margin:1rem 0 .4rem"><img src="/images/egretta-ts-a200as.jpg" alt="Egretta(エグレッタ) TS-A200as" style="max-width:100%;height:auto;max-height:560px"></p>
    <p style="font-size:1.2rem;margin:.4rem 0 1.4rem"><a href="https://www.amazon.co.jp/dp/B0832TFPDL" target="_blank" rel="noopener noreferrer">Egretta(エグレッタ)　TS-A200as<br>ペアで258,500円(税込)</a></p>
    <p style="font-size:1rem;margin:0 0 1.4rem">購入方法(ふるさと納税以外): <a href="https://www.egretta.jp/shop/" target="_blank" rel="noopener noreferrer">メーカー公式の取扱店(ビックカメラ・ヨドバシカメラ・エディオンなど)</a> ・ <a href="https://store.shopping.yahoo.co.jp/oasaelec/" target="_blank" rel="noopener noreferrer">オオアサ電子 公式ストア(Yahoo!ショッピング)</a> ・ <a href="https://www.amazon.co.jp/dp/B0832TFPDL" target="_blank" rel="noopener noreferrer">Amazon</a> ・ <a href="https://www.fujiya-avic.co.jp/shop/g/g200000052689/" target="_blank" rel="noopener noreferrer">フジヤエービック</a></p>
    <p style="font-size:1rem;margin:0 0 1.4rem">価格・レビュー: <a href="https://kakaku.com/item/K0001276754/" target="_blank" rel="noopener noreferrer">価格.com(価格比較。クチコミは0件)</a> ・ <a href="https://weekly.ascii.jp/elem/000/001/997/1997570/" target="_blank" rel="noopener noreferrer">週刊アスキー(試聴記事)</a></p>
    <p style="text-align:center;margin:1rem 0 .4rem"><img src="/images/egretta-ts-a200.jpg" alt="Egretta(エグレッタ) TS-A200" style="max-width:100%;height:auto;max-height:560px"></p>
    <p style="font-size:1.2rem;margin:.4rem 0 1rem">Egretta(エグレッタ)　TS-A200<br>スピーカーを2台セットにしたステレオモデル<a href="https://www.amazon.co.jp/dp/B0832TFPDL" target="_blank" rel="noopener noreferrer">「TS-A200as」</a>が215,600円(税込・発売時)<br>DACやアンプを抜いた、パッシブ型スピーカー<a href="https://www.amazon.co.jp/s?k=Egretta+TS-A200s" target="_blank" rel="noopener noreferrer">「TS-A200s」</a>は1台79,200円(税込・発売時)。</p>
    <p style="font-size:1rem;margin:0 0 1.4rem">購入方法(ふるさと納税以外): <a href="https://www.egretta.jp/shop/" target="_blank" rel="noopener noreferrer">メーカー公式の取扱店(ビックカメラ・ヨドバシカメラ・エディオンなど)</a> ・ <a href="https://store.shopping.yahoo.co.jp/oasaelec/" target="_blank" rel="noopener noreferrer">オオアサ電子 公式ストア(Yahoo!ショッピング)</a></p>
    <p style="font-size:1rem;margin:0 0 1.4rem">価格・レビュー: <a href="https://kakaku.com/item/K0001276754/" target="_blank" rel="noopener noreferrer">価格.com(TS-A200asの価格比較。クチコミは0件)</a> ・ <a href="https://weekly.ascii.jp/elem/000/001/997/1997570/" target="_blank" rel="noopener noreferrer">週刊アスキー(試聴記事)</a></p>
    <p style="text-align:center;margin:1rem 0 .4rem"><img src="/images/egretta-ts550f.jpg" alt="Egretta(エグレッタ) TS550F" style="max-width:100%;height:auto;max-height:560px"></p>
    <p style="font-size:1.2rem;margin:.4rem 0 1.4rem"><a href="https://www.amazon.co.jp/dp/B0B5D8W5W6" target="_blank" rel="noopener noreferrer">Egretta(エグレッタ)　TS550F<br>ペアで122,100円(税込)<br>Amazon</a></p>
    <p style="font-size:1rem;margin:0 0 1.4rem">購入方法(ふるさと納税以外): <a href="https://www.egretta.jp/shop/" target="_blank" rel="noopener noreferrer">メーカー公式の取扱店(ビックカメラ・ヨドバシカメラ・エディオンなど)</a> ・ <a href="https://store.shopping.yahoo.co.jp/oasaelec/" target="_blank" rel="noopener noreferrer">オオアサ電子 公式ストア(Yahoo!ショッピング)</a> ・ <a href="https://www.amazon.co.jp/dp/B0B5D8W5W6" target="_blank" rel="noopener noreferrer">Amazon</a></p>
    <p style="font-size:1rem;margin:0 0 1.4rem">価格・レビュー: <a href="https://kakaku.com/search_results/egretta/" target="_blank" rel="noopener noreferrer">価格.com(Egrettaの価格比較)</a> ・ <a href="https://www.phileweb.com/news/audio/202207/04/23481.html" target="_blank" rel="noopener noreferrer">PHILE WEB(紹介記事)</a> ・ <a href="https://av.watch.impress.co.jp/docs/news/1422230.html" target="_blank" rel="noopener noreferrer">AV Watch(紹介記事)</a></p>
  </div>
<section class="top-news" style="max-width:640px;margin:1rem auto;text-align:center;">
<p style="font-size:1.2rem;font-weight:700;margin:1.2rem 0 .6rem;"><a href="https://www.youtube.com/shorts/j_PGMrbhkj0" target="_blank" rel="noopener noreferrer">"4時間待ち"それでも外国人が殺到する日本のアニメイベントとは？</a></p>
<div style="position:relative;width:100%;max-width:640px;aspect-ratio:9/16;margin:0 auto 1.2rem;background:#000;border-radius:8px;overflow:hidden;">
<button type="button" aria-label="動画を再生 / Play video" style="all:unset;position:absolute;inset:0;cursor:pointer;" onclick="var f=document.createElement('iframe');f.src='https://www.youtube-nocookie.com/embed/j_PGMrbhkj0?autoplay=1&amp;playsinline=1&amp;rel=0';f.allow='autoplay; encrypted-media; picture-in-picture';f.allowFullscreen=true;f.title='"4時間待ち"それでも外国人が殺到する日本のアニメイベントとは？';f.style.cssText='position:absolute;inset:0;width:100%;height:100%;border:0';this.replaceWith(f)"><img src="https://i.ytimg.com/vi/j_PGMrbhkj0/hqdefault.jpg" alt=""4時間待ち"それでも外国人が殺到する日本のアニメイベントとは？" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block;"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);width:64px;height:64px;border-radius:50%;background:rgba(0,0,0,.65);color:#fff;font-size:30px;line-height:64px;text-align:center;">▶</span></button>
</div>
<p style="font-size:1.2rem;font-weight:700;margin:1.2rem 0 .6rem;"><a href="https://www.youtube.com/watch?v=FODDxEZDVsY" target="_blank" rel="noopener noreferrer">フランスのコミケで大号泣した</a></p>
<div style="position:relative;width:100%;aspect-ratio:16/9;margin:0 auto 1.2rem;background:#000;border-radius:8px;overflow:hidden;">
<button type="button" aria-label="動画を再生 / Play video" style="all:unset;position:absolute;inset:0;cursor:pointer;" onclick="var f=document.createElement('iframe');f.src='https://www.youtube-nocookie.com/embed/FODDxEZDVsY?autoplay=1&amp;playsinline=1&amp;rel=0';f.allow='autoplay; encrypted-media; picture-in-picture';f.allowFullscreen=true;f.title='フランスのコミケで大号泣した';f.style.cssText='position:absolute;inset:0;width:100%;height:100%;border:0';this.replaceWith(f)"><img src="https://i.ytimg.com/vi/FODDxEZDVsY/hqdefault.jpg" alt="フランスのコミケで大号泣した" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block;"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);width:64px;height:64px;border-radius:50%;background:rgba(0,0,0,.65);color:#fff;font-size:30px;line-height:64px;text-align:center;">▶</span></button>
</div>
<p style="font-size:1.2rem;margin:0 0 1.2rem;">ラピダスより重要OIST　EUV露光装置をASMLと比較　消費電力1/10　装置本体コスト1/4<br><a href="https://ameblo.jp/www-aon/entry-12982430784.html" target="_blank" rel="noopener noreferrer">https://ameblo.jp/www-aon/entry-12982430784.html</a></p>
<p style="font-size:1.2rem;font-weight:700;margin:0 0 .6rem;"><a href="https://www.youtube.com/watch?v=WWL5t0t_tEs" target="_blank" rel="noopener noreferrer">【ファナック】なぜ世界の工場は日本製の「頭脳」を選ぶのか？｜CNCが利益を生む本当の理由</a></p>
<div style="position:relative;width:100%;aspect-ratio:16/9;margin:0 auto 1.2rem;background:#000;border-radius:8px;overflow:hidden;">
<button type="button" aria-label="動画を再生 / Play video" style="all:unset;position:absolute;inset:0;cursor:pointer;" onclick="var f=document.createElement('iframe');f.src='https://www.youtube-nocookie.com/embed/WWL5t0t_tEs?autoplay=1&amp;playsinline=1&amp;rel=0';f.allow='autoplay; encrypted-media; picture-in-picture';f.allowFullscreen=true;f.title='【ファナック】なぜ世界の工場は日本製の「頭脳」を選ぶのか？｜CNCが利益を生む本当の理由';f.style.cssText='position:absolute;inset:0;width:100%;height:100%;border:0';this.replaceWith(f)"><img src="https://i.ytimg.com/vi/WWL5t0t_tEs/hqdefault.jpg" alt="【ファナック】なぜ世界の工場は日本製の「頭脳」を選ぶのか？｜CNCが利益を生む本当の理由" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block;"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);width:64px;height:64px;border-radius:50%;background:rgba(0,0,0,.65);color:#fff;font-size:30px;line-height:64px;text-align:center;">▶</span></button>
</div>
<p style="font-size:1.2rem;font-weight:700;margin:0 0 .6rem;"><a href="https://www.youtube.com/watch?v=L3ugAc-fgVY" target="_blank" rel="noopener noreferrer">【海外の反応】日本の製造業をなめるな！ファナック・安川電機がロボットアームで世界を動かす理由…エプソン・川崎重工・ヤマハ・安川電機【日本の技術】</a></p>
<div style="position:relative;width:100%;aspect-ratio:16/9;margin:0 auto 1.2rem;background:#000;border-radius:8px;overflow:hidden;">
<button type="button" aria-label="動画を再生 / Play video" style="all:unset;position:absolute;inset:0;cursor:pointer;" onclick="var f=document.createElement('iframe');f.src='https://www.youtube-nocookie.com/embed/L3ugAc-fgVY?autoplay=1&amp;playsinline=1&amp;rel=0';f.allow='autoplay; encrypted-media; picture-in-picture';f.allowFullscreen=true;f.title='【海外の反応】日本の製造業をなめるな！ファナック・安川電機がロボットアームで世界を動かす理由…エプソン・川崎重工・ヤマハ・安川電機【日本の技術】';f.style.cssText='position:absolute;inset:0;width:100%;height:100%;border:0';this.replaceWith(f)"><img src="https://i.ytimg.com/vi/L3ugAc-fgVY/hqdefault.jpg" alt="【海外の反応】日本の製造業をなめるな！ファナック・安川電機がロボットアームで世界を動かす理由…エプソン・川崎重工・ヤマハ・安川電機【日本の技術】" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block;"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);width:64px;height:64px;border-radius:50%;background:rgba(0,0,0,.65);color:#fff;font-size:30px;line-height:64px;text-align:center;">▶</span></button>
</div>
<p style="font-size:1.2rem;font-weight:700;margin:0 0 .6rem;"><a href="https://www.youtube.com/watch?v=5MFwZbfpKEk" target="_blank" rel="noopener noreferrer">この世界最大のタブー〝YHWH〟の正体がついに暴露されました。理想社会「弥勒の世」を実現する鍵は日本人が大切にしてきた〇〇だった！？この動画から時代がさらに動きます。</a></p>
<p style="font-size:1.2rem;margin:0 0 .6rem;">出口王仁三郎のひ孫の出口光（ひかる）さんがYHWHと一体になれる体験をビデオにまとめてくれております。</p>
<div style="position:relative;width:100%;aspect-ratio:16/9;margin:0 auto 1.2rem;background:#000;border-radius:8px;overflow:hidden;">
<button type="button" aria-label="動画を再生 / Play video" style="all:unset;position:absolute;inset:0;cursor:pointer;" onclick="var f=document.createElement('iframe');f.src='https://www.youtube-nocookie.com/embed/5MFwZbfpKEk?autoplay=1&amp;playsinline=1&amp;rel=0';f.allow='autoplay; encrypted-media; picture-in-picture';f.allowFullscreen=true;f.title='YHWH';f.style.cssText='position:absolute;inset:0;width:100%;height:100%;border:0';this.replaceWith(f)"><img src="https://i.ytimg.com/vi/5MFwZbfpKEk/hqdefault.jpg" alt="YHWH" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block;"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);width:64px;height:64px;border-radius:50%;background:rgba(0,0,0,.65);color:#fff;font-size:30px;line-height:64px;text-align:center;">▶</span></button>
</div>
<p style="font-size:.9rem;margin:-.6rem 0 1.2rem;">YouTubeのリンクが切れたら次のURLでもご視聴になれます。 / If the YouTube link breaks, you can also watch it via the following URL:<br><a href="https://www.facebook.com/reel/1702321541402846?locale=ja_JP" target="_blank" rel="noopener noreferrer">https://www.facebook.com/reel/1702321541402846?locale=ja_JP</a></p>
<p style="font-size:1.2rem;margin:1.2rem 0;">学校の教科書から マインクラフトを超えたオンライン3D GAMEを作る方法！<br><a href="https://ameblo.jp/www-aon/entry-12981886969.html" target="_blank" rel="noopener noreferrer">https://ameblo.jp/www-aon/entry-12981886969.html</a></p>
<p style="font-size:1.2rem;font-weight:700;margin:1.2rem 0 .6rem;"><a href="https://www.youtube.com/watch?v=TooMBNl0lAw" target="_blank" rel="noopener noreferrer">日本が「石油」を超えた！世界を変える新発明の正体</a></p>
<div style="position:relative;width:100%;aspect-ratio:16/9;margin:0 auto 1.2rem;background:#000;border-radius:8px;overflow:hidden;">
<button type="button" aria-label="動画を再生 / Play video" style="all:unset;position:absolute;inset:0;cursor:pointer;" onclick="var f=document.createElement('iframe');f.src='https://www.youtube-nocookie.com/embed/TooMBNl0lAw?autoplay=1&amp;playsinline=1&amp;rel=0';f.allow='autoplay; encrypted-media; picture-in-picture';f.allowFullscreen=true;f.title='日本が「石油」を超えた！世界を変える新発明の正体';f.style.cssText='position:absolute;inset:0;width:100%;height:100%;border:0';this.replaceWith(f)"><img src="https://i.ytimg.com/vi/TooMBNl0lAw/hqdefault.jpg" alt="日本が「石油」を超えた！世界を変える新発明の正体" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block;"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);width:64px;height:64px;border-radius:50%;background:rgba(0,0,0,.65);color:#fff;font-size:30px;line-height:64px;text-align:center;">▶</span></button>
</div>
<p style="font-size:1.2rem;margin:1.2rem 0;">V6＆V8＆V10＆V12マルチステージターボエンジン開発を国家プロジェクトにするアイデア！<br><a href="https://ameblo.jp/www-aon/entry-12981432312.html" target="_blank" rel="noopener noreferrer">https://ameblo.jp/www-aon/entry-12981432312.html</a></p>
<p style="font-size:1.2rem;font-weight:700;margin:0 0 .6rem;"><a href="https://ameblo.jp/www-aon/entry-12981724246.html" target="_blank" rel="noopener noreferrer">今の上海は経済崩壊でシャッター街なの？</a></p>
<div style="position:relative;width:100%;max-width:640px;aspect-ratio:9/16;margin:0 auto;background:#000;border-radius:8px;overflow:hidden;">
<button type="button" aria-label="動画を再生 / Play video" style="all:unset;position:absolute;inset:0;cursor:pointer;" onclick="var f=document.createElement('iframe');f.src='https://www.youtube-nocookie.com/embed/oPGV1n8niXw?autoplay=1&amp;playsinline=1&amp;rel=0';f.allow='autoplay; encrypted-media; picture-in-picture';f.allowFullscreen=true;f.title='今の上海は経済崩壊でシャッター街なの？';f.style.cssText='position:absolute;inset:0;width:100%;height:100%;border:0';this.replaceWith(f)"><img src="https://i.ytimg.com/vi/oPGV1n8niXw/hqdefault.jpg" alt="今の上海は経済崩壊でシャッター街なの？" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block;"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);width:64px;height:64px;border-radius:50%;background:rgba(0,0,0,.65);color:#fff;font-size:30px;line-height:64px;text-align:center;">▶</span></button>
</div>
<p style="font-size:1.2rem;margin:1.2rem 0;">TOYOTAの方針では、<br>中国と日本の工場の生産コストを同じにする為に、ロボットや3Dプリンターなどのハイテクと熟練職人を育てて高級車を担当して頂く事などのバランスを重要視していくと言われております。</p>
<p style="font-size:1.2rem;font-weight:700;margin:1.2rem 0 .6rem;"><a href="https://www.youtube.com/shorts/2lileYb0OnQ" target="_blank" rel="noopener noreferrer">ロボットの巨人！安川電機のIoT！精緻なデータ収集が可能！【IIFES】</a></p>
<div style="position:relative;width:100%;max-width:640px;aspect-ratio:9/16;margin:0 auto;background:#000;border-radius:8px;overflow:hidden;">
<button type="button" aria-label="動画を再生 / Play video" style="all:unset;position:absolute;inset:0;cursor:pointer;" onclick="var f=document.createElement('iframe');f.src='https://www.youtube-nocookie.com/embed/2lileYb0OnQ?autoplay=1&amp;playsinline=1&amp;rel=0';f.allow='autoplay; encrypted-media; picture-in-picture';f.allowFullscreen=true;f.title='ロボットの巨人！安川電機のIoT！精緻なデータ収集が可能！【IIFES】';f.style.cssText='position:absolute;inset:0;width:100%;height:100%;border:0';this.replaceWith(f)"><img src="https://i.ytimg.com/vi/2lileYb0OnQ/hqdefault.jpg" alt="ロボットの巨人！安川電機のIoT！精緻なデータ収集が可能！【IIFES】" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block;"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);width:64px;height:64px;border-radius:50%;background:rgba(0,0,0,.65);color:#fff;font-size:30px;line-height:64px;text-align:center;">▶</span></button>
</div>
<p style="font-size:1.2rem;font-weight:700;margin:1.2rem 0 .6rem;"><a href="https://www.youtube.com/shorts/jW5qS1ZQbsg" target="_blank" rel="noopener noreferrer">中韓には一生不可能な技術を日本がサクっと開発</a></p>
<div style="position:relative;width:100%;max-width:640px;aspect-ratio:9/16;margin:0 auto;background:#000;border-radius:8px;overflow:hidden;">
<button type="button" aria-label="動画を再生 / Play video" style="all:unset;position:absolute;inset:0;cursor:pointer;" onclick="var f=document.createElement('iframe');f.src='https://www.youtube-nocookie.com/embed/jW5qS1ZQbsg?autoplay=1&amp;playsinline=1&amp;rel=0';f.allow='autoplay; encrypted-media; picture-in-picture';f.allowFullscreen=true;f.title='中韓には一生不可能な技術を日本がサクっと開発';f.style.cssText='position:absolute;inset:0;width:100%;height:100%;border:0';this.replaceWith(f)"><img src="https://i.ytimg.com/vi/jW5qS1ZQbsg/hqdefault.jpg" alt="中韓には一生不可能な技術を日本がサクっと開発" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block;"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);width:64px;height:64px;border-radius:50%;background:rgba(0,0,0,.65);color:#fff;font-size:30px;line-height:64px;text-align:center;">▶</span></button>
</div>
<p style="font-size:1.2rem;font-weight:700;margin:1.2rem 0 .6rem;"><a href="https://www.youtube.com/watch?v=l5qJAHhvvBs" target="_blank" rel="noopener noreferrer">ロボットで一括りにするな！ロボット業界（ファナック・安川電機）を完全解説。業界研究・企業研究・志望動機、これ1本！</a></p>
<div style="position:relative;width:100%;aspect-ratio:16/9;margin:0 auto;background:#000;border-radius:8px;overflow:hidden;">
<button type="button" aria-label="動画を再生 / Play video" style="all:unset;position:absolute;inset:0;cursor:pointer;" onclick="var f=document.createElement('iframe');f.src='https://www.youtube-nocookie.com/embed/l5qJAHhvvBs?autoplay=1&amp;playsinline=1&amp;rel=0';f.allow='autoplay; encrypted-media; picture-in-picture';f.allowFullscreen=true;f.title='ロボットで一括りにするな！ロボット業界（ファナック・安川電機）を完全解説。業界研究・企業研究・志望動機、これ1本！';f.style.cssText='position:absolute;inset:0;width:100%;height:100%;border:0';this.replaceWith(f)"><img src="https://i.ytimg.com/vi/l5qJAHhvvBs/hqdefault.jpg" alt="ロボットで一括りにするな！ロボット業界（ファナック・安川電機）を完全解説。業界研究・企業研究・志望動機、これ1本！" loading="lazy" style="width:100%;height:100%;object-fit:cover;display:block;"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);width:64px;height:64px;border-radius:50%;background:rgba(0,0,0,.65);color:#fff;font-size:30px;line-height:64px;text-align:center;">▶</span></button>
</div>
<p style="font-size:1.2rem;margin:1.2rem 0 .6rem;">オリエンタルモーターの低コスト産業用ロボット<br><a href="https://www.youtube.com/results?search_query=%E3%82%AA%E3%83%AA%E3%82%A8%E3%83%B3%E3%82%BF%E3%83%AB%E3%83%A2%E3%83%BC%E3%82%BF%E3%83%BC%E3%81%AE%E4%BD%8E%E3%82%B3%E3%82%B9%E3%83%88%E7%94%A3%E6%A5%AD%E7%94%A8%E3%83%AD%E3%83%9C%E3%83%83%E3%83%88" target="_blank" rel="noopener noreferrer">Youtube検索結果はこちら</a></p>
<p style="font-size:1.2rem;margin:1.2rem 0 .6rem;">Google検索ワード<br><a href="https://www.google.com/search?q=%E4%BA%BA%E5%B7%A5%E5%85%89%E5%90%88%E6%88%90%E3%82%92%E5%88%A9%E7%94%A8%E3%81%97%E3%81%A6%E4%BA%8C%E9%85%B8%E5%8C%96%E7%82%AD%E7%B4%A0%EF%BC%88CO2%EF%BC%89%E3%81%A8%E6%B0%B4%E3%81%8B%E3%82%89%E5%B8%B8%E6%B8%A9%E3%83%BB%E5%B8%B8%E5%9C%A7%E3%81%A7%E6%89%B1%E3%81%88%E3%81%A6%E3%80%81%E8%BC%B8%E9%80%81%E3%81%AB%E9%AB%98%E5%9C%A7%E3%83%9C%E3%83%B3%E3%83%99%E3%82%84%E6%A5%B5%E4%BD%8E%E6%B8%A9%E5%86%B7%E5%8D%B4%E3%81%8C%E5%BF%85%E8%A6%81%E7%84%A1%E3%81%84%E3%80%8C%E3%82%AE%E9%85%B8%EF%BC%88HCOOH%EF%BC%89%E3%80%8D%E3%82%92%E7%94%9F%E6%88%90%E3%81%99%E3%82%8B%E6%8A%80%E8%A1%93" target="_blank" rel="noopener noreferrer">人工光合成を利用して二酸化炭素（CO2）と水から常温・常圧で扱えて、輸送に高圧ボンベや極低温冷却が必要無い「ギ酸（HCOOH）」を生成する技術</a><br>上記リンクをクリック後、AIモードをクリックしてご覧下さいませ。</p>
<p style="font-size:1.2rem;margin:0;"><br>ギ酸は、水素キャリアになります。<br>常温核融合は、数年に一度、水素燃料を補充するだけで、発電し続ける様です。</p>
<p style="font-size:1.2rem;margin:0;"><br>Youtube検索ワード<br><a href="https://www.youtube.com/results?search_query=%E5%B8%B8%E6%B8%A9%E6%A0%B8%E8%9E%8D%E5%90%88%E3%82%B9%E3%83%88%E3%83%BC%E3%83%96++%E5%B8%B8%E6%B8%A9%E6%A0%B8%E8%9E%8D%E5%90%88%E6%9A%96%E6%88%BF%E6%A9%9F++%E5%B0%8F%E5%9E%8B%E6%A0%B8%E8%9E%8D%E5%90%88%E6%9A%96%E6%88%BF%E6%A9%9F+%E3%83%9F%E3%83%A5%E3%82%A6%E3%82%AA%E3%83%B3++%E3%80%90%E3%83%9F%E3%83%A5%E3%82%AA%E3%83%B3%E3%81%AB%E3%82%88%E3%82%8B%E5%B8%B8%E6%B8%A9%E6%A0%B8%E8%9E%8D%E5%90%88%E3%80%91" target="_blank" rel="noopener noreferrer">常温核融合ストーブ  常温核融合暖房機  小型核融合暖房機 ミュウオン  【ミュオンによる常温核融合】</a></p>
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
        return rust_json::parse_strict(&text).map_err(|e| e.to_string());
    }
    let url = format!("{CACHE_BASE}/{filename}");
    let text = reqwest::get(&url).await.map_err(|e| e.to_string())?.text().await.map_err(|e| e.to_string())?;
    rust_json::parse_strict(&text).map_err(|e| e.to_string())
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
    let columns: Vec<&String> = first.keys().collect();
    let uniform = items.iter().all(|v| {
        v.as_object().map(|o| {
            let keys: Vec<&String> = o.keys().collect();
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
                let cell = item.get(col.as_str());
                let rendered = match cell {
                    Some(Value::String(s)) if s.starts_with("http://") || s.starts_with("https://") => {
                        format!(r#"<a href="{0}" target="_blank" rel="noopener noreferrer">🔎 リンク</a>"#, html_escape(s))
                    }
                    Some(Value::String(s)) => html_escape(s),
                    Some(v) => html_escape(&v.to_string()),
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
                let pairs: Vec<String> = obj
                    .iter()
                    .map(|(k, v)| format!("{}: {}", html_escape(k), html_escape(&v.to_string())))
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
        return html_escape(&data.to_string());
    };

    // 先に見出し的なスカラーフィールド(updated_at/disclaimer等)を出す。
    for key in ["updated_at", "crawled_at", "disclaimer"] {
        if let Some(v) = obj.get(key).and_then(|v| v.as_str()) {
            out.push_str(&format!("<p class=\"disclaimer\">{}: {}</p>", html_escape(key), html_escape(v)));
        }
    }

    for (key, value) in obj {
        if matches!(key.as_str(), "updated_at" | "crawled_at" | "disclaimer") {
            continue;
        }
        match value {
            Value::String(s) if s.starts_with("http://") || s.starts_with("https://") => {
                out.push_str(&format!(
                    r#"<p><strong>{}:</strong> <a href="{}" target="_blank" rel="noopener noreferrer">🔎 リンク</a></p>"#,
                    html_escape(key), html_escape(s)
                ));
            }
            Value::String(s) => {
                out.push_str(&format!("<p><strong>{}:</strong> {}</p>", html_escape(key), html_escape(s)));
            }
            Value::Number(_) | Value::Bool(_) => {
                out.push_str(&format!("<p><strong>{}:</strong> {}</p>", html_escape(key), html_escape(&value.to_string())));
            }
            Value::Array(items) if items.iter().all(|v| v.is_string()) => {
                let list: String = items.iter().map(|v| format!("<li>{}</li>", html_escape(v.as_str().unwrap_or("")))).collect();
                out.push_str(&format!("<h{n}>{k}</h{n}><ul>{list}</ul>", n = (depth + 2).min(6), k = html_escape(key)));
            }
            Value::Array(items) if !items.is_empty() && items.iter().all(|v| v.is_object()) => {
                out.push_str(&format!("<h{n}>{k}</h{n}>", n = (depth + 2).min(6), k = html_escape(key)));
                out.push_str(&render_object_array(items));
            }
            Value::Object(_) => {
                out.push_str(&format!("<h{n}>{k}</h{n}>", n = (depth + 2).min(6), k = html_escape(key)));
                out.push_str(&render_value_generic(value, depth + 1));
            }
            _ => {}
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
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
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

    let empty = Value::Null;
    let caba = caba.as_ref().unwrap_or(&empty);
    let jukujo = jukujo.as_ref().unwrap_or(&empty);
    let tv_group = tv_group.as_ref().unwrap_or(&empty);
    let tv_normal = tv_normal.as_ref().unwrap_or(&empty);

    let empty_rows: Vec<Value> = Vec::new();
    let caba_tokyo23: &[Value] = caba.get("tokyo_23").and_then(|v| v.get("rows")).and_then(|v| v.as_array()).unwrap_or(&empty_rows);
    let caba_tama: &[Value] = caba.get("tokyo_tama").and_then(|v| v.get("rows")).and_then(|v| v.as_array()).unwrap_or(&empty_rows);
    let caba_national: &[Value] = caba.get("national").and_then(|v| v.get("rows")).and_then(|v| v.as_array()).unwrap_or(&empty_rows);
    let caba_disclaimer = get_disp(caba, "disclaimer");
    let caba_updated = get_disp(caba, "updated_at");

    let jukujo_rows: &[Value] = jukujo.get("rows").and_then(|v| v.as_array()).unwrap_or(&empty_rows);
    let jukujo_disclaimer = get_disp(jukujo, "disclaimer");
    let jukujo_updated = get_disp(jukujo, "updated_at");

    let tvg_rows: &[Value] = tv_group.get("rows").and_then(|v| v.as_array()).unwrap_or(&empty_rows);
    let tvg_disclaimer = get_disp(tv_group, "disclaimer");
    let tvg_updated = get_disp(tv_group, "updated_at");

    let tvn_rows: &[Value] = tv_normal.get("rows").and_then(|v| v.as_array()).unwrap_or(&empty_rows);
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
        let mut keys: Vec<&String> = entries.keys().filter(|k| k.starts_with(&format!("{prefix}:"))).collect();
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

    let empty = Value::Null;
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

    let empty = Value::Null;
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
<div class="yt-click-play" data-vid="3DrVc07KdyQ" style="font-size:1.2rem;margin:.8rem auto;max-width:640px;text-align:center">
<p class="blog-link" style="font-size:1.2rem;margin:0 0 .5rem"><a href="https://www.youtube.com/watch?v=3DrVc07KdyQ" onclick="return acClickPlay(this)">▶ 歌心りえ　雪の華</a><br><small style="font-size:.8rem"><a href="https://www.facebook.com/reel/1138017215474814?locale=ja_JP" target="_blank" rel="noopener noreferrer">Facebook(予備 / backup)</a></small></p>
<a class="yt-thumb" href="https://www.youtube.com/watch?v=3DrVc07KdyQ" onclick="return acClickPlay(this)" style="display:block;position:relative"><img src="https://i.ytimg.com/vi/3DrVc07KdyQ/hqdefault.jpg" alt="歌心りえ　雪の華" style="width:100%;height:auto;display:block;border-radius:8px"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);font-size:3rem;color:#fff;background:rgba(0,0,0,.6);border-radius:50%;width:5rem;height:5rem;line-height:5rem;text-align:center">▶</span></a>
</div>
<div class="yt-click-play" data-vid="sRh9t3q2R2M" style="font-size:1.2rem;margin:.8rem auto;max-width:640px;text-align:center">
<p class="blog-link" style="font-size:1.2rem;margin:0 0 .5rem"><a href="https://www.youtube.com/watch?v=sRh9t3q2R2M" onclick="return acClickPlay(this)">▶ 歌心りえ　雪の華　ライブ#TVCHOSUNMUSIC (TV CHOSUN 251231 방송)</a><br><small style="font-size:.8rem"><a href="https://www.facebook.com/reel/1442808854663565?locale=ja_JP" target="_blank" rel="noopener noreferrer">Facebook(予備 / backup)</a></small></p>
<a class="yt-thumb" href="https://www.youtube.com/watch?v=sRh9t3q2R2M" onclick="return acClickPlay(this)" style="display:block;position:relative"><img src="https://i.ytimg.com/vi/sRh9t3q2R2M/hqdefault.jpg" alt="歌心りえ　雪の華　ライブ#TVCHOSUNMUSIC (TV CHOSUN 251231 방송)" style="width:100%;height:auto;display:block;border-radius:8px"><span style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);font-size:3rem;color:#fff;background:rgba(0,0,0,.6);border-radius:50%;width:5rem;height:5rem;line-height:5rem;text-align:center">▶</span></a>
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
<p class="blog-link" style="font-size:1.2rem;margin:0 0 .5rem"><a href="https://www.youtube.com/watch?v=u2-aAlOPzIY" onclick="return acClickPlay(this)">▶ 歌心りえの家族　瑠璃色の地球</a></p>
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
{}", TOP_AMP_HTML), 1))
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
