//! Parsing and serializing archives.
//!
//! Parsing is lossless: serializing an unmodified archive reproduces the input byte for byte.
//! That holds because extents are kept in file order with their trailing padding, each extent
//! remembers its offset modulo 32 so alignment survives relayout, relocation tables are written
//! sorted (every vanilla file already is), and the original string table is reused when the
//! symbol tables are unchanged.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::Result;
use crate::error::Error;

/// Size of the fixed header that precedes the data section.
pub const HEADER_SIZE: u32 = 0x20;

/// Value that terminates an external reference chain.
pub const CHAIN_END: u32 = 0xFFFF_FFFF;

/// Alignment that texture data needs in memory. Archives load at 32-byte aligned addresses, so a
/// data offset modulo 32 is the memory alignment of that struct.
pub const ALIGN: u32 = 32;

/// A pointer field inside an extent, targeting the start of another extent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pointer {
    /// Offset of the 4-byte field within the extent.
    pub field: u32,
    /// Index of the target extent.
    pub target: usize,
}

/// A run of bytes between two pointer targets. Pointer and chain fields are zero in `data`; the
/// serializer fills them in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Extent {
    pub data: Vec<u8>,
    /// Data offset modulo 32 this extent must be placed at.
    pub phase: u32,
    pub pointers: Vec<Pointer>,
    /// Data offset this extent had in the archive it was parsed from. Informational only.
    pub origin: u32,
}

