//! Read-only archive sessions. Open IDs never escape their owning channel table.
#![allow(dead_code)]
use super::fs_protocol as p;
use super::initramfs::{Archive, Entry, Kind};
#[derive(Clone, Copy)]
struct Open<'a> {
    id: u64,
    entry: Option<Entry<'a>>,
}
pub struct Server<'a> {
    archive: Archive<'a>,
    opens: [[Option<Open<'a>>; p::MAX_OPEN]; p::MAX_SESSIONS],
    next: [u64; p::MAX_SESSIONS],
}
impl<'a> Server<'a> {
    pub fn new(archive: Archive<'a>) -> Self {
        Self {
            archive,
            opens: [[None; p::MAX_OPEN]; p::MAX_SESSIONS],
            next: [1; p::MAX_SESSIONS],
        }
    }
    pub fn disconnect(&mut self, s: usize) {
        self.opens[s].fill(None);
    }
    pub fn reply(&mut self, s: usize, b: &[u8], out: &mut [u8; p::MAX_MESSAGE]) -> usize {
        let Some(mut h) = p::decode(b) else { return 0 };
        let op = h.op;
        let flags = h.flags;
        let object = h.object;
        h.status = 0;
        h.flags = 0;
        let body = &b[p::HEADER..];
        let mut n = 0;
        let result = (|| -> Result<(), i32> {
            if s >= p::MAX_SESSIONS || p::decode(b).unwrap().status != 0 {
                return Err(-22);
            }
            if op == p::OPEN {
                if object != 0 || !matches!(flags, p::FILE | p::DIRECTORY) {
                    return Err(-22);
                }
                let path = core::str::from_utf8(body).map_err(|_| -22)?;
                let entry = if path == "/" {
                    None
                } else {
                    if !super::initramfs::valid_path(path) {
                        return Err(-22);
                    }
                    Some(self.archive.find(path).ok_or(-2)?)
                };
                let kind = entry.map_or(p::DIRECTORY, |e| {
                    if e.kind == Kind::File {
                        p::FILE
                    } else {
                        p::DIRECTORY
                    }
                });
                if kind != flags {
                    return Err(-22);
                }
                let slot = self.opens[s].iter().position(Option::is_none).ok_or(-24)?;
                let id = self.next[s];
                self.next[s] = id.checked_add(1).ok_or(-28)?;
                self.opens[s][slot] = Some(Open { id, entry });
                h.object = id;
                out[p::HEADER..p::HEADER + 4].copy_from_slice(&kind.to_le_bytes());
                out[p::HEADER + 4..p::HEADER + 8].fill(0);
                out[p::HEADER + 8..p::HEADER + 16]
                    .copy_from_slice(&(entry.map_or(0, |e| e.data.len()) as u64).to_le_bytes());
                n = 16;
                return Ok(());
            }
            if flags != 0 {
                return Err(-22);
            }
            let slot = self.opens[s]
                .iter()
                .position(|x| x.is_some_and(|x| x.id == object))
                .ok_or(-9)?;
            let open = self.opens[s][slot].unwrap();
            match op {
                p::CLOSE => {
                    if !body.is_empty() {
                        return Err(-22);
                    }
                    self.opens[s][slot] = None;
                }
                p::STAT => {
                    if !body.is_empty() {
                        return Err(-22);
                    }
                    let kind = open.entry.map_or(p::DIRECTORY, |e| {
                        if e.kind == Kind::File {
                            p::FILE
                        } else {
                            p::DIRECTORY
                        }
                    });
                    out[p::HEADER..p::HEADER + 4].copy_from_slice(&kind.to_le_bytes());
                    out[p::HEADER + 4..p::HEADER + 8].fill(0);
                    out[p::HEADER + 8..p::HEADER + 16].copy_from_slice(
                        &(open.entry.map_or(0, |e| e.data.len()) as u64).to_le_bytes(),
                    );
                    n = 16;
                }
                p::READ => {
                    if body.len() != 16 || body[12..] != [0; 4] {
                        return Err(-22);
                    }
                    let entry = open.entry.filter(|e| e.kind == Kind::File).ok_or(-22)?;
                    let offset = usize::try_from(u64::from_le_bytes(body[..8].try_into().unwrap()))
                        .map_err(|_| -22)?;
                    let length = u32::from_le_bytes(body[8..12].try_into().unwrap()) as usize;
                    if length == 0 || length > p::CHUNK || offset > entry.data.len() {
                        return Err(-22);
                    }
                    n = length.min(entry.data.len() - offset);
                    out[p::HEADER..p::HEADER + n].copy_from_slice(&entry.data[offset..offset + n]);
                    if offset + n == entry.data.len() {
                        h.flags = p::EOF;
                    }
                }
                p::LIST => {
                    if body.len() != 8
                        || body[4..] != [0; 4]
                        || open.entry.is_some_and(|e| e.kind != Kind::Directory)
                    {
                        return Err(-22);
                    }
                    let cursor = u32::from_le_bytes(body[..4].try_into().unwrap()) as usize;
                    let parent = open.entry.map_or("", |e| e.path);
                    let child = self
                        .archive
                        .entries()
                        .filter_map(|e| {
                            let name = if parent.is_empty() {
                                e.path
                            } else {
                                e.path.strip_prefix(parent)?.strip_prefix('/')?
                            };
                            (!name.contains('/')).then_some((e, name))
                        })
                        .nth(cursor);
                    if let Some((e, name)) = child {
                        let next = cursor
                            .checked_add(1)
                            .and_then(|x| u32::try_from(x).ok())
                            .ok_or(-22)?;
                        out[p::HEADER..p::HEADER + 4].copy_from_slice(&next.to_le_bytes());
                        out[p::HEADER + 4..p::HEADER + 8].copy_from_slice(
                            &(if e.kind == Kind::File {
                                p::FILE
                            } else {
                                p::DIRECTORY
                            })
                            .to_le_bytes(),
                        );
                        out[p::HEADER + 8..p::HEADER + 16]
                            .copy_from_slice(&(e.data.len() as u64).to_le_bytes());
                        out[p::HEADER + 16..p::HEADER + 16 + name.len()]
                            .copy_from_slice(name.as_bytes());
                        n = 16 + name.len();
                    } else {
                        h.flags = p::EOF;
                    }
                }
                _ => return Err(-22),
            }
            Ok(())
        })();
        if let Err(status) = result {
            h.status = status;
            h.flags = 0;
            n = 0;
        }
        p::encode(h, out);
        p::HEADER + n
    }
}
