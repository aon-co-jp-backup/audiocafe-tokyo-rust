#!/usr/bin/env python3
"""毎朝、ダイエットレシピの一覧ページ3つを巡回し、先頭のレシピが変わったら「最新」と「一つ前」を更新する。
あわせて「時短 ダイエット レシピ」の検索結果(aruaru-search)を取得し、英訳(aruaru-llm)を付けて更新する。
他社ページの写しは保存しない(題名・URL・短い要約と、その英訳だけを記録)。"""
import html, json, os, re, sys, time, urllib.request
from datetime import datetime, timedelta, timezone

UA = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120 Safari/537.36"
STATE = os.environ.get("DIET_STATE", "/root/diet-recipes/state.json")
OUTS = os.environ.get("DIET_OUTS", "/root/aruaru.tokyo/static/diet-recipes.json,/var/www/audiocafe.tokyo/diet-recipes.json").split(",")
SOURCES = [
    ("kikkoman", "https://www.kikkoman.co.jp/homecook/theme/popular/healthy-200calories-recipes.html", r'href="(/homecook/search/recipe/\d+/)"'),
    ("nadia", "https://oceans-nadia.com/recipe_list/5060", r'href="(/user/\d+/recipe/\d+)"'),
    ("rakuten", "https://recipe.rakuten.co.jp/category/39-504/", r'href="(/recipe/\d+/)"'),
]
JST = timezone(timedelta(hours=9))
SEARCH_URL = "http://127.0.0.1:4610/aruaru_search.v1.Search/Search"
LLM_URL = "http://127.0.0.1:4600/aruaru_llm.v1.Llm/ChatProvidersCompletePriority"
QUERY = "時短 ダイエット レシピ"
SKIP = ("instagram.com", "youtube.com", "tiktok.com", "twitter.com", "x.com", "facebook.com")


def get(url):
    req = urllib.request.Request(url, headers={"User-Agent": UA, "Accept-Language": "ja"})
    with urllib.request.urlopen(req, timeout=30) as r:
        return r.read().decode("utf-8", "replace")


def post(url, body, timeout=60):
    req = urllib.request.Request(url, data=json.dumps(body).encode("utf-8"), headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.loads(r.read().decode("utf-8"))


def title_of(page):
    m = re.search(r"<title[^>]*>([^<]*)</title>", page)
    t = html.unescape(m.group(1)).strip() if m else ""
    t = re.split(r"\s+[|｜]\s+|\s+-\s+(?=楽天レシピ|Nadia)", t)[0].strip()
    return t


def translate(title, snippet):
    prompt = ("Translate the following Japanese web search result into natural English. Output exactly two lines:\n"
              "TITLE: <translated title>\nSUMMARY: <translated summary>\n\n"
              "Japanese title: " + title + "\nJapanese summary: " + snippet)
    try:
        text = post(LLM_URL, {"prompt": prompt})["reply"]["text"]
    except Exception as e:
        print("translate error:", e, file=sys.stderr)
        return "", ""
    t = re.search(r"TITLE:\s*(.+)", text)
    m = re.search(r"SUMMARY:\s*(.+)", text)
    return (t.group(1).strip() if t else ""), (m.group(1).strip() if m else "")


def crawl_search(state):
    try:
        res = post(SEARCH_URL, {"q": QUERY, "n": 12, "hl": "ja", "gl": "jp"}, 60).get("results", [])
    except Exception as e:
        print("search error:", e, file=sys.stderr)
        return
    old = {r["url"]: r for r in state.get("search", {}).get("results", [])}
    out = []
    for r in res:
        link, title = r.get("link", ""), (r.get("title") or "").strip()
        if not link.startswith("http") or not title or any(d in link for d in SKIP):
            continue
        title = html.unescape(title)[:90]
        snippet = html.unescape((r.get("snippet") or "").strip())[:160]
        prev = old.get(link)
        if prev and prev.get("title") == title and prev.get("title_en"):
            out.append(prev)
        else:
            te, se = translate(title, snippet)
            out.append({"title": title, "title_en": te, "url": link, "snippet": snippet, "snippet_en": se})
            time.sleep(3)
        if len(out) >= 6:
            break
    if len(out) >= 3:
        state["search"] = {"query": QUERY, "date": datetime.now(JST).strftime("%Y-%m-%d"), "results": out}
        print("search updated:", len(out))


def main():
    try:
        state = json.load(open(STATE, encoding="utf-8"))
    except Exception:
        state = {}
    today = datetime.now(JST).strftime("%Y-%m-%d")
    changed = False
    for key, page_url, rx in SOURCES:
        try:
            listing = get(page_url)
            m = re.search(rx, listing)
            if not m:
                print(key, "no recipe link found", file=sys.stderr)
                continue
            base = re.match(r"https?://[^/]+", page_url).group(0)
            url = base + m.group(1)
            st = state.setdefault(key, {"page_url": page_url, "page_title": title_of(listing), "latest": None, "previous": None})
            st["page_title"] = title_of(listing) or st.get("page_title", "")
            if st["latest"] and st["latest"]["url"] == url:
                continue
            title = title_of(get(url)) or url
            if st["latest"]:
                st["previous"] = st["latest"]
            st["latest"] = {"title": title, "url": url, "date": today}
            changed = True
            print(key, "updated:", title, url)
            time.sleep(2)
        except Exception as e:
            print(key, "error:", e, file=sys.stderr)
    crawl_search(state)
    os.makedirs(os.path.dirname(STATE), exist_ok=True)
    json.dump(state, open(STATE, "w", encoding="utf-8"), ensure_ascii=False, indent=1)
    pub = {"updated": today, "sources": {k: v for k, v in state.items() if k != "search"}, "search": state.get("search")}
    for out in OUTS:
        if os.path.isdir(os.path.dirname(out)):
            json.dump(pub, open(out, "w", encoding="utf-8"), ensure_ascii=False)
    print("done changed=%s" % changed)


main()