impl Extent {
    pub fn len(&self) -> u32 {
        self.data.len() as u32
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

/// An exported symbol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Root {
    pub name: String,
    pub extent: usize,
}

/// An imported symbol and the pointer fields that should receive its address, in chain order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternRef {
    pub name: String,
    /// `(extent, field)` pairs.
    pub locations: Vec<(usize, u32)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct StringTable {
    bytes: Vec<u8>,
    root_offsets: Vec<u32>,
    ref_offsets: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Archive {
    pub version: [u8; 4],
    /// Bytes 0x18..0x20 of the header, preserved verbatim.
    pub header_tail: [u8; 8],
    pub extents: Vec<Extent>,
    pub roots: Vec<Root>,
    pub refs: Vec<ExternRef>,
    strings: Option<StringTable>,
}

fn be32(bytes: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn need(bytes: &[u8], needed: usize) -> Result<()> {
    if bytes.len() < needed {
        return Err(Error::Truncated {
            needed,
            have: bytes.len(),
        });
    }
    Ok(())
}

fn read_cstr(table: &[u8], at: u32) -> Result<String> {
    let start = at as usize;
    need(table, start)?;
    let end = table[start..]
        .iter()
        .position(|&b| b == 0)
        .map(|p| start + p)
        .unwrap_or(table.len());
    Ok(String::from_utf8_lossy(&table[start..end]).into_owned())
}

impl Default for Archive {
    /// An archive with no data, roots, or references.
    fn default() -> Self {
        Archive {
            version: [0; 4],
            header_tail: [0; 8],
            extents: Vec::new(),
            roots: Vec::new(),
            refs: Vec::new(),
            strings: None,
        }
    }
}

impl Archive {
    /// Parse an archive from its bytes.
    pub fn parse(bytes: &[u8]) -> Result<Archive> {
        need(bytes, HEADER_SIZE as usize)?;
        let file_size = be32(bytes, 0);
        let data_size = be32(bytes, 4);
        let reloc_count = be32(bytes, 8) as usize;
        let root_count = be32(bytes, 12) as usize;
        let ref_count = be32(bytes, 16) as usize;
        let mut version = [0u8; 4];
        version.copy_from_slice(&bytes[0x14..0x18]);
        let mut header_tail = [0u8; 8];
        header_tail.copy_from_slice(&bytes[0x18..0x20]);

        if file_size as usize != bytes.len() {
            return Err(Error::SizeMismatch {
                header: file_size,
                actual: bytes.len(),
            });
        }

        let data_start = HEADER_SIZE as usize;
        let data_end = data_start + data_size as usize;
        let reloc_off = data_end;
        let roots_off = reloc_off + reloc_count * 4;
        let refs_off = roots_off + root_count * 8;
        let strings_off = refs_off + ref_count * 8;
        need(bytes, strings_off)?;

        let data = &bytes[data_start..data_end];
        let strings = &bytes[strings_off..];

        // Relocation table: every pointer field, and the extent starts they imply.
        let mut fields = Vec::with_capacity(reloc_count);
        let mut boundaries = BTreeSet::new();
        boundaries.insert(0);
        boundaries.insert(data_size);
        for i in 0..reloc_count {
            let field = be32(bytes, reloc_off + i * 4);
            if field as usize + 4 > data.len() {
                return Err(Error::PointerOutOfRange { field, target: 0 });
            }
            let target = be32(data, field as usize);
            if target > data_size {
                return Err(Error::PointerOutOfRange { field, target });
            }
            boundaries.insert(target);
            fields.push((field, target));
        }

        // Roots.
        let mut root_offsets = Vec::with_capacity(root_count);
        let mut root_names = Vec::with_capacity(root_count);
        let mut root_targets = Vec::with_capacity(root_count);
        for i in 0..root_count {
            let target = be32(bytes, roots_off + i * 8);
            let str_off = be32(bytes, roots_off + i * 8 + 4);
            let name = read_cstr(strings, str_off)?;
            if target > data_size {
                return Err(Error::RootOutOfRange { name, target });
            }
            boundaries.insert(target);
            root_offsets.push(str_off);
            root_names.push(name);
            root_targets.push(target);
        }

        // External references and their chains.
        let mut ref_offsets = Vec::with_capacity(ref_count);
        let mut ref_names = Vec::with_capacity(ref_count);
        let mut ref_chains = Vec::with_capacity(ref_count);
        for i in 0..ref_count {
            let head = be32(bytes, refs_off + i * 8);
            let str_off = be32(bytes, refs_off + i * 8 + 4);
            let name = read_cstr(strings, str_off)?;
            let mut chain = Vec::new();
            let mut at = head;
            loop {
                if at as usize + 4 > data.len() || chain.len() > data.len() / 4 {
                    return Err(Error::BadChain { name, at });
                }
                chain.push(at);
                let next = be32(data, at as usize);
                if next == CHAIN_END {
                    break;
                }
                at = next;
            }
            ref_offsets.push(str_off);
            ref_names.push(name);
            ref_chains.push(chain);
        }

        // Cut the data section into extents.
        // A pointer or root may target the very end of the data section: an empty struct. Keep it
        // as a zero-length extent so the pointer survives.
        let end_targeted = fields.iter().any(|&(_, t)| t == data_size) || root_targets.iter().any(|&t| t == data_size);
        let starts: Vec<u32> = boundaries
            .iter()
            .copied()
            .filter(|&b| b < data_size || (end_targeted && b == data_size))
            .collect();
        let mut index_of = BTreeMap::new();
        let mut extents = Vec::with_capacity(starts.len());
        for (i, &start) in starts.iter().enumerate() {
            let end = starts.get(i + 1).copied().unwrap_or(data_size);
            index_of.insert(start, i);
            extents.push(Extent {
                data: data[start as usize..end as usize].to_vec(),
                phase: start % ALIGN,
                pointers: Vec::new(),
                origin: start,
            });
        }

        // Locate the extent containing a field and check the field fits inside it.
        let lens: Vec<usize> = extents.iter().map(|e| e.data.len()).collect();
        let locate = |field: u32| -> Result<(usize, u32)> {
            let (&start, &idx) = index_of.range(..=field).next_back().expect("boundary at 0");
            let within = field - start;
            if within as usize + 4 > lens[idx] {
                return Err(Error::FieldStraddlesBoundary { field });
            }
            Ok((idx, within))
        };

        for (field, target) in fields {
            let (idx, within) = locate(field)?;
            let target_idx = match index_of.get(&target) {
                Some(&t) => t,
                None => return Err(Error::PointerOutOfRange { field, target }),
            };
            extents[idx].data[within as usize..within as usize + 4].fill(0);
            extents[idx].pointers.push(Pointer {
                field: within,
                target: target_idx,
            });
        }

        let roots = root_names
            .into_iter()
            .zip(root_targets)
            .map(|(name, target)| Root {
                name,
                extent: index_of[&target],
            })
            .collect();

        let mut refs = Vec::with_capacity(ref_count);
        for (name, chain) in ref_names.into_iter().zip(ref_chains) {
            let mut locations = Vec::with_capacity(chain.len());
            for at in chain {
                let (idx, within) = locate(at)?;
                extents[idx].data[within as usize..within as usize + 4].fill(0);
                locations.push((idx, within));
            }
            refs.push(ExternRef { name, locations });
        }

        Ok(Archive {
            version,
            header_tail,
            extents,
            roots,
            refs,
            strings: Some(StringTable {
                bytes: strings.to_vec(),
                root_offsets,
                ref_offsets,
            }),
        })
    }

    /// Serialize the archive. Layout is deterministic: extents in order, each placed at the next
    /// offset with its recorded alignment phase.
    pub fn serialize(&self) -> Result<Vec<u8>> {
        // Lay out extents.
        let mut offsets = Vec::with_capacity(self.extents.len());
        let mut cursor: u32 = 0;
        for extent in &self.extents {
            cursor += (extent.phase + ALIGN - cursor % ALIGN) % ALIGN;
            offsets.push(cursor);
            cursor += extent.len();
        }
        let data_size = cursor;

        // Data section with pointers and chains filled in.
        let mut data = vec![0u8; data_size as usize];
        let mut relocs = Vec::new();
        for (i, extent) in self.extents.iter().enumerate() {
            let base = offsets[i] as usize;
            data[base..base + extent.data.len()].copy_from_slice(&extent.data);
            for p in &extent.pointers {
                let at = base + p.field as usize;
                data[at..at + 4].copy_from_slice(&offsets[p.target].to_be_bytes());
                relocs.push(at as u32);
            }
        }
        relocs.sort_unstable();

        let mut ref_heads = Vec::with_capacity(self.refs.len());
        for r in &self.refs {
            if r.locations.is_empty() {
                return Err(Error::EmptyRef { name: r.name.clone() });
            }
            let place = |(extent, field): (usize, u32)| offsets[extent] + field;
            for (n, &loc) in r.locations.iter().enumerate() {
                let value = r.locations.get(n + 1).map(|&next| place(next)).unwrap_or(CHAIN_END);
                let at = place(loc) as usize;
                data[at..at + 4].copy_from_slice(&value.to_be_bytes());
            }
            ref_heads.push(place(r.locations[0]));
        }

        // String table: reuse the original when the symbols are unchanged.
        let (strings, root_str, ref_str) = match self.reusable_strings() {
            Some(t) => (t.bytes.clone(), t.root_offsets.clone(), t.ref_offsets.clone()),
            None => {
                let mut bytes = Vec::new();
                let mut root_str = Vec::new();
                let mut ref_str = Vec::new();
                for name in self.roots.iter().map(|r| &r.name) {
                    root_str.push(bytes.len() as u32);
                    bytes.extend_from_slice(name.as_bytes());
                    bytes.push(0);
                }
                for name in self.refs.iter().map(|r| &r.name) {
                    ref_str.push(bytes.len() as u32);
                    bytes.extend_from_slice(name.as_bytes());
                    bytes.push(0);
                }
                (bytes, root_str, ref_str)
            },
        };

        let file_size =
            HEADER_SIZE as usize + data.len() + relocs.len() * 4 + (self.roots.len() + self.refs.len()) * 8 + strings.len();
        let mut out = Vec::with_capacity(file_size);
        out.extend_from_slice(&(file_size as u32).to_be_bytes());
        out.extend_from_slice(&data_size.to_be_bytes());
        out.extend_from_slice(&(relocs.len() as u32).to_be_bytes());
        out.extend_from_slice(&(self.roots.len() as u32).to_be_bytes());
        out.extend_from_slice(&(self.refs.len() as u32).to_be_bytes());
        out.extend_from_slice(&self.version);
        out.extend_from_slice(&self.header_tail);
        out.extend_from_slice(&data);
        for r in relocs {
            out.extend_from_slice(&r.to_be_bytes());
        }
        for (root, str_off) in self.roots.iter().zip(root_str) {
            out.extend_from_slice(&offsets[root.extent].to_be_bytes());
            out.extend_from_slice(&str_off.to_be_bytes());
        }
        for (head, str_off) in ref_heads.iter().zip(ref_str) {
            out.extend_from_slice(&head.to_be_bytes());
            out.extend_from_slice(&str_off.to_be_bytes());
        }
        out.extend_from_slice(&strings);
        Ok(out)
    }

    fn reusable_strings(&self) -> Option<&StringTable> {
        let t = self.strings.as_ref()?;
        if t.root_offsets.len() != self.roots.len() || t.ref_offsets.len() != self.refs.len() {
            return None;
        }
        let same = |offsets: &[u32], names: &mut dyn Iterator<Item = &String>| {
            offsets
                .iter()
                .zip(names)
                .all(|(&o, n)| read_cstr(&t.bytes, o).map(|s| &s == n).unwrap_or(false))
        };
        if same(&t.root_offsets, &mut self.roots.iter().map(|r| &r.name))
            && same(&t.ref_offsets, &mut self.refs.iter().map(|r| &r.name))
        {
            Some(t)
        } else {
            None
        }
    }

    /// Forget the original string table so the next serialize rebuilds it.
    pub fn invalidate_strings(&mut self) {
        self.strings = None;
    }

    /// Index of the extent that started at `data_offset` in the file this archive was parsed from.
    pub fn extent_at_origin(&self, data_offset: u32) -> Option<usize> {
        self.extents.iter().position(|e| e.origin == data_offset)
    }

    /// Data offsets the extents would be serialized at.
    pub fn layout(&self) -> Vec<u32> {
        let mut offsets = Vec::with_capacity(self.extents.len());
        let mut cursor: u32 = 0;
        for extent in &self.extents {
            cursor += (extent.phase + ALIGN - cursor % ALIGN) % ALIGN;
            offsets.push(cursor);
            cursor += extent.len();
        }
        offsets
    }

    /// Extents reachable from `starts` through pointers, in breadth-first order. `cut` may veto a
    /// pointer; it receives the extent holding the pointer and the pointer itself.
    pub fn reachable(&self, starts: &[usize], mut cut: impl FnMut(usize, &Pointer) -> bool) -> Vec<usize> {
        let mut seen = vec![false; self.extents.len()];
        let mut order = Vec::new();
        let mut queue: VecDeque<usize> = VecDeque::new();
        for &s in starts {
            if !seen[s] {
                seen[s] = true;
                queue.push_back(s);
            }
        }
        while let Some(idx) = queue.pop_front() {
            order.push(idx);
            for p in &self.extents[idx].pointers {
                if cut(idx, p) || seen[p.target] {
                    continue;
                }
                seen[p.target] = true;
                queue.push_back(p.target);
            }
        }
        order
    }

    /// Extents reachable from any root.
    pub fn reachable_from_roots(&self) -> Vec<usize> {
        let starts: Vec<usize> = self.roots.iter().map(|r| r.extent).collect();
        self.reachable(&starts, |_, _| false)
    }

    /// Drop the given extents and renumber everything else. Fails if a root, a remaining pointer,
    /// or a reference location still needs one of them.
    pub fn remove_extents(&mut self, remove: &[usize]) -> Result<()> {
        let mut gone = vec![false; self.extents.len()];
        for &i in remove {
            if i >= self.extents.len() {
                return Err(Error::NoSuchExtent { extent: i });
            }
            gone[i] = true;
        }
        let mut new_index = vec![usize::MAX; self.extents.len()];
        let mut next = 0;
        for (i, &g) in gone.iter().enumerate() {
            if !g {
                new_index[i] = next;
                next += 1;
            }
        }
        for root in &self.roots {
            if gone[root.extent] {
                return Err(Error::CannotLinkRoot { extent: root.extent });
            }
        }
        for r in &self.refs {
            if r.locations.iter().any(|&(e, _)| gone[e]) {
                return Err(Error::RefInsideLinkedSubtree { name: r.name.clone() });
            }
        }
        for (i, extent) in self.extents.iter().enumerate() {
            if gone[i] {
                continue;
            }
            if let Some(p) = extent.pointers.iter().find(|p| gone[p.target]) {
                return Err(Error::PointerOutOfRange {
                    field: p.field,
                    target: self.extents[p.target].origin,
                });
            }
        }

        let mut kept = Vec::with_capacity(next);
        for (i, mut extent) in std::mem::take(&mut self.extents).into_iter().enumerate() {
            if gone[i] {
                continue;
            }
            for p in &mut extent.pointers {
                p.target = new_index[p.target];
            }
            kept.push(extent);
        }
        self.extents = kept;
        for root in &mut self.roots {
            root.extent = new_index[root.extent];
        }
        for r in &mut self.refs {
            for loc in &mut r.locations {
                loc.0 = new_index[loc.0];
            }
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A tiny archive: root -> A (two pointers) -> B (buffer) and C (buffer, shared by A twice).
    pub(crate) fn sample() -> Archive {
        Archive {
            version: *b"001B",
            header_tail: [0; 8],
            extents: vec![
                Extent {
                    data: vec![0; 16],
                    phase: 0,
                    pointers: vec![
                        Pointer { field: 0, target: 1 },
                        Pointer { field: 4, target: 2 },
                        Pointer { field: 8, target: 2 },
                    ],
                    origin: 0,
                },
                Extent {
                    data: (1..=40).collect(),
                    phase: 16,
                    pointers: vec![],
                    origin: 16,
                },
                Extent {
                    data: vec![7; 64],
                    phase: 0,
                    pointers: vec![],
                    origin: 64,
                },
            ],
            roots: vec![Root {
                name: "sample_root".into(),
                extent: 0,
            }],
            refs: vec![],
            strings: None,
        }
    }

    #[test]
    fn serialize_then_parse_is_identity() {
        let a = sample();
        let bytes = a.serialize().unwrap();
        let b = Archive::parse(&bytes).unwrap();
        assert_eq!(b.serialize().unwrap(), bytes);
        assert_eq!(b.extents.len(), 3);
        assert_eq!(b.extents[0].pointers.len(), 3);
        assert_eq!(b.roots[0].name, "sample_root");
        // alignment phases survive
        let offsets = b.layout();
        assert_eq!(offsets[1] % ALIGN, 16);
        assert_eq!(offsets[2] % ALIGN, 0);
    }

    #[test]
    fn extern_chain_round_trips() {
        let mut a = sample();
        // Turn both pointers to C into an external reference with a two-location chain.
        a.extents[0].pointers.retain(|p| p.target != 2);
        a.refs.push(ExternRef {
            name: "ext_sym".into(),
            locations: vec![(0, 4), (0, 8)],
        });
        a.remove_extents(&[2]).unwrap();
        let bytes = a.serialize().unwrap();
        let b = Archive::parse(&bytes).unwrap();
        assert_eq!(b.refs.len(), 1);
        assert_eq!(b.refs[0].locations, vec![(0, 4), (0, 8)]);
        assert_eq!(b.serialize().unwrap(), bytes);
        // chain fields are not relocation entries
        assert_eq!(u32::from_be_bytes(bytes[8..12].try_into().unwrap()), 1);
    }
}
