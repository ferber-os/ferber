//! Bounded QEMU-virt console discovery; rejects bus translations and ambiguous devices.
#![allow(dead_code)]
use super::{aligned4, be32, cstr, section, DeviceTree, Region, MAX_DEPTH};

#[derive(Clone, Copy)]
pub struct ConsolePlatform {
    pub uart: Region,
    pub uart_irq: u32,
    pub plic: Region,
    pub plic_context: u32,
    pub timebase_hz: u32,
}
#[derive(Clone, Copy)]
struct Node {
    seen: u32,
    ac: u32,
    sc: u32,
    child_ac: u32,
    child_sc: u32,
    reg: usize,
    reg_len: usize,
    enabled: bool,
    translated: bool,
    cpus: bool,
    cpu: bool,
    cpu_intc: bool,
    uart: bool,
    plic: bool,
    hart: Option<u64>,
    phandle: u32,
    parent: u32,
    irq: u32,
    ext: usize,
    ext_len: usize,
    ndev: u32,
    int_cells: u32,
    shift: u32,
    width: u32,
}
impl Node {
    const EMPTY: Self = Self {
        seen: 0,
        ac: 2,
        sc: 1,
        child_ac: 2,
        child_sc: 1,
        reg: 0,
        reg_len: 0,
        enabled: true,
        translated: false,
        cpus: false,
        cpu: false,
        cpu_intc: false,
        uart: false,
        plic: false,
        hart: None,
        phandle: 0,
        parent: 0,
        irq: 0,
        ext: 0,
        ext_len: 0,
        ndev: 0,
        int_cells: 0,
        shift: 0,
        width: 1,
    };
}
fn scalar(b: &[u8], at: usize, cells: u32) -> Result<u64, &'static str> {
    match cells {
        0 => Ok(0),
        1 => Ok(be32(b, at)? as u64),
        2 => Ok(((be32(b, at)? as u64) << 32) | be32(b, at + 4)? as u64),
        _ => Err("unsupported console FDT cells"),
    }
}
fn region(b: &[u8], n: Node) -> Result<Region, &'static str> {
    if n.translated
        || n.ac > 2
        || n.sc > 2
        || n.sc == 0
        || n.reg_len != ((n.ac + n.sc) * 4) as usize
    {
        return Err("unsupported console FDT reg");
    }
    let start = scalar(b, n.reg, n.ac)?;
    let len = scalar(b, n.reg + n.ac as usize * 4, n.sc)?;
    let end = start
        .checked_add(len)
        .filter(|end| start != 0 && *end > start)
        .ok_or("invalid console FDT range")?;
    Ok(Region { start, end })
}
impl DeviceTree<'_> {
    pub fn console_platform(&self, boot_hart: u64) -> Result<ConsolePlatform, &'static str> {
        let mut stack = [Node::EMPTY; MAX_DEPTH];
        let mut depth = 0;
        let mut offset = 0;
        let mut root_seen = false;
        let mut uart = None;
        let mut plic = None;
        let mut frequency = None;
        let mut intcs = [(0u32, 0u64); 64];
        let mut count = 0;
        loop {
            let token = be32(self.structure, offset)?;
            offset += 4;
            match token {
                1 => {
                    if depth == MAX_DEPTH {
                        return Err("FDT nesting too deep");
                    }
                    if depth == 0 {
                        if root_seen {
                            return Err("multiple console FDT roots");
                        }
                        root_seen = true;
                    }
                    let name = cstr(self.structure, offset)?;
                    offset = aligned4(offset + name.len() + 1)?;
                    let p = if depth == 0 {
                        Node::EMPTY
                    } else {
                        stack[depth - 1]
                    };
                    let mut n = Node {
                        ac: p.child_ac,
                        sc: p.child_sc,
                        child_ac: 2,
                        child_sc: 1,
                        enabled: p.enabled,
                        translated: p.translated,
                        parent: p.parent,
                        ..Node::EMPTY
                    };
                    n.cpus = depth == 1 && name == b"cpus";
                    n.cpu = p.cpus && name.starts_with(b"cpu@");
                    if p.cpu {
                        if p.reg_len != p.ac as usize * 4 {
                            return Err("invalid CPU hart reg");
                        }
                        n.hart = Some(scalar(self.structure, p.reg, p.ac)?);
                    }
                    stack[depth] = n;
                    depth += 1;
                }
                2 => {
                    if depth == 0 {
                        return Err("unbalanced FDT nodes");
                    }
                    depth -= 1;
                    let n = stack[depth];
                    if !n.enabled {
                        continue;
                    }
                    if n.cpu_intc {
                        if n.phandle == 0
                            || n.int_cells != 1
                            || n.hart.is_none()
                            || count == intcs.len()
                            || intcs[..count].iter().any(|x| x.0 == n.phandle)
                        {
                            return Err("invalid CPU interrupt controller");
                        }
                        intcs[count] = (n.phandle, n.hart.unwrap());
                        count += 1;
                    }
                    if n.uart && uart.replace(n).is_some() {
                        return Err("ambiguous console UART");
                    }
                    if n.plic && plic.replace(n).is_some() {
                        return Err("ambiguous console PLIC");
                    }
                }
                3 => {
                    if depth == 0 {
                        return Err("FDT property outside node");
                    }
                    let len = be32(self.structure, offset)? as usize;
                    let nameoff = be32(self.structure, offset + 4)? as usize;
                    offset += 8;
                    let value = section(self.structure, offset, len)?;
                    let name = cstr(self.strings, nameoff)?;
                    let n = &mut stack[depth - 1];
                    let property = match name {
                        b"#address-cells" => Some(0),
                        b"#size-cells" => Some(1),
                        b"reg" => Some(2),
                        b"status" => Some(3),
                        b"ranges" => Some(4),
                        b"compatible" => Some(5),
                        b"phandle" => Some(6),
                        b"interrupt-parent" => Some(7),
                        b"interrupts" => Some(8),
                        b"#interrupt-cells" => Some(9),
                        b"interrupts-extended" => Some(10),
                        b"riscv,ndev" => Some(11),
                        b"reg-shift" => Some(12),
                        b"reg-io-width" => Some(13),
                        b"timebase-frequency" => Some(14),
                        _ => None,
                    };
                    if let Some(bit) = property {
                        if n.seen & (1 << bit) != 0 {
                            return Err("duplicate console FDT property");
                        }
                        n.seen |= 1 << bit;
                    }

                    let word = || {
                        if len == 4 {
                            be32(value, 0)
                        } else {
                            Err("invalid console FDT scalar")
                        }
                    };
                    match name {
                        b"#address-cells" => {
                            n.child_ac = word()?;
                        }
                        b"#size-cells" => {
                            n.child_sc = word()?;
                        }
                        b"reg" => {
                            if n.reg_len != 0 {
                                return Err("duplicate console reg");
                            }
                            n.reg = offset;
                            n.reg_len = len;
                        }
                        b"status" => n.enabled &= value == b"okay\0" || value == b"ok\0",
                        b"ranges" => n.translated |= len != 0,
                        b"compatible" => {
                            if value.last() != Some(&0) {
                                return Err("invalid console compatible");
                            }
                            for v in value.split(|b| *b == 0) {
                                match v {
                                    b"ns16550a" => n.uart = true,
                                    b"riscv,plic0" | b"sifive,plic-1.0.0" => n.plic = true,
                                    b"riscv,cpu-intc" => n.cpu_intc = true,
                                    _ => {}
                                }
                            }
                        }
                        b"phandle" => {
                            if n.phandle != 0 {
                                return Err("duplicate console phandle");
                            }
                            n.phandle = word()?;
                        }
                        b"interrupt-parent" => n.parent = word()?,
                        b"interrupts" => n.irq = if len == 4 { word()? } else { 0 },
                        b"#interrupt-cells" => n.int_cells = word()?,
                        b"interrupts-extended" => {
                            if n.ext_len != 0 {
                                return Err("duplicate PLIC interrupts");
                            }
                            n.ext = offset;
                            n.ext_len = len;
                        }
                        b"riscv,ndev" => n.ndev = word()?,
                        b"reg-shift" => n.shift = word()?,
                        b"reg-io-width" => n.width = word()?,
                        b"timebase-frequency" if n.cpus => {
                            if frequency.replace(word()?).is_some() {
                                return Err("duplicate timebase");
                            }
                        }
                        _ => {}
                    }
                    offset = aligned4(offset.checked_add(len).ok_or("FDT offset overflow")?)?;
                }
                4 => {}
                9 if depth == 0 => break,
                _ => return Err("invalid FDT token"),
            }
        }
        let u = uart.ok_or("missing console UART")?;
        let p = plic.ok_or("missing console PLIC")?;
        if p.phandle == 0
            || p.int_cells != 1
            || u.parent != p.phandle
            || u.irq == 0
            || u.irq > p.ndev
            || p.ndev > 1023
            || u.shift != 0
            || u.width != 1
            || p.ext_len == 0
            || p.ext_len % 8 != 0
        {
            return Err("unsupported console interrupt route");
        }
        let mut context = None;
        for i in 0..p.ext_len / 8 {
            let ph = be32(self.structure, p.ext + i * 8)?;
            let irq = be32(self.structure, p.ext + i * 8 + 4)?;
            let hart = intcs[..count]
                .iter()
                .find(|x| x.0 == ph)
                .ok_or("unknown PLIC CPU phandle")?
                .1;
            if hart == boot_hart && irq == 9 && context.replace(i as u32).is_some() {
                return Err("duplicate PLIC supervisor context");
            }
        }
        let plic = region(self.structure, p)?;
        let uart = region(self.structure, u)?;
        if plic.start & 3 != 0 || (uart.start < plic.end && plic.start < uart.end) {
            return Err("overlapping or misaligned console MMIO");
        }
        let ctx = context.ok_or("missing PLIC supervisor context")?;
        let needed = 0x20_0008u64
            .checked_add(ctx as u64 * 0x1000)
            .ok_or("PLIC context overflow")?;
        if plic.end - plic.start < needed || uart.end - uart.start < 8 {
            return Err("console MMIO range too small");
        }
        let timebase_hz = frequency
            .filter(|f| *f != 0)
            .ok_or("missing timebase frequency")?;
        Ok(ConsolePlatform {
            uart,
            uart_irq: u.irq,
            plic,
            plic_context: ctx,
            timebase_hz,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Tree {
        s: Vec<u8>,
        names: Vec<u8>,
    }
    impl Tree {
        fn new() -> Self {
            Self {
                s: Vec::new(),
                names: Vec::new(),
            }
        }
        fn word(&mut self, v: u32) {
            self.s.extend_from_slice(&v.to_be_bytes());
        }
        fn begin(&mut self, n: &str) {
            self.word(1);
            self.s.extend_from_slice(n.as_bytes());
            self.s.push(0);
            while self.s.len() % 4 != 0 {
                self.s.push(0);
            }
        }
        fn end(&mut self) {
            self.word(2);
        }
        fn prop(&mut self, n: &str, v: &[u8]) {
            let at = self.names.len();
            self.names.extend_from_slice(n.as_bytes());
            self.names.push(0);
            self.word(3);
            self.word(v.len() as u32);
            self.word(at as u32);
            self.s.extend_from_slice(v);
            while self.s.len() % 4 != 0 {
                self.s.push(0);
            }
        }
        fn num(&mut self, n: &str, v: u32) {
            self.prop(n, &v.to_be_bytes());
        }
        fn reg(&mut self, start: u32, len: u32) {
            let mut b = [0; 16];
            b[4..8].copy_from_slice(&start.to_be_bytes());
            b[12..].copy_from_slice(&len.to_be_bytes());
            self.prop("reg", &b);
        }
        fn parse(&self) -> Result<ConsolePlatform, &'static str> {
            DeviceTree {
                bytes: &[0; 16],
                structure: &self.s,
                strings: &self.names,
                reserve_offset: 0,
            }
            .console_platform(3)
        }
    }
    fn fixture(irq: u32, disabled: bool, translated: bool, duplicate: bool) -> Tree {
        let mut f = Tree::new();
        f.begin("");
        f.num("#address-cells", 2);
        f.num("#size-cells", 2);
        f.begin("cpus");
        f.num("#address-cells", 1);
        f.num("#size-cells", 0);
        f.num("timebase-frequency", 10_000_000);
        f.begin("cpu@3");
        f.num("reg", 3);
        f.begin("interrupt-controller");
        f.prop("compatible", b"riscv,cpu-intc\0");
        f.num("#interrupt-cells", 1);
        f.num("phandle", 7);
        f.end();
        f.end();
        f.end();
        f.begin("soc");
        f.num("#address-cells", 2);
        f.num("#size-cells", 2);
        f.prop("ranges", if translated { b"bad" } else { b"" });
        f.begin("plic@c000000");
        f.prop("compatible", b"sifive,plic-1.0.0\0riscv,plic0\0");
        f.reg(0xc000000, 0x600000);
        f.num("phandle", 42);
        f.num("#interrupt-cells", 1);
        f.num("riscv,ndev", 96);
        let mut ext = Vec::new();
        for x in [7u32, 11, 7, 9] {
            ext.extend_from_slice(&x.to_be_bytes());
        }
        f.prop("interrupts-extended", &ext);
        f.end();
        for _ in 0..if duplicate { 2 } else { 1 } {
            f.begin("uart@10000000");
            f.prop("compatible", b"ns16550a\0");
            f.reg(0x10000000, 0x100);
            f.num("interrupt-parent", 42);
            f.num("interrupts", irq);
            if disabled {
                f.prop("status", b"disabled\0");
            }
            f.end();
        }
        f.end();
        f.end();
        f.word(9);
        f
    }
    #[test]
    fn console_route_uses_boot_hart_supervisor_context() {
        let f = fixture(10, false, false, false);
        let p = f.parse().unwrap();
        assert_eq!(p.uart.start, 0x10000000);
        assert_eq!(p.plic.start, 0xc000000);
        assert_eq!(p.plic_context, 1);
        assert_eq!(p.uart_irq, 10);
        assert_eq!(p.timebase_hz, 10_000_000);
        let tree = DeviceTree {
            bytes: &[0; 16],
            structure: &f.s,
            strings: &f.names,
            reserve_offset: 0,
        };
        assert!(tree.console_platform(0).is_err());
    }
    #[test]
    fn rejects_unsupported_and_ambiguous_devices() {
        for (irq, disabled, translated, duplicate) in [
            (0, false, false, false),
            (97, false, false, false),
            (10, true, false, false),
            (10, false, true, false),
            (10, false, false, true),
        ] {
            assert!(fixture(irq, disabled, translated, duplicate)
                .parse()
                .is_err());
        }
    }
    #[test]
    fn all_structure_truncations_and_mutations_are_bounded() {
        let f = fixture(10, false, false, false);
        for n in 0..f.s.len() {
            let t = Tree {
                s: f.s[..n].to_vec(),
                names: f.names.clone(),
            };
            assert!(t.parse().is_err(), "truncation {n}");
        }
        for i in 0..f.s.len() {
            let mut t = Tree {
                s: f.s.clone(),
                names: f.names.clone(),
            };
            t.s[i] ^= 0xff;
            let _ = t.parse();
        }
    }
    #[test]
    fn duplicate_properties_and_multiple_roots_are_rejected() {
        let mut f = fixture(10, false, false, false);
        let name = f
            .names
            .windows(11)
            .position(|n| n == b"interrupts\0")
            .unwrap() as u32;
        let mut header = Vec::new();
        for n in [3u32, 4, name] {
            header.extend_from_slice(&n.to_be_bytes());
        }
        let at = f.s.windows(12).position(|n| n == header).unwrap();
        let record = f.s[at..at + 16].to_vec();
        f.s.splice(at..at, record);
        assert!(f.parse().is_err());
        let mut f = fixture(10, false, false, false);
        f.s.truncate(f.s.len() - 4);
        f.begin("second");
        f.end();
        f.word(9);
        assert!(f.parse().is_err());
    }
}
