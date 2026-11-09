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

## API 方針の補足: 用語の整理と使い分け (2026-11-04)

### 日本語
**1つに統一するものではなく、別の層の話です。** 「REST の代わり」になるのは Connect・gRPC・GraphQL だけで、WASM は画面を作る技術です。

| 用語 | 何か | REST の代わりか |
|---|---|---|
| REST | URL と HTTP メソッドで操作を表す API の作り方 | 全廃する側 |
| **Connect** | `POST /サービス名/メソッド名` に JSON を送る形式。`curl` でもブラウザの `fetch` でも呼べる | はい(**採用**) |
| **gRPC** | 同じ「サービス/メソッド」の考え方を、バイナリ(protobuf)と HTTP/2 で行う形式。速く、ストリーミングも得意 | はい(**サービス同士に採用**) |
| gRPC-web | gRPC のブラウザ向けの変形版 | はい(必要なら) |
| GraphQL | 欲しい項目を問い合わせる形式 | 一部だけ(**問い合わせ(読み取り)専用**) |
| WASM | Rust をブラウザの中で動かす仕組み | いいえ(画面を作る技術) |

- **ブラウザの画面や `curl` から呼ぶ API は Connect**(POST、JSON を RS-JSON で読み書き)。ブラウザの WASM は素の gRPC を話せず、Connect なら `fetch` のままで、`prost` を足さずに済む(画面のサイズも増えない)。
- **サービス同士の通信(サーバー間、重い処理、ストリーミング)は gRPC。** 多くは Connect で足りるので、ストリーミングが必要になった所だけ gRPC に上げる。
- **GraphQL は、まとめて問い合わせたいときの読み取り専用。** 書き込み(操作)には使わない。
- **WASM は画面を作る技術として、そのまま使う。** 画面から呼ぶ API が Connect になっただけ。
- 外部の規格が形を決めるもの(ACME、OAuth のコールバック、第三者の Webhook、ヘルスチェック、静的ファイル、OTLP)は、規格どおりの HTTP のまま。
- **迷ったら Connect。**

### English
**These are not alternatives to unify; they sit at different layers.** Only Connect, gRPC and GraphQL can replace REST; WASM is a way to build the UI.

| Term | What it is | Replaces REST? |
|---|---|---|
| REST | API style that expresses operations through URLs and HTTP methods | The thing being abolished |
| **Connect** | `POST /Service/Method` with a JSON body; callable from `curl` and from a browser `fetch` | Yes (**adopted**) |
| **gRPC** | The same service/method idea over binary (protobuf) and HTTP/2; fast, good at streaming | Yes (**for service-to-service**) |
| gRPC-web | A browser-friendly variant of gRPC | Yes (if needed) |
| GraphQL | A query language: ask for exactly the fields you want | Partly (**read-only queries**) |
| WASM | Runs Rust inside the browser | No (a UI technology) |

- **APIs called from browser screens or `curl` use Connect** (POST, JSON read and written with RS-JSON). Browser WASM cannot speak plain gRPC; with Connect it stays a `fetch`, and `prost` is not needed (the binary does not grow).
- **Service-to-service calls (server to server, heavy work, streaming) use gRPC.** Connect is enough in most places; move a call to gRPC only where streaming is actually needed.
- **GraphQL is read-only**, for queries that gather several pieces of data. It is not used for writes (operations).
- **WASM stays as the way to build the UI.** The API it calls simply became Connect.
- Interfaces whose shape an outside standard dictates (ACME, OAuth callbacks, third-party webhooks, health checks, static files, OTLP) stay plain HTTP.
- **When in doubt, use Connect.**

### Deutsch (KI-Übersetzung, sprachlich überarbeitet; noch nicht von Muttersprachlern geprüft)
**Es geht nicht darum, eine einzige Technik auszuwählen – die Begriffe liegen auf unterschiedlichen Ebenen.** Als Ersatz für REST kommen nur Connect, gRPC und GraphQL in Frage; WASM dagegen ist eine Technik, mit der die Benutzeroberfläche gebaut wird.

| Begriff | Was ist das? | Ersatz für REST? |
|---|---|---|
| REST | Ein API-Stil, bei dem Operationen über URLs und HTTP-Methoden ausgedrückt werden | wird abgeschafft |
| **Connect** | `POST /Dienst/Methode` mit JSON im Body; aufrufbar mit `curl` ebenso wie mit `fetch` im Browser | ja (**gewählt**) |
| **gRPC** | Dasselbe Prinzip aus Dienst und Methode, aber über binäres protobuf und HTTP/2; schnell und gut für Streaming geeignet | ja (**zwischen Diensten**) |
| gRPC-web | Eine für den Browser angepasste Variante von gRPC | ja (bei Bedarf) |
| GraphQL | Abfragesprache, mit der man genau die benötigten Felder anfordert | teilweise (**nur für lesende Abfragen**) |
| WASM | Führt Rust-Code direkt im Browser aus | nein (Technik für die Oberfläche) |

