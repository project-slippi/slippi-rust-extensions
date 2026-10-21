//! Creating and resolving Slippi asset links.
//!
//! A link is an external symbol of the form
//!
//! ```text
//! slp:<disc file>:<anchor>[:at=0x<file offset>][:cut=<field>[,<field>...]][:sha=<8 hex>][:strict=1]
//! ```
//!
//! The anchor names the asset's struct in the disc file, either as a path
//! `<root symbol>/<field>/<field>...`, the pointer fields to follow from that root, written in hex
//! with `*n` for a repeated hop (so `MnSelectChrDataTable/68/0/4*6/8`), or as an absolute file
//! offset `0x<hex>`, as HSDRawViewer shows it. Paths are preferred: they survive a re-saved or
//! modified disc file as long as its structure is unchanged, so a user's edited texture is picked
//! up instead of breaking the link. `at` gives an offset to try when a path does not resolve.
//!
//! The asset is that struct plus everything it points to. `cut` names pointer fields of the first
//! struct which are not followed, so a joint can be linked without its siblings. `sha` records the
//! first eight hex digits of the asset's structural hash; a mismatch is reported as a warning and
//! the disc's version is used, unless `strict=1` makes it an error.

use std::collections::{BTreeMap, VecDeque};

use crate::archive::{Archive, Extent, ExternRef, HEADER_SIZE, Pointer};
use crate::error::Error;
use crate::hash;
use crate::{LINK_PREFIX, Result};

/// How a locator names the asset's struct in the disc file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Anchor {
    /// Absolute file offset of the struct.
    Offset(u32),
    /// A root symbol and the pointer fields to follow from it.
    Path { root: String, hops: Vec<u32> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Locator {
    pub file: String,
    pub anchor: Anchor,
    /// Absolute file offset tried when a path anchor does not resolve.
    pub at: Option<u32>,
    pub cut: Vec<u32>,
    pub sha: Option<String>,
    /// A hash mismatch is an error rather than a warning.
    pub strict: bool,
}

/// How a link's asset was found on the disc.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkStatus {
    /// Found by its anchor and matching the recorded hash.
    Exact,
    /// Found by its anchor, but the disc's asset differs from the recorded one (a modded disc).
    Modified,
    /// The path did not resolve; found at the fallback offset, confirmed by the hash.
    Fallback,
    /// Found by its anchor; the link records no hash to check against.
    Unchecked,
}

impl LinkStatus {
    pub fn name(self) -> &'static str {
        match self {
            LinkStatus::Exact => "exact",
            LinkStatus::Modified => "modified",
            LinkStatus::Fallback => "fallback",
            LinkStatus::Unchecked => "unchecked",
        }
    }
}

/// One link as it was resolved.
#[derive(Clone, Debug)]
pub struct ResolvedLink {
    pub symbol: String,
    pub locator: Locator,
    pub status: LinkStatus,
    /// Extents of the resolved archive copied in for this link, the linked struct first.
    pub extents: Vec<usize>,
}

/// The outcome of resolving an archive.
#[derive(Clone, Debug, Default)]
pub struct Resolution {
    pub archive: Archive,
    /// Links that resolved to something other than the recorded asset, and other notices.
    pub warnings: Vec<String>,
    /// Disc files that were read, in order.
    pub sources: Vec<String>,
    /// Every `slp:` link, in the order of the input's reference table.
    pub links: Vec<ResolvedLink>,
}

fn parse_hex(symbol: &str, text: &str) -> Result<u32> {
    let digits = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")).unwrap_or(text);
    u32::from_str_radix(digits, 16).map_err(|_| Error::BadLocator {
        symbol: symbol.into(),
        reason: format!("bad number {text}"),
    })
}

impl Locator {
    pub fn is_link(symbol: &str) -> bool {
        symbol.starts_with(LINK_PREFIX)
    }

