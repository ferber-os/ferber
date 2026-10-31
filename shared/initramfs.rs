//! Bounded immutable named files; no permissions or implicit authority.
#![allow(dead_code)]
pub const MAGIC: &[u8; 8] = b"FRARCH01";
pub const HEADER: usize = 32;
pub const RECORD: usize = 32;
pub const MAX_SIZE: usize = 4 * 1024 * 1024;
pub const MAX_ENTRIES: usize = 64;
pub const MAX_PATH: usize = 255;
pub const MAX_COMPONENTS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArchiveError;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    File,
    Directory,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry<'a> {
    pub path: &'a str,
    pub kind: Kind,
    pub data: &'a [u8],
}
#[derive(Clone, Copy)]
pub struct Archive<'a> {
    bytes: &'a [u8],
    count: usize,
    strings: usize,
}
fn u32_at(bytes: &[u8], offset: usize) -> usize {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize
}
fn u64_at(bytes: &[u8], offset: usize) -> Result<usize, ArchiveError> {
    usize::try_from(u64::from_le_bytes(
        bytes[offset..offset + 8].try_into().unwrap(),
    ))
    .map_err(|_| ArchiveError)
}
fn align8(value: usize) -> Result<usize, ArchiveError> {
    value.checked_add(7).map(|v| v & !7).ok_or(ArchiveError)
}
pub fn valid_path(path: &str) -> bool {
    if path.is_empty() || path.len() > MAX_PATH {
        return false;
    }
    let mut count = 0;
    for part in path.split('/') {
        count += 1;
        if count > MAX_COMPONENTS
            || part.is_empty()
            || part == "."
            || part == ".."
            || !part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        {
            return false;
        }
    }
    true
}
impl<'a> Archive<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, ArchiveError> {
        if bytes.len() < HEADER
            || bytes.len() > MAX_SIZE
            || &bytes[..8] != MAGIC
            || u32_at(bytes, 8) != 1
            || u32_at(bytes, 24) != bytes.len()
            || u32_at(bytes, 28) != 0
        {
            return Err(ArchiveError);
        }
        let count = u32_at(bytes, 12);
        if count > MAX_ENTRIES {
            return Err(ArchiveError);
        }
        let strings = HEADER + count * RECORD;
        let strings_end = strings.checked_add(u32_at(bytes, 16)).ok_or(ArchiveError)?;
        let data_start = align8(strings_end)?;
        if data_start > bytes.len()
            || u32_at(bytes, 20) != data_start
            || bytes[strings_end..data_start].iter().any(|b| *b != 0)
        {
            return Err(ArchiveError);
        }
        let archive = Self {
            bytes,
            count,
            strings,
        };
        let mut path_end = 0usize;
        let mut cursor = data_start;
        let mut previous = "";
        for index in 0..count {
            let base = HEADER + index * RECORD;
            let path_offset = u32_at(bytes, base);
            let path_len = u32_at(bytes, base + 4);
            if path_offset != path_end || path_len > MAX_PATH || u32_at(bytes, base + 12) != 0 {
                return Err(ArchiveError);
            }
            path_end = path_offset.checked_add(path_len).ok_or(ArchiveError)?;
            if path_end > strings_end - strings {
                return Err(ArchiveError);
            }
            let path = core::str::from_utf8(&bytes[strings + path_offset..strings + path_end])
                .map_err(|_| ArchiveError)?;
            if !valid_path(path) || path <= previous {
                return Err(ArchiveError);
            }
            if let Some((parent, _)) = path.rsplit_once('/') {
                if !(0..index).any(|i| {
                    let entry = archive.entry(i);
                    entry.path == parent && entry.kind == Kind::Directory
                }) {
                    return Err(ArchiveError);
                }
            }
            let offset = u64_at(bytes, base + 16)?;
            let length = u64_at(bytes, base + 24)?;
            match u32_at(bytes, base + 8) {
                1 => {
                    let aligned = align8(cursor)?;
                    let end = offset.checked_add(length).ok_or(ArchiveError)?;
                    if offset != aligned
                        || end > bytes.len()
                        || bytes[cursor..aligned].iter().any(|b| *b != 0)
                    {
                        return Err(ArchiveError);
                    }
                    cursor = end;
                }
                2 if offset == 0 && length == 0 => {}
                _ => return Err(ArchiveError),
            }
            previous = path;
        }
        if path_end != strings_end - strings || cursor != bytes.len() {
            return Err(ArchiveError);
        }
        Ok(archive)
    }
    fn entry(self, index: usize) -> Entry<'a> {
        let base = HEADER + index * RECORD;
        let path_start = self.strings + u32_at(self.bytes, base);
        let path_end = path_start + u32_at(self.bytes, base + 4);
        let path = core::str::from_utf8(&self.bytes[path_start..path_end]).unwrap();
        let kind = if u32_at(self.bytes, base + 8) == 1 {
            Kind::File
        } else {
            Kind::Directory
        };
        let offset = u64_at(self.bytes, base + 16).unwrap();
        let length = u64_at(self.bytes, base + 24).unwrap();
        Entry {
            path,
            kind,
            data: &self.bytes[offset..offset + length],
        }
    }
    pub fn entries(self) -> impl Iterator<Item = Entry<'a>> {
        (0..self.count).map(move |i| self.entry(i))
    }
    pub fn find(self, path: &str) -> Option<Entry<'a>> {
        let mut low = 0;
        let mut high = self.count;
        while low < high {
            let middle = low + (high - low) / 2;
            let entry = self.entry(middle);
            match entry.path.cmp(path) {
                core::cmp::Ordering::Less => low = middle + 1,
                core::cmp::Ordering::Greater => high = middle,
                core::cmp::Ordering::Equal => return Some(entry),
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(items: &[(&str, Option<&[u8]>)]) -> Vec<u8> {
        let mut bytes = vec![0; HEADER + items.len() * RECORD];
        let mut string_len = 0;
        for (i, (path, data)) in items.iter().enumerate() {
            let base = HEADER + i * RECORD;
            set32(&mut bytes, base, string_len);
            set32(&mut bytes, base + 4, path.len());
            set32(&mut bytes, base + 8, if data.is_some() { 1 } else { 2 });
            bytes.extend_from_slice(path.as_bytes());
            string_len += path.len();
        }
        bytes.resize((bytes.len() + 7) & !7, 0);
        let data_start = bytes.len();
        for (i, (_, data)) in items.iter().enumerate() {
            if let Some(data) = data {
                bytes.resize((bytes.len() + 7) & !7, 0);
                let offset = bytes.len();
                bytes.extend_from_slice(data);
                let base = HEADER + i * RECORD;
                bytes[base + 16..base + 24].copy_from_slice(&(offset as u64).to_le_bytes());
                bytes[base + 24..base + 32].copy_from_slice(&(data.len() as u64).to_le_bytes());
            }
        }
        bytes[..8].copy_from_slice(MAGIC);
        set32(&mut bytes, 8, 1);
        set32(&mut bytes, 12, items.len());
        set32(&mut bytes, 16, string_len);
        set32(&mut bytes, 20, data_start);
        let length = bytes.len();
        set32(&mut bytes, 24, length);
        bytes
    }
    fn set32(bytes: &mut [u8], offset: usize, value: usize) {
        bytes[offset..offset + 4].copy_from_slice(&(value as u32).to_le_bytes());
    }
    fn basic() -> Vec<u8> {
        fixture(&[
            ("a", None),
            ("a/file", Some(b"hello")),
            ("b", Some(b"")),
            ("c", Some(b"!")),
        ])
    }
    #[test]
    fn lookup_iteration_and_empty_entries() {
        let bytes = basic();
        let archive = Archive::parse(&bytes).unwrap();
        assert_eq!(archive.entries().count(), 4);
        assert_eq!(archive.find("a").unwrap().kind, Kind::Directory);
        assert_eq!(archive.find("a/file").unwrap().data, b"hello");
        assert_eq!(archive.find("b").unwrap().data, b"");
        for missing in ["", "a/absent", "z", "/a", "a/../c"] {
            assert!(archive.find(missing).is_none());
        }
        assert!(Archive::parse(&fixture(&[]))
            .unwrap()
            .entries()
            .next()
            .is_none());
        assert!(Archive::parse(&fixture(&[("a", None)])).is_ok());
    }
    #[test]
    fn rejects_every_truncation_and_hidden_trailing_data() {
        let bytes = basic();
        for length in 0..bytes.len() {
            assert!(Archive::parse(&bytes[..length]).is_err(), "length {length}");
        }
        let mut extra = bytes;
        extra.push(0);
        let length = extra.len();
        set32(&mut extra, 24, length);
        assert!(Archive::parse(&extra).is_err());
    }
    #[test]
    fn rejects_ambiguous_paths_and_missing_parents() {
        for path in [
            "", "/a", "a/", "a//b", ".", "..", "a/../b", "a/./b", "a\\b", "a b", "\0", "é",
        ] {
            assert!(
                Archive::parse(&fixture(&[(path, Some(b"x"))])).is_err(),
                "{path:?}"
            );
        }
        for entries in [
            vec![("b", Some(&b"x"[..])), ("a", Some(&b"y"[..]))],
            vec![("a", None), ("a", None)],
            vec![("a/file", Some(&b"x"[..]))],
            vec![("a", Some(&b"x"[..])), ("a/file", Some(&b"x"[..]))],
        ] {
            assert!(Archive::parse(&fixture(&entries)).is_err());
        }
        assert!(!valid_path(&"a".repeat(MAX_PATH + 1)));
        assert!(!valid_path(&vec!["a"; MAX_COMPONENTS + 1].join("/")));
        assert!(valid_path(&"a".repeat(MAX_PATH)));
        assert!(valid_path(&vec!["a"; MAX_COMPONENTS].join("/")));
    }
    #[test]
    fn rejects_bad_header_records_extents_and_padding() {
        let bytes = basic();
        for (offset, value) in [
            (8, 2),
            (12, MAX_ENTRIES + 1),
            (16, usize::MAX),
            (20, 0),
            (24, 0),
            (28, 1),
            (HEADER, 1),
            (HEADER + 4, 0),
            (HEADER + 8, 3),
            (HEADER + 12, 1),
            (HEADER + 16, 1),
            (HEADER + 24, 1),
            (HEADER + RECORD, 0),
            (HEADER + RECORD + 8, 2),
        ] {
            let mut bad = bytes.clone();
            set32(&mut bad, offset, value);
            assert!(Archive::parse(&bad).is_err(), "offset {offset}");
        }
        for offset in [HEADER + RECORD + 16, HEADER + RECORD + 24] {
            let mut bad = bytes.clone();
            bad[offset..offset + 8].fill(255);
            assert!(Archive::parse(&bad).is_err());
        }
        let mut bad = bytes.clone();
        bad[0] ^= 1;
        assert!(Archive::parse(&bad).is_err());
        let string_end = HEADER + 4 * RECORD + u32_at(&bytes, 16);
        for offset in string_end..u32_at(&bytes, 20) {
            let mut bad = bytes.clone();
            bad[offset] = 1;
            assert!(Archive::parse(&bad).is_err());
        }
        let first_end = u64_at(&bytes, HEADER + RECORD + 16).unwrap() + 5;
        for offset in first_end..align8(first_end).unwrap() {
            let mut bad = bytes.clone();
            bad[offset] = 1;
            assert!(Archive::parse(&bad).is_err());
        }
        let mut bad = bytes;
        bad.resize(MAX_SIZE + 1, 0);
        assert!(Archive::parse(&bad).is_err());
    }
    #[test]
    fn exact_resource_limits_are_accepted() {
        let names: Vec<_> = (0..MAX_ENTRIES).map(|i| format!("f{i:02}")).collect();
        let entries: Vec<_> = names
            .iter()
            .map(|name| (name.as_str(), Some(&b""[..])))
            .collect();
        assert_eq!(
            Archive::parse(&fixture(&entries))
                .unwrap()
                .entries()
                .count(),
            MAX_ENTRIES
        );
        let path = "a".repeat(MAX_PATH);
        assert!(Archive::parse(&fixture(&[(&path, Some(b"x"))])).is_ok());
        let payload = vec![0; MAX_SIZE - ((HEADER + RECORD + 1 + 7) & !7)];
        let bytes = fixture(&[("a", Some(&payload))]);
        assert_eq!(bytes.len(), MAX_SIZE);
        assert!(Archive::parse(&bytes).is_ok());
    }
    #[test]
    fn arbitrary_metadata_never_panics() {
        let bytes = basic();
        for offset in 0..bytes.len() {
            for value in [0, 1, 127, 255] {
                let mut mutated = bytes.clone();
                mutated[offset] = value;
                if let Ok(archive) = Archive::parse(&mutated) {
                    for entry in archive.entries() {
                        assert_eq!(archive.find(entry.path), Some(entry));
                    }
                }
            }
        }
    }
}
