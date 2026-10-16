mod utils;
use utils::Args;

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::result::Result;

use libwing::{WingConsole, WingResponse, WingNodeDef, NodeType, NodeUnit};

// Read-only alternative to wingschema. It only ever sends "request definition"
// and "request data" messages, never set_*, so it will not change the mixer.
// The trade-off is that dynamic subtrees (FX, gate, EQ slots, etc...) are only
// mapped for the model currently loaded in each slot.
//
// The map is built in two passes:
//   1. walk the whole definition tree (names, labels, types, ranges)
//   2. read the current value of every property
// The passes must not be interleaved: get_node_def() matches replies to
// requests by counting RequestEnd markers, and value replies mixed into that
// stream shift the count so later nodes come back with no children.

struct Entry {
    def: WingNodeDef,
    fullname: String,
    label: String,
    value: Option<String>,
}

fn get_node_def(wing: &mut WingConsole, parents: &[i32]) -> Result<Vec<Vec<WingNodeDef>>, libwing::Error> {
    for parent in parents {
        wing.request_node_definition(*parent)?;
    }

    let mut ret = Vec::new();
    for parent in parents {
        let mut ret2 = Vec::new();
        loop {
            match wing.read()? {
                WingResponse::NodeData(_, _) => { }
                WingResponse::NodeDef(def) => {
                    if def.parent_id == *parent {
                        ret2.push(def);
                    }
                }
                WingResponse::RequestEnd => {
                    break;
                }
            }
        }
        ret.push(ret2);
    }
    Ok(ret)
}

fn walk(wing: &mut WingConsole, entries: &mut Vec<Entry>, parent_fullname: &str, parent_label: &str, nodes: Vec<WingNodeDef>) -> Result<(), libwing::Error> {
    let children = get_node_def(wing, &nodes.iter().map(|x| x.id).collect::<Vec<_>>())?;

    for (def, kids) in nodes.into_iter().zip(children) {
        let part = if def.name.is_empty() { def.index.to_string() } else { def.name.clone() };
        let fullname = format!("{}/{}", parent_fullname, part);

        // Human label: prefer the console's long name, e.g. "CHANNEL › 1 › INPUT › TRIM".
        let label_part = if !def.long_name.is_empty() { def.long_name.clone() } else { part };
        let label = if parent_label.is_empty() { label_part } else { format!("{} › {}", parent_label, label_part) };

        entries.push(Entry { def, fullname: fullname.clone(), label: label.clone(), value: None });

        print!("\rMapped {} properties", entries.len());
        std::io::stdout().flush().unwrap();

        walk(wing, entries, &fullname, &label, kids)?;
    }
    Ok(())
}

// Requests values in batches. Each batch is followed by a root definition
// request as a fence: replies arrive in order, so once the root's children
// and the RequestEnd after them come back, every value in the batch has been
// answered (or never will be), and the stream is clean for the next batch.
fn read_values(wing: &mut WingConsole, entries: &mut [Entry]) -> Result<(), libwing::Error> {
    const BATCH: usize = 100;
    let idx: Vec<usize> = (0..entries.len()).filter(|&i| entries[i].def.node_type != NodeType::Node).collect();
    let total = idx.len();
    let mut done = 0;

    for chunk in idx.chunks(BATCH) {
        let mut want: HashMap<i32, usize> = HashMap::new();
        for &i in chunk {
            want.insert(entries[i].def.id, i);
            wing.request_node_data(entries[i].def.id)?;
        }
        wing.request_node_definition(0)?;

        let mut fence_seen = false;
        loop {
            match wing.read()? {
                WingResponse::NodeData(id, data) => {
                    if let Some(&i) = want.get(&id) {
                        entries[i].value = Some(data.get_string());
                    }
                }
                WingResponse::NodeDef(def) => {
                    if def.parent_id == 0 { fence_seen = true; }
                }
                WingResponse::RequestEnd => {
                    if fence_seen { break; }
                }
            }
        }

        done += chunk.len();
        print!("\rRead {}/{} values", done, total);
        std::io::stdout().flush().unwrap();
    }
    println!();
    Ok(())
}