    pub fn parse(symbol: &str) -> Result<Locator> {
        let bad = |reason: &str| Error::BadLocator {
            symbol: symbol.into(),
            reason: reason.into(),
        };
        let body = symbol.strip_prefix(LINK_PREFIX).ok_or_else(|| bad("missing slp: prefix"))?;
        let mut parts = body.split(':');
        let file = parts
            .next()
            .filter(|f| !f.is_empty())
            .ok_or_else(|| bad("missing file name"))?
            .to_string();
        let anchor_text = parts.next().ok_or_else(|| bad("missing anchor"))?;
        let anchor = if let Some((root, hops)) = anchor_text.split_once('/') {
            if root.is_empty() {
                return Err(bad("missing root symbol"));
            }
            let mut fields = Vec::new();
            for hop in hops.split('/').filter(|h| !h.is_empty()) {
                let (field, count) = match hop.split_once('*') {
                    Some((f, n)) => (f, n.parse::<usize>().map_err(|_| bad(&format!("bad repeat count {n}")))?),
                    None => (hop, 1),
                };
                let field = parse_hex(symbol, field)?;
                fields.extend(std::iter::repeat_n(field, count));
            }
            Anchor::Path {
                root: root.to_string(),
                hops: fields,
            }
        } else {
            let offset = parse_hex(symbol, anchor_text)?;
            if offset < HEADER_SIZE {
                return Err(bad("offset is inside the header"));
            }
            Anchor::Offset(offset)
        };
        let mut locator = Locator {
            file,
            anchor,
            at: None,
            cut: Vec::new(),
            sha: None,
            strict: false,
        };
        for part in parts {
            match part.split_once('=') {
                Some(("at", offset)) => {
                    let offset = parse_hex(symbol, offset)?;
                    if offset < HEADER_SIZE {
                        return Err(bad("at offset is inside the header"));
                    }
                    locator.at = Some(offset);
                },
                Some(("cut", list)) => {
                    for item in list.split(',') {
                        locator.cut.push(parse_hex(symbol, item)?);
                    }
                },
                Some(("sha", digest)) if digest.len() == 8 && digest.chars().all(|c| c.is_ascii_hexdigit()) => {
                    locator.sha = Some(digest.to_ascii_lowercase());
                },
                Some(("strict", "1")) => locator.strict = true,
                _ => return Err(bad(&format!("unknown option {part}"))),
            }
        }
        Ok(locator)
    }

    pub fn symbol(&self) -> String {
        let anchor = self.anchor.text();
        let mut s = format!("{LINK_PREFIX}{}:{anchor}", self.file);
        if let Some(at) = self.at {
            s.push_str(&format!(":at=0x{at:x}"));
        }
        if !self.cut.is_empty() {
            let cuts: Vec<String> = self.cut.iter().map(|c| format!("{c:x}")).collect();
            s.push_str(&format!(":cut={}", cuts.join(",")));
        }
        if let Some(sha) = &self.sha {
            s.push_str(&format!(":sha={sha}"));
        }
        if self.strict {
            s.push_str(":strict=1");
        }
        s
    }
}

impl Anchor {
    /// The anchor as written in a symbol: `0x<offset>` or `<root>/<hops>`.
    pub fn text(&self) -> String {
        match self {
            Anchor::Offset(offset) => format!("0x{offset:x}"),
            Anchor::Path { root, hops } => {
                let mut text = format!("{root}/");
                let mut i = 0;
                while i < hops.len() {
                    let mut n = 1;
                    while i + n < hops.len() && hops[i + n] == hops[i] {
                        n += 1;
                    }
                    if i > 0 {
                        text.push('/');
                    }
                    text.push_str(&format!("{:x}", hops[i]));
                    if n > 1 {
                        text.push_str(&format!("*{n}"));
                    }
                    i += n;
                }
                text
            },
        }
    }
}

/// Short structural hash of an asset in `archive`, as recorded in a locator's `sha` option.
pub fn asset_sha(archive: &Archive, extent: usize, cut: &[u32]) -> String {
    // Hash the subtree with the cut pointers removed, without touching the archive.
    let mut view = archive.clone();
    view.extents[extent].pointers.retain(|p| !cut.contains(&p.field));
    hash::hex(&hash::extent_hash(&view, extent))[..8].to_string()
}

