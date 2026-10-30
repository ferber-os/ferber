#[path = "fs_protocol.rs"]
mod fs_protocol;
#[path = "fs_server.rs"]
mod fs_server;
#[path = "initramfs.rs"]
mod initramfs;
#[path = "services_config.rs"]
mod services_config;
#[cfg(test)]
mod tests {
    use super::{fs_protocol as p, fs_server::Server, initramfs::Archive};
    fn archive() -> Vec<u8> {
        let records = [
            ("etc", 2, &b""[..]),
            ("etc/config", 1, &b"version 1\n"[..]),
            ("zero", 1, &b""[..]),
        ];
        let strings: Vec<u8> = records.iter().flat_map(|(p, _, _)| p.bytes()).collect();
        let start = (32 + records.len() * 32 + strings.len() + 7) & !7;
        let mut b = vec![0; start];
        b[..8].copy_from_slice(b"FRARCH01");
        b[8..12].copy_from_slice(&1u32.to_le_bytes());
        b[12..16].copy_from_slice(&(records.len() as u32).to_le_bytes());
        b[16..20].copy_from_slice(&(strings.len() as u32).to_le_bytes());
        b[20..24].copy_from_slice(&(start as u32).to_le_bytes());
        b[32 + records.len() * 32..32 + records.len() * 32 + strings.len()]
            .copy_from_slice(&strings);
        let mut path = 0;
        for (i, (name, kind, data)) in records.iter().enumerate() {
            let base = 32 + i * 32;
            b[base..base + 4].copy_from_slice(&(path as u32).to_le_bytes());
            b[base + 4..base + 8].copy_from_slice(&(name.len() as u32).to_le_bytes());
            b[base + 8..base + 12].copy_from_slice(&(*kind as u32).to_le_bytes());
            if *kind == 1 {
                let aligned = (b.len() + 7) & !7;
                b.resize(aligned, 0);
                b[base + 16..base + 24].copy_from_slice(&(aligned as u64).to_le_bytes());
                b[base + 24..base + 32].copy_from_slice(&(data.len() as u64).to_le_bytes());
                b.extend_from_slice(data);
            }
            path += name.len();
        }
        let len = b.len() as u32;
        b[24..28].copy_from_slice(&len.to_le_bytes());
        b
    }
    fn rpc(
        server: &mut Server<'_>,
        session: usize,
        op: u16,
        id: u64,
        flags: u32,
        body: &[u8],
    ) -> (p::Header, Vec<u8>) {
        let mut b = vec![0; p::HEADER + body.len()];
        p::encode(
            p::Header {
                op,
                request: 7,
                status: 0,
                flags,
                object: id,
            },
            &mut b,
        );
        b[p::HEADER..].copy_from_slice(body);
        let mut output = [0; p::MAX_MESSAGE];
        let len = server.reply(session, &b, &mut output);
        assert!(len >= p::HEADER);
        (
            p::decode(&output[..len]).unwrap(),
            output[p::HEADER..len].to_vec(),
        )
    }
    #[test]
    fn sessions_and_stale_ids() {
        let b = archive();
        let mut s = Server::new(Archive::parse(&b).unwrap());
        let (h, _) = rpc(&mut s, 0, p::OPEN, 0, p::FILE, b"etc/config");
        assert_eq!(h.status, 0);
        assert_eq!(rpc(&mut s, 1, p::STAT, h.object, 0, b"").0.status, -9);
        assert_eq!(rpc(&mut s, 0, p::CLOSE, h.object, 0, b"").0.status, 0);
        let (next, _) = rpc(&mut s, 0, p::OPEN, 0, p::FILE, b"etc/config");
        assert!(next.object > h.object);
        assert_eq!(rpc(&mut s, 0, p::STAT, h.object, 0, b"").0.status, -9);
        s.disconnect(0);
        assert_eq!(rpc(&mut s, 0, p::STAT, next.object, 0, b"").0.status, -9);
    }
    #[test]
    fn bounded_reads_and_directory() {
        let b = archive();
        let mut s = Server::new(Archive::parse(&b).unwrap());
        let (h, _) = rpc(&mut s, 0, p::OPEN, 0, p::FILE, b"etc/config");
        let mut req = [0; 16];
        req[8..12].copy_from_slice(&4096u32.to_le_bytes());
        let (r, data) = rpc(&mut s, 0, p::READ, h.object, 0, &req);
        assert_eq!(r.flags, p::EOF);
        assert_eq!(data, b"version 1\n");
        req[..8].copy_from_slice(&10u64.to_le_bytes());
        assert_eq!(rpc(&mut s, 0, p::READ, h.object, 0, &req).1.len(), 0);
        req[..8].copy_from_slice(&11u64.to_le_bytes());
        assert_eq!(rpc(&mut s, 0, p::READ, h.object, 0, &req).0.status, -22);
        assert_eq!(
            rpc(&mut s, 0, p::OPEN, 0, p::FILE, b"etc/../zero").0.status,
            -22
        );
        let (dir, _) = rpc(&mut s, 0, p::OPEN, 0, p::DIRECTORY, b"/");
        let (r, data) = rpc(&mut s, 0, p::LIST, dir.object, 0, &[0; 8]);
        assert_eq!(r.status, 0);
        assert_eq!(&data[16..], b"etc");
        let (dir, _) = rpc(&mut s, 0, p::OPEN, 0, p::DIRECTORY, b"etc");
        let (_, data) = rpc(&mut s, 0, p::LIST, dir.object, 0, &[0; 8]);
        assert_eq!(&data[16..], b"config");
    }
    #[test]
    fn open_exhaustion_does_not_replace() {
        let b = archive();
        let mut s = Server::new(Archive::parse(&b).unwrap());
        let mut ids = Vec::new();
        for _ in 0..p::MAX_OPEN {
            let (h, _) = rpc(&mut s, 0, p::OPEN, 0, p::FILE, b"zero");
            assert_eq!(h.status, 0);
            ids.push(h.object);
        }
        assert_eq!(rpc(&mut s, 0, p::OPEN, 0, p::FILE, b"zero").0.status, -24);
        for id in ids {
            assert_eq!(rpc(&mut s, 0, p::STAT, id, 0, b"").0.status, 0);
        }
    }
}