fn unit_str(u: &NodeUnit) -> &'static str {
    match u {
        NodeUnit::None         => "",
        NodeUnit::Db           => "dB",
        NodeUnit::Percent      => "%",
        NodeUnit::Milliseconds => "ms",
        NodeUnit::Hertz        => "Hz",
        NodeUnit::Meters       => "m",
        NodeUnit::Seconds      => "s",
        NodeUnit::Octaves      => "oct",
    }
}

// Turns a raw value into what you'd see on the console screen.
fn pretty(e: &Entry) -> String {
    let Some(raw) = &e.value else { return String::new() };
    let def = &e.def;
    match def.node_type {
        NodeType::StringEnum => {
            if let Some(items) = &def.string_enum {
                if let Some(it) = items.iter().find(|it| &it.item == raw) {
                    if !it.long_item.is_empty() && it.long_item != it.item {
                        return format!("{} ({})", it.long_item, it.item);
                    }
                }
            }
            raw.clone()
        }
        NodeType::Integer => {
            if def.min_int == Some(0) && def.max_int == Some(1) {
                match raw.as_str() { "0" => "OFF".into(), "1" => "ON".into(), _ => raw.clone() }
            } else {
                raw.clone()
            }
        }
        NodeType::FaderLevel | NodeType::LinearFloat | NodeType::LogarithmicFloat | NodeType::FloatEnum => {
            match raw.parse::<f32>() {
                Ok(f) if def.node_type == NodeType::FaderLevel && f <= -144.0 => "-inf dB".into(),
                Ok(f) => {
                    let s = if f.abs() >= 100.0 { format!("{:.0}", f) } else if f.abs() >= 10.0 { format!("{:.1}", f) } else { format!("{:.2}", f) };
                    let u = unit_str(&def.unit);
                    if u.is_empty() { s } else { format!("{} {}", s, u) }
                }
                Err(_) => raw.clone(),
            }
        }
        _ => raw.clone(),
    }
}

fn type_str(t: &NodeType) -> &'static str {
    match t {
        NodeType::Node             => "folder",
        NodeType::LinearFloat      => "number",
        NodeType::LogarithmicFloat => "number",
        NodeType::FaderLevel       => "fader",
        NodeType::Integer          => "integer",
        NodeType::StringEnum       => "choice",
        NodeType::FloatEnum        => "choice",
        NodeType::String           => "text",
    }
}

