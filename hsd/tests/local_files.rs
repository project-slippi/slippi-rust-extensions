//! Tests against real game files. They need two environment variables and are skipped, with a
//! message, when either is missing:
//!
//!   SLIPPI_ROOT_DISC   a folder holding the files of a Melee disc (extracted root)
//!   SLIPPI_GAMEFILES   a folder holding Slippi's shipped game files (Sys/GameFiles/GALE01)

use std::fs;
use std::path::PathBuf;

use slippi_hsd::archive::HEADER_SIZE;
use slippi_hsd::{Anchor, Archive, AutolinkOptions, SourceIndex, autolink, hash, link, locate, resolve};

fn env_dir(name: &str) -> Option<PathBuf> {
    let p = PathBuf::from(std::env::var_os(name)?);
    if p.is_dir() {
        Some(p)
    } else {
        eprintln!("{name} is set but {} is not a directory", p.display());
        None
    }
}

fn read(dir: &PathBuf, name: &str) -> Vec<u8> {
    fs::read(dir.join(name)).unwrap_or_else(|e| panic!("{}: {e}", dir.join(name).display()))
}

/// The shipped copy of a game file may already carry links. Resolve it against the disc so the
/// tests always start from the full form, and say whether that was necessary.
fn full_copy(gamefiles: &PathBuf, disc: &PathBuf, name: &str) -> (Archive, bool) {
    let parsed = Archive::parse(&read(gamefiles, name)).unwrap();
    if parsed.refs.is_empty() {
        return (parsed, false);
    }
    let resolved = resolve(&parsed, &mut |n| fs::read(disc.join(n)).map_err(|e| e.to_string())).unwrap();
    (resolved.archive, true)
}

#[test]
fn every_disc_archive_round_trips_byte_for_byte() {
    let Some(disc) = env_dir("SLIPPI_ROOT_DISC") else {
        eprintln!("SLIPPI_ROOT_DISC not set, skipping");
        return;
    };
    let mut checked = 0;
    let mut failures = Vec::new();
    for entry in fs::read_dir(&disc).unwrap().flatten() {
        let path = entry.path();
        let is_archive = path
            .extension()
            .map(|x| x.eq_ignore_ascii_case("dat") || x.eq_ignore_ascii_case("usd"))
            .unwrap_or(false);
        if !is_archive {
            continue;
        }
        let bytes = fs::read(&path).unwrap();
        let archive = match Archive::parse(&bytes) {
            Ok(a) => a,
            // Pl*AJ.dat files are containers of many concatenated archives, not one archive.
            Err(slippi_hsd::Error::SizeMismatch { header, actual }) if (header as usize) < actual => continue,
            Err(e) => {
                failures.push(format!("{}: parse: {e}", path.display()));
                continue;
            },
        };
        match archive.serialize() {
            Ok(out) if out == bytes => checked += 1,
            Ok(_) => failures.push(format!("{}: not byte-identical", path.display())),
            Err(e) => failures.push(format!("{}: serialize: {e}", path.display())),
        }
    }
    assert!(failures.is_empty(), "{} failures:\n{}", failures.len(), failures.join("\n"));
    assert!(checked > 800, "only {checked} archives checked");
}

