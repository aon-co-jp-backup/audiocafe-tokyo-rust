# PORTING.md — audiocafe-tokyo-rust reusable files

> This single file is enough to introduce/relocate `audiocafe-tokyo-rust` into another
> project. Covers version 0.3.0 (2026-07-19, added flag-click navigation, translation links,
> YouTube series switching — see the HANDOFF log in `CLAUDE.md` for detail).

## 0. Scope of this repository

`audiocafe-tokyo-rust` (formerly folder-named `audiocafe-tokyo-server`) gradually migrates
the existing PHP monolith [`audiocafe-tokyo`](https://github.com/aon-co-jp/audiocafe-tokyo)
(a single 445KB `index.php`, 189 functions, 8,146 lines) to Rust + **RPoem**
(`open-runo-router::hyper_compat`, this ecosystem's policy of cutting the direct dependency
on the external `poem` crate — migration completed 2026-07-19). The existing PHP
implementation is not overwritten; this repo is operated as an independent migration target.

**State as of 2026-07-19**: the top page (`/`), `/aruaru`, `/aruaru-lady`, and
`/rakuten-mobile` — **4 pages** — have been ported to a level that matches the PHP version
both in content and appearance (CSS/class structure), and **the Rust version is actually
live on the production `audiocafe.tokyo` domain** (proxied per individual path via nginx
`location = /` and `location /aruaru/` etc.). The top page includes all 147/147 language
cards, each card's full essay text (including political/religious statements, copied as-is
from what is actually already published), `cardLinks`, the YouTube background player
(**77-series switching + NEXT + an OPEN/CLOSE panel, re-implemented**), and the free
wallpaper corner. The flag images themselves are also clickable (routing to the main
audiocafe.tokyo site via a Google Translate proxy), and the `/aruaru`, `/aruaru-lady`, and
`/rakuten-mobile` links in `.card-actions` also go through the selected language's Google
Translate proxy (2026-07-19). This Rust site had previously followed a "no client-side JS"
policy, but introduced a small, self-contained script for the first time specifically for
the YouTube series-switching feature (see the 2026-07-19 HANDOFF entry in `CLAUDE.md` for
detail and out-of-scope items).

## 1. What to bring (file list)

```
audiocafe-tokyo-rust/
├── Cargo.toml / Cargo.lock
├── src/
│   ├── main.rs        # routing, the generic renderer (render_value_generic), composite pages
│   ├── scraper.rs      # scraping logic for /discover (the real-algorithm part ported from PHP)
│   └── seed_urls.rs    # 360 seed URLs, mechanically ported as-is
├── PORTING.md (this file)
├── CLAUDE.md
└── README.md
```

To relocate wholesale, copy the folder and confirm `cargo build --release` succeeds.

## 2. Build & run

```bash
cargo build --release
./target/release/audiocafe-tokyo-server   # default bind 127.0.0.1:4400
```

## 3. Page structure

- `/` — the ranking list currently supported
- `/ranking/:slug` — a single ranking view (`aruaru-caba`/`aruaru-eikaiwa`/`aruaru-jukujo-caba`,
  8 kinds total)
- `/page/:slug` — a composite page (`aruaru`/`aruaru-lady`/`rakuten-mobile`, bundling several
  caches into one page)
- `/discover` — a video/article/photo collection page (`src/scraper.rs`, gathered from 360
  seed URLs)
- `/healthz` — health check

## 4. Data-fetch approach

