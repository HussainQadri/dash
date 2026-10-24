#!/usr/bin/env python3
"""Dump KEATS (Moodle) courses, deadlines, announcements and materials to Markdown.

Token: ~/.config/keats/token (or $KEATS_TOKEN). The file may hold either the raw
token or the full moodledl://token=... redirect URL from the mobile launch flow.

Moodleoverflow Q&A forums aren't exposed to the web service API, so they're
scraped with a browser session cookie kept in ~/.config/keats/session. When it
expires a new one is minted through the mobile app autologin, which needs the
private token that `dash login` saves; without it the forum step is skipped
and everything else still runs.
"""
import base64
import html
import json
import os
import re
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from datetime import datetime
from pathlib import Path

SITE = "https://keats.kcl.ac.uk"
TOKEN_FILE = Path.home() / ".config/keats/token"
SESSION_FILE = Path.home() / ".config/keats/session"
OUT = Path(__file__).resolve().parent / "out"
FILES = Path(__file__).resolve().parent / "files"
DELAY = 0.3  # seconds between API calls
FORUM_RECENT_DAYS = 14
# Course shortname prefixes whose files get downloaded into FILES
DOWNLOAD_COURSES = ["5CCS2FC2"]


def token_parts():
    raw = (os.environ.get("KEATS_TOKEN") or TOKEN_FILE.read_text()).strip()
    m = re.search(r"token=([A-Za-z0-9+/=]+)", raw)
    if not m:
        return [None, raw]
    # signature:::token[:::privatetoken]
    return base64.b64decode(m.group(1)).decode().split(":::")


def load_token():
    return token_parts()[1]


TOKEN = None


def call(fn, **params):
    flat = {"wstoken": TOKEN, "wsfunction": fn, "moodlewsrestformat": "json"}
    for k, v in params.items():
        if isinstance(v, list):
            for i, item in enumerate(v):
                flat[f"{k}[{i}]"] = item
        else:
            flat[k] = v
    data = urllib.parse.urlencode(flat).encode()
    with urllib.request.urlopen(f"{SITE}/webservice/rest/server.php", data, timeout=30) as r:
        res = json.load(r)
    time.sleep(DELAY)
    if isinstance(res, dict) and "exception" in res:
        raise RuntimeError(f"{fn}: {res.get('errorcode')} - {res.get('message')}")
    return res


def try_call(fn, **params):
    try:
        return call(fn, **params)
    except Exception as e:
        print(f"  ! {e}", file=sys.stderr)
        return None


def fmt_ts(ts):
    return datetime.fromtimestamp(ts).strftime("%a %d %b %Y %H:%M") if ts else "-"


def strip_html(s, limit=400):
    s = html.unescape(re.sub(r"<[^>]+>", " ", s or ""))
    s = re.sub(r"\s+", " ", s).strip()
    return s if len(s) <= limit else s[:limit] + "…"


def slug(s):
    return re.sub(r"[^A-Za-z0-9]+", "_", s).strip("_")[:60]


def safe_name(s):
    # Server-supplied names become path components: no separators, no dot-only names
    s = html.unescape(re.sub(r"<[^>]+>", "", s or "")).strip()
    s = re.sub(r'[/\\\x00-\x1f:*?"<>|]+', "_", s).strip(" .")
    return s[:100] or "_"


