//! Command line front end, so tooling and CI run the same code Dolphin does.
//!
//!   slippi-hsd info <archive>
//!   slippi-hsd roundtrip <file or dir>...        parse and re-serialize, report mismatches
//!   slippi-hsd compare <a> <b>                   structural comparison
//!   slippi-hsd same <a> <b> --disc <dir>         same links and same resolved content? exit 0 if so, 1 if not
//!   slippi-hsd locate <archive> <file offset> [cut...]  link symbol for an asset (path, offset, hash)
//!   slippi-hsd link <archive> -o <out> <file offset>=<symbol>...
//!   slippi-hsd resolve <archive> --disc <dir> -o <out> [--map <json>]   --map lists each link's structs
//!   slippi-hsd autolink <archive> --disc <dir> -o <out> [--min N] [--prefer a,b] [--only a,b] [--report <file>]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use slippi_hsd::archive::HEADER_SIZE;
use slippi_hsd::{Archive, AutolinkOptions, Locator, Resolution, SourceIndex, autolink, difference, hash, link, resolve};

fn usage() -> ExitCode {
    eprintln!(
        "usage:\n  slippi-hsd info <archive>\n  slippi-hsd roundtrip <file or dir>...\n  slippi-hsd compare <a> <b>\n  slippi-hsd same <a> <b> --disc <dir>\n  slippi-hsd link <archive> -o <out> <file offset>=<symbol>...\n  slippi-hsd resolve <archive> --disc <dir> -o <out> [--map <json>]\n  slippi-hsd autolink <archive> --disc <dir> -o <out> [--min N] [--prefer a,b] [--only a,b] [--report <file>]"
    );
    ExitCode::from(2)
}

fn load(path: &Path) -> Result<Archive, String> {
    let bytes = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Archive::parse(&bytes).map_err(|e| format!("{}: {e}", path.display()))
}

fn info(path: &Path) -> Result<(), String> {
    let a = load(path)?;
    let data: usize = a.extents.iter().map(|e| e.data.len()).sum();
    let pointers: usize = a.extents.iter().map(|e| e.pointers.len()).sum();
    println!(
        "{}: {} extents, {data} data bytes, {pointers} pointers, version {:?}",
        path.display(),
        a.extents.len(),
        String::from_utf8_lossy(&a.version)
    );
    let layout = a.layout();
    for r in &a.roots {
        println!(
            "  root {} -> 0x{:x}",
            r.name,
            layout[r.extent] + slippi_hsd::archive::HEADER_SIZE
        );
    }
    for r in &a.refs {
        let tag = if Locator::is_link(&r.name) { "link" } else { "ref " };
        println!("  {tag} {} ({} locations)", r.name, r.locations.len());
    }
    println!("  structural hash {}", hash::hex(&hash::archive_hash(&a)));
    Ok(())
}

fn collect(paths: &[String]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for p in paths {
        let path = PathBuf::from(p);
        if path.is_dir() {
            if let Ok(entries) = fs::read_dir(&path) {
                let mut files: Vec<PathBuf> = entries
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| {
                        p.extension()
                            .map(|x| x.eq_ignore_ascii_case("dat") || x.eq_ignore_ascii_case("usd"))
                            .unwrap_or(false)
                    })
                    .collect();
                files.sort();
                out.extend(files);
            }
        } else {
            out.push(path);
        }
    }
    out
}

fn roundtrip(paths: &[String]) -> Result<(), String> {
    let mut ok = 0;
    let mut bad = Vec::new();
    let mut skipped = 0;
    let start = Instant::now();
    for path in collect(paths) {
        let bytes = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let a = match Archive::parse(&bytes) {
            Ok(a) => a,
            Err(e) => {
                skipped += 1;
                eprintln!("skip {}: {e}", path.display());
                continue;
            },
        };
        match a.serialize() {
            Ok(out) if out == bytes => ok += 1,
            Ok(out) => {
                let at = out
                    .iter()
                    .zip(&bytes)
                    .position(|(x, y)| x != y)
                    .unwrap_or(out.len().min(bytes.len()));
                bad.push(format!(
                    "{}: differs at 0x{at:x} (sizes {} vs {})",
                    path.display(),
                    out.len(),
                    bytes.len()
                ));
            },
            Err(e) => bad.push(format!("{}: {e}", path.display())),
        }
    }
    println!(
        "round trip: {ok} identical, {} differ, {skipped} skipped, {:.1?}",
        bad.len(),
        start.elapsed()
    );
    for b in &bad {
        println!("  {b}");
    }
    if bad.is_empty() {
        Ok(())
    } else {
        Err(format!("{} files did not round trip", bad.len()))
    }
}

