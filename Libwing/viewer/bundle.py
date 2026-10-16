"""Build a single self-contained index.html of the dashboard for sharing.

Usage (from the Libwing folder):  python3 viewer/bundle.py
Writes ~/Desktop/Wing Dashboard YYYY-MM-DD.zip containing one index.html with the
dashboard code, the newest console map and the snapshot map (if any) built in.
Recipients unzip it and open index.html; nothing else is needed.
Read-only: this only reads files in this folder; it never talks to the console.
"""
import datetime, os, re, zipfile

here = os.path.dirname(os.path.abspath(__file__))
maps = os.path.join(here, "..", "maps")

def read(path):
    with open(path, encoding="utf-8") as f:
        return f.read()

def inline(src):
    # "</script" inside an inline script would end it early; "<\/script" means the same in JS.
    # data-map marks map data so "Share this map" in the page can swap it out.
    return '<script data-map="">\n' + src.replace("</script", "<\\/script") + "\n</script>"

html = read(os.path.join(here, "index.html"))
scripts = {
    "../maps/church-wing-map.js": os.path.join(maps, "church-wing-map.js"),
    "../maps/snapmap/snap-map.js": os.path.join(maps, "snapmap", "snap-map.js"),
}
for src, path in scripts.items():
    tag = f'<script src="{src}"></script>'
    if tag not in html:
        raise SystemExit(f"index.html no longer contains {tag}; update bundle.py")
    html = html.replace(tag, inline(read(path)) if os.path.exists(path) else "", 1)
if re.search(r"^\s*<script src=", html, re.M):
    raise SystemExit("index.html loads a script bundle.py doesn't know about; update bundle.py")

stamp = datetime.datetime.fromtimestamp(os.path.getmtime(os.path.join(maps, "church-wing-map.js"))).strftime("%Y-%m-%d")
out = os.path.expanduser(f"~/Desktop/Wing Dashboard {stamp}.zip")
with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as z:
    z.writestr("index.html", html)
print(f"Wrote {out} ({os.path.getsize(out) / 1e6:.1f} MB zipped, index.html is {len(html.encode()) / 1e6:.1f} MB)")
