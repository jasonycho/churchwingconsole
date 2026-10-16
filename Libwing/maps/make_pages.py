"""Turn wingmap's Mermaid output (diagram.txt) into human-friendly Graphviz pages.

Usage: python3 make_pages.py diagram.txt OUT_DIR
Writes 01..06 *.dot files into OUT_DIR; render each with: dot -Tpng -Gdpi=150 X.dot -o X.png
Read-only: this only reads the map file; it never talks to the console.
"""
import html, re, sys
from collections import defaultdict

src, outdir = sys.argv[1], sys.argv[2]

NODE = re.compile(r'^\s*([A-Za-z0-9_]+)(\(\[|\{\{|\[)"(.*)"(\]\)|\}\}|\])\s*$')
EDGE = re.compile(r'^\s*([A-Za-z0-9_]+)\s+(-->|-\.->)(?:\|(.*?)\|)?\s+([A-Za-z0-9_]+)\s*$')
CLASS = re.compile(r'^\s*class\s+([\w,]+)\s+muted\s*$')

nodes, audio, control, muted = {}, [], [], set()
for ln in open(src, encoding="utf-8"):
    if m := NODE.match(ln):
        parts = [html.unescape(p) for p in m[3].split("<br/>")]
        small = [re.sub(r"</?small>", "", p) for p in parts if p.startswith("<small>")]
        plain = [p for p in parts if not p.startswith("<small>")]
        nodes[m[1]] = {"id": plain[0], "name": plain[1] if len(plain) > 1 else "", "small": small}
    elif m := EDGE.match(ln):
        (audio if m[2] == "-->" else control).append((m[1], m[3], m[4]))
    elif m := CLASS.match(ln):
        muted.update(m[1].split(","))

def num(nid):
    return int(re.sub(r"\D", "", nid) or 0)

def kind(nid):
    return re.match(r"[A-Z]+", nid)[0]

def name(nid):
    n = nodes[nid]
    return n["name"] or n["id"]

def fader(nid):
    return nodes[nid]["small"][0] if nodes[nid]["small"] else ""

def db(s):
    if not s:
        return None
    v = s.replace(" dB", "")
    return float("-inf") if v == "-inf" else float(v)

e = html.escape
INK, GREY, FAINT, MUTE = "#1A1A1A", "#6B6B6B", "#B0B0B0", "#C00000"
FONT = 'face="Helvetica"'

def page(title, subtitle, body, extra=""):
    return ("digraph G {\n"
            '  graph [fontname="Helvetica", labelloc=t, fontsize=22, pad=0.4, nodesep=0.4, ranksep=0.8,\n'
            f'         label=<<table border="0"><tr><td><font point-size="22"><b>{e(title)}</b></font></td></tr>'
            f'<tr><td><font point-size="13" color="{GREY}">{e(subtitle)}</font></td></tr>'
            + (f'<tr><td>{extra}</td></tr>' if extra else "") + '</table>>];\n'
            '  node [fontname="Helvetica", shape=plain];\n'
            '  edge [fontname="Helvetica", fontsize=11, color="#555555"];\n'
            f"{body}\n}}\n")

def write(fname, text):
    open(f"{outdir}/{fname}", "w", encoding="utf-8").write(text)

def who(nid):
    """Bold human name plus grey 'CH 3' helper, as one HTML fragment."""
    tag = f' <font color="{MUTE}"><b>MUTED</b></font>' if nid in muted else ""
    return f'<b>{e(name(nid))}</b>  <font point-size="10" color="{GREY}">{e(nodes[nid]["id"])}</font>{tag}'

def level_bg(v):
    if v is None or v == float("-inf") or v < -60: return "#FFFFFF"
    if v >= -6:  return "#2E6FBF"
    if v >= -15: return "#7FA8DC"
    if v >= -30: return "#C5D7EE"
    return "#EEF3FA"

def level_fg(v):
    if v is None or v == float("-inf") or v < -60: return FAINT
    return "#FFFFFF" if v >= -6 else INK

src_of = {b: a for a, lab, b in audio if a.startswith("SRC_")}
ins = defaultdict(list)          # destination -> [(source, level or None)]
for a, lab, b in audio:
    if not a.startswith("SRC_"):
        ins[b].append((a, lab))

def bus_from_port(src_id):
    # Bus source ports are stereo pairs: ports 2n-1/2n carry bus n.
    return f"BUS{(num(src_id) + 1) // 2}"

