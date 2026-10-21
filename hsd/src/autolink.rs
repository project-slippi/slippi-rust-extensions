//! Automatic linking: replace every part of an archive that is an exact copy of something in a
//! disc archive with a link to it, largest subtree first.
//!
//! Matching is by structural hash, so a copy is recognized regardless of where it sits in either
//! file or how it is padded. The target is walked from its roots; a struct whose whole subtree
//! exists in a disc file becomes one link and is not descended into, otherwise its children are
//! examined in turn. That way a copied joint set becomes a single link, while a Slippi-built tree
//! that merely contains vanilla textures gets one link per texture.

use std::collections::{BTreeMap, HashMap, VecDeque};

use crate::Result;
use crate::archive::Archive;
use crate::hash::Hasher;
use crate::link::{link, locate};

/// Where a hash was seen in the disc archives.
#[derive(Clone, Copy, Debug)]
struct Candidate {
    file: usize,
    extent: usize,
}

/// Hashes of every reachable struct in a set of disc archives.
#[derive(Default)]
pub struct SourceIndex {
    pub files: Vec<String>,
    by_hash: HashMap<[u8; 20], Vec<Candidate>>,
}

impl SourceIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Index every struct reachable from a root of `archive`, known by `name`.
    pub fn add(&mut self, name: &str, archive: &Archive) {
        let file = self.files.len();
        self.files.push(name.to_string());
        let mut hasher = Hasher::new(archive);
        for extent in archive.reachable_from_roots() {
            if trimmed_len(&archive.extents[extent].data) == 0 && archive.extents[extent].pointers.is_empty() {
                continue; // all-zero buffers match everything and mean nothing
            }
            let h = hasher.extent(extent);
            self.by_hash.entry(h).or_default().push(Candidate { file, extent });
        }
    }

    pub fn len(&self) -> usize {
        self.by_hash.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_hash.is_empty()
    }
}

#[derive(Clone, Debug, Default)]
pub struct Options {
    /// Smallest subtree, in trimmed bytes, worth linking. Below this, matches are likely
    /// coincidental and each link is a runtime dependency for nothing.
    pub min_bytes: usize,
    /// Disc files to use in preference to others when an asset exists in several.
    pub prefer: Vec<String>,
    /// If not empty, the only disc files links may point into. Small structs match bytes in
    /// unrelated files by coincidence; restricting sources keeps the file from depending on,
    /// say, a stage file that users commonly replace.
    pub only: Vec<String>,
    /// Pointer-free buffers at least this large that matched nothing are listed for review.
    pub report_unmatched_from: usize,
}

impl Options {
    pub fn defaults() -> Self {
        Options {
            min_bytes: 64,
            prefer: Vec::new(),
            only: Vec::new(),
            report_unmatched_from: 256,
        }
    }
}

#[derive(Clone, Debug)]
pub struct LinkEntry {
    pub symbol: String,
    /// Where the asset sat in the target, as a path from its root.
    pub target: String,
    pub extents: usize,
    pub bytes: usize,
}

#[derive(Clone, Debug)]
pub struct Unmatched {
    pub target: String,
    pub bytes: usize,
}

#[derive(Clone, Debug, Default)]
pub struct Report {
    pub links: Vec<LinkEntry>,
    /// Disc files used and how many links point into each.
    pub sources: Vec<(String, usize)>,
    pub extents_linked: usize,
    pub bytes_linked: usize,
    /// Trimmed bytes reachable from the target's roots before linking.
    pub bytes_total: usize,
    /// Pointer-free buffers that matched no disc archive, largest first.
    pub unmatched: Vec<Unmatched>,
}

fn trimmed_len(data: &[u8]) -> usize {
    data.iter().rposition(|&b| b != 0).map(|p| p + 1).unwrap_or(0)
}

fn subtree_bytes(archive: &Archive, extent: usize) -> (usize, usize) {
    let closure = archive.reachable(&[extent], |_, _| false);
    let bytes = closure.iter().map(|&i| trimmed_len(&archive.extents[i].data)).sum();
    (closure.len(), bytes)
}

