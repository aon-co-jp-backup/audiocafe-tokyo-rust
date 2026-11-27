> Latest translations / 翻訳の最新版: [0-LATEST-TRANSLATIONS.md](0-LATEST-TRANSLATIONS.md)

# audiocafe-tokyo-server

`audiocafe.tokyo`の既存PHPモノリスをRust + Poemへ段階的に移行するプロジェクト(第一段)。
既存PHP実装は[audiocafe-tokyo](https://github.com/aon-co-jp/audiocafe-tokyo)リポジトリのまま。

## 現状

`*-cache.json`(ファイルベースのジャンル別ランキングキャッシュ、
`https://audiocafe.tokyo/`直下に静的公開)をHTTP経由で取得し、
[rust-json](https://github.com/aon-co-jp/Rust-JSON)でパースして表示する。
対応済み・未対応の範囲は`CLAUDE.md`に正直に記載。

## ページ

- `/` — 対応済みランキング一覧、およびYouTube再生リストシリーズ機能
  (PHP版`SEARCH_SERIES`の移植、`assets/search_series.json`)
- `/ranking/:slug` — 個別ランキング表示(`aruaru-caba`/`aruaru-eikaiwa`/`aruaru-jukujo-caba`)
- `/healthz` — ヘルスチェック

## YouTube再生リストシリーズの編集について(重要)

`assets/search_series.json`は`include_str!`でコンパイル時に埋め込まれる
ため、**編集してpushしただけでは本番に反映されない**。必ずVPS側で
`git pull` → `cargo build --release` → `systemctl restart
audiocafe-tokyo-rust`まで行い、実HTTPで反映を確認すること。詳細な
データ設計上の注意点(動画を含まないシリーズのリンク挙動等)は
`PORTING.md`の該当節を参照。

## ビルド・起動

```bash
cargo build --release
./target/release/audiocafe-tokyo-server   # 127.0.0.1:4400
```

## ライセンス

Apache-2.0 OR MIT

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

## 2026-11-15 JSON読み込みの切替完了 / JSON reading switch completed
- **日本語**: キャッシュJSONは `parse_secure_sonic`+`sonic_rs::Value` で読む(出力は旧実装と同一、本番反映済み)。再開用メッセージは `PORTING.md` 末尾「2026-11-15 到達点・再開用メッセージ」。
- **English**: Cache JSON is read with `parse_secure_sonic` + `sonic_rs::Value` (output identical to the old code, deployed). The resume message is at the end of `PORTING.md` ("2026-11-15 到達点・再開用メッセージ").

## 2026-11-26 Rust化: 旧PHPの多言語ページを移植 / Rustify: legacy PHP multilingual pages ported
**日本語**: 全リポジトリのRust化(`runo/RUSTIFY-INVENTORY.md`)の優先1として、PHP版で動いていた `/aruaru/`・`/aruaru-lady/`・`/rakuten-mobile/` の `index-<言語>.php`(51ページ)を Rust へ移しました(`src/i18n_pages.rs`、本文は `templates/i18n/`、`be24174`)。本番で PHP 版と Rust 版の出力が**51ページすべてバイト一致**、存在しない言語は404。VPS のバイナリを入れ替え(旧版 `audiocafe-tokyo-server.before-i18n-20261126`)、`/root/open-web-server/domains.toml` の3つの経路を 4401(PHP)→4400(Rust)に切り替え(旧版 `domains.toml.before-i18n-20261126`)、公開URLで51ページ一致・PHP側にアクセスが来ないこと・他ドメイン200・エラー0を実測。**残るPHP**: トップの `/index.php`(8,382行)、残りの全部を受ける経路(`path_prefix` なし→4401)、毎朝の `aruaru/cron.php`(技術ランキング・学習価格)と `aruaru-lady/cron.php`(キャバ系ランキング)。
**English**: As priority 1 of the repo-wide Rust migration (`runo/RUSTIFY-INVENTORY.md`), the 51 `index-<lang>.php` pages under `/aruaru/`, `/aruaru-lady/` and `/rakuten-mobile/` moved from PHP to Rust (`src/i18n_pages.rs`, bodies in `templates/i18n/`, `be24174`). In production the Rust output is **byte-identical to PHP for all 51 pages**; unknown languages return 404. The VPS binary was swapped (old: `audiocafe-tokyo-server.before-i18n-20261126`) and three routes in `/root/open-web-server/domains.toml` moved from 4401 (PHP) to 4400 (Rust) (old: `domains.toml.before-i18n-20261126`). Measured on the public URLs: 51 pages identical, no hits reach PHP, other domains 200, no errors. **PHP still left**: the top `/index.php` (8,382 lines), the catch-all route (no `path_prefix` → 4401), and the daily `aruaru/cron.php` (tech ranking, learning prices) and `aruaru-lady/cron.php` (club rankings).

## 2026-11-26 Rust化の再開用メモ / Rustify resume memo
**日本語**: 到達点=一覧 `runo/RUSTIFY-INVENTORY.md`(102リポジトリ)作成、audiocafe.tokyo の多言語ページ51本を Rust へ移植し本番切替(audiocafe-tokyo-rust `be24174`、PHP版とバイト一致、PHPへのアクセス0)。未了=audiocafe.tokyo のトップ `/index.php`(8,382行)・残り全部を受ける経路(4401)・`aruaru/cron.php`・`aruaru-lady/cron.php`、その後は一覧の優先2以降(old.aon.tokyo・aruaru-db-archive の稼働確認、ネットワークに出る Python)。PHPサービスは移植が済むまで止めない。
**再開用メッセージ(そのまま貼り付け)**: 前の続きです。`F:\runo\RUSTIFY-INVENTORY.md` と audiocafe-tokyo-rust の PORTING.md 末尾を読んでから、残りの PHP(トップ `/index.php`・4401 の残り経路・2つの cron.php)を Rust へ移し、PHP版と同じ出力を実測してから本番を切り替え(バックアップ付き、`vps-build.sh`+`check-glibc.sh`)、確認なしで一覧の次へ進んでください。決まり: 本物で確認してから完了、秘密は書かない、新しいRESTは作らない、返答は日本語のみ。
**English**: Reached: the inventory `runo/RUSTIFY-INVENTORY.md` (102 repos); 51 audiocafe.tokyo multilingual pages ported to Rust and switched in production (audiocafe-tokyo-rust `be24174`, byte-identical, no PHP hits). Open: the top `/index.php`, the catch-all route (4401), `aruaru/cron.php` and `aruaru-lady/cron.php`, then inventory priority 2 onward. Keep the PHP services running until ported. Resume: read the inventory and the end of audiocafe-tokyo-rust's PORTING.md, port the remaining PHP, verify identical output, switch production with backups, and continue down the list without asking.

## Versionless API の方針 / Versionless API policy (2026-11-25)
**日本語**: このリポジトリのAPIは **Versionless**(URL・パッケージ名に `v1` 等を付けず、互換性を壊さず足すだけで進化させる)。欄は足すだけ・足す欄は省略可能・知らない欄は無視・名前変更は新名を足して旧を非推奨に(期限は既定で6か月以上、使用0を確認してから削除)・enumは未知の値を許容・安全上の緊急修正は例外。既存の `.v1` は呼び出し側を壊さないよう当面残し、版なしの別名を追加して段階的に畳む。詳細・出典は [`runo/API-POLICY.md`](https://github.com/aon-co-jp/runo/blob/main/API-POLICY.md) 末尾。適用はこれから順に行う。
**English**: This repository's APIs are **Versionless**: no `v1` in URLs or package names; evolve by adding only, without breaking compatibility. Only add fields, new fields are optional, ignore unknown fields, rename by adding the new name and deprecating the old (default deadline 6 months or more, remove only after confirming zero use), tolerate unknown enum values, emergency security fixes are the exception. Existing `.v1` names stay for now so callers do not break; a versionless alias is added and `.v1` is retired in stages. Details and sources: the end of [`runo/API-POLICY.md`](https://github.com/aon-co-jp/runo/blob/main/API-POLICY.md). Rollout is step by step.

## 完成まで作り切る方針 / Finish everything policy (2026-11-27)
**日本語**: 全リポジトリで、**未確認は自動で確認し、未実施は実施し、未了は完成させる**(確認を求めず進める)。使用頻度が低いと判断される機能や、**1から作り直す大規模開発になる内容でも、やめずに最後まで開発・TEST・DEBUGする**(別の方式で済ませない、後回しにしない)。進め方: 調査(英語・日本語の検索とGitHub) → 設計 → 実装 → 実機/実データで確認 → 失敗は原因を直して再試験 → README・CLAUDE・PORTING・POLICY を日英で更新 → push。「できた」と書くのは実測した範囲だけ、確認できなかった所は「未確認」と正直に書く。例外(確認を取る): 元に戻せない削除、秘密の公開、所有者の認証・決済・実機操作が要るもの、利用者のシステム設定の変更。
**English**: In every repository, **verify what is unverified, do what is not yet done, and finish what is unfinished** automatically, without asking. Even features that look rarely used, or work that means **a large rewrite from scratch, are developed, tested and debugged to the end** (no substituting another approach, no postponing). Method: research (English and Japanese search plus GitHub) -> design -> implement -> verify on real devices or data -> fix causes and retest on failure -> update README, CLAUDE, PORTING and POLICY in Japanese and English -> push. Write "done" only for what was measured, and say "unverified" honestly for the rest. Exceptions that still need confirmation: irreversible deletions, publishing secrets, anything needing the owner's authentication, payment or real-device actions, and changes to the user's system settings.
## 2026-11-27 静的ファイルの Rust 移植(本番切替済み) / Static files ported to Rust (live)
**日本語**: 旧PHP(4401)が配っていた静的ファイル(analog-amp・streaming-amp・top・world・manual・cancer・images・video・mono-sub-out)を `src/static_files.rs` で配るようにした。配るのは決めたフォルダーの決めた拡張子だけで、ドット始まり・`~`・`.php`・`..` は 404。動画は Range(206)に対応した(PHPは非対応で全量を送っていた)。本番で156ファイルがPHP版とバイト一致し、フォルダーの入口と日本語名も一致した。open-web-server の `domains.toml` に9つの入口を足した(バックアップは `domains.toml.before-static-20261127`、旧バイナリは `audiocafe-tokyo-server.before-static-20261127`)。公開URLへの要求はPHPへ届いていない(実測)。`top/contact.php` は読み込む部品が無く壊れていたうえ、どこからもリンクされていないため移植せず、404にした。**注意(所有者の判断待ち)**: `top/` の履歴書・職歴書(pdf・xlsx)は、以前と同じく公開のまま。**残り**: トップの `/index.php`(8,382行)、その他の入口(catch-all)、`cron.php` 2本。
**English**: Static files formerly served by the PHP server (4401) now come from `src/static_files.rs`. Only allow-listed folders and extensions are served; dotfiles, `~`, `.php` and `..` return 404. Video supports Range (206). In production 156 files are byte-identical to PHP. Nine prefixes were added to `domains.toml` (backups `*.before-static-20261127`). Public requests no longer reach PHP (measured). The broken, unlinked `top/contact.php` was not ported and now returns 404. **Owner decision pending**: the CV/résumé files in `top/` stay public, as before. **Left**: `/index.php`, the remaining catch-all, and the two `cron.php` jobs.

## 2026-11-27 aruaru-lady の cron を Rust へ(本番切替済み) / aruaru-lady cron ported to Rust (live)
**日本語**: 旧 `aruaru-lady/cron.php`(`--cron-all`、キャバクラ・熟女キャバ・TVチャットレディ2種のランキング)を `src/lady_cron.rs`(`--cron-lady <出力先>`)へ移した。固定の一覧は PHP の master_pool から項目の順番ごと書き出して `data/lady/*.json` に置き、週・日ごとの種による並べ替えと PHP と同じ JSON の書き方(4スペース、`/` は `\/`)を再現した。試験: PHP 8.3 で5つの種から作った結果(`tests/fixtures/lady-expected.txt`)と1文字単位で一致。本番: 同時刻に作った4ファイルが PHP 版と完全一致(更新時刻も含む)。毎朝の `audiocafe-cron-php.service` は、ドロップイン `20-rust-lady.conf` で aruaru-lady 分だけ Rust に置き換え、成功を確認した(戻すときはこのファイルを消す。旧ファイルは `*.before-rs-20261127`、旧バイナリは `*.before-lady-20261127`)。**残り**: `aruaru/cron.php`(`--cron-tech`: 技術ランキング・学習価格・AI学習コメント・aruaru-llm への学習)、トップの `/index.php`、その他の入口(catch-all)。
**English**: The legacy `aruaru-lady/cron.php` (four rankings) is now `src/lady_cron.rs` (`--cron-lady <dir>`). The fixed lists were exported from PHP with key order kept (`data/lady/*.json`), and the seeded sorting and PHP's pretty JSON format are reproduced. Tests match PHP 8.3 output character for character for 5 seeds. In production, the four files built at the same time are identical to PHP's, timestamps included. The morning `audiocafe-cron-php.service` now runs the Rust step through the drop-in `20-rust-lady.conf` (delete it to roll back); a run succeeded. **Left**: `aruaru/cron.php` (`--cron-tech`), the top `/index.php` and the catch-all.

## 2026-11-27 aruaru の毎朝の処理(--cron-tech)を Rust へ(本番切替済み) / aruaru morning job (--cron-tech) ported to Rust (live)
**日本語**: 旧 `aruaru/cron.php`(技術ランキング・学習価格・AI学習リンク・価格訂正・aruaru-llm への学習)を `src/tech_cron.rs`(`--cron-tech <dir>`)へ移した。ベースラインは PHP から書き出した `data/tech/baseline.json`。JSON は PHP の整形(4桁字下げ・`\/`・キー順・空は`[]`)を自前で再現。aruaru-llm は版なし `/aruaru_llm.Llm/` を先に試し、404 なら旧 `.v1` へ。比較: VPS で外部取得7件を録画し(`TECH_FIX`)、同じ入力で PHP と Rust を動かすと3つのキャッシュ・ログが一致、画面表示は試験フォルダーのパスだけ違った(第三者のページは録画のみでコミットしない)。AI の部分は出力が毎回変わるため、本番で通しを確認。本番: `audiocafe-cron-php.service` の drop-in `20-rust-lady.conf` で PHP を Rust(`target/release/audiocafe-tokyo-cron`)に置き換え。戻し方は `/root/20-rust-lady.conf.before-tech-20261127` を戻して daemon-reload、キャッシュの控えは `/root/aruaru-cache-before-tech-20261127/`。トップの `/index.php` と catch-all は**PHPのまま**(ユーザー判断)。**判明**: 本番 aruaru-llm は版なしのみ(`.v1` は404)で、PHP版は最近ずっとAI学習・KnowledgeAdd が0件だった。Rust版は版なしで実際に応答を得る。
**English**: The legacy `aruaru/cron.php` (tech ranking, learning prices, AI learning links, price fixes, teaching aruaru-llm) is now `src/tech_cron.rs` (`--cron-tech <dir>`). The baseline was exported from PHP into `data/tech/baseline.json`; PHP's pretty JSON is reproduced exactly. aruaru-llm is called on the versionless `/aruaru_llm.Llm/` first, falling back to `.v1` on 404. With 7 recorded external responses (`TECH_FIX`), PHP and Rust produced identical caches and logs; stdout differed only in the test folder path (third-party pages are not committed). The AI part is non-deterministic and was verified live. Production: the drop-in `20-rust-lady.conf` now runs the Rust binary instead of PHP; roll back by restoring `/root/20-rust-lady.conf.before-tech-20261127` and daemon-reload; cache backups are in `/root/aruaru-cache-before-tech-20261127/`. The top `/index.php` and the catch-all **stay PHP** (user decision). **Found**: production aruaru-llm now serves only the versionless path (`.v1` returns 404), so the PHP job had been producing 0 AI updates and 0 KnowledgeAdd calls; the Rust job gets real answers.