def source_cell(chan):
    s = src_of.get(chan)
    if not s:
        return f'<font color="{FAINT}">no source</font>', ""
    n = nodes[s]
    port, label = n["id"], n["name"]
    if s.startswith("SRC_BUS_"):
        b = bus_from_port(s)
        label = f"{name(b)} return" if b in nodes else label
        port = f"{port} (= {nodes[b]['id']} output)" if b in nodes else port
    phantom = "48V" if "48V" in n["small"] else ""
    return (f'<b>{e(label or port)}</b><br/><font point-size="10" color="{GREY}">{e(port)}</font>', phantom)

# ---------- 01 input patch -------------------------------------------------
def patch_rows(ids):
    rows = []
    for i, c in enumerate(ids):
        bg = "#F6F6F6" if i % 2 else "#FFFFFF"
        scell, ph = source_cell(c)
        phcell = f'<font color="#B35C00"><b>{ph}</b></font>' if ph else ""
        status = f'<font color="{MUTE}"><b>MUTED</b></font>' if c in muted else ""
        rows.append(
            f'<tr><td align="left" bgcolor="{bg}"><b>{e(name(c))}</b><br/><font point-size="10" color="{GREY}">{e(nodes[c]["id"])}</font></td>'
            f'<td align="left" bgcolor="{bg}">{scell}</td>'
            f'<td bgcolor="{bg}">{phcell}</td>'
            f'<td align="right" bgcolor="{bg}"><font color="{GREY}">{e(fader(c))}</font></td>'
            f'<td bgcolor="{bg}">{status}</td></tr>')
    return "".join(rows)

def header(*cols):
    return "<tr>" + "".join(f'<td bgcolor="#3A3A3A" align="left"><font color="white"><b>{c}</b></font></td>' for c in cols) + "</tr>"

def section(title, span):
    return f'<tr><td colspan="{span}" align="left" bgcolor="#FFF6D6"><b>{e(title)}</b></td></tr>'

chs = sorted([n for n in nodes if kind(n) == "CH"], key=num)
auxs = sorted([n for n in nodes if kind(n) == "AUX"], key=num)
tbl = (f'<table border="0" cellborder="0" cellspacing="0" cellpadding="7">'
       + header("Channel", "Comes from (stage label / port)", "48V", "Fader", "Status")
       + section("Input channels", 5) + patch_rows(chs)
       + section("Aux channels (playback, computers, effect returns)", 5) + patch_rows(auxs)
       + "</table>")
write("01-input-patch.dot", page("1 · Input patch", "Who is on which channel, and where the signal comes in",
                                 f"  patch [label=<{tbl}>];"))

# ---------- 02/03 bus panels ----------------------------------------------
dest_of = defaultdict(list)
for a, lab, b in audio:
    dest_of[a].append(b)
returns = {bus_from_port(s): c for c, s in src_of.items() if s.startswith("SRC_BUS_")}

def bus_panel(b):
    feeds = sorted(ins[b], key=lambda t: -(db(t[1]) if db(t[1]) is not None else 99))
    live = [f for f in feeds if db(f[1]) is None or db(f[1]) >= -60]
    off = [f for f in feeds if f not in live]
    rows = []
    for s, lab in live:
        v = db(lab)
        rows.append(f'<tr><td align="left">{who(s)}</td>'
                    f'<td align="right" bgcolor="{level_bg(v)}" width="70"><font color="{level_fg(v)}"><b>{e(lab or "on")}</b></font></td></tr>')
    if not live:
        rows.append(f'<tr><td colspan="2" align="left"><font color="{FAINT}">nothing sent here</font></td></tr>')
    notes = []
    if off:
        notes.append("Assigned but effectively silent (below −60 dB): " + ", ".join(f"{name(s)} {lab}" for s, lab in off))
    goes = [nodes[d]["name"] + " (" + nodes[d]["id"] + ")" for d in dest_of.get(b, [])]
    if b in returns:
        goes.append(f"returns on {nodes[returns[b]]['id']} ({name(returns[b])})")
    if goes:
        notes.append("Goes to: " + ", ".join(goes))
    note_rows = "".join(f'<tr><td colspan="2" align="left" balign="left"><font point-size="10" color="{GREY}">{wrap(e(n))}</font></td></tr>' for n in notes)
    m = " · MUTED" if b in muted else ""
    return (f'<table border="1" color="#AAAA33" cellborder="0" cellspacing="0" cellpadding="5" bgcolor="white">'
            f'<tr><td colspan="2" align="left" bgcolor="#FFF6D6"><font point-size="16"><b>{e(name(b))}</b></font><br/>'
            f'<font point-size="10" color="{GREY}">{e(nodes[b]["id"])} · master {e(fader(b))}{m}</font></td></tr>'
            f'<tr><td align="left"><font point-size="10" color="{GREY}">WHO IS IN IT</font></td><td align="right"><font point-size="10" color="{GREY}">SEND</font></td></tr>'
            + "".join(rows) + note_rows + "</table>")