def download_course_files(course, sections):
    """Mirror a course's files to FILES/<course>/<NN section>/<activity>/<folder path>/<file>.

    A manifest records each file's timemodified/size so unchanged files are skipped.
    Returns the list of newly downloaded paths.
    """
    root = FILES / safe_name(course["shortname"].split()[0])
    manifest_file = root / ".manifest.json"
    try:
        manifest = json.loads(manifest_file.read_text())
    except (OSError, ValueError):
        manifest = {}
    new = []
    for i, s in enumerate(sections):
        sec_dir = f"{i:02d} {safe_name(s['name'])}"
        for m in s.get("modules", []):
            if not m.get("uservisible", True) or m["modname"] == "url":
                continue
            for f in m.get("contents") or []:
                if f.get("type") != "file" or not f.get("fileurl"):
                    continue
                folder = [safe_name(p) for p in (f.get("filepath") or "/").split("/") if p]
                rel = Path(sec_dir, safe_name(m["name"]), *folder, safe_name(f["filename"]))
                stamp = [f.get("timemodified"), f.get("filesize")]
                dest = root / rel
                if manifest.get(str(rel)) == stamp and dest.exists():
                    continue
                dest.parent.mkdir(parents=True, exist_ok=True)
                sep = "&" if "?" in f["fileurl"] else "?"
                try:
                    urllib.request.urlretrieve(f"{f['fileurl']}{sep}token={TOKEN}", dest)
                except Exception as e:
                    print(f"  ! download failed: {rel}: {e}", file=sys.stderr)
                    continue
                time.sleep(DELAY)
                manifest[str(rel)] = stamp
                new.append(rel)
    root.mkdir(parents=True, exist_ok=True)
    manifest_file.write_text(json.dumps(manifest, indent=1))
    return new


class SessionExpired(Exception):
    pass


def web_get(path, cookie):
    req = urllib.request.Request(f"{SITE}{path}", headers={"Cookie": f"MoodleSession={cookie}"})
    with urllib.request.urlopen(req, timeout=30) as r:
        final = urllib.parse.urlparse(r.geturl())
        body = r.read().decode("utf-8", "replace")
    time.sleep(DELAY)
    if final.netloc != urllib.parse.urlparse(SITE).netloc or final.path.startswith("/login/"):
        raise SessionExpired()
    return body


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args):
        return None


def new_session():
    """Mint a MoodleSession cookie via the mobile app autologin and save it."""
    parts = token_parts()
    if len(parts) < 3:
        raise SessionExpired()
    data = urllib.parse.urlencode({"wstoken": TOKEN, "wsfunction": "tool_mobile_get_autologin_key",
                                   "moodlewsrestformat": "json", "privatetoken": parts[2]}).encode()
    # Moodle only hands autologin keys to its own app.
    req = urllib.request.Request(f"{SITE}/webservice/rest/server.php", data,
                                 headers={"User-Agent": "MoodleMobile"})
    with urllib.request.urlopen(req, timeout=30) as r:
        res = json.load(r)
    if "exception" in res:
        print(f"  ! autologin: {res.get('errorcode')} - {res.get('message')}", file=sys.stderr)
        raise SessionExpired()
    userid = call("core_webservice_get_site_info")["userid"]
    url = res["autologinurl"] + "?" + urllib.parse.urlencode(
        {"userid": userid, "key": res["key"], "urltogo": f"{SITE}/my/"})
    try:
        headers = urllib.request.build_opener(NoRedirect).open(url, timeout=30).headers
    except urllib.error.HTTPError as e:
        headers = e.headers
    cookies = [m.group(1) for c in headers.get_all("Set-Cookie") or []
               if (m := re.match(r"MoodleSession=([^;]+)", c)) and m.group(1) != "deleted"]
    if not cookies:
        raise SessionExpired()
    SESSION_FILE.touch(mode=0o600)
    SESSION_FILE.write_text(cookies[-1])
    return cookies[-1]


def parse_short_ts(s):
    # Moodleoverflow short format: "18/10/26, 15:03"
    try:
        return int(datetime.strptime(s.strip(), "%d/%m/%y, %H:%M").timestamp())
    except ValueError:
        return 0