- **APIs, die von Browseroberflächen oder per `curl` aufgerufen werden, verwenden Connect** (POST; JSON wird mit RS-JSON gelesen und geschrieben). WASM im Browser kann kein reines gRPC sprechen. Mit Connect bleibt es bei einem gewöhnlichen `fetch`, und `prost` ist nicht nötig – die WASM-Datei wird dadurch nicht größer.
- **Für die Kommunikation zwischen Diensten (Server zu Server, rechenintensive Aufgaben, Streaming) wird gRPC eingesetzt.** In den meisten Fällen reicht Connect aus; auf gRPC wird nur dort umgestellt, wo Streaming tatsächlich gebraucht wird.
- **GraphQL dient ausschließlich lesenden Abfragen**, wenn mehrere Informationen gebündelt abgefragt werden sollen. Für Schreibzugriffe (Operationen) wird es nicht verwendet.
- **WASM bleibt die Technik für die Oberfläche.** Geändert hat sich lediglich, dass die aufgerufene API nun auf Connect beruht.
- Schnittstellen, deren Form durch einen externen Standard festgelegt ist (ACME, OAuth-Callbacks, Webhooks von Drittanbietern, Health-Checks, statische Dateien, OTLP), bleiben gewöhnliches HTTP.
- **Im Zweifel: Connect.**

### Русский (перевод выполнен с помощью ИИ и литературно отредактирован; носителями языка пока не проверен)
**Это не варианты, из которых нужно выбрать один, а технологии разных уровней.** Заменить REST могут только Connect, gRPC и GraphQL; WASM же — это способ создания пользовательского интерфейса.

| Термин | Что это | Заменяет REST? |
|---|---|---|
| REST | Стиль построения API, при котором операции выражаются через URL и HTTP-методы | отменяется |
| **Connect** | `POST /Сервис/Метод` с телом в формате JSON; вызывается как из `curl`, так и из браузерного `fetch` | да (**выбран**) |
| **gRPC** | Тот же принцип «сервис — метод», но поверх двоичного формата protobuf и HTTP/2; работает быстро и хорошо подходит для потоковой передачи | да (**для связи между сервисами**) |
| gRPC-web | Адаптированный для браузера вариант gRPC | да (при необходимости) |
| GraphQL | Язык запросов: клиент запрашивает ровно те поля, которые ему нужны | частично (**только запросы на чтение**) |
| WASM | Позволяет выполнять код на Rust прямо в браузере | нет (технология для интерфейса) |

- **API, которые вызываются из браузерных интерфейсов или через `curl`, строятся на Connect** (POST; JSON читается и записывается через RS-JSON). Браузерный WASM не умеет работать с «чистым» gRPC. С Connect достаточно обычного `fetch`, а `prost` не требуется — размер WASM-файла не увеличивается.
- **Для обмена между сервисами (сервер — сервер, ресурсоёмкие задачи, потоковая передача) применяется gRPC.** В большинстве случаев достаточно Connect; на gRPC переходят только там, где потоковая передача действительно необходима.
- **GraphQL используется только для чтения**, когда нужно получить сразу несколько связанных данных. Для записи (операций) он не применяется.
- **WASM по-прежнему остаётся технологией для интерфейса.** Изменилось лишь то, что вызываемый им API теперь построен на Connect.
- Интерфейсы, форма которых задана внешним стандартом (ACME, обратные вызовы OAuth, вебхуки сторонних сервисов, проверки работоспособности, статические файлы, OTLP), остаются обычным HTTP.
- **Если есть сомнения — выбирайте Connect.**

## aruaru-llm 呼び出しの Connect 化 (2026-11-05)

- aruaru-llm の REST 全廃に伴い、本リポジトリの呼び出し先を `POST /aruaru_llm.v1.Llm/<Method>`(JSON本文)へ変更した(旧 `/v1/...` は aruaru-llm 側で削除済み)。
- 対象は aruaru-llm を呼ぶ箇所のみ。本リポジトリ自身のAPI・他サービス(aruaru-search 等)の呼び出しは別途。
- 未検証: ビルド/実サーバー往復(aruaru-llm の `connect-migration` ブランチが VPS 未反映のため)。aruaru-llm の main マージ・VPS反映と同時に反映すること。

## 2026-11-09 キャッシュJSONの読み込みを parse_secure_sonic へ(完了)
- `src/main.rs` のキャッシュJSONを `rust_json::parse_secure_sonic`(上限: 16MiB・深さ32・重複キー拒否)+ `sonic_rs::Value` で読む形に切替(`a949306`)。
- 旧 `serde_json::Value` はキーがアルファベット順だったため `sorted_entries` で同じ順に揃えた。擬似データ(入れ子・未整列キー・数値・真偽値・null・URL・日本語・エスケープ)で旧/新の出力が**完全に同一**なことを実測し、cargo test 15件成功。
- 本番(`/root/audiocafe-tokyo-rust`)へWSLビルド→scp→入れ替え。反映前後で7ページ(/, /aruaru, /aruaru-lady, /rakuten-mobile, /discover, /summary, /healthz)のHTMLがバイト単位で同一。ロールバック用に旧バイナリは `target/release/audiocafe-tokyo-server.before-sonic-20261109`。
- 未適用: `cron.rs`・`scraper.rs` は `parse_strict` + `serde_json` のまま(小さな書き込み中心で速度の利点が薄い)。VPSの worktree `/root/repository/atr-sonic`(旧WIPブランチ)は不要になったが残してある。