/// Build a locator for `extent` of `archive`: a path from the nearest root when one exists,
/// otherwise the file offset, with the offset as fallback and the hash recorded. `file` is the
/// disc file name to record.
pub fn locate(archive: &Archive, file: &str, extent: usize, cut: &[u32]) -> Locator {
    let offset = archive.extents[extent].origin + HEADER_SIZE;
    // Breadth-first search from every root, remembering how each extent was reached.
    let mut parent: Vec<Option<(usize, u32)>> = vec![None; archive.extents.len()];
    let mut root_of: Vec<Option<usize>> = vec![None; archive.extents.len()];
    let mut queue = VecDeque::new();
    for (ri, r) in archive.roots.iter().enumerate() {
        if root_of[r.extent].is_none() {
            root_of[r.extent] = Some(ri);
            queue.push_back(r.extent);
        }
    }
    while let Some(idx) = queue.pop_front() {
        if idx == extent {
            break;
        }
        let mut pointers: Vec<&Pointer> = archive.extents[idx].pointers.iter().collect();
        pointers.sort_by_key(|p| p.field);
        for p in pointers {
            if root_of[p.target].is_none() {
                root_of[p.target] = root_of[idx];
                parent[p.target] = Some((idx, p.field));
                queue.push_back(p.target);
            }
        }
    }
    let anchor = match root_of[extent] {
        Some(ri) => {
            let mut hops = Vec::new();
            let mut cur = extent;
            while let Some((from, field)) = parent[cur] {
                hops.push(field);
                cur = from;
            }
            hops.reverse();
            Anchor::Path {
                root: archive.roots[ri].name.clone(),
                hops,
            }
        },
        None => Anchor::Offset(offset),
    };
    Locator {
        file: file.to_string(),
        anchor,
        at: Some(offset),
        cut: cut.to_vec(),
        sha: Some(asset_sha(archive, extent, cut)),
        strict: false,
    }
}

/// Follow a path anchor through `archive`.
fn follow(archive: &Archive, root: &str, hops: &[u32]) -> std::result::Result<usize, String> {
    let mut cur = archive
        .roots
        .iter()
        .find(|r| r.name == root)
        .map(|r| r.extent)
        .ok_or_else(|| format!("no root symbol {root}"))?;
    for (n, hop) in hops.iter().enumerate() {
        cur = archive.extents[cur]
            .pointers
            .iter()
            .find(|p| p.field == *hop)
            .map(|p| p.target)
            .ok_or_else(|| format!("hop {n} (field 0x{hop:x}) is not a pointer"))?;
    }
    Ok(cur)
}

/// Find the extent a locator names in its (already loaded) disc file.
/// Returns the extent and whether the fallback offset had to be used.
fn find_asset(src: &Archive, locator: &Locator, symbol: &str, warnings: &mut Vec<String>) -> Result<(usize, bool)> {
    let file = &locator.file;
    let by_offset = |offset: u32| {
        src.extent_at_origin(offset.wrapping_sub(HEADER_SIZE))
            .ok_or_else(|| Error::NotAStruct {
                file: file.clone(),
                offset,
            })
    };
    match &locator.anchor {
        Anchor::Offset(offset) => by_offset(*offset).map(|e| (e, false)),
        Anchor::Path { root, hops } => match follow(src, root, hops) {
            Ok(extent) => Ok((extent, false)),
            Err(reason) => {
                let bad_path = || Error::BadPath {
                    symbol: symbol.to_string(),
                    reason: reason.clone(),
                };
                let Some(at) = locator.at else {
                    return Err(bad_path());
                };
                // The fallback offset is only trusted when the recorded hash confirms it.
                let extent = by_offset(at).map_err(|_| bad_path())?;
                if let Some(expected) = &locator.sha
                    && asset_sha(src, extent, &locator.cut) != *expected
                {
                    return Err(bad_path());
                }
                warnings.push(format!("{symbol}: path did not resolve ({reason}); used the fallback offset"));
                Ok((extent, true))
            },
        },
    }
}