def parse_overflow_list(page):
    """Discussion cards on a Moodleoverflow view.php page -> [(id, subject, posts)]."""
    out = []
    for card in page.split('class="moodleoverflowdiscussion discussioncard')[1:]:
        d = re.search(r'discussion\.php\?d=(\d+)">([^<]*)', card)
        n = re.search(r'title="(\d+) Answers?"', card)  # counts the question too
        if d:
            out.append((int(d.group(1)), html.unescape(d.group(2)).strip(),
                        int(n.group(1)) if n else 0))
    pages = [int(p) for p in re.findall(r'data-page-number="(\d+)"', page)]
    return out, max(pages, default=1)


def parse_overflow_discussion(page):
    main = page[page.find('<div role="main">'):page.find("activity-navigation")]
    posts = []
    for p in re.split(r'(?=<div id="p\d+" class="moodleoverflowpost)', main)[1:]:
        cls = re.search(r'class="([^"]*)"', p)
        cls = cls.group(1).split() if cls else []
        author = re.search(r'aria-label="([^"]*)"', p)
        text = re.search(r'<div class="post-text">([\s\S]*?)</div>\s*<div class="attachments', p)
        if "moodleoverflowcomment" in cls:
            # "Questioner - 15/10/26, 00:58", or the name wrapped in a profile link
            when = re.search(r'<span class="text-muted">[\s\S]*? - ([\d/]+, [\d:]+)</span>', p)
        else:
            when = re.search(r'<div class="user-action-shorttime">\s*([^<]*)', p)
        posts.append({
            "author": html.unescape(author.group(1)) if author else "?",
            "time": parse_short_ts(when.group(1)) if when else 0,
            "kind": ("question" if "firstpost" in cls else
                     "comment" if "moodleoverflowcomment" in cls else "answer"),
            "solution": "markedsolution" in cls,
            "helpful": "markedhelpful" in cls,
            "text": strip_html(text.group(1) if text else "", 1500),
        })
    return posts


def crawl_overflow(forums, cookie):
    """forums: [(course name, forum name, cmid)] -> list of discussion dicts.

    Discussions whose post count hasn't changed since the last run are taken
    from the cache instead of being refetched.
    """
    cache_file = OUT / "forums_cache.json"
    try:
        cache = json.loads(cache_file.read_text())
    except (OSError, ValueError):
        cache = {}
    result = []
    for course, forum, cmid in forums:
        print(f"  forum: {forum}")
        page, last = 0, 1
        while page < last:
            cards, last = parse_overflow_list(
                web_get(f"/mod/moodleoverflow/view.php?id={cmid}&page={page}", cookie))
            if not cards:
                break
            for did, subject, nposts in cards:
                hit = cache.get(str(did))
                if not hit or hit["nposts"] != nposts:
                    posts = parse_overflow_discussion(
                        web_get(f"/mod/moodleoverflow/discussion.php?d={did}", cookie))
                    hit = {"nposts": nposts, "posts": posts}
                    cache[str(did)] = hit
                result.append({"id": did, "course": course, "forum": forum,
                               "subject": subject, "posts": hit["posts"],
                               "url": f"{SITE}/mod/moodleoverflow/discussion.php?d={did}"})
            page += 1
    cache_file.write_text(json.dumps(cache, indent=1))
    return result


def write_forum_files(discussions):
    (OUT / "forums").mkdir(exist_ok=True)
    by_course = {}
    for d in discussions:
        by_course.setdefault(d["course"], []).append(d)
    for course, ds in by_course.items():
        L = [f"# Q&A forums - {course}", ""]
        for forum in dict.fromkeys(d["forum"] for d in ds):
            L += [f"## {forum}", ""]
            for d in sorted((d for d in ds if d["forum"] == forum),
                            key=lambda d: -max((p["time"] for p in d["posts"]), default=0)):
                L += [f"### {d['subject']}", d["url"], ""]
                for p in d["posts"]:
                    flags = "".join([" [SOLUTION]" if p["solution"] else "",
                                     " [HELPFUL]" if p["helpful"] else ""])
                    indent = "  " if p["kind"] == "comment" else ""
                    L.append(f"{indent}- **{p['kind']}** by {p['author']}, "
                             f"{fmt_ts(p['time'])}{flags}: {p['text']}")
                L.append("")
        (OUT / "forums" / f"{slug(course)}.md").write_text("\n".join(L))