/// Convert one texture buffer of GameSetup_gui.dat into a link, resolve it from the disc, and
/// check the result is structurally identical to the shipped file.
#[test]
fn link_one_buffer_and_resolve() {
    let (Some(disc), Some(gamefiles)) = (env_dir("SLIPPI_ROOT_DISC"), env_dir("SLIPPI_GAMEFILES")) else {
        eprintln!("SLIPPI_ROOT_DISC or SLIPPI_GAMEFILES not set, skipping");
        return;
    };
    let (shipped, _) = full_copy(&gamefiles, &disc, "GameSetup_gui.dat");
    let source = Archive::parse(&read(&disc, "GmTou1p.usd")).unwrap();

    // Find a pointer-free extent of at least 1 KB whose bytes also occur in the source, and whose
    // middle 64 bytes are distinctive enough to search for.
    let trimmed = |d: &[u8]| d.iter().rposition(|&b| b != 0).map(|p| p + 1).unwrap_or(0);
    let distinctive = |w: &[u8]| {
        let mut seen = [false; 256];
        w.iter().for_each(|&b| seen[b as usize] = true);
        seen.iter().filter(|&&s| s).count() >= 12
    };
    let mut found = None;
    for (i, e) in shipped.extents.iter().enumerate() {
        let n = trimmed(&e.data);
        if !e.pointers.is_empty() || n < 1024 || !distinctive(&e.data[n / 2..n / 2 + 64]) {
            continue;
        }
        if let Some((j, _)) = source
            .extents
            .iter()
            .enumerate()
            .find(|(_, s)| s.pointers.is_empty() && trimmed(&s.data) == n && s.data[..n] == e.data[..n])
        {
            found = Some((i, j, n));
            break;
        }
    }
    let (target_extent, source_extent, n) = found.expect("a shared buffer between GameSetup_gui.dat and GmTou1p.usd");
    let payload = shipped.extents[target_extent].data[..n].to_vec();
    let sample = payload[n / 2..n / 2 + 64].to_vec();
    let copies_before = shipped
        .extents
        .iter()
        .filter(|e| e.data.len() >= n && e.data[..n] == payload[..])
        .count();

    let locator = locate(&source, "GmTou1p.usd", source_extent, &[]);
    assert!(
        matches!(locator.anchor, Anchor::Path { .. }),
        "a texture buffer is reachable from the root"
    );
    eprintln!(
        "linking {copies_before} copies of a {n}-byte buffer (shipped file offset 0x{:x}) to {}",
        shipped.extents[target_extent].origin + HEADER_SIZE,
        locator.symbol()
    );
    // Every identical copy links to the same symbol; link() merges them into one reference.
    let mut linked = shipped.clone();
    while let Some(i) = linked
        .extents
        .iter()
        .position(|e| e.data.len() >= n && e.data[..n] == payload[..])
    {
        link(&mut linked, i, &locator.symbol()).unwrap();
    }
    let linked_bytes = linked.serialize().unwrap();
    assert!(
        !linked_bytes.windows(64).any(|w| w == sample.as_slice()),
        "the linked {n}-byte buffer must not remain in the file"
    );
    assert!(linked_bytes.len() < shipped.serialize().unwrap().len());

    let reparsed = Archive::parse(&linked_bytes).unwrap();
    assert_eq!(reparsed.refs.len(), 1);
    let resolution = resolve(&reparsed, &mut |name| fs::read(disc.join(name)).map_err(|e| e.to_string())).unwrap();
    assert!(resolution.warnings.is_empty(), "{:?}", resolution.warnings);
    let resolved = resolution.archive;
    assert!(resolved.refs.is_empty());
    assert!(
        hash::same_structure(&shipped, &resolved),
        "resolved archive must match the shipped one structurally"
    );
    let out = resolved.serialize().unwrap();
    let back = Archive::parse(&out).unwrap();
    assert!(hash::same_structure(&shipped, &back));
    assert!(
        out.windows(64).any(|w| w == sample.as_slice()),
        "the buffer is back after resolving"
    );

    // A modified disc file: re-laid out (an extent grows, shifting every later offset) with the
    // linked texture edited. The path finds it, the edit comes through, and only a warning is logged.
    let mut modded = source.clone();
    modded.extents[0].data.extend_from_slice(&[0; 32]);
    let mut edited = modded.extents[source_extent].data[..n].to_vec();
    edited[n / 2] ^= 0xFF;
    modded.extents[source_extent].data[..n].copy_from_slice(&edited);
    let modded_bytes = modded.serialize().unwrap();
    let resolution = resolve(&reparsed, &mut |_| Ok(modded_bytes.clone())).unwrap();
    assert_eq!(resolution.warnings.len(), 1, "{:?}", resolution.warnings);
    let out = resolution.archive.serialize().unwrap();
    assert!(
        !out.windows(64).any(|w| w == sample.as_slice()),
        "the vanilla bytes are not used"
    );
    assert!(
        out.windows(64).any(|w| w == &edited[n / 2..n / 2 + 64]),
        "the user's edited texture is what got resolved"
    );
}