def wrap(text, width=48):
    out, line = [], ""
    for w in text.split(" "):
        if len(line) + len(w) + 1 > width and line:
            out.append(line); line = w
        else:
            line = f"{line} {w}".strip()
    out.append(line)
    return "<br/>".join(out)

def panels(cells, cols, prefix):
    """Free-standing panel nodes in columns (rankdir=LR, one rank per column) so each keeps its own height."""
    out = ["  graph [rankdir=LR, nodesep=0.35, ranksep=0.35];"]
    ids = [[] for _ in range(cols)]
    for i, c in enumerate(cells):
        nid = f"{prefix}{i}"
        ids[i % cols].append(nid)
        out.append(f"  {nid} [label=<{c}>];")
    for col in ids:
        out.append("  { rank=same; " + " ".join(col) + " }")
        out += [f"  {a} -> {b} [style=invis];" for a, b in zip(col, col[1:])]
    for left, right in zip(ids, ids[1:]):
        out += [f"  {a} -> {b} [style=invis];" for a, b in zip(left, right)]
    return "\n".join(out)

def grid(cells, cols):
    rows = ["<tr>" + "".join(f'<td valign="top">{c}</td>' for c in cells[i:i + cols]) + "</tr>" for i in range(0, len(cells), cols)]
    return f'<table border="0" cellspacing="18" cellpadding="0">{"".join(rows)}</table>'

legend = (f'<table border="0" cellspacing="4" cellpadding="4"><tr><td><font color="{GREY}">Send level:</font></td>'
          + "".join(f'<td bgcolor="{level_bg(v)}"><font color="{level_fg(v)}">{t}</font></td>' for v, t in
                    [(0, "louder than −6 dB"), (-10, "−6 to −15"), (-20, "−15 to −30"), (-40, "−30 to −60"), (-80, "below −60 (≈ off)")])
          + "</tr></table>")

buses = sorted([n for n in nodes if kind(n) == "BUS"], key=num)
mon = [b for b in buses if num(b) == 1 or 5 <= num(b) <= 12]
fx = [b for b in buses if b not in mon]
write("02-monitor-mixes.dot", page("2 · Monitor mixes", "One box per mix: who each person hears, loudest first",
      panels([bus_panel(b) for b in mon], 3, "m"), legend))
write("03-effects-and-utility-buses.dot", page("3 · Effects and utility buses", "Reverbs, exciter, talkback, woofer and vocal sends",
      panels([bus_panel(b) for b in fx], 4, "f"), legend))

# ---------- 04 outputs -----------------------------------------------------
mains = sorted([n for n in nodes if kind(n) == "MAIN"], key=num)
mtxs = sorted([n for n in nodes if kind(n) == "MTX"], key=num)
body = []
for mn in mains:
    feeds = sorted(ins[mn], key=lambda t: (["CH", "AUX", "BUS"].index(kind(t[0])) if kind(t[0]) in ("CH", "AUX", "BUS") else 9, num(t[0])))
    items = [f'<tr><td align="left">{who(s)}</td></tr>' for s, _ in feeds] or [f'<tr><td><font color="{FAINT}">nothing assigned</font></td></tr>']
    warn = ""
    if db(fader(mn)) == float("-inf"):
        warn = f'<tr><td align="left"><font color="{MUTE}" point-size="11"><b>Fader is at −inf: nothing comes out</b></font></td></tr>'
    lbl = (f'<table border="1" color="#9370DB" cellborder="0" cellspacing="0" cellpadding="4" bgcolor="white">'
           f'<tr><td align="left" bgcolor="#ECECFF"><font point-size="18"><b>{e(name(mn))}</b></font><br/>'
           f'<font point-size="10" color="{GREY}">{e(nodes[mn]["id"])} · fader {e(fader(mn))} · {len(feeds)} sources</font></td></tr>'
           + warn + "".join(items) + "</table>")
    body.append(f"  {mn} [label=<{lbl}>];")