Fetches the per-genre ranking caches (`*-cache.json`, generated file-based by the PHP side and
published statically under `https://audiocafe.tokyo/`) **over HTTP** (a loosely-coupled design
that does not assume direct file sharing between servers), parses them with
[`rust_json::parse_strict`](https://github.com/aon-co-jp/Rust-JSON), and renders them
generically via `render_value_generic` (`src/main.rs`). There are 8 cache schema shapes, but
rather than writing shape-specific code, a single fully-recursive generic renderer covers all
of them.

If a new cache-JSON shape appears at a new destination, first check whether the existing
`render_value_generic` already handles it before resorting to shape-specific code.

## 5. What is not ported yet (disclosed honestly, updated 2026-07-19)

- **The JS animation of the original PHP destination-choice modal (`#acNavChoiceModal`)
  itself**: the reachable destinations (the main audiocafe.tokyo site / aruaru / aruaru-lady /
  rakuten-mobile / aruaru.tokyo / Google Translate — all reachable from both the flag image and
  `.card-actions` via the selected language's Google Translate proxy) are the same, but the
  modal's own open/close animation remains a static direct link.
- **The YouTube playlist series feature (`SEARCH_SERIES`, actually 77 entries** — an older
  HANDOFF's "84 entries" was an unverified estimate, corrected to 77 via a lossless Node.js
  count): revived on 2026-07-19. All 77 series buttons, NEXT-based queue advancing, and the
  OPEN/CLOSE panel toggle are implemented (this Rust site's first client-side JS, a small
  self-contained script). However, the PHP version's YouTube IFrame API integration
  (auto-queue-advance, shuffle, search-driven video-switch animation) is out of scope — only
  manual advancing via the NEXT button. Non-playable series (e.g. those with only a
  `results?search_query=` link) navigate to the real YouTube search-results page in a new tab
  (following the no-scraping policy documented in `audiocafe.tokyo/CLAUDE.md`).
- Multi-language versions (`index-en.php`, `index-fr.php`, etc. — 12 languages for `/aruaru/`
  alone) — only the Japanese-equivalent version exists. Viewing via the Google Translate proxy
  through the top page's language cards is still possible.
- **The `/cancer`, `/Python`, `/video`, `/world` (planned for removal) directories**: static
  content / distribution-tool files, not yet ported on the Rust side — still served via
  nginx's static file serving (the same document root as the PHP version).
- Client-side JavaScript flourishes other than the YouTube series switching noted above
  (search-driven video-switch animation, shuffling, etc.).

## 6. Production deployment (2026-07-19, cutover completed)

Built with `cargo build --release` in `/root/audiocafe-tokyo-rust` on the VPS (cloned from
GitHub, under git management), and turned into a systemd service
(`audiocafe-tokyo-rust.service`, `127.0.0.1:4400`).

The following was added to `/etc/nginx/conf.d/audiocafe.tokyo.conf`, and **the Rust version
is now actually serving `https://audiocafe.tokyo/` in production**:
- `location = /` (exact match only, takes priority over prefix matches) → `127.0.0.1:4400/`
- `location /aruaru/`, `/aruaru-lady/`, `/rakuten-mobile/` (each a prefix match) →
  `127.0.0.1:4400/aruaru` etc.

All other paths (the `location /` prefix match, `/top/`, `/cancer/`, `/Python/`, `/video/`,
static cache JSON, etc.) **continue to be handled by the PHP side, unchanged** — this relies
on nginx's normal rule that `location =` only claims exact matches, so existing content is
unaffected. Always back up with
`cp audiocafe.tokyo.conf audiocafe.tokyo.conf.bak-<timestamp>` before editing the
configuration.

When relocating to another environment, first investigate that domain's actual document-root
layout (whether domain-specific personal-info/distribution directories like `/top/` exist),
then perform a safe, gradual cutover using the same "claim only exact matches" pattern —
replacing `location /` wholesale causes every path the Rust side hasn't implemented to 404,
which has been confirmed as a real-world failure mode (see the 2026-07-19 HANDOFF entry in
`CLAUDE.md`).

## 7. Naming conventions

- Crate/binary name: `audiocafe-tokyo-server` (the executable name was kept as-is from before
  the migration)
- Repository/folder name: `audiocafe-tokyo-rust` (renamed 2026-07-18 to match the remote
  repository name)

## 8-1. Notes on maintaining `assets/search_series.json` (the YouTube playlist series data) (added 2026-07-29)

- `search_series.json` is embedded at compile time via `include_str!`, so **editing this
  JSON (or the identically-named data in the corresponding PHP `index.php`) and pushing it
  alone does NOT take effect in production** — you must `git pull` → `cargo build --release`
  → `systemctl restart audiocafe-tokyo-rust` on the VPS (and, on the PHP side, upload
  `index.php` directly to `/var/www/audiocafe.tokyo/`), then verify over real HTTP. (This
  step was actually skipped once on 2026-07-29, and the user reported "I pushed but it's not
  reflected" as a real incident — see the CLAUDE.md HANDOFF for detail.)
- For a series containing no video ID at all (e.g. only a link to the official company site
  or a Google search/image-search page), the design is to use **the first URL in `s.urls`
  as-is** as the link destination (fixed 2026-07-29; the previous implementation had a real
  bug where it always synthesized a YouTube search from the label string instead). When
  adding a new series with no video, put exactly one URL — the one you actually want opened —
  into `urls`.

## 8. Cautions when porting/extending

When a new cache-JSON shape appears, first try rendering it with the existing generic
renderer (`render_value_generic`). Carelessly adding shape-specific code risks reverting to
the "branch-per-shape hell" that was already resolved once when covering all 8 existing
shapes. When unsure about a technology choice or how to interpret the PHP side's algorithm,
don't rely solely on guesses from training data — check the actual PHP source and real data
on the VPS before porting.

## Online-learning sites, AI-driven-development research article, and moving servers (2026-10-08/14)
Added five online-learning sites and the research article (`ONLINE_LEARNING_LIST_HTML` / `AI_DRIVEN_RESEARCH_HTML`) at the very top of `/aruaru` and at the start of "Recommended learning services TOP50". Verified on production (also shown on the `aruaru.tokyo/aruaru/` mirror).
**Not done**: showing the AI-made learning links (`aruaru/aruaru-tech-learning-ai-cache.json`, 80 technologies, ~770 links, prices corrected) in the Rust app.
**Moving servers**: production is `/root/audiocafe-tokyo-rust`: `cargo build --release` then `systemctl restart audiocafe-tokyo-rust.service` (port 4400). It reads the cache JSON from `/var/www/audiocafe.tokyo`, so the PHP daily crawl must run first.
The full procedure, systemd and nginx files are in `PORTING.md` and `deploy/vps/` of `audiocafe-tokyo`.