fn csv(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn write_jsonl(path: &str, entries: &[Entry]) {
    let mut out = BufWriter::new(File::create(path).unwrap());
    for e in entries {
        let mut json = e.def.to_json();
        json.insert("fullname", e.fullname.clone()).unwrap();
        json.insert("label", e.label.clone()).unwrap();
        if let Some(v) = &e.value {
            json.insert("value", v.clone()).unwrap();
            json.insert("display", pretty(e)).unwrap();
        }
        writeln!(out, "{}", jzon::stringify(json)).unwrap();
    }
}

// Same map as the .jsonl, wrapped in a script so viewer/index.html can load it
// automatically: browsers block file:// pages from reading a .jsonl directly,
// but they will run a <script src>. Each JSONL line is also a valid JS object.
fn write_js(path: &str, entries: &[Entry], jsonl_path: &str, archived: &str, host_desc: &str) {
    let mut out = BufWriter::new(File::create(path).unwrap());
    let file = std::path::Path::new(jsonl_path).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    let created = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    writeln!(out, "// Generated by wingmap from {}. Loaded automatically by viewer/index.html.", file).unwrap();
    writeln!(out, "window.WING_MAPS = window.WING_MAPS || {{}};").unwrap();
    writeln!(out, "window.WING_MAPS.live = {{ \"kind\": \"live\", \"file\": {}, \"archived\": {}, \"host\": {}, \"created\": {}, \"lines\": [",
        jzon::stringify(file.as_str()), jzon::stringify(archived), jzon::stringify(host_desc), created).unwrap();
    let mut first = true;
    for e in entries {
        let mut json = e.def.to_json();
        json.insert("fullname", e.fullname.clone()).unwrap();
        json.insert("label", e.label.clone()).unwrap();
        if let Some(v) = &e.value {
            json.insert("value", v.clone()).unwrap();
        }
        writeln!(out, "{}{}", if first { "" } else { "," }, jzon::stringify(json)).unwrap();
        first = false;
    }
    writeln!(out, "]}};").unwrap();
}

fn write_csv(path: &str, entries: &[Entry]) {
    let mut out = BufWriter::new(File::create(path).unwrap());
    writeln!(out, "Setting,Value,Path,Type,Read only").unwrap();
    for e in entries.iter().filter(|e| e.def.node_type != NodeType::Node) {
        writeln!(out, "{},{},{},{},{}",
            csv(&e.label), csv(&pretty(e)), csv(&e.fullname),
            type_str(&e.def.node_type), if e.def.read_only { "yes" } else { "" }).unwrap();
    }
}

// Console overview: one line per strip with its name, source, fader and mute.
fn write_summary(path: &str, entries: &[Entry], host_desc: &str) {
    let by_path: HashMap<&str, &Entry> = entries.iter().map(|e| (e.fullname.as_str(), e)).collect();
    let val = |p: &str| -> String { by_path.get(p).map(|e| pretty(e)).unwrap_or_default() };
    let raw = |p: &str| -> String { by_path.get(p).and_then(|e| e.value.clone()).unwrap_or_default() };
    let count = |prefix: &str| -> usize {
        (1..=128).take_while(|n| by_path.contains_key(format!("{}/{}", prefix, n).as_str())).count()
    };

    let mut out = BufWriter::new(File::create(path).unwrap());
    let w = &mut out;

    writeln!(w, "WING CONSOLE MAP").unwrap();
    writeln!(w, "================").unwrap();
    writeln!(w, "Console : {}", host_desc).unwrap();
    let name = raw("/$syscfg/consolename");
    if !name.is_empty() { writeln!(w, "Name    : {}", name).unwrap(); }
    writeln!(w, "Properties mapped: {}", entries.len()).unwrap();
    writeln!(w).unwrap();
    writeln!(w, "Every individual setting is listed in the .csv file next to this one").unwrap();
    writeln!(w, "(open it in Excel / Numbers / Google Sheets).").unwrap();

    // Strip tables
    let strips: [(&str, &str); 6] = [
        ("/ch", "INPUT CHANNELS"), ("/aux", "AUX CHANNELS"), ("/bus", "BUSES"),
        ("/main", "MAINS"), ("/mtx", "MATRIX"), ("/dca", "DCAs"),
    ];
    for (prefix, title) in strips {
        let n = count(prefix);
        if n == 0 { continue; }
        writeln!(w, "\n\n{}\n{}", title, "-".repeat(title.len())).unwrap();
        let has_src = by_path.contains_key(format!("{}/1/in/conn/grp", prefix).as_str());
        if has_src {
            writeln!(w, "{:<5} {:<18} {:<30} {:>9}  {:<5} {:<6}", "#", "Name", "Source", "Fader", "Mute", "Color").unwrap();
        } else {
            writeln!(w, "{:<5} {:<18} {:>9}  {:<5} {:<6}", "#", "Name", "Fader", "Mute", "Color").unwrap();
        }
        for i in 1..=n {
            let p = format!("{}/{}", prefix, i);
            let mut nm = raw(&format!("{}/name", p));
            if nm.is_empty() { nm = "(unnamed)".into(); }
            let fdr = val(&format!("{}/fdr", p));
            let mute = val(&format!("{}/mute", p));
            let col = raw(&format!("{}/col", p));
            if has_src {
                let grp = raw(&format!("{}/in/conn/grp", p));
                let inp = raw(&format!("{}/in/conn/in", p));
                let src = if grp.is_empty() || grp == "OFF" {
                    "OFF".to_string()
                } else {
                    let grp_long = by_path.get(format!("{}/in/conn/grp", p).as_str())
                        .and_then(|e| e.def.string_enum.as_ref())
                        .and_then(|items| items.iter().find(|it| it.item == grp).map(|it| it.long_item.clone()))
                        .filter(|s| !s.is_empty())
                        .unwrap_or(grp.clone());
                    let src_name = raw(&format!("/io/in/{}/{}/name", grp, inp));
                    if src_name.is_empty() { format!("{} {}", grp_long, inp) } else { format!("{} {} \"{}\"", grp_long, inp, src_name) }
                };
                writeln!(w, "{:<5} {:<18} {:<30} {:>9}  {:<5} {:<6}", i, nm, src, fdr, mute, col).unwrap();
            } else {
                writeln!(w, "{:<5} {:<18} {:>9}  {:<5} {:<6}", i, nm, fdr, mute, col).unwrap();
            }
        }
    }

    // Mute groups
    let n = count("/mgrp");
    if n > 0 {
        writeln!(w, "\n\nMUTE GROUPS\n-----------").unwrap();
        for i in 1..=n {
            let nm = raw(&format!("/mgrp/{}/name", i));
            writeln!(w, "{:<5} {:<18} mute {}", i, if nm.is_empty() { "(unnamed)".into() } else { nm }, val(&format!("/mgrp/{}/mute", i))).unwrap();
        }
    }

    // FX slots
    let n = count("/fx");
    if n > 0 {
        writeln!(w, "\n\nFX SLOTS\n--------").unwrap();
        for i in 1..=n {
            writeln!(w, "FX{:<3} {}", i, val(&format!("/fx/{}/mdl", i))).unwrap();
        }
    }

    // Physical / network input names
    writeln!(w, "\n\nNAMED INPUT SOURCES\n-------------------").unwrap();
    for e in entries.iter().filter(|e| e.fullname.starts_with("/io/in/") && e.def.name == "name") {
        if let Some(v) = &e.value {
            if !v.is_empty() {
                let base = e.fullname.trim_end_matches("/name");
                let parts: Vec<&str> = base.split('/').collect();
                let mut extra = Vec::new();
                if raw(&format!("{}/vph", base)) == "1" { extra.push("48V"); }
                if raw(&format!("{}/mute", base)) == "1" { extra.push("MUTED"); }
                let grp_label = by_path.get(parts[..parts.len()-1].join("/").as_str())
                    .map(|g| g.def.long_name.clone()).unwrap_or_default();
                writeln!(w, "{:<16} {:<4} {:<18} {}", grp_label, parts.last().unwrap(), v, extra.join(" ")).unwrap();
            }
        }
    }
}

// Mermaid signal-flow diagram: input sources -> channels -> buses/mains -> matrix,
// with DCAs shown as dashed control links. Only strips that are named or have
// a source patched are drawn, and only sends that are switched on with a level
// above -inf, so the picture shows what is actually in use.
fn write_mermaid(path: &str, entries: &[Entry]) {
    let by_path: HashMap<&str, &Entry> = entries.iter().map(|e| (e.fullname.as_str(), e)).collect();
    let raw = |p: &str| -> String { by_path.get(p).and_then(|e| e.value.clone()).unwrap_or_default() };
    let val = |p: &str| -> String { by_path.get(p).map(|e| pretty(e)).unwrap_or_default() };
    let count = |prefix: &str| -> usize {
        (1..=128).take_while(|n| by_path.contains_key(format!("{}/{}", prefix, n).as_str())).count()
    };
    let esc = |s: &str| -> String { s.replace('"', "#quot;").replace('<', "#lt;").replace('>', "#gt;") };
    let on = |p: &str| raw(p) == "1";
    let audible = |p: &str| raw(p).parse::<f32>().map(|f| f > -144.0).unwrap_or(false);

    let mut edges: Vec<String> = Vec::new();
    let mut muted: Vec<String> = Vec::new();
    let mut drawn: std::collections::HashSet<String> = std::collections::HashSet::new();

    // strip prefix, node id prefix, display prefix
    let strips: [(&str, &str, &str); 5] = [
        ("/ch", "CH", "CH"), ("/aux", "AUX", "AUX"), ("/bus", "BUS", "BUS"),
        ("/main", "MAIN", "MAIN"), ("/mtx", "MTX", "MTX"),
    ];
    let mut groups: Vec<(&str, Vec<String>)> = Vec::new();
    for (prefix, idp, disp) in strips {
        let mut lines = Vec::new();
        for i in 1..=count(prefix) {
            let p = format!("{}/{}", prefix, i);
            let name = raw(&format!("{}/name", p));
            let grp = raw(&format!("{}/in/conn/grp", p));
            let patched = !grp.is_empty() && grp != "OFF";
            // Mains are always drawn; other strips only if named or patched.
            if prefix != "/main" && name.is_empty() && !patched { continue; }
            let id = format!("{}{}", idp, i);
            let label = if name.is_empty() { format!("{} {}", disp, i) } else { format!("{} {}<br/>{}", disp, i, esc(&name)) };
            let fdr = val(&format!("{}/fdr", p));
            lines.push(format!("    {}[\"{}<br/><small>{}</small>\"]", id, label, fdr));
            if on(&format!("{}/mute", p)) { muted.push(id.clone()); }
            drawn.insert(id);
        }
        groups.push((disp, lines));
    }

    // Input sources feeding channels/aux
    let mut sources: Vec<String> = Vec::new();
    for (prefix, idp) in [("/ch", "CH"), ("/aux", "AUX")] {
        for i in 1..=count(prefix) {
            let id = format!("{}{}", idp, i);
            if !drawn.contains(&id) { continue; }
            let grp = raw(&format!("{}/{}/in/conn/grp", prefix, i));
            let inp = raw(&format!("{}/{}/in/conn/in", prefix, i));
            if grp.is_empty() || grp == "OFF" { continue; }
            let src_id = format!("SRC_{}_{}", grp.replace(|c: char| !c.is_ascii_alphanumeric(), ""), inp);
            if !drawn.contains(&src_id) {
                let grp_long = by_path.get(format!("/io/in/{}", grp).as_str())
                    .map(|e| e.def.long_name.clone()).filter(|s| !s.is_empty()).unwrap_or(grp.clone());
                let src_name = raw(&format!("/io/in/{}/{}/name", grp, inp));
                let mut label = format!("{} {}", grp_long, inp);
                if !src_name.is_empty() { label += &format!("<br/>{}", esc(&src_name)); }
                if on(&format!("/io/in/{}/{}/vph", grp, inp)) { label += "<br/><small>48V</small>"; }
                sources.push(format!("    {}([\"{}\"])", src_id, label));
                drawn.insert(src_id.clone());
            }
            edges.push(format!("  {} --> {}", src_id, id));
        }
    }

    // Sends: ch/aux/bus -> main, ch/aux -> bus, ch/aux/bus/main -> matrix
    for (prefix, idp) in [("/ch", "CH"), ("/aux", "AUX"), ("/bus", "BUS"), ("/main", "MAIN")] {
        for i in 1..=count(prefix) {
            let from = format!("{}{}", idp, i);
            if !drawn.contains(&from) { continue; }
            let p = format!("{}/{}", prefix, i);
            if prefix != "/main" {
                for m in 1..=count(&format!("{}/main", p)) {
                    let to = format!("MAIN{}", m);
                    if drawn.contains(&to) && on(&format!("{}/main/{}/on", p, m)) {
                        edges.push(format!("  {} --> {}", from, to));
                    }
                }
            }
            if prefix == "/ch" || prefix == "/aux" {
                for b in 1..=16 {
                    let s = format!("{}/send/{}", p, b);
                    let to = format!("BUS{}", b);
                    if drawn.contains(&to) && on(&format!("{}/on", s)) && audible(&format!("{}/lvl", s)) {
                        edges.push(format!("  {} -->|{}| {}", from, val(&format!("{}/lvl", s)), to));
                    }
                }
            }
            for x in 1..=8 {
                let s = format!("{}/send/MX{}", p, x);
                let to = format!("MTX{}", x);
                if drawn.contains(&to) && on(&format!("{}/on", s)) && audible(&format!("{}/lvl", s)) {
                    edges.push(format!("  {} -->|{}| {}", from, val(&format!("{}/lvl", s)), to));
                }
            }
        }
    }

    // DCAs and mute groups are assigned through strip tags, e.g. "#D1,#M2"
    let mut dcas: Vec<String> = Vec::new();
    let mut ctl_edges: Vec<String> = Vec::new();
    for (kind, prefix, idp, disp) in [('D', "/dca", "DCA", "DCA"), ('M', "/mgrp", "MGRP", "MUTE GRP")] {
        for n in 1..=count(prefix) {
            let ctl_id = format!("{}{}", idp, n);
            let tag = format!("#{}{}", kind, n);
            let mut members = Vec::new();
            for (sp, sid) in [("/ch", "CH"), ("/aux", "AUX"), ("/bus", "BUS"), ("/main", "MAIN"), ("/mtx", "MTX")] {
                for i in 1..=count(sp) {
                    let id = format!("{}{}", sid, i);
                    let tags = raw(&format!("{}/{}/tags", sp, i));
                    if drawn.contains(&id) && tags.split(|c: char| c == ',' || c.is_whitespace()).any(|t| t == tag) {
                        members.push(id);
                    }
                }
            }
            if members.is_empty() { continue; }
            let name = raw(&format!("{}/{}/name", prefix, n));
            let label = if name.is_empty() { format!("{} {}", disp, n) } else { format!("{} {}<br/>{}", disp, n, esc(&name)) };
            dcas.push(format!("    {}{{{{\"{}\"}}}}", ctl_id, label));
            if on(&format!("{}/{}/mute", prefix, n)) { muted.push(ctl_id.clone()); }
            for m in members { ctl_edges.push(format!("  {} -.-> {}", ctl_id, m)); }
        }
    }

    let mut out = BufWriter::new(File::create(path).unwrap());
    let w = &mut out;
    writeln!(w, "%% Behringer Wing signal flow, generated by wingmap").unwrap();
    writeln!(w, "%% Paste into https://mermaid.live or any Markdown viewer that supports Mermaid.").unwrap();
    writeln!(w, "%% Solid arrows = audio (labels are send levels). Dashed = DCA / mute group control. Red = muted.").unwrap();
    writeln!(w, "flowchart LR").unwrap();
    if !sources.is_empty() {
        writeln!(w, "  subgraph SOURCES[\"Input Sources\"]\n{}\n  end", sources.join("\n")).unwrap();
    }
    for (disp, lines) in &groups {
        if lines.is_empty() { continue; }
        let title = match *disp { "CH" => "Input Channels", "AUX" => "Aux Channels", "BUS" => "Buses", "MAIN" => "Mains", _ => "Matrix" };
        writeln!(w, "  subgraph G_{}[\"{}\"]\n{}\n  end", disp, title, lines.join("\n")).unwrap();
    }
    if !dcas.is_empty() {
        writeln!(w, "  subgraph CONTROL[\"DCAs & Mute Groups\"]\n{}\n  end", dcas.join("\n")).unwrap();
    }
    for e in edges.iter().chain(ctl_edges.iter()) { writeln!(w, "{}", e).unwrap(); }
    writeln!(w, "  classDef muted fill:#fdd,stroke:#c00,color:#900").unwrap();
    if !muted.is_empty() { writeln!(w, "  class {} muted", muted.join(",")).unwrap(); }
}

// Local date and time for archive file names, e.g. "2026-10-11_1358".
fn timestamp() -> String {
    std::process::Command::new("date").arg("+%Y-%m-%d_%H%M").output().ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0).to_string())
}