MODULE_CODE = re.compile(r"\d[A-Z]{3,4}\d[A-Z0-9]+")
ASSESSMENT_WORDS = re.compile(r"deadline|coursework|assessment|deliverable|submi(t|ssion)|\bdue\b", re.I)


def table_text(s):
    """HTML -> text with each table row on one line, cells split by " | "."""
    def row(m):
        cells = re.findall(r"<t[dh][^>]*>([\s\S]*?)</t[dh]>", m.group(0))
        return " | ".join(strip_html(c, 10**6) for c in cells) + "<br>"
    s = re.sub(r"<tr[\s\S]*?</tr>", row, s or "")
    s = re.sub(r"<br\s*/?>|</p>|</li>|</h\d>", "\n", s)
    s = html.unescape(re.sub(r"<[^>]+>", " ", s))
    return "\n".join(l for l in (re.sub(r"\s+", " ", l).strip() for l in s.split("\n")) if l)


def assessment_text(sections):
    """Everything in a course that might state a coursework deadline, for the
    dash deadline extractor. Modules phrase this inconsistently (tables, link
    names like "Part 1 (due 4 January, 4pm)", "TBD"), so this only gathers."""
    out = []
    for s in sections:
        mods = [m for m in s.get("modules", []) if m.get("uservisible", True)]
        parts = []
        if ASSESSMENT_WORDS.search(s.get("summary") or ""):
            parts.append(table_text(s["summary"]))
        for m in mods:
            desc = m.get("description") or ""
            if ASSESSMENT_WORDS.search(desc) and m["modname"] in ("label", "page", "assign", "lti", "url"):
                parts.append(f"[{m['modname']}] {html.unescape(m['name'])}\n{table_text(desc)}")
            elif ASSESSMENT_WORDS.search(m["name"]) and m["modname"] != "quiz":
                parts.append(f"[{m['modname']}] {html.unescape(m['name'])}")
        if parts:
            out.append(f"## {strip_html(s['name'], 120)}\n" + "\n\n".join(parts))
    return "\n\n".join(out)


def course_label(c):
    code = (c.get("shortname") or "").split(" ")[0]
    if MODULE_CODE.fullmatch(code):
        return code
    name = html.unescape(c["fullname"])
    return re.split(r" [–-] | \(|\(", name)[0].strip()


def write_items(now, courses, events, dated, news, discussions, forum_note):
    """Structured output for the dash TUI (OUT/items.json)."""
    label = {c["fullname"]: course_label(c) for c in courses}
    org = lambda fullname: label.get(fullname) or html.unescape(fullname)
    items, seen = [], set()
    # Timeline events are what KEATS still wants from you; other dated
    # assignments/quizzes are kept but flagged as not pending (likely submitted).
    for e in events:
        url = e.get("url", "")
        seen.add(url)
        items.append({"id": f"event:{e['id']}", "section": "due",
                      "tag": e.get("modulename") or "event",
                      "org": org(e.get("course", {}).get("fullname", "?")),
                      "title": html.unescape(e["name"]), "due": e["timesort"],
                      "url": url, "pending": True})
    for ts, course, name, url in dated:
        if ts < now - 86400 or url in seen:
            continue
        items.append({"id": url, "section": "due",
                      "tag": "quiz" if "/mod/quiz/" in url else "assign",
                      "org": org(course), "title": html.unescape(name), "due": ts,
                      "url": url, "pending": False})
    for ts, course, name, msg, url in news:
        if ts >= now - FORUM_RECENT_DAYS * 86400:
            items.append({"id": url, "section": "news", "org": org(course),
                          "title": html.unescape(name), "when": ts, "url": url, "note": msg})
    for d in discussions:
        last = max((p["time"] for p in d["posts"]), default=0)
        if last < now - FORUM_RECENT_DAYS * 86400:
            continue
        answers = sum(p["kind"] == "answer" for p in d["posts"])
        solved = " · solved" if any(p["solution"] for p in d["posts"]) else ""
        items.append({"id": d["url"], "section": "forum", "org": org(d["course"]),
                      "title": d["subject"], "when": last, "url": d["url"],
                      "note": f"{answers} answer{'s' * (answers != 1)}{solved}"})
    tmp = OUT / "items.json.tmp"
    tmp.write_text(json.dumps({"generated": now, "forum_note": forum_note, "items": items},
                              indent=1, ensure_ascii=False))
    tmp.replace(OUT / "items.json")


