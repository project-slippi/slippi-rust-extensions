//! Layout-independent hashing, for comparing archives structurally.
//!
//! Two archives compare equal when their root symbols lead to the same structures with the same
//! bytes, regardless of where those structures sit in the file. Pointer fields are already zero
//! in extent data, and trailing zero padding is ignored, so an archive re-saved by another tool
//! with different alignment still hashes the same.

use sha1::{Digest, Sha1};

use crate::archive::Archive;

fn trimmed(data: &[u8]) -> &[u8] {
    let end = data.iter().rposition(|&b| b != 0).map(|p| p + 1).unwrap_or(0);
    &data[..end]
}

/// Hashes of every extent's reachable structure, memoized.
pub struct Hasher<'a> {
    archive: &'a Archive,
    memo: Vec<Option<[u8; 20]>>,
    in_progress: Vec<bool>,
}

impl<'a> Hasher<'a> {
    pub fn new(archive: &'a Archive) -> Self {
        Hasher {
            archive,
            memo: vec![None; archive.extents.len()],
            in_progress: vec![false; archive.extents.len()],
        }
    }

    /// Hash of an extent and everything it points to.
    pub fn extent(&mut self, idx: usize) -> [u8; 20] {
        if let Some(h) = self.memo[idx] {
            return h;
        }
        if self.in_progress[idx] {
            return Sha1::digest(b"cycle").into();
        }
        self.in_progress[idx] = true;
        let extent = &self.archive.extents[idx];
        let mut sha = Sha1::new();
        sha.update(trimmed(&extent.data));
        let mut pointers = extent.pointers.clone();
        pointers.sort_by_key(|p| p.field);
        for p in pointers {
            sha.update(p.field.to_be_bytes());
            sha.update(self.extent(p.target));
        }
        let h: [u8; 20] = sha.finalize().into();
        self.in_progress[idx] = false;
        self.memo[idx] = Some(h);
        h
    }

    /// Hash of the whole archive as seen through its symbol tables.
    pub fn archive(&mut self) -> [u8; 20] {
        let mut sha = Sha1::new();
        for root in &self.archive.roots {
            sha.update(root.name.as_bytes());
            sha.update([0]);
            sha.update(self.extent(root.extent));
        }
        for r in &self.archive.refs {
            sha.update(r.name.as_bytes());
            sha.update([0]);
            for &(extent, field) in &r.locations {
                sha.update(self.extent(extent));
                sha.update(field.to_be_bytes());
            }
        }
        sha.finalize().into()
    }
}

/// Structural hash of an archive.
pub fn archive_hash(archive: &Archive) -> [u8; 20] {
    Hasher::new(archive).archive()
}

/// Structural hash of one extent's subtree.
pub fn extent_hash(archive: &Archive, idx: usize) -> [u8; 20] {
    Hasher::new(archive).extent(idx)
}

/// Whether two archives are structurally identical.
pub fn same_structure(a: &Archive, b: &Archive) -> bool {
    archive_hash(a) == archive_hash(b)
}

pub fn hex(hash: &[u8]) -> String {
    hash.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::{Extent, Pointer};

    #[test]
    fn relayout_does_not_change_hash() {
        let a = crate::archive::tests::sample();
        let mut b = a.clone();
        // Reorder extents 1 and 2, fix the pointers, pad extent 1 with zeros.
        b.extents.swap(1, 2);
        for p in &mut b.extents[0].pointers {
            p.target = match p.target {
                1 => 2,
                2 => 1,
                t => t,
            };
        }
        b.extents[2].data.extend_from_slice(&[0; 24]);
        assert!(same_structure(&a, &b));
        assert_ne!(a.serialize().unwrap(), b.serialize().unwrap());
    }

    #[test]
    fn content_change_changes_hash() {
        let a = crate::archive::tests::sample();
        let mut b = a.clone();
        b.extents[2].data[3] = 9;
        assert!(!same_structure(&a, &b));
        let mut c = a.clone();
        c.extents.push(Extent {
            data: vec![1, 2, 3, 4],
            phase: 0,
            pointers: vec![],
            origin: 0,
        });
        c.extents[0].pointers.push(Pointer { field: 12, target: 3 });
        assert!(!same_structure(&a, &c));
    }
}