/// Link the whole Background joint set to the tournament scene and check the resolved subtree is
/// exactly the tournament one. It differs from the shipped set only by animation loop flags, so
/// the whole-archive comparison is expected to fail while the subtree comparison passes.
#[test]
fn link_background_joint_set() {
    let (Some(disc), Some(gamefiles)) = (env_dir("SLIPPI_ROOT_DISC"), env_dir("SLIPPI_GAMEFILES")) else {
        eprintln!("SLIPPI_ROOT_DISC or SLIPPI_GAMEFILES not set, skipping");
        return;
    };
    let (shipped, was_linked) = full_copy(&gamefiles, &disc, "GameSetup_gui.dat");
    let source = Archive::parse(&read(&disc, "GmTou1p.dat")).unwrap();

    // ScGamTour_scene_data -> JOBJDescs array (field 0) -> desc N (field 4*N).
    let desc = |a: &Archive, n: u32| -> usize {
        let scene = a.roots[0].extent;
        let array = a.extents[scene].pointers.iter().find(|p| p.field == 0).unwrap().target;
        a.extents[array].pointers.iter().find(|p| p.field == 4 * n).unwrap().target
    };
    let mine = desc(&shipped, 0);
    let theirs = desc(&source, 5);

    let locator = locate(&source, "GmTou1p.dat", theirs, &[]);
    assert_eq!(
        locator.anchor,
        Anchor::Path {
            root: "ScGamTour_scene_data".into(),
            hops: vec![0, 0x14]
        }
    );
    eprintln!(
        "background: shipped desc at file offset 0x{:x}, symbol {}",
        shipped.extents[mine].origin + HEADER_SIZE,
        locator.symbol()
    );
    let mut linked = shipped.clone();
    let before = linked.extents.len();
    link(&mut linked, mine, &locator.symbol()).unwrap();
    let removed = before - linked.extents.len();
    assert!(
        removed > 100,
        "the joint set should take its display objects, materials, textures and geometry with it, removed {removed}"
    );

    let resolved = resolve(&linked, &mut |name| fs::read(disc.join(name)).map_err(|e| e.to_string()))
        .unwrap()
        .archive;
    let resolved_desc = desc(&resolved, 0);
    assert_eq!(
        hash::extent_hash(&resolved, resolved_desc),
        hash::extent_hash(&source, theirs),
        "the resolved set is the tournament set"
    );
    // The original shipped set carries loop flags the vanilla set lacks; a copy that was already
    // linked resolves to the vanilla set and so matches.
    assert_eq!(hash::same_structure(&shipped, &resolved), was_linked);
    Archive::parse(&resolved.serialize().unwrap()).unwrap();
}

/// Auto-link both shipped files against the whole disc and check they resolve back exactly.
#[test]
fn autolink_shipped_files() {
    let (Some(disc), Some(gamefiles)) = (env_dir("SLIPPI_ROOT_DISC"), env_dir("SLIPPI_GAMEFILES")) else {
        eprintln!("SLIPPI_ROOT_DISC or SLIPPI_GAMEFILES not set, skipping");
        return;
    };
    let mut index = SourceIndex::new();
    for entry in fs::read_dir(&disc).unwrap().flatten() {
        let path = entry.path();
        let is_archive = path
            .extension()
            .map(|x| x.eq_ignore_ascii_case("dat") || x.eq_ignore_ascii_case("usd"))
            .unwrap_or(false);
        if !is_archive {
            continue;
        }
        if let Ok(a) = Archive::parse(&fs::read(&path).unwrap()) {
            index.add(&path.file_name().unwrap().to_string_lossy(), &a);
        }
    }
    let mut load = |name: &str| fs::read(disc.join(name)).map_err(|e| e.to_string());
    let options = AutolinkOptions::defaults();

    for (name, min_links) in [("GameSetup_gui.dat", 100), ("SlippiCSS_gui.dat", 5)] {
        let (full, _) = full_copy(&gamefiles, &disc, name);
        let (linked, report) = autolink(&full, &index, &mut load, &options).unwrap();
        eprintln!(
            "{name}: {} links, {} of {} bytes, sources {:?}, {} unmatched buffers",
            report.links.len(),
            report.bytes_linked,
            report.bytes_total,
            report.sources,
            report.unmatched.len()
        );
        assert!(report.links.len() >= min_links, "{name}: only {} links", report.links.len());
        let bytes = linked.serialize().unwrap();
        let resolved = resolve(&Archive::parse(&bytes).unwrap(), &mut load).unwrap();
        assert!(resolved.warnings.is_empty(), "{:?}", resolved.warnings);
        assert!(
            hash::same_structure(&full, &resolved.archive),
            "{name} must resolve back to its full form"
        );
        if name == "GameSetup_gui.dat" {
            let bg = report
                .links
                .iter()
                .find(|l| l.target == "ScGamTour_scene_data/0*2")
                .expect("Background is one link");
            assert!(bg.bytes > 60_000, "the whole joint set went, {} bytes", bg.bytes);
            assert!(bg.symbol.starts_with("slp:GmTou1p.dat:"), "{}", bg.symbol);
        }
    }
}