def main():
    global TOKEN
    TOKEN = load_token()
    OUT.mkdir(exist_ok=True)
    (OUT / "courses").mkdir(exist_ok=True)
    now = int(time.time())

    info = call("core_webservice_get_site_info")
    print(f"Logged in as {info['fullname']}")
    courses = call("core_enrol_get_users_courses", userid=info["userid"])
    courses = [c for c in courses if not c.get("hidden")]
    ids = [c["id"] for c in courses]
    names = {c["id"]: c["fullname"] for c in courses}

    # To-do timeline (what the KEATS dashboard shows: unsubmitted, actionable)
    events = try_call("core_calendar_get_action_events_by_timesort",
                      timesortfrom=now - 7 * 86400, limitnum=50) or {"events": []}

    # All assignment due dates
    assigns = []
    res = try_call("mod_assign_get_assignments", courseids=ids) or {"courses": []}
    for c in res["courses"]:
        for a in c["assignments"]:
            assigns.append((a["duedate"], c["fullname"], a["name"],
                            f"{SITE}/mod/assign/view.php?id={a['cmid']}"))
    quizzes = []
    res = try_call("mod_quiz_get_quizzes_by_courses", courseids=ids) or {"quizzes": []}
    for q in res["quizzes"]:
        quizzes.append((q.get("timeclose") or 0, names.get(q["course"], "?"), q["name"],
                        f"{SITE}/mod/quiz/view.php?id={q['coursemodule']}"))

    # Announcements: modules don't consistently use the "news" type (some are
    # "general" or "blog"), so also match on the forum name.
    news = []
    forums = try_call("mod_forum_get_forums_by_courses", courseids=ids) or []
    for f in forums:
        if f.get("type") != "news" and "announcement" not in f["name"].lower():
            continue
        d = try_call("mod_forum_get_forum_discussions", forumid=f["id"], page=0, perpage=5)
        for disc in (d or {}).get("discussions", []):
            news.append((disc.get("timemodified") or disc.get("created", 0),
                         names.get(f["course"], "?"), disc["name"],
                         strip_html(disc.get("message")),
                         f"{SITE}/mod/forum/discuss.php?d={disc['discussion']}"))

    # Per-course materials
    overflow_forums, new_files, assessments = [], [], {}
    for c in courses:
        print(f"  {c['shortname']}")
        sections = try_call("core_course_get_contents", courseid=c["id"]) or []
        if MODULE_CODE.fullmatch(course_label(c)):
            assessments[course_label(c)] = {"url": f"{SITE}/course/view.php?id={c['id']}",
                                            "text": assessment_text(sections)}
        lines = [f"# {c['fullname']}", f"{SITE}/course/view.php?id={c['id']}", ""]
        for s in sections:
            mods = [m for m in s.get("modules", []) if m.get("uservisible", True)]
            overflow_forums += [(c["fullname"], html.unescape(m["name"]), m["id"])
                                for m in mods if m["modname"] == "moodleoverflow"]
            if not mods:
                continue
            lines.append(f"## {strip_html(s['name'], 120) or 'Untitled section'}")
            if s.get("summary"):
                lines.append(f"> {strip_html(s['summary'])}")
            for m in mods:
                lines.append(f"- [{m['modname']}] **{m['name']}** - {m.get('url', '')}")
                for f in m.get("contents", []) or []:
                    if f.get("type") == "file":
                        lines.append(f"  - {f['filename']} (modified {fmt_ts(f.get('timemodified'))})")
            lines.append("")
        (OUT / "courses" / f"{slug(c['shortname'])}.md").write_text("\n".join(lines))
        (OUT / "courses" / f"{slug(c['shortname'])}.json").write_text(json.dumps(sections, indent=1))
        if any(c["shortname"].startswith(p) for p in DOWNLOAD_COURSES):
            for rel in download_course_files(c, sections):
                new_files.append((c["fullname"], rel))

    # Moodleoverflow Q&A forums (needs a browser session cookie)
    discussions, forum_note = [], None
    cookie = SESSION_FILE.read_text().strip() if SESSION_FILE.exists() else ""
    try:
        try:
            if not cookie:
                raise SessionExpired()
            discussions = crawl_overflow(overflow_forums, cookie)
        except SessionExpired:
            discussions = crawl_overflow(overflow_forums, new_session())
        write_forum_files(discussions)
    except SessionExpired:
        forum_note = "skipped: session expired - run dash login"
    if forum_note:
        print(f"  ! Q&A forums {forum_note}", file=sys.stderr)

    # Summary
    L = [f"# KEATS summary - generated {fmt_ts(now)}", "", "## Courses"]
    L += [f"- {c['fullname']} ({c['shortname']}) - courses/{slug(c['shortname'])}.md" for c in courses]
    L += ["", "## To-do (KEATS timeline: not yet submitted)"]
    for e in events["events"]:
        L.append(f"- {fmt_ts(e['timesort'])} | {e.get('course', {}).get('fullname', '?')} | "
                 f"{e['name']} - {e.get('url', '')}")
    L += ["", "## Upcoming due dates (assignments + quizzes)"]
    for ts, course, name, url in sorted(x for x in assigns + quizzes if x[0] >= now):
        L.append(f"- {fmt_ts(ts)} | {course} | {name} - {url}")
    L += ["", "## Recent announcements"]
    for ts, course, name, msg, _ in sorted(news, reverse=True)[:25]:
        L += [f"- {fmt_ts(ts)} | {course} | **{name}**", f"  {msg}"]
    L += ["", f"## Files downloaded this run (into {FILES})"]
    L += [f"- {course} | {rel}" for course, rel in new_files] or ["- none new"]
    L += ["", f"## Q&A forum activity (last {FORUM_RECENT_DAYS} days)"]
    if forum_note:
        L.append(f"_{forum_note}_")
    recent = []
    for d in discussions:
        last = max((p["time"] for p in d["posts"]), default=0)
        if last >= now - FORUM_RECENT_DAYS * 86400:
            recent.append((last, d))
    for last, d in sorted(recent, key=lambda x: -x[0]):
        answers = sum(p["kind"] == "answer" for p in d["posts"])
        solved = " (solved)" if any(p["solution"] for p in d["posts"]) else ""
        L.append(f"- {fmt_ts(last)} | {d['course']} | {d['forum']} | **{d['subject']}** - "
                 f"{answers} answer(s){solved} - {d['url']}")
    (OUT / "summary.md").write_text("\n".join(L) + "\n")
    print(f"Wrote {OUT / 'summary.md'}")

    write_items(now, courses, events["events"], assigns + quizzes, news, discussions, forum_note)
    print(f"Wrote {OUT / 'items.json'}")
    (OUT / "assessments.json").write_text(json.dumps(assessments, indent=1, ensure_ascii=False))
    print(f"Wrote {OUT / 'assessments.json'}")


if __name__ == "__main__":
    main()