/// Replace `extent` and everything only reachable through it with an external reference named
/// `symbol`. Every pointer that targeted `extent` becomes a location of that reference.
pub fn link(archive: &mut Archive, extent: usize, symbol: &str) -> Result<()> {
    if extent >= archive.extents.len() {
        return Err(Error::NoSuchExtent { extent });
    }
    if archive.roots.iter().any(|r| r.extent == extent) {
        return Err(Error::CannotLinkRoot { extent });
    }
    let before = archive.reachable_from_roots();

    let mut locations = Vec::new();
    for (i, e) in archive.extents.iter_mut().enumerate() {
        e.pointers.retain(|p| {
            if p.target == extent {
                locations.push((i, p.field));
                false
            } else {
                true
            }
        });
    }
    if locations.is_empty() {
        return Err(Error::Unreferenced { extent });
    }
    match archive.refs.iter_mut().find(|r| r.name == symbol) {
        Some(existing) => existing.locations.extend(locations),
        None => archive.refs.push(ExternRef {
            name: symbol.to_string(),
            locations,
        }),
    }

    let after = archive.reachable_from_roots();
    let mut still = vec![false; archive.extents.len()];
    for i in after {
        still[i] = true;
    }
    let removed: Vec<usize> = before.into_iter().filter(|&i| !still[i]).collect();
    archive.remove_extents(&removed)?;
    archive.invalidate_strings();
    Ok(())
}

/// Resolve every `slp:` reference by copying the linked assets in from disc files supplied by
/// `load`, which receives a disc file name and returns its bytes. Other references are kept.
pub fn resolve(archive: &Archive, load: &mut dyn FnMut(&str) -> std::result::Result<Vec<u8>, String>) -> Result<Resolution> {
    let mut out = archive.clone();
    out.refs.clear();
    let mut sources: BTreeMap<String, Archive> = BTreeMap::new();
    let mut resolution = Resolution::default();
    let mut resolved_any = false;

    for r in &archive.refs {
        if !Locator::is_link(&r.name) {
            out.refs.push(r.clone());
            continue;
        }
        let locator = Locator::parse(&r.name)?;
        if !sources.contains_key(&locator.file) {
            let bytes = load(&locator.file).map_err(|reason| Error::Source {
                file: locator.file.clone(),
                reason,
            })?;
            let parsed = Archive::parse(&bytes).map_err(|e| Error::Source {
                file: locator.file.clone(),
                reason: e.to_string(),
            })?;
            sources.insert(locator.file.clone(), parsed);
            resolution.sources.push(locator.file.clone());
        }
        let src = &sources[&locator.file];
        let (root, fallback) = find_asset(src, &locator, &r.name, &mut resolution.warnings)?;

        let mut status = match (&locator.sha, fallback) {
            (_, true) => LinkStatus::Fallback,
            (Some(_), false) => LinkStatus::Exact,
            (None, false) => LinkStatus::Unchecked,
        };
        if let Some(expected) = &locator.sha {
            let found = asset_sha(src, root, &locator.cut);
            if &found != expected {
                status = LinkStatus::Modified;
                if locator.strict {
                    return Err(Error::HashMismatch {
                        symbol: r.name.clone(),
                        expected: expected.clone(),
                        found,
                    });
                }
                resolution.warnings.push(format!(
                    "{}: the asset on the disc differs from the recorded one ({expected} vs {found}); using the disc's version",
                    r.name
                ));
            }
        }

        let closure = src.reachable(&[root], |e, p| e == root && locator.cut.contains(&p.field));
        let mut ordered = closure.clone();
        ordered.sort_by_key(|&i| src.extents[i].origin);
        let mut new_index = BTreeMap::new();
        for &i in &ordered {
            new_index.insert(i, out.extents.len());
            out.extents.push(Extent {
                data: src.extents[i].data.clone(),
                phase: src.extents[i].phase,
                pointers: Vec::new(),
                origin: src.extents[i].origin,
            });
        }
        for &i in &ordered {
            let pointers = src.extents[i]
                .pointers
                .iter()
                .filter(|p| !(i == root && locator.cut.contains(&p.field)))
                .filter_map(|p| {
                    new_index.get(&p.target).map(|&t| Pointer {
                        field: p.field,
                        target: t,
                    })
                })
                .collect();
            out.extents[new_index[&i]].pointers = pointers;
        }
        let target = new_index[&root];
        for &(extent, field) in &r.locations {
            out.extents[extent].pointers.push(Pointer { field, target });
        }
        let mut extents = vec![target];
        extents.extend(new_index.values().copied().filter(|&e| e != target));
        resolution.links.push(ResolvedLink {
            symbol: r.name.clone(),
            locator,
            status,
            extents,
        });
        resolved_any = true;
    }

    if resolved_any {
        out.invalidate_strings();
    }
    resolution.archive = out;
    Ok(resolution)
}