fn compare(a: &Path, b: &Path) -> Result<(), String> {
    let (x, y) = (load(a)?, load(b)?);
    let (hx, hy) = (hash::archive_hash(&x), hash::archive_hash(&y));
    println!("{} {}\n{} {}", hash::hex(&hx), a.display(), hash::hex(&hy), b.display());
    if hx == hy {
        println!("structurally identical");
        Ok(())
    } else {
        Err("archives differ".into())
    }
}

fn same(a: &Path, b: &Path, disc: &Path) -> Result<(), String> {
    let (x, y) = (load(a)?, load(b)?);
    let mut read_disc = |name: &str| fs::read(disc.join(name)).map_err(|e| e.to_string());
    match difference(&x, &y, &mut read_disc).map_err(|e| e.to_string())? {
        None => {
            println!("same: identical links and resolved content");
            Ok(())
        },
        Some(reason) => Err(format!("different: {reason}")),
    }
}

fn locate_cmd(archive: &Path, offset: &str, cuts: &[String]) -> Result<(), String> {
    let a = load(archive)?;
    let parse = |t: &str| u32::from_str_radix(t.trim_start_matches("0x"), 16).map_err(|_| format!("bad number {t}"));
    let offset = parse(offset)?;
    let extent = a
        .extent_at_origin(offset.wrapping_sub(slippi_hsd::archive::HEADER_SIZE))
        .ok_or_else(|| format!("no struct starts at file offset 0x{offset:x}"))?;
    let cut: Vec<u32> = cuts.iter().map(|c| parse(c)).collect::<Result<_, _>>()?;
    let closure = a.reachable(&[extent], |e, p| e == extent && cut.contains(&p.field));
    let bytes: usize = closure.iter().map(|&i| a.extents[i].data.len()).sum();
    let file = archive
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default();
    let locator = slippi_hsd::locate(&a, &file, extent, &cut);
    println!("{} extents, {bytes} bytes, symbol {}", closure.len(), locator.symbol());
    Ok(())
}

