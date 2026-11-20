# Development policy & environment rules (audiocafe-tokyo-server) — condensed English version

> This is a condensed English translation covering the current state only. For the full
> historical HANDOFF log, see the Japanese original, [CLAUDE.md](CLAUDE.md).

This section follows [`open-raid-z`](https://github.com/aon-co-jp/open-raid-z)'s `CLAUDE.md`
as the canonical source, copied into each project per this ecosystem's sync convention.

## Role of this repository (phase one of the PHP→Rust migration)

A new site that gradually migrates `audiocafe.tokyo`'s existing PHP monolith (the
[`audiocafe-tokyo`](https://github.com/aon-co-jp/audiocafe-tokyo) repository, a single 445KB
`index.php` with 189 functions) to Rust + Poem. The existing PHP repository is not
overwritten; this repo was newly created as an independent migration target.

Uses the same tech stack as
[`aruaru-tokyo-server`](https://github.com/aon-co-jp/aruaru-tokyo-server),
[`aon-tokyo`](https://github.com/aon-co-jp/aon-tokyo), and
[`karu-tokyo`](https://github.com/aon-co-jp/karu-tokyo): Rust+Poem, no database dependency,
a single self-contained binary, no template engine.

## Migration approach & current coverage (honest disclosure)

Investigation of the existing PHP side (2026-07-17) found that most real data comes from
per-genre ranking displays backed by `*-cache.json` (file-based caches, already published
statically under `https://audiocafe.tokyo/`, with almost no database connectivity). The Rust
side fetches that cache JSON **over HTTP** (a loosely-coupled design that doesn't assume
direct file sharing between servers) and renders it generically via
[`rust_json::parse_strict`](https://github.com/aon-co-jp/Rust-JSON).

The cache schema originally had 8 shapes that weren't fully unified, but as of 2026-07-17
these are all covered by a single **fully-recursive generic renderer**
(`render_value_generic`, `src/main.rs`):

- Scalars (string/number/boolean) → `<p>`
- URL strings → clickable links
- String arrays → `<ul>`
- Object arrays with a uniform key set (e.g. regional-ranking `rows`, `ai-tech-ranking`'s
  `languages`/`frameworks`/`databases`) → a table
- Object arrays with inconsistent keys → falls back to a bulleted list
- Nested objects (e.g. per-region `tokyo_23`, `sources`) → recursion

Because no shape-specific code is needed, all 8 shapes
(`aruaru-caba`/`aruaru-eikaiwa`/`aruaru-jukujo-caba`/`ai-tech-ranking`/
`aruaru-learning-prices`/`rakuten-mobile`/`rakuten-intl-call`/`rakuten-platinum`) are covered
by one implementation.

## Composite pages (`/page/:slug`)

The existing PHP `/aruaru/`, `/aruaru-lady/`, and `/rakuten-mobile/` pages turned out not to
be single rankings but **integrated pages bundling multiple cache files into one** (confirmed
by inspecting the actual file layout on the VPS) — e.g. `/aruaru/` bundles 10 caches (4 of its
own genre's rankings + 4 Rakuten Mobile-related + doda job listings) into one page.

`COMPOSITE_PAGES` (`src/main.rs`) enumerates each section's heading and the cache's relative
path (including subdirectory, e.g. `aruaru/rakuten-mobile-cache.json`), and
`render_composite_body` renders each section in order via `render_value_generic` — again, no
shape-specific code was added (confirmed that the existing generic renderer already handles
the newer caches `rakuten-smartphone`, `doda-jobs`, and `aruaru-tvchat-normal/group` as-is).

Served as `/page/rakuten-mobile`, `/page/aruaru`, and `/page/aruaru-lady`.

**Not yet ported (disclosed honestly)**:
- The original PHP pages' HTML structure/design/layout (decoration, image placement, etc.) is
  not reproduced — only functionally-equivalent minimal HTML.
- Multi-language versions (`index-en.php`, `index-fr.php`, etc. — 12 languages for `/aruaru/`
  alone) are not supported; only the Japanese-equivalent version exists.
- The cron-driven cache auto-refresh logic itself (the PHP side's `--cron-all` etc.) has not
  been ported — the Rust side only reads existing cache files; refreshing them still depends
  on the PHP side's cron.

## Deployment

Built with `cargo build --release` in `/root/audiocafe-tokyo-rust` on the VPS (cloned from
GitHub, under git management), turned into a systemd service
(`audiocafe-tokyo-rust.service`, `127.0.0.1:4400`). In production,
`/etc/nginx/conf.d/audiocafe.tokyo.conf` proxies the exact paths `/`, `/aruaru/`,
`/aruaru-lady/`, and `/rakuten-mobile/` to this Rust binary; every other path continues to be
served by PHP-FPM. Adding a new Rust route requires a matching nginx `location` block (an
exact `cargo build`+`systemctl restart` alone is NOT reachable from the production domain
without one) — always verify over real HTTP (check the actual page `<title>`, not just the
status code) after adding one, and back up the nginx config with a timestamped `.bak` file
before editing it.

## Related projects

- [audiocafe-tokyo](https://github.com/aon-co-jp/audiocafe-tokyo) — the existing PHP
  implementation (migration source)
- [Rust-JSON](https://github.com/aon-co-jp/Rust-JSON) — used for JSON parsing
- [aruaru-tokyo-server](https://github.com/aon-co-jp/aruaru-tokyo-server) — the source of this
  repo's tech-stack choice

## Latest HANDOFF entry (see CLAUDE.md for the full log)

- **2026-08-17 — Added a McIntosh Amplifier link to the YouTube background player** (per user
  instruction: "after SPEC in audiocafe.tokyo's YouTube [series], paste the McIntosh Amplifier
  link https://ameblo.jp/www-aon/entry-12976022104.html"). Added a new entry to
  `assets/search_series.json`, placed immediately after the "SPEC RPA-MG1000 RPA-MG3000 image
  search" entry and immediately before "Pass Labs USA". The same entry, in the same position,
  was also added to the sister repository `audiocafe-tokyo-php`'s `SEARCH_SERIES` in
  `index.php` (see that repo's CLAUDE.md). Confirmed `cargo build --release` correctly parses
  the `include_str!`-embedded JSON. Reminder: as documented in the "Editing the YouTube
  playlist series" section, this change is NOT live in production until the VPS side runs
  `git pull` → `cargo build --release` → `systemctl restart audiocafe-tokyo-rust`. The English
  README/CLAUDE/PORTING documents (this set of files) were created alongside this change.

## Online-learning sites, AI-driven-development research article, and moving servers (2026-10-08/14)
Added five online-learning sites and the research article (`ONLINE_LEARNING_LIST_HTML` / `AI_DRIVEN_RESEARCH_HTML`) at the very top of `/aruaru` and at the start of "Recommended learning services TOP50". Verified on production (also shown on the `aruaru.tokyo/aruaru/` mirror).
**Not done**: showing the AI-made learning links (`aruaru/aruaru-tech-learning-ai-cache.json`, 80 technologies, ~770 links, prices corrected) in the Rust app.
**Moving servers**: production is `/root/audiocafe-tokyo-rust`: `cargo build --release` then `systemctl restart audiocafe-tokyo-rust.service` (port 4400). It reads the cache JSON from `/var/www/audiocafe.tokyo`, so the PHP daily crawl must run first.
The full procedure, systemd and nginx files are in `PORTING.md` and `deploy/vps/` of `audiocafe-tokyo`.