for mx in mtxs:
    body.append(f'  {mx} [shape=box, style="rounded,filled", fillcolor="#E8F4EA", color="#3C8D4F", margin="0.2,0.1", '
                f'label=<<font point-size="16"><b>{e(name(mx))}</b></font><br/><font point-size="10" color="{GREY}">{e(nodes[mx]["id"])} · fader {e(fader(mx))}</font>>];')
for a, lab, b in audio:
    if kind(a) == "MAIN" and kind(b) == "MTX":
        body.append(f'  {a} -> {b} [label="{lab}", penwidth=1.6];')
body.append("  { rank=same; " + " ".join(mains) + " }")
write("04-outputs.dot", page("4 · Outputs: what goes where", "Each main mix with everything assigned to it; Main 1 then feeds the matrix outputs", "\n".join(body)))

# ---------- 05 control -----------------------------------------------------
members = defaultdict(list)
for a, _, b in control:
    members[a].append(b)
cells = []
for g in sorted([n for n in nodes if kind(n) in ("DCA", "MGRP")], key=lambda n: (kind(n) != "DCA", num(n))):
    is_m = g in muted
    head_bg = "#FDDDDD" if is_m else ("#ECECFF" if kind(g) == "DCA" else "#F0F0F0")
    state = f'<br/><font color="{MUTE}"><b>MUTE IS ON: everything below is silenced</b></font>' if is_m else ""
    what = "Volume group" if kind(g) == "DCA" else "Mute group"
    items = "".join(f'<tr><td align="left">{who(m)}</td></tr>' for m in sorted(members[g], key=lambda n: (kind(n), num(n))))
    cells.append(f'<table border="1" color="{MUTE if is_m else "#9370DB"}" cellborder="0" cellspacing="0" cellpadding="4" bgcolor="white">'
                 f'<tr><td align="left" bgcolor="{head_bg}"><font point-size="16"><b>{e(name(g))}</b></font><br/>'
                 f'<font point-size="10" color="{GREY}">{e(nodes[g]["id"])} · {what}</font>{state}</td></tr>{items}</table>')
write("05-dcas-and-mute-groups.dot", page("5 · DCAs and mute groups", "What each volume group and mute button controls",
      panels(cells, 4, "c")))

# ---------- 06 send grid ---------------------------------------------------
cols = buses + mains
send = {(a, b): lab for a, lab, b in audio}
rows_ids = [n for n in chs + auxs if any((n, c) in send for c in cols)]
hdr = '<tr><td bgcolor="#3A3A3A"></td>' + "".join(
    f'<td bgcolor="{"#3A3A3A" if kind(c) == "BUS" else "#4B3F7A"}"><font color="white" point-size="10"><b>{e(name(c))}</b><br/>{e(nodes[c]["id"])}</font></td>' for c in cols) + "</tr>"
body_rows = []
for i, r in enumerate(rows_ids):
    tds = []
    for c in cols:
        if (r, c) not in send:
            tds.append('<td bgcolor="#FFFFFF"> </td>')
        elif kind(c) == "MAIN":
            tds.append('<td bgcolor="#DCD6F2"><font color="#4B3F7A"><b>●</b></font></td>')
        else:
            v = db(send[(r, c)])
            tds.append(f'<td bgcolor="{level_bg(v)}"><font color="{level_fg(v)}" point-size="10">{e(send[(r, c)].replace(" dB", ""))}</font></td>')
    stripe = "#F6F6F6" if i % 2 else "#FFFFFF"
    body_rows.append(f'<tr><td align="left" bgcolor="{stripe}">{who(r)}</td>{"".join(tds)}</tr>')
gt = f'<table border="0" cellborder="1" color="#DDDDDD" cellspacing="0" cellpadding="5">{hdr}{"".join(body_rows)}</table>'
write("06-send-grid.dot", page("6 · Send grid: everything at once", "Rows = channels, columns = buses (send level in dB) and mains (● = assigned)",
      f'  g [label=<<table border="0"><tr><td>{legend}</td></tr><tr><td>{gt}</td></tr></table>>];'))
print(f"nodes={len(nodes)} audio={len(audio)} control={len(control)} muted={len(muted)}")