fn do_link(archive: &Path, out: &Path, specs: &[String]) -> Result<(), String> {
    let mut a = load(archive)?;
    for spec in specs {
        let (offset, symbol) = spec
            .split_once('=')
            .ok_or_else(|| format!("bad link spec {spec}, want <file offset>=<symbol>"))?;
        let digits = offset.trim_start_matches("0x");
        let offset = u32::from_str_radix(digits, 16).map_err(|_| format!("bad offset {offset}"))?;
        let extent = a
            .extent_at_origin(offset.wrapping_sub(slippi_hsd::archive::HEADER_SIZE))
            .ok_or_else(|| format!("no struct starts at file offset 0x{offset:x}"))?;
        Locator::parse(symbol).map_err(|e| e.to_string())?;
        link(&mut a, extent, symbol).map_err(|e| e.to_string())?;
        println!("linked 0x{offset:x} -> {symbol}");
    }
    fs::write(out, a.serialize().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    println!("wrote {}", out.display());
    Ok(())
}

fn do_resolve(archive: &Path, disc: &Path, out: &Path, map: Option<PathBuf>) -> Result<(), String> {
    let a = load(archive)?;
    let start = Instant::now();
    let mut reads = Vec::new();
    let resolution = resolve(&a, &mut |name| {
        reads.push(name.to_string());
        fs::read(disc.join(name)).map_err(|e| e.to_string())
    })
    .map_err(|e| e.to_string())?;
    for warning in &resolution.warnings {
        println!("warning: {warning}");
    }
    let bytes = resolution.archive.serialize().map_err(|e| e.to_string())?;
    fs::write(out, &bytes).map_err(|e| e.to_string())?;
    if let Some(map) = map {
        let json = serde_json::to_string_pretty(&link_map(&resolution)).map_err(|e| e.to_string())?;
        fs::write(&map, json).map_err(|e| format!("{}: {e}", map.display()))?;
    }
    println!(
        "resolved {} links using {} disc file(s) {:?} in {:.1?}; wrote {} ({} bytes)",
        a.refs.iter().filter(|r| Locator::is_link(&r.name)).count(),
        reads.len(),
        reads,
        start.elapsed(),
        out.display(),
        bytes.len()
    );
    Ok(())
}

/// Where each link's structs sit in the resolved file, for tools such as HSDRawViewer.
///
/// Offsets are absolute file offsets of struct starts, the same numbers HSDRawViewer shows. The
/// first struct of a link is the one the link points at; the rest are only reachable through it.
fn link_map(resolution: &Resolution) -> serde_json::Value {
    let layout = resolution.archive.layout();
    let links: Vec<serde_json::Value> = resolution
        .links
        .iter()
        .map(|l| {
            let structs: Vec<u32> = l.extents.iter().map(|&e| HEADER_SIZE + layout[e]).collect();
            let bytes: usize = l.extents.iter().map(|&e| resolution.archive.extents[e].data.len()).sum();
            serde_json::json!({
                "symbol": l.symbol,
                "file": l.locator.file,
                "anchor": l.locator.anchor.text(),
                "status": l.status.name(),
                "offset": structs[0],
                "bytes": bytes,
                "structs": structs,
            })
        })
        .collect();
    serde_json::json!({
        "version": 1,
        "sources": resolution.sources,
        "warnings": resolution.warnings,
        "links": links,
    })
}

fn build_index(disc: &Path) -> Result<SourceIndex, String> {
    let start = Instant::now();
    let mut index = SourceIndex::new();
    let mut files = 0;
    for path in collect(&[disc.to_string_lossy().into_owned()]) {
        let bytes = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if let Ok(a) = Archive::parse(&bytes) {
            index.add(&path.file_name().unwrap().to_string_lossy(), &a);
            files += 1;
        }
    }
    println!(
        "indexed {files} disc archives, {} distinct structures, in {:.1?}",
        index.len(),
        start.elapsed()
    );
    Ok(index)
}

fn report_text(report: &slippi_hsd::Report) -> String {
    let mut t = String::new();
    t.push_str(&format!(
        "linked {} assets ({} structs, {} of {} bytes, {:.1}%) from {} disc file(s)\n",
        report.links.len(),
        report.extents_linked,
        report.bytes_linked,
        report.bytes_total,
        100.0 * report.bytes_linked as f64 / report.bytes_total.max(1) as f64,
        report.sources.len()
    ));
    for (file, n) in &report.sources {
        t.push_str(&format!("  {n:4} links -> {file}\n"));
    }
    let mut links = report.links.clone();
    links.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    t.push_str("links, largest first:\n");
    for l in &links {
        t.push_str(&format!(
            "  {:>8} B {:>4} structs  {} <- {}\n",
            l.bytes, l.extents, l.target, l.symbol
        ));
    }
    t.push_str(&format!(
        "unmatched buffers of {} bytes or more: {}\n",
        256,
        report.unmatched.len()
    ));
    for u in &report.unmatched {
        t.push_str(&format!("  {:>8} B  {}\n", u.bytes, u.target));
    }
    t
}

fn do_autolink(
    archive: &Path,
    disc: &Path,
    out: &Path,
    min: Option<usize>,
    prefer: Vec<String>,
    only: Vec<String>,
    report_path: Option<PathBuf>,
) -> Result<(), String> {
    let parsed = load(archive)?;
    let mut read_disc = |name: &str| fs::read(disc.join(name)).map_err(|e| e.to_string());
    // Start from the full form when the input already carries links.
    let full = if parsed.refs.iter().any(|r| Locator::is_link(&r.name)) {
        let r = resolve(&parsed, &mut read_disc).map_err(|e| e.to_string())?;
        println!("input had links; resolved it first ({} warning(s))", r.warnings.len());
        r.archive
    } else {
        parsed
    };
    let index = build_index(disc)?;
    let mut options = AutolinkOptions::defaults();
    if let Some(m) = min {
        options.min_bytes = m;
    }
    options.prefer = prefer;
    options.only = only;
    let start = Instant::now();
    let (linked, report) = autolink(&full, &index, &mut read_disc, &options).map_err(|e| e.to_string())?;
    let bytes = linked.serialize().map_err(|e| e.to_string())?;
    fs::write(out, &bytes).map_err(|e| e.to_string())?;
    println!(
        "autolink took {:.1?}; wrote {} ({} bytes, was {})",
        start.elapsed(),
        out.display(),
        bytes.len(),
        full.serialize().map(|b| b.len()).unwrap_or(0)
    );

    // Verify: the linked file must resolve back to the full form.
    let check = resolve(&Archive::parse(&bytes).map_err(|e| e.to_string())?, &mut read_disc).map_err(|e| e.to_string())?;
    // Structural equality cannot tell a shared struct from two identical copies, so the struct
    // count must match as well: a link that duplicated something would add structs.
    let same = hash::same_structure(&full, &check.archive) && check.archive.extents.len() == full.extents.len();
    println!(
        "verify: resolved output is {} to the input ({} structs in, {} out{})",
        if same { "structurally identical" } else { "DIFFERENT" },
        full.extents.len(),
        check.archive.extents.len(),
        if check.warnings.is_empty() {
            String::new()
        } else {
            format!(", {} warnings", check.warnings.len())
        }
    );

    let text = report_text(&report);
    match report_path {
        Some(p) => {
            fs::write(&p, &text).map_err(|e| e.to_string())?;
            println!(
                "{}",
                text.lines().take(2 + report.sources.len()).collect::<Vec<_>>().join("\n")
            );
            println!("full report written to {}", p.display());
        },
        None => print!("{text}"),
    }
    if same { Ok(()) } else { Err("verification failed".into()) }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .map(PathBuf::from)
    };
    let result = match args.first().map(String::as_str) {
        Some("info") if args.len() == 2 => info(Path::new(&args[1])),
        Some("roundtrip") if args.len() >= 2 => roundtrip(&args[1..]),
        Some("compare") if args.len() == 3 => compare(Path::new(&args[1]), Path::new(&args[2])),
        Some("same") if args.len() == 5 => match flag("--disc") {
            Some(disc) => same(Path::new(&args[1]), Path::new(&args[2]), &disc),
            None => return usage(),
        },
        Some("locate") if args.len() >= 3 => locate_cmd(Path::new(&args[1]), &args[2], &args[3..]),
        Some("link") if args.len() >= 5 => match flag("-o") {
            Some(out) => {
                let specs: Vec<String> = args[2..].iter().filter(|a| a.contains('=')).cloned().collect();
                do_link(Path::new(&args[1]), &out, &specs)
            },
            None => return usage(),
        },
        Some("resolve") if args.len() >= 6 => match (flag("--disc"), flag("-o")) {
            (Some(disc), Some(out)) => do_resolve(Path::new(&args[1]), &disc, &out, flag("--map")),
            _ => return usage(),
        },
        Some("autolink") if args.len() >= 6 => match (flag("--disc"), flag("-o")) {
            (Some(disc), Some(out)) => {
                let min = flag("--min").and_then(|m| m.to_string_lossy().parse::<usize>().ok());
                let list = |name: &str| -> Vec<String> {
                    flag(name)
                        .map(|p| {
                            p.to_string_lossy()
                                .split(',')
                                .map(|s| s.trim().to_string())
                                .filter(|s| !s.is_empty())
                                .collect()
                        })
                        .unwrap_or_default()
                };
                do_autolink(
                    Path::new(&args[1]),
                    &disc,
                    &out,
                    min,
                    list("--prefer"),
                    list("--only"),
                    flag("--report"),
                )
            },
            _ => return usage(),
        },
        _ => return usage(),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        },
    }
}
