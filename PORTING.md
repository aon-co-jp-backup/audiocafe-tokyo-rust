# PORTING.md — audiocafe-tokyo-rust お引越しファイル

> このファイル1枚で、他プロジェクトへ `audiocafe-tokyo-rust` を導入・移設できます。
> 対象バージョン: 0.3.0(2026-07-19、国旗クリック導線・翻訳リンク・
> YouTubeシリーズ切替を追加——詳細は`CLAUDE.md`のHANDOFFログ参照)。

## 0. このリポジトリのスコープ

`audiocafe-tokyo-rust`(旧フォルダ名 `audiocafe-tokyo-server`)は、
既存PHPモノリス [`audiocafe-tokyo`](https://github.com/aon-co-jp/audiocafe-tokyo)
(`index.php`単体445KB・189関数・8146行)をRust + **RPoem**
(`open-runo-router::hyper_compat`、外部`poem`クレートへの直接依存を
断つエコシステム方針、2026-07-19に移行完了)へ段階的に移行する
リポジトリ。既存PHP実装は上書きせず、独立した移行先として運用している。

**2026-07-19時点の到達度**:
トップページ(`/`)・`/aruaru`・`/aruaru-lady`・`/rakuten-mobile`の
**4ページ**は、PHP版と**内容+見た目(CSS/クラス構造)の両方が一致**
するレベルまで移植済みで、**本番`audiocafe.tokyo`ドメインで実際に
Rust版が稼働中**(nginx `location = /`および`location /aruaru/`等の
個別パスでプロキシ)。トップページは147/147言語カード全件・各カードの
全文エッセイ(政治・宗教等の主張を含め実際に公開済みの内容をそのまま
複製)・`cardLinks`・YouTube背景プレイヤー(**77件のシリーズ切替+
NEXT+OPEN/CLOSEパネル、再実装済み**)・無料壁紙コーナーまで含む。
さらに国旗画像自体がクリック可能(Google翻訳プロキシ経由でaudiocafe.tokyo
本体へ)、`.card-actions`の`/aruaru`・`/aruaru-lady`・`/rakuten-mobile`
リンクも選択言語のGoogle翻訳プロキシ経由になった(2026-07-19)。
このRustサイトは従来「クライアント側JSを持たない」方針だったが、
YouTubeシリーズ切替機能に限り小さな自己完結スクリプトを初めて導入した
(詳細・スコープ外事項は`CLAUDE.md`のHANDOFFログ2026-07-19エントリ参照)。

## 1. 持っていくもの(ファイル一覧)

```
audiocafe-tokyo-rust/
├── Cargo.toml / Cargo.lock
├── src/
│   ├── main.rs        # ルーティング・汎用レンダラー(render_value_generic)・複合ページ
│   ├── scraper.rs      # /discover 用スクレイピングロジック(旧PHPの実アルゴリズム部分)
│   └── seed_urls.rs    # シードURL360件(機械的にそのまま移植)
├── PORTING.md(本ファイル)
├── CLAUDE.md
└── README.md
```

丸ごと移設する場合はフォルダごとコピーして `cargo build --release` が
通れば移設成功。

## 2. ビルド・起動

```bash
cargo build --release
./target/release/audiocafe-tokyo-server   # 既定バインド 127.0.0.1:4400
```

## 3. ページ構成

- `/` — 対応済みランキング一覧
- `/ranking/:slug` — 個別ランキング表示(`aruaru-caba`/`aruaru-eikaiwa`/`aruaru-jukujo-caba`等、全8種)
- `/page/:slug` — 複合ページ(`aruaru`/`aruaru-lady`/`rakuten-mobile`、複数キャッシュを1ページに統合)
- `/discover` — 動画・記事・写真収集ページ(`src/scraper.rs`、シードURL360件から収集)
- `/healthz` — ヘルスチェック

## 4. データ取得方式

`*-cache.json`(PHP側がファイルベースで生成し `https://audiocafe.tokyo/`
直下に静的公開しているジャンル別ランキングキャッシュ)を**HTTP経由で
取得**し(サーバー間の直接ファイル共有を前提にしない疎結合設計)、
[`rust_json::parse_strict`](https://github.com/aon-co-jp/Rust-JSON)で
パースして `render_value_generic`(`src/main.rs`)で汎用的にレンダリングする。
キャッシュのスキーマは8種類あるが、形状ごとの専用コードは書かず、
完全再帰の汎用レンダラー1つで全種類をカバーしている。

移設先で別のキャッシュJSON形状が増えても、既存の`render_value_generic`
がそのまま通用するかをまず確認し、専用コードの追加は最終手段とすること。

## 5. まだ移植していないもの(ごまかさず明記、2026-07-19更新)

- **PHP版の元の遷移先選択モーダル(`#acNavChoiceModal`)自体のJS
  アニメーション**: 到達できる行き先(audiocafe.tokyo本体/aruaru/
  aruaru-lady/rakuten-mobile/aruaru.tokyo/Google Translate、いずれも
  国旗画像・`.card-actions`双方から選択言語のGoogle翻訳プロキシ経由で
  到達可能)は同一だが、モーダルの開閉演出自体は静的直リンクのまま。
- **YouTube再生リストのシリーズ機能(`SEARCH_SERIES`、実際は77件——
  旧HANDOFFの「84件」という記載は未検証の見積もりで、Node.jsでの
  lossless評価により77件と訂正)**: 2026-07-19に復活済み。77件全ての
  シリーズボタン・NEXTでのキュー送り・OPEN/CLOSEパネル切替を実装
  (このRustサイト初のクライアント側JS、小さな自己完結スクリプト)。
  ただし、PHP版が持つYouTube IFrame API連携(自動キュー送り・
  シャッフル・検索駆動の動画切替アニメーション)は対象外——
  NEXTボタンでの手動送りのみ。非再生可能なシリーズ(`results?
  search_query=`のみ等)は実際のYouTube検索結果ページへ新規タブで
  遷移(`audiocafe.tokyo/CLAUDE.md`のスクレイプ禁止方針を踏襲)。
- 多言語版(`index-en.php`・`index-fr.php`等、`/aruaru/`だけで12言語)— 日本語版相当のみ。トップページの言語カード経由でのGoogle翻訳プロキシ閲覧は可能。
- **`/cancer`・`/Python`・`/video`・`/world`(削除予定)ディレクトリ**:
  静的コンテンツ・配布ツールファイルで、Rust側では未移植のままnginxの
  静的配信(PHP版と同じドキュメントルート)に依存している。
- クライアント側JavaScriptの演出のうち、上記のYouTubeシリーズ切替
  以外の細部(検索駆動の動画切替アニメーション、シャッフル等)。

## 6. 本番デプロイ(2026-07-19、カットオーバー実施済み)

VPS上 `/root/audiocafe-tokyo-rust`(GitHubからclone、git管理下)で
`cargo build --release`、systemdサービス化
(`audiocafe-tokyo-rust.service`、`127.0.0.1:4400`)。

`/etc/nginx/conf.d/audiocafe.tokyo.conf`に以下を追加し、**本番
`https://audiocafe.tokyo/`で実際にRust版が稼働中**:
- `location = /`(完全一致のみ、prefix matchより優先)→ `127.0.0.1:4400/`
- `location /aruaru/`・`/aruaru-lady/`・`/rakuten-mobile/`
  (各prefix match)→ `127.0.0.1:4400/aruaru`等

これら以外の全パス(`location /`のprefix match、`/top/`・`/cancer/`・
`/Python/`・`/video/`・静的キャッシュJSON等)は**無変更のままPHP側が
処理を継続**——`location =`は完全一致のみを奪うnginxの通常規則を
利用しているため、既存コンテンツへの影響は無い。設定変更前は必ず
`cp audiocafe.tokyo.conf audiocafe.tokyo.conf.bak-<timestamp>`で
バックアップを取ってから編集すること。

他環境へ移設する場合は、まず対象ドメインの実際のドキュメントルート
構成(このドメイン固有の`/top/`等の個人情報・配布物ディレクトリの
有無)を調査してから、同じ「完全一致のみを奪う」パターンで安全に
段階的カットオーバーすること——`location /`を丸ごと置き換えると、
Rust側に実装のないパスが軒並み404になる実害があることを確認済み
(`CLAUDE.md`のHANDOFFログ2026-07-19参照)。

## 7. 命名規約

- クレート名・バイナリ名: `audiocafe-tokyo-server`(実行ファイル名は移行時のまま据え置き)
- リポジトリ名・フォルダ名: `audiocafe-tokyo-rust`(2026-07-18、ローカルフォルダ名をリモートリポジトリ名に合わせて改名)

## 8-1. `assets/search_series.json`(YouTube再生リストシリーズ)のデータ整備注意点(2026-07-29追記)

- `search_series.json`は`include_str!`でコンパイル時に埋め込まれる
  ため、**このJSON/対応するPHP版`index.php`内の同名データを編集して
  もpushしただけでは本番に反映されない**——必ずVPS側で`git pull`→
  `cargo build --release`→`systemctl restart audiocafe-tokyo-rust`
  (PHP側は`index.php`を`/var/www/audiocafe.tokyo/`へ直接アップロード)
  まで行い、実HTTPで反映を確認すること(2026-07-29に実際にこの手順を
  怠り、ユーザーから「pushしたのに反映されていない」と指摘された
  実障害あり、詳細はCLAUDE.md HANDOFF参照)。
- 動画IDを1本も含まないシリーズ(会社公式サイト・Google検索/画像検索
  リンクのみ等)は、`s.urls`の**最初のURLをそのまま**リンク先として
  使う設計(2026-07-29修正、旧実装はラベル文字列から常にYouTube検索を
  勝手に合成する実バグがあった)。新しいシリーズを追加する際、動画を
  含めないなら`urls`に実際に開いてほしいURLを1件だけ入れること。

## 8. 移植・拡張時の注意

新しいキャッシュJSONの形状が増えた場合は、まず既存の汎用レンダラー
(`render_value_generic`)で描画できるかを試すこと。形状別の専用コードを
安易に追加すると、8種類対応時に一度解消した「形状ごとの分岐地獄」に
逆戻りする。技術選定・PHP側アルゴリズムの解釈で迷う場合は、
学習データからの推測のみに頼らず、実際のVPS上のPHPソース・実データを
確認してから移植すること。

## オンライン学習サイト・AI駆動開発の研究記事の掲載と引越し(2026-10-08/14)
`/aruaru`の一番上と「おすすめ学習サービスTOP50」の冒頭に、オンライン学習サイト5件と研究記事(`ONLINE_LEARNING_LIST_HTML` / `AI_DRIVEN_RESEARCH_HTML`)を追加。本番で表示確認済み(`aruaru.tokyo/aruaru/`のミラーにも出る)。
**未了**: AIが作った学習リンク(`aruaru/aruaru-tech-learning-ai-cache.json`、80技術・約770件、価格欄は訂正済み)のRust版への表示。
**お引越し**: 本番は`/root/audiocafe-tokyo-rust`で`cargo build --release`→`systemctl restart audiocafe-tokyo-rust.service`(ポート4400)。キャッシュJSONは`/var/www/audiocafe.tokyo`から読むため、PHP側の毎朝クロールが先に必要。
手順全体・systemd・nginxの設定は`audiocafe-tokyo`の`PORTING.md`と`deploy/vps/`。

## API 方針: REST 全廃・gRPC / Connect へ (2026-11-04)

### 日本語
- **REST は全廃する。新しい API は REST で作らない。** ブラウザや `curl` から呼ぶ API は **Connect の単純呼び出し**(`POST /<パッケージ>.<Service>/<Method>`、`content-type: application/json`、本文は JSON)、サービス間は **gRPC / gRPC-web**(RPoem の管理API `open_runo.admin.v1.Admin` と同じ流儀)。ストリーミングは、必要になってから。
- **既存の REST は、触るときに順に移す。** 移したら旧 REST の経路は消す。移行が終わるまで残るが、**新規には追加しない**。
- **JSON は RS-JSON に統一**する(読み込みは `from_str_secure` / `parse_secure` でサイズ・深さ・重複キーの上限つき、書き出しは `rust_json::fast`)。ブラウザの WASM に `prost` は入れない(サイズが増えるため)。
- **例外(自分たちの API ではなく、相手の規格が決めているもの)**: ACME、OAuth/OIDC のコールバック、第三者(GitHub など)からの Webhook、ヘルスチェック、静的ファイル配信、OTLP など。これらは規格どおりの HTTP を受ける。
- 参考実装: `RPoem`(`open_runo.admin.v1.Admin`)、`open-easy-web`(`server/src/connect.rs`、`proto/open_easy_web/admin/v1/admin.proto`)。
- ※ このリポジトリのコードが既に方針どおりかどうかは、各リポジトリの現状による。この節は方針の記録であり、コードの変更を意味しない。

### English
- **REST is being abolished. New APIs are not built as REST.** APIs called from browsers or `curl` use **Connect unary calls** (`POST /<package>.<Service>/<Method>`, `content-type: application/json`, JSON body); service-to-service calls use **gRPC / gRPC-web** (the same style as RPoem's admin API `open_runo.admin.v1.Admin`). Streaming only when it is actually needed.
- **Existing REST endpoints are migrated one by one, when they are touched.** The old REST route is removed once migrated. Until migration is finished they remain, but **no new ones are added**.
- **JSON is unified on RS-JSON**: reading with `from_str_secure` / `parse_secure` (size, depth and duplicate-key limits), writing with `rust_json::fast`. `prost` is not put into browser WASM (it would grow the binary).
- **Exceptions (shapes dictated by someone else's standard, not our own API)**: ACME, OAuth/OIDC callbacks, webhooks from third parties (e.g. GitHub), health checks, static file serving, OTLP, and so on. These accept plain HTTP as the standard requires.
- Reference implementations: `RPoem` (`open_runo.admin.v1.Admin`) and `open-easy-web` (`server/src/connect.rs`, `proto/open_easy_web/admin/v1/admin.proto`).
- Note: whether the code in a given repository already follows this policy depends on that repository. This section records the policy; it does not mean the code has been changed.

### Deutsch (KI-Übersetzung, redaktionell überarbeitet; noch nicht von Muttersprachlern geprüft)
- **REST wird vollständig abgeschafft; neue APIs entstehen nicht mehr als REST.** APIs, die aus dem Browser oder per `curl` aufgerufen werden, verwenden **unäre Connect-Aufrufe** (`POST /<Paket>.<Service>/<Methode>`, `content-type: application/json`, JSON-Body). Zwischen Diensten kommen **gRPC bzw. gRPC-web** zum Einsatz, im selben Stil wie die Admin-API von RPoem (`open_runo.admin.v1.Admin`). Streaming wird erst ergänzt, wenn es tatsächlich gebraucht wird.
- **Bestehende REST-Endpunkte werden schrittweise umgestellt, und zwar immer dann, wenn man ohnehin an ihnen arbeitet.** Sobald ein Endpunkt umgestellt ist, wird die alte REST-Route entfernt. Bis zum Abschluss der Migration bleiben die übrigen bestehen, es werden aber **keine neuen mehr hinzugefügt**.
- **JSON wird einheitlich über RS-JSON verarbeitet**: Gelesen wird mit `from_str_secure` bzw. `parse_secure` (mit Obergrenzen für Größe, Verschachtelungstiefe und doppelte Schlüssel), geschrieben mit `rust_json::fast`. `prost` wird nicht in das Browser-WASM aufgenommen, da es die Datei vergrößern würde.
- **Ausnahmen (Schnittstellen, deren Form ein fremder Standard vorgibt und die nicht zu unserer eigenen API gehören)**: ACME, OAuth/OIDC-Callbacks, Webhooks von Drittanbietern (z. B. GitHub), Health-Checks, die Auslieferung statischer Dateien, OTLP und Ähnliches. Diese nehmen, wie vom jeweiligen Standard verlangt, gewöhnliches HTTP entgegen.
- Referenzimplementierungen: `RPoem` (`open_runo.admin.v1.Admin`) und `open-easy-web` (`server/src/connect.rs`, `proto/open_easy_web/admin/v1/admin.proto`).
- Hinweis: Ob der Code eines einzelnen Repositorys diese Richtlinie bereits erfüllt, hängt vom jeweiligen Repository ab. Dieser Abschnitt dokumentiert lediglich die Richtlinie; er besagt nicht, dass der Code schon angepasst wurde.

### Русский (перевод выполнен с помощью ИИ и отредактирован; носителями языка пока не проверен)
- **От REST мы полностью отказываемся; новые API больше не создаются в стиле REST.** API, к которым обращаются из браузера или через `curl`, используют **унарные вызовы Connect** (`POST /<пакет>.<Service>/<Method>`, `content-type: application/json`, тело запроса — JSON). Для взаимодействия между сервисами применяются **gRPC и gRPC-web** — в том же стиле, что и административный API RPoem (`open_runo.admin.v1.Admin`). Потоковая передача данных добавляется лишь тогда, когда в ней действительно возникает необходимость.
- **Существующие REST-эндпоинты переводятся поэтапно — по мере того, как с ними приходится работать.** После перевода старый REST-маршрут удаляется. Пока миграция не завершена, оставшиеся эндпоинты сохраняются, но **новые не добавляются**.
- **Работа с JSON унифицирована на основе RS-JSON**: чтение — через `from_str_secure` или `parse_secure` (с ограничениями на размер, глубину вложенности и повторяющиеся ключи), запись — через `rust_json::fast`. `prost` в браузерный WASM не включается, поскольку это увеличило бы размер файла.
- **Исключения (интерфейсы, форму которых определяет чужой стандарт, а не наш собственный API)**: ACME, обратные вызовы OAuth/OIDC, вебхуки сторонних сервисов (например, GitHub), проверки работоспособности, раздача статических файлов, OTLP и тому подобное. Такие интерфейсы принимают обычный HTTP, как того требует соответствующий стандарт.
- Эталонные реализации: `RPoem` (`open_runo.admin.v1.Admin`) и `open-easy-web` (`server/src/connect.rs`, `proto/open_easy_web/admin/v1/admin.proto`).
- Примечание: соответствует ли код конкретного репозитория этой политике, зависит от самого репозитория. Данный раздел лишь фиксирует политику и не означает, что код уже изменён.