fn main() -> Result<(),libwing::Error> {
    let mut args = Args::new(r#"
Usage: wingmap [-h host] [-n] [-o name]

   -h host : IP address or hostname of Wing mixer. Default is to discover and connect to the first mixer found.
   -n      : Don't read current values (structure only; much faster but no names/settings).
   -o name : Output base name. Default is "maps/church-wing-map". Writes the latest map:
               name.txt    console overview: channel/bus/DCA names, sources, faders, mutes
               name.csv    every setting with a readable label and its current value
               name.jsonl  full machine-readable map
               name.js     same map, loaded automatically by viewer/index.html
             diagram.txt Mermaid signal-flow diagram (same folder as name)
             and keeps a dated copy of each session in archive/ next to name:
               archive/name_YYYY-MM-DD_HHMM.{txt,csv,jsonl}
             and a shareable one-page dashboard: ~/Desktop/Wing Dashboard YYYY-MM-DD.zip

   Read-only: maps the mixer's property tree without changing any settings.
"#);
    let mut host = None;
    let mut with_values = true;
    let mut base = String::from("maps/church-wing-map");
    while args.has_next() {
        match args.next().as_str() {
            "-h" => host = Some(args.next()),
            "-n" => with_values = false,
            "-v" => with_values = true, // old flag, values are now on by default
            "-o" => base = args.next(),
            _ => { args.print_help(None); std::process::exit(1); }
        }
    }
    for ext in [".jsonl", ".csv", ".txt", ".js"] {
        if let Some(b) = base.strip_suffix(ext) { base = b.to_string(); }
    }

    if let Some(parent) = std::path::Path::new(&base).parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).unwrap();
    }

    let mut wing = WingConsole::connect(host.as_deref())?;
    let host_desc = host.clone().unwrap_or_else(|| "auto-discovered".into());
    println!("Connected!");

    let mut entries = Vec::new();
    let root = get_node_def(&mut wing, &[0])?.remove(0);
    walk(&mut wing, &mut entries, "", "", root)?;
    println!();

    if with_values {
        read_values(&mut wing, &mut entries)?;
    }

    let base_path = std::path::Path::new(&base);
    let dir = base_path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(std::path::Path::new("."));
    let stem = base_path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_else(|| "wingmap".into());
    let archive = dir.join("archive");
    std::fs::create_dir_all(&archive).unwrap();
    let stamp = timestamp();
    let archived = |ext: &str| archive.join(format!("{}_{}.{}", stem, stamp, ext));

    let (txt, csvp, jsonl, js) = (format!("{}.txt", base), format!("{}.csv", base), format!("{}.jsonl", base), format!("{}.js", base));
    let archived_jsonl = format!("archive/{}_{}.jsonl", stem, stamp);
    write_jsonl(&jsonl, &entries);
    write_js(&js, &entries, &jsonl, &archived_jsonl, &host_desc);
    write_csv(&csvp, &entries);
    write_summary(&txt, &entries, &host_desc);
    let diagram = base_path.with_file_name("diagram.txt");
    let diagram = diagram.to_string_lossy();
    write_mermaid(&diagram, &entries);
    for (latest, ext) in [(&txt, "txt"), (&csvp, "csv"), (&jsonl, "jsonl")] {
        std::fs::copy(latest, archived(ext)).unwrap();
    }
    println!("Wrote {}\n      {}\n      {}\n      {}\n      {}", txt, csvp, jsonl, js, diagram);
    println!("Archived this session as {}", archived("jsonl").display());

    // Shareable one-file dashboard of this session on the Desktop (viewer/bundle.py).
    let bundle = dir.join("..").join("viewer").join("bundle.py");
    if bundle.exists() {
        match std::process::Command::new("python3").arg(&bundle).status() {
            Ok(s) if s.success() => {}
            _ => println!("Couldn't build the shareable dashboard; run: python3 {}", bundle.display()),
        }
    }
    Ok(())
}