/// Whether every struct reachable from `extent` is reachable only through it. A subtree that
/// shares a struct with the rest of the archive cannot be linked whole: resolving would copy the
/// shared struct, and anything that identifies it by address, such as a skinned mesh naming its
/// bones, would then point at a duplicate instead of the loaded one.
fn is_owned(archive: &Archive, roots: &[usize], extent: usize) -> bool {
    let closure = archive.reachable(&[extent], |_, _| false);
    let without = archive.reachable(roots, |_, p| p.target == extent);
    let mut outside = vec![false; archive.extents.len()];
    for i in without {
        outside[i] = true;
    }
    closure.iter().all(|&i| !outside[i])
}

/// Path of `extent` from the nearest root of `archive`, for reports.
pub fn target_path(archive: &Archive, extent: usize) -> String {
    locate(archive, "", extent, &[]).anchor.text()
}

/// Link everything in `target` that is a copy of an indexed disc asset. `load` reads a disc file
/// by name when its structure is needed to build a locator.
pub fn autolink(
    target: &Archive,
    index: &SourceIndex,
    load: &mut dyn FnMut(&str) -> std::result::Result<Vec<u8>, String>,
    options: &Options,
) -> Result<(Archive, Report)> {
    // Plan: walk from the roots, largest matching subtree first.
    let mut hasher = Hasher::new(target);
    let mut visited = vec![false; target.extents.len()];
    let mut is_root = vec![false; target.extents.len()];
    let mut queue = VecDeque::new();
    for r in &target.roots {
        is_root[r.extent] = true;
        if !visited[r.extent] {
            visited[r.extent] = true;
            queue.push_back(r.extent);
        }
    }
    struct Planned {
        origin: u32,
        candidates: Vec<Candidate>,
        extents: usize,
        bytes: usize,
        chosen: Option<usize>,
    }
    let mut plan: Vec<Planned> = Vec::new();
    let root_extents: Vec<usize> = target.roots.iter().map(|r| r.extent).collect();
    // Disc archives are parsed on demand, while planning and while linking.
    let mut sources: BTreeMap<usize, Archive> = BTreeMap::new();
    let mut load_source = |file: usize, sources: &mut BTreeMap<usize, Archive>| -> Result<()> {
        if !sources.contains_key(&file) {
            let name = &index.files[file];
            let bytes = load(name).map_err(|reason| crate::Error::Source {
                file: name.clone(),
                reason,
            })?;
            sources.insert(file, Archive::parse(&bytes)?);
        }
        Ok(())
    };
    while let Some(idx) = queue.pop_front() {
        let mut descend = true;
        if !is_root[idx] {
            if let Some(cands) = index.by_hash.get(&hasher.extent(idx)) {
                let (extents, bytes) = subtree_bytes(target, idx);
                if bytes >= options.min_bytes && is_owned(target, &root_extents, idx) {
                    // The structural hash cannot tell one shared struct from two identical
                    // copies, so a candidate must also have the same number of structs, or
                    // resolving it would add (or merge) structs.
                    let mut same_shape = Vec::new();
                    let allowed = cands
                        .iter()
                        .filter(|c| options.only.is_empty() || options.only.contains(&index.files[c.file]));
                    for c in allowed {
                        load_source(c.file, &mut sources)?;
                        if sources[&c.file].reachable(&[c.extent], |_, _| false).len() == extents {
                            same_shape.push(*c);
                        }
                    }
                    if same_shape.is_empty() {
                        descend = true;
                    } else {
                        plan.push(Planned {
                            origin: target.extents[idx].origin,
                            candidates: same_shape,
                            extents,
                            bytes,
                            chosen: None,
                        });
                        descend = false;
                    }
                }
            }
        }
        if descend {
            let mut pointers: Vec<_> = target.extents[idx].pointers.iter().collect();
            pointers.sort_by_key(|p| p.field);
            for p in pointers {
                if !visited[p.target] {
                    visited[p.target] = true;
                    queue.push_back(p.target);
                }
            }
        }
    }

    // Choose disc files: preferred ones first, then whichever covers the most remaining links.
    for entry in &mut plan {
        for pref in &options.prefer {
            if let Some(c) = entry.candidates.iter().find(|c| &index.files[c.file] == pref) {
                entry.chosen = Some(c.file);
                break;
            }
        }
    }
    loop {
        let mut coverage: BTreeMap<usize, usize> = BTreeMap::new();
        for entry in plan.iter().filter(|e| e.chosen.is_none()) {
            let mut seen = Vec::new();
            for c in &entry.candidates {
                if !seen.contains(&c.file) {
                    seen.push(c.file);
                    *coverage.entry(c.file).or_default() += 1;
                }
            }
        }
        let Some((&file, _)) = coverage
            .iter()
            .max_by(|a, b| a.1.cmp(b.1).then_with(|| index.files[*b.0].cmp(&index.files[*a.0])))
        else {
            break;
        };
        for entry in plan.iter_mut().filter(|e| e.chosen.is_none()) {
            if entry.candidates.iter().any(|c| c.file == file) {
                entry.chosen = Some(file);
            }
        }
    }

    // Apply the links largest closure first. If a planned subtree holds a pointer to another
    // planned asset, its closure strictly contains that asset's closure, so this order guarantees
    // the container is linked (and its structs removed) before the inner asset turns pointers
    // inside it into chain locations.
    plan.sort_by(|a, b| b.extents.cmp(&a.extents).then_with(|| a.origin.cmp(&b.origin)));

    // Build symbols from the chosen files, then apply the links.
    let mut out = target.clone();
    let mut report = Report {
        bytes_total: target
            .reachable_from_roots()
            .iter()
            .map(|&i| trimmed_len(&target.extents[i].data))
            .sum(),
        ..Report::default()
    };
    let mut per_source: BTreeMap<String, usize> = BTreeMap::new();
    for entry in &plan {
        let file = entry.chosen.expect("every planned link has a chosen file");
        let Some(idx) = out.extent_at_origin(entry.origin) else {
            continue; // already taken out as part of an earlier link
        };
        // One symbol resolves to one struct, so two distinct structs of the target must not
        // share a symbol or resolving would merge them. Use the first candidate whose symbol is
        // still free, the chosen file first; if every candidate is taken, keep the struct.
        let mut ordered: Vec<Candidate> = entry.candidates.clone();
        ordered.sort_by_key(|c| (c.file != file, c.file, c.extent));
        let mut picked = None;
        for c in ordered {
            load_source(c.file, &mut sources)?;
            let symbol = locate(&sources[&c.file], &index.files[c.file], c.extent, &[]).symbol();
            if !out.refs.iter().any(|r| r.name == symbol) {
                picked = Some((c.file, symbol));
                break;
            }
        }
        let Some((file, symbol)) = picked else {
            continue;
        };
        let name = &index.files[file];
        let path = target_path(target, target.extent_at_origin(entry.origin).expect("planned extent exists"));
        let before = out.extents.len();
        link(&mut out, idx, &symbol)?;
        let removed = before - out.extents.len();
        report.links.push(LinkEntry {
            symbol,
            target: path,
            extents: entry.extents,
            bytes: entry.bytes,
        });
        report.extents_linked += removed;
        report.bytes_linked += entry.bytes;
        *per_source.entry(name.clone()).or_default() += 1;
    }
    report.sources = per_source.into_iter().collect();
    report.sources.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    // What is left that looks like it could have been an asset: sizeable pointer-free buffers.
    for i in target.reachable_from_roots() {
        let e = &target.extents[i];
        let n = trimmed_len(&e.data);
        if !e.pointers.is_empty() || n < options.report_unmatched_from || out.extent_at_origin(e.origin).is_none() {
            continue;
        }
        report.unmatched.push(Unmatched {
            target: target_path(target, i),
            bytes: n,
        });
    }
    report
        .unmatched
        .sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.target.cmp(&b.target)));

    Ok((out, report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::{Extent, Pointer, Root};
    use crate::hash;
    use crate::link::resolve;

    /// Disc file: sample(). Target: a Slippi root that points at a copy of the disc's whole tree,
    /// a separate copy of one of its buffers, and a Slippi-only buffer.
    fn fixture() -> (Archive, Archive) {
        let disc = crate::archive::tests::sample();
        let mut target = disc.clone();
        target.roots = vec![Root {
            name: "slp_root".into(),
            extent: 3,
        }];
        // extent 3: Slippi's own struct -> 0 (copied tree), 4 (copied buffer), 5 (Slippi buffer)
        target.extents.push(Extent {
            data: vec![0; 32],
            phase: 0,
            pointers: vec![
                Pointer { field: 0, target: 0 },
                Pointer { field: 4, target: 4 },
                Pointer { field: 8, target: 5 },
            ],
            origin: 96,
        });
        target.extents.push(Extent {
            data: (1..=40).collect(),
            phase: 0,
            pointers: vec![],
            origin: 128,
        });
        target.extents.push(Extent {
            data: vec![0x55; 300],
            phase: 0,
            pointers: vec![],
            origin: 192,
        });
        (disc, target)
    }

    #[test]
    fn links_largest_subtrees_and_reports_the_rest() {
        let (disc, target) = fixture();
        let disc_bytes = disc.serialize().unwrap();
        let mut index = SourceIndex::new();
        index.add("Sample.dat", &disc);
        let mut load = |name: &str| {
            if name == "Sample.dat" {
                Ok(disc_bytes.clone())
            } else {
                Err("no".to_string())
            }
        };
        let mut options = Options::defaults();
        options.min_bytes = 16;
        let (linked, report) = autolink(&target, &index, &mut load, &options).unwrap();

        assert_eq!(report.links.len(), 2, "{:?}", report.links);
        assert!(
            report
                .links
                .iter()
                .any(|l| l.symbol.starts_with("slp:Sample.dat:sample_root/:") && l.target == "slp_root/0")
        );
        assert!(
            report
                .links
                .iter()
                .any(|l| l.symbol.starts_with("slp:Sample.dat:sample_root/0:") && l.target == "slp_root/4")
        );
        assert_eq!(report.sources, vec![("Sample.dat".to_string(), 2)]);
        assert_eq!(report.unmatched.len(), 1);
        assert_eq!(report.unmatched[0].target, "slp_root/8");
        assert_eq!(report.unmatched[0].bytes, 300);
        assert_eq!(linked.extents.len(), 2, "only the Slippi struct and buffer remain");

        let bytes = linked.serialize().unwrap();
        assert!(!bytes.windows(64).any(|w| w == [7u8; 64]));
        let resolved = resolve(&Archive::parse(&bytes).unwrap(), &mut load).unwrap();
        assert!(resolved.warnings.is_empty());
        assert!(hash::same_structure(&target, &resolved.archive));
    }

    /// A skinned mesh names a bone that is also a joint of the tree. The display object's closure
    /// matches the disc byte for byte but must not be linked whole, because resolving it would
    /// copy the bone and the mesh would point at the copy. The bone itself is linkable, since
    /// every pointer to it becomes part of the same link.
    #[test]
    fn shared_structs_keep_a_subtree_from_being_linked_whole() {
        // R (joint) -> child J (bone, leaf) and -> D (display object) -> P (mesh) -> E (envelope) -> J
        let node = |data: Vec<u8>, pointers: Vec<Pointer>, origin: u32| Extent {
            data,
            phase: 0,
            pointers,
            origin,
        };
        let mut disc = Archive::default();
        disc.extents = vec![
            node(
                vec![0; 64],
                vec![Pointer { field: 8, target: 1 }, Pointer { field: 16, target: 2 }],
                0,
            ),
            node((1..=64).collect(), vec![], 64),
            node(vec![0; 16], vec![Pointer { field: 12, target: 3 }], 128),
            node(vec![0; 24], vec![Pointer { field: 20, target: 4 }], 160),
            node(vec![0; 16], vec![Pointer { field: 0, target: 1 }], 192),
        ];
        disc.roots = vec![Root {
            name: "disc_root".into(),
            extent: 0,
        }];
        let mut target = disc.clone();
        target.extents[0].data[1] = 0xAA; // Slippi edited the joint, so its closure no longer matches
        target
            .extents
            .push(node(vec![0; 32], vec![Pointer { field: 0, target: 0 }], 256));
        target.roots = vec![Root {
            name: "slp_root".into(),
            extent: 5,
        }];
        let disc_bytes = disc.serialize().unwrap();
        let mut index = SourceIndex::new();
        index.add("Sample.dat", &disc);
        let mut load = |_: &str| Ok(disc_bytes.clone());
        let mut options = Options::defaults();
        options.min_bytes = 16;
        let (linked, report) = autolink(&target, &index, &mut load, &options).unwrap();

        let targets: Vec<&str> = report.links.iter().map(|l| l.target.as_str()).collect();
        assert_eq!(
            targets,
            vec!["slp_root/0/8"],
            "only the bone is linked; the display object shares it: {:?}",
            report.links
        );
        assert_eq!(
            linked.refs[0].locations.len(),
            2,
            "both the joint and the envelope reach the bone through the link"
        );
        let resolved = resolve(&Archive::parse(&linked.serialize().unwrap()).unwrap(), &mut load).unwrap();
        assert!(hash::same_structure(&target, &resolved.archive));
        assert_eq!(
            resolved.archive.extents.len(),
            target.extents.len(),
            "no struct was duplicated"
        );
    }

    /// Two distinct but identical copies of a disc buffer must not end up behind one symbol,
    /// which would resolve to a single shared struct. One is linked; the other stays local.
    #[test]
    fn identical_copies_are_not_merged_by_one_symbol() {
        let (disc, mut target) = fixture();
        let copy = target.extents[4].clone();
        target.extents.push(Extent { origin: 512, ..copy });
        target.extents[3].pointers.push(Pointer { field: 12, target: 6 });

        let mut index = SourceIndex::new();
        index.add("Sample.dat", &disc);
        let disc_bytes = disc.serialize().unwrap();
        let mut load = |_: &str| Ok(disc_bytes.clone());
        let mut options = Options::defaults();
        options.min_bytes = 16;
        let (linked, report) = autolink(&target, &index, &mut load, &options).unwrap();

        let mut symbols: Vec<&str> = linked.refs.iter().map(|r| r.name.as_str()).collect();
        symbols.sort();
        symbols.dedup();
        assert_eq!(symbols.len(), linked.refs.len(), "no symbol is used twice");
        assert_eq!(report.links.len(), linked.refs.len());

        let resolved = crate::link::resolve(&linked, &mut load).unwrap();
        assert_eq!(
            resolved.archive.reachable_from_roots().len(),
            target.reachable_from_roots().len(),
            "resolving neither merged nor duplicated a struct"
        );
        assert!(crate::hash::same_structure(&target, &resolved.archive));
    }

    #[test]
    fn prefers_the_named_source() {
        let (disc, target) = fixture();
        let mut index = SourceIndex::new();
        index.add("A.dat", &disc);
        index.add("B.dat", &disc);
        let disc_bytes = disc.serialize().unwrap();
        let mut load = |_: &str| Ok(disc_bytes.clone());
        let mut options = Options::defaults();
        options.min_bytes = 16;
        let (_, report) = autolink(&target, &index, &mut load, &options).unwrap();
        assert!(
            report.links.iter().all(|l| l.symbol.starts_with("slp:A.dat:")),
            "lexicographic tie-break"
        );
        options.prefer = vec!["B.dat".into()];
        let (_, report) = autolink(&target, &index, &mut load, &options).unwrap();
        assert!(
            report.links.iter().all(|l| l.symbol.starts_with("slp:B.dat:")),
            "preferred file wins"
        );
    }
}