/// Whether two linked archives are the same file in every way that matters: the same external
/// references, and the same structure once resolved, struct for struct. Byte layout and padding
/// are ignored. Returns `None` when they match, or what differs.
///
/// Tools use this to leave a file untouched when a save would only have moved bytes around.
pub fn difference(
    a: &Archive,
    b: &Archive,
    load: &mut dyn FnMut(&str) -> std::result::Result<Vec<u8>, String>,
) -> Result<Option<String>> {
    let names = |x: &Archive| {
        let mut n: Vec<String> = x.refs.iter().map(|r| r.name.clone()).collect();
        n.sort();
        n
    };
    let (na, nb) = (names(a), names(b));
    if na != nb {
        let only_a = na.iter().filter(|n| !nb.contains(n)).count();
        let only_b = nb.iter().filter(|n| !na.contains(n)).count();
        return Ok(Some(format!(
            "links differ ({only_a} only in the first, {only_b} only in the second)"
        )));
    }
    let (ra, rb) = (resolve(a, load)?.archive, resolve(b, load)?.archive);
    let (ca, cb) = (ra.reachable_from_roots().len(), rb.reachable_from_roots().len());
    if ca != cb {
        return Ok(Some(format!("struct counts differ ({ca} vs {cb})")));
    }
    if !hash::same_structure(&ra, &rb) {
        return Ok(Some("content differs".into()));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disc_loader(bytes: &[u8]) -> impl FnMut(&str) -> std::result::Result<Vec<u8>, String> + '_ {
        move |name| {
            if name == "Sample.dat" {
                Ok(bytes.to_vec())
            } else {
                Err("no such file".into())
            }
        }
    }

    #[test]
    fn locator_round_trip() {
        let l = Locator::parse("slp:GmTou1p.dat:0x1a2b0:cut=c,4:sha=deadbeef").unwrap();
        assert_eq!(l.file, "GmTou1p.dat");
        assert_eq!(l.anchor, Anchor::Offset(0x1a2b0));
        assert_eq!(l.cut, vec![0xc, 0x4]);
        assert_eq!(l.sha.as_deref(), Some("deadbeef"));
        assert_eq!(l.symbol(), "slp:GmTou1p.dat:0x1a2b0:cut=c,4:sha=deadbeef");

        let p = Locator::parse("slp:MnSlChr.usd:MnSelectChrDataTable/68/0/4*3/8:at=0x1234:sha=0badf00d:strict=1").unwrap();
        assert_eq!(
            p.anchor,
            Anchor::Path {
                root: "MnSelectChrDataTable".into(),
                hops: vec![0x68, 0, 4, 4, 4, 8]
            }
        );
        assert_eq!(p.at, Some(0x1234));
        assert!(p.strict);
        assert_eq!(
            p.symbol(),
            "slp:MnSlChr.usd:MnSelectChrDataTable/68/0/4*3/8:at=0x1234:sha=0badf00d:strict=1"
        );
        assert_eq!(
            Locator::parse("slp:GmTitle.usd:ScTitle_fog_foganim/").unwrap().anchor,
            Anchor::Path {
                root: "ScTitle_fog_foganim".into(),
                hops: vec![]
            }
        );

        assert!(Locator::parse("slp:GmTou1p.dat").is_err());
        assert!(Locator::parse("slp:GmTou1p.dat:0x10").is_err());
        assert!(Locator::parse("slp:GmTou1p.dat:0x40:bogus=1").is_err());
        assert!(Locator::parse("slp:GmTou1p.dat:/4").is_err());
        assert!(!Locator::is_link("ScTitle_fog_foganim"));
    }

    #[test]
    fn locate_prefers_a_path() {
        let a = crate::archive::tests::sample();
        let l = locate(&a, "Sample.dat", 2, &[]);
        assert_eq!(
            l.anchor,
            Anchor::Path {
                root: "sample_root".into(),
                hops: vec![4]
            }
        );
        assert_eq!(l.at, Some(HEADER_SIZE + 64));
        assert_eq!(l.sha.as_deref(), Some(asset_sha(&a, 2, &[]).as_str()));
        assert_eq!(follow(&a, "sample_root", &[4]).unwrap(), 2);
        assert_eq!(follow(&a, "sample_root", &[8]).unwrap(), 2);
        assert!(follow(&a, "sample_root", &[0xc]).is_err());
    }

    #[test]
    fn difference_ignores_layout_but_not_content_or_links() {
        let original = crate::archive::tests::sample();
        let disc = original.serialize().unwrap();
        let symbol = locate(&original, "Sample.dat", 2, &[]).symbol();
        let mut linked = original.clone();
        link(&mut linked, 2, &symbol).unwrap();

        // the same file with extra padding on its first struct
        let mut padded = linked.clone();
        padded.extents[0].data.extend_from_slice(&[0; 32]);
        assert_eq!(difference(&linked, &padded, &mut disc_loader(&disc)).unwrap(), None);

        let mut edited = linked.clone();
        edited.extents[0].data[0] ^= 0xFF;
        assert!(difference(&linked, &edited, &mut disc_loader(&disc)).unwrap().is_some());

        assert!(
            difference(&linked, &original, &mut disc_loader(&disc)).unwrap().is_some(),
            "links differ"
        );
    }

    #[test]
    fn link_then_resolve_restores_structure() {
        let original = crate::archive::tests::sample();
        let disc = original.serialize().unwrap();
        let mut linked = original.clone();
        let symbol = locate(&original, "Sample.dat", 2, &[]).symbol();
        link(&mut linked, 2, &symbol).unwrap();
        assert_eq!(linked.extents.len(), 2, "the linked buffer is gone");
        assert_eq!(linked.refs.len(), 1);
        assert_eq!(
            linked.refs[0].locations.len(),
            2,
            "both pointers to it became chain locations"
        );
        let bytes = linked.serialize().unwrap();
        assert!(!bytes.windows(64).any(|w| w == [7u8; 64]), "linked bytes are not in the file");

        let reparsed = Archive::parse(&bytes).unwrap();
        let resolved = resolve(&reparsed, &mut disc_loader(&disc)).unwrap();
        assert!(resolved.warnings.is_empty(), "{:?}", resolved.warnings);
        assert_eq!(resolved.sources, vec!["Sample.dat"]);
        assert!(resolved.archive.refs.is_empty());
        assert_eq!(resolved.links.len(), 1);
        assert_eq!(resolved.links[0].status, LinkStatus::Exact);
        assert_eq!(resolved.links[0].symbol, symbol);
        let copied = &resolved.links[0].extents;
        assert_eq!(copied.len(), 1);
        assert_eq!(resolved.archive.extents[copied[0]].data, original.extents[2].data);
        assert!(hash::same_structure(&original, &resolved.archive));
        let out = resolved.archive.serialize().unwrap();
        assert!(hash::same_structure(&original, &Archive::parse(&out).unwrap()));
    }

    /// A re-saved disc file with a different layout and an edited buffer: the path still finds
    /// the asset, the edit is picked up, and the hash mismatch is a warning.
    #[test]
    fn modified_source_is_followed_by_path() {
        let original = crate::archive::tests::sample();
        let mut modded = original.clone();
        modded.extents.swap(1, 2);
        for p in &mut modded.extents[0].pointers {
            p.target = match p.target {
                1 => 2,
                2 => 1,
                t => t,
            };
        }
        modded.extents[0].data.extend_from_slice(&[0; 32]); // shifts every later offset
        modded.extents[1].data[3] = 9; // the linked buffer, now at index 1, is edited
        let modded_disc = modded.serialize().unwrap();

        let mut linked = original.clone();
        let symbol = locate(&original, "Sample.dat", 2, &[]).symbol();
        link(&mut linked, 2, &symbol).unwrap();

        let resolved = resolve(&linked, &mut disc_loader(&modded_disc)).unwrap();
        assert_eq!(resolved.warnings.len(), 1, "{:?}", resolved.warnings);
        assert!(resolved.warnings[0].contains("differs"));
        assert_eq!(resolved.links[0].status, LinkStatus::Modified);
        let asset = follow(&resolved.archive, "sample_root", &[4]).unwrap();
        assert_eq!(
            resolved.archive.extents[asset].data[3], 9,
            "the user's edit is what got resolved"
        );
        assert!(!hash::same_structure(&original, &resolved.archive));

        // strict links refuse instead
        let mut strict = original.clone();
        let mut l = locate(&original, "Sample.dat", 2, &[]);
        l.strict = true;
        link(&mut strict, 2, &l.symbol()).unwrap();
        let err = resolve(&strict, &mut disc_loader(&modded_disc)).unwrap_err();
        assert!(matches!(err, Error::HashMismatch { .. }), "{err}");

        // an offset anchor alone lands on the wrong struct after the relayout
        let mut by_offset = original.clone();
        link(&mut by_offset, 2, "slp:Sample.dat:0x60:strict=1:sha=00000000").unwrap();
        assert!(resolve(&by_offset, &mut disc_loader(&modded_disc)).is_err());
    }

    #[test]
    fn fallback_offset_needs_a_matching_hash() {
        let original = crate::archive::tests::sample();
        let disc = original.serialize().unwrap();
        let sha = asset_sha(&original, 2, &[]);

        let mut ok = original.clone();
        link(&mut ok, 2, &format!("slp:Sample.dat:no_such_root/4:at=0x60:sha={sha}")).unwrap();
        let resolved = resolve(&ok, &mut disc_loader(&disc)).unwrap();
        assert_eq!(resolved.warnings.len(), 1);
        assert!(resolved.warnings[0].contains("fallback"));
        assert_eq!(resolved.links[0].status, LinkStatus::Fallback);
        assert!(hash::same_structure(&original, &resolved.archive));

        let mut bad = original.clone();
        link(&mut bad, 2, "slp:Sample.dat:no_such_root/4:at=0x60:sha=00000000").unwrap();
        let err = resolve(&bad, &mut disc_loader(&disc)).unwrap_err();
        assert!(matches!(err, Error::BadPath { .. }), "{err}");

        let mut none = original.clone();
        link(&mut none, 2, "slp:Sample.dat:no_such_root/4").unwrap();
        assert!(matches!(
            resolve(&none, &mut disc_loader(&disc)).unwrap_err(),
            Error::BadPath { .. }
        ));
    }

    #[test]
    fn resolve_rejects_missing_file_and_bad_offset() {
        let original = crate::archive::tests::sample();
        let disc = original.serialize().unwrap();
        let mut linked = original.clone();
        link(&mut linked, 2, "slp:Sample.dat:0x60").unwrap();
        let err = resolve(&linked, &mut |_| Err("gone".into())).unwrap_err();
        assert!(matches!(err, Error::Source { .. }), "{err}");
        let mut bad = original.clone();
        link(&mut bad, 2, "slp:Sample.dat:0x61").unwrap();
        let err = resolve(&bad, &mut disc_loader(&disc)).unwrap_err();
        assert!(matches!(err, Error::NotAStruct { .. }), "{err}");
    }

    #[test]
    fn cut_stops_at_siblings() {
        // A -> B, B.next -> C. Link B with its next pointer cut: C must not come along.
        let mut a = crate::archive::tests::sample();
        a.extents[1].pointers.push(Pointer { field: 4, target: 2 });
        a.extents[0].pointers.retain(|p| p.target != 2); // A no longer points at C directly
        let disc = a.serialize().unwrap();
        let mut linked = a.clone();
        link(&mut linked, 1, "slp:Sample.dat:sample_root/0:cut=4").unwrap();
        assert_eq!(linked.extents.len(), 1, "B and C both left the file");
        let resolved = resolve(&linked, &mut disc_loader(&disc)).unwrap().archive;
        assert_eq!(resolved.extents.len(), 2, "only B came back");
        assert!(resolved.extents[1].pointers.is_empty(), "the cut pointer is null");
    }
}
