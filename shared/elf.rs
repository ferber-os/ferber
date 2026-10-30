//! Bounded validation of static ELF64 user images, before allocating any backing.

const PAGE_SIZE: usize = 4096;
pub const MAX_IMAGE_SIZE: usize = 4 * 1024 * 1024;
pub const MAX_MAPPED_SIZE: usize = 16 * 1024 * 1024;
const MAX_PROGRAM_HEADERS: usize = 32;
const MAX_SECTION_HEADERS: usize = 256;
const MAX_LOAD_SEGMENTS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElfError {
    Header,
    Bounds,
    Unsupported,
    Alignment,
    Permissions,
    Overlap,
    Entry,
    Limit,
}

#[derive(Debug, Clone, Copy)]
pub struct LoadSegment<'a> {
    pub data: &'a [u8],
    pub virtual_address: usize,
    pub page_start: usize,
    pub page_size: usize,
    pub memory_size: usize,
    pub readable: bool,
    pub writable: bool,
    pub executable: bool,
}

#[derive(Debug)]
pub struct ElfImage<'a> {
    entry: usize,
    segments: [Option<LoadSegment<'a>>; MAX_LOAD_SEGMENTS],
}

impl<'a> ElfImage<'a> {
    /// `user_max` is the exclusive end of the architecture's lower user range.
    pub fn parse(bytes: &'a [u8], machine: u16, user_max: usize) -> Result<Self, ElfError> {
        if bytes.len() > MAX_IMAGE_SIZE {
            return Err(ElfError::Limit);
        }
        let header = bytes.get(..64).ok_or(ElfError::Header)?;
        if header[..7] != *b"\x7fELF\x02\x01\x01"
            || header[7] != 0
            || header[8] != 0
            || u16_at(header, 16) != 2
            || u16_at(header, 18) != machine
            || u32_at(header, 20) != 1
            || u16_at(header, 52) != 64
            || !matches!(machine, 62 | 243)
        {
            return Err(ElfError::Header);
        }
        // ABI flags do not prove instruction use; the bundled image must stay integer-only.
        let flags = u32_at(header, 48);
        if (machine == 62 && flags != 0) || (machine == 243 && !matches!(flags, 0 | 1 | 5)) {
            return Err(ElfError::Unsupported);
        }
        let entry = usize_at(header, 24)?;
        let phoff = usize_at(header, 32)?;
        let phnum = u16_at(header, 56) as usize;
        if phnum == 0 || u16_at(header, 54) != 56 || phoff < 64 {
            return Err(ElfError::Header);
        }
        if phnum > MAX_PROGRAM_HEADERS {
            return Err(ElfError::Limit);
        }
        let table = range(bytes, phoff, phnum * 56)?;
        validate_sections(bytes, header)?;

        let mut image = Self {
            entry,
            segments: [None; MAX_LOAD_SEGMENTS],
        };
        let mut count = 0;
        let mut total = 0usize;
        let mut entry_valid = false;
        for ph in table.as_chunks::<56>().0 {
            match u32_at(ph, 0) {
                1 => {}
                // No runtime processing is needed for notes or the PHDR description.
                0 | 4 | 6 => continue,
                0x7000_0003 if machine == 243 => {
                    range(bytes, usize_at(ph, 8)?, usize_at(ph, 32)?)?;
                    continue;
                }
                0x6474_e551 if u32_at(ph, 4) & 1 == 0 => continue,
                _ => return Err(ElfError::Unsupported),
            }
            if count == MAX_LOAD_SEGMENTS {
                return Err(ElfError::Limit);
            }
            let permissions = u32_at(ph, 4);
            if !matches!(permissions, 4..=6) {
                return Err(ElfError::Permissions);
            }
            let offset = usize_at(ph, 8)?;
            let virtual_address = usize_at(ph, 16)?;
            let file_size = usize_at(ph, 32)?;
            let memory_size = usize_at(ph, 40)?;
            let align = usize_at(ph, 48)?;
            if memory_size == 0 || file_size > memory_size {
                return Err(ElfError::Bounds);
            }
            if (align > 1
                && (!align.is_power_of_two() || virtual_address % align != offset % align))
                || virtual_address % PAGE_SIZE != offset % PAGE_SIZE
            {
                return Err(ElfError::Alignment);
            }
            let data = range(bytes, offset, file_size)?;
            let end = virtual_address
                .checked_add(memory_size)
                .ok_or(ElfError::Bounds)?;
            let page_start = virtual_address & !(PAGE_SIZE - 1);
            let page_end =
                end.checked_add(PAGE_SIZE - 1).ok_or(ElfError::Bounds)? & !(PAGE_SIZE - 1);
            if page_start < PAGE_SIZE || page_end > user_max {
                return Err(ElfError::Bounds);
            }
            let page_size = page_end - page_start;
            total = total.checked_add(page_size).ok_or(ElfError::Limit)?;
            if total > MAX_MAPPED_SIZE {
                return Err(ElfError::Limit);
            }
            for previous in image.segments() {
                if page_start < previous.page_start + previous.page_size
                    && previous.page_start < page_end
                {
                    return Err(ElfError::Overlap);
                }
            }
            let executable = permissions & 1 != 0;
            entry_valid |=
                executable && entry >= virtual_address && entry - virtual_address < file_size;
            image.segments[count] = Some(LoadSegment {
                data,
                virtual_address,
                page_start,
                page_size,
                memory_size,
                readable: true,
                writable: permissions & 2 != 0,
                executable,
            });
            count += 1;
        }
        if !entry_valid || (machine == 243 && entry & 1 != 0) {
            return Err(ElfError::Entry);
        }
        Ok(image)
    }

    pub fn entry(&self) -> usize {
        self.entry
    }

    pub fn segments(&self) -> impl Iterator<Item = LoadSegment<'a>> + '_ {
        self.segments.iter().flatten().copied()
    }
}

fn validate_sections(bytes: &[u8], header: &[u8]) -> Result<(), ElfError> {
    let offset = usize_at(header, 40)?;
    let count = u16_at(header, 60) as usize;
    let entry_size = u16_at(header, 58);
    let names_index = u16_at(header, 62) as usize;
    if count == 0 {
        if offset != 0 || names_index != 0 || !matches!(entry_size, 0 | 64) {
            return Err(ElfError::Header);
        }
        return Ok(());
    }
    if count > MAX_SECTION_HEADERS {
        return Err(ElfError::Limit);
    }
    if offset < 64 || entry_size != 64 || names_index >= count {
        return Err(ElfError::Header);
    }
    for section in range(bytes, offset, count * 64)?.as_chunks::<64>().0 {
        let kind = u32_at(section, 4);
        if matches!(kind, 4 | 9 | 19) || u64_at(section, 8) & 0x400 != 0 {
            return Err(ElfError::Unsupported);
        }
        if !matches!(kind, 0 | 8) {
            range(bytes, usize_at(section, 24)?, usize_at(section, 32)?)?;
        }
    }
    Ok(())
}

fn range(bytes: &[u8], offset: usize, size: usize) -> Result<&[u8], ElfError> {
    let end = offset.checked_add(size).ok_or(ElfError::Bounds)?;
    bytes.get(offset..end).ok_or(ElfError::Bounds)
}

fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn u64_at(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

fn usize_at(bytes: &[u8], offset: usize) -> Result<usize, ElfError> {
    usize::try_from(u64_at(bytes, offset)).map_err(|_| ElfError::Bounds)
}
