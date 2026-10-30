//! Dependency-free Linux host tooling; the shared parser is authoritative.
#[path = "../shared/initramfs.rs"]
mod initramfs;
use initramfs::{Archive, Kind};
use std::os::{
    fd::{AsRawFd, FromRawFd},
    unix::fs::{DirBuilderExt, OpenOptionsExt},
};
use std::{
    env, fs,
    io::{self, Read, Write},
    path::Path,
};
const O_DIRECTORY: i32 = 0x10000;
const O_NOFOLLOW: i32 = 0x20000;
const O_CLOEXEC: i32 = 0x80000;
extern "C" {
    fn openat(fd: i32, name: *const std::ffi::c_char, flags: i32, mode: u32) -> i32;
    fn mkdirat(fd: i32, name: *const std::ffi::c_char, mode: u32) -> i32;
}
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
fn fail(message: &str) -> Box<dyn std::error::Error> {
    message.into()
}
fn read_bounded(path: &Path) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(O_NOFOLLOW | 0x800)
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err(fail("input must be a regular file"));
    }
    file.take(initramfs::MAX_SIZE as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > initramfs::MAX_SIZE {
        return Err(fail("archive/file exceeds 4 MiB"));
    }
    Ok(bytes)
}
fn validate(bytes: &[u8]) -> Result<Archive<'_>> {
    Archive::parse(bytes).map_err(|_| fail("invalid initramfs archive"))
}
fn collect(root: &Path, relative: &str, files: &mut Vec<(String, Option<Vec<u8>>)>) -> Result<()> {
    for item in fs::read_dir(root.join(relative))? {
        let item = item?;
        let name = item
            .file_name()
            .into_string()
            .map_err(|_| fail("non-ASCII source name"))?;
        let path = if relative.is_empty() {
            name
        } else {
            format!("{relative}/{name}")
        };
        if !initramfs::valid_path(&path) {
            return Err(fail("noncanonical source path"));
        }
        if files.len() >= initramfs::MAX_ENTRIES {
            return Err(fail("too many entries"));
        }
        let ty = item.file_type()?;
        if ty.is_dir() {
            files.push((path.clone(), None));
            collect(root, &path, files)?;
        } else if ty.is_file() {
            files.push((path, Some(read_bounded(&item.path())?)));
            if files
                .iter()
                .filter_map(|(_, data)| data.as_ref())
                .map(Vec::len)
                .sum::<usize>()
                > initramfs::MAX_SIZE
            {
                return Err(fail("archive exceeds 4 MiB"));
            }
        } else {
            return Err(fail("source contains a symlink or special file"));
        }
    }
    Ok(())
}
fn put32(bytes: &mut [u8], at: usize, value: usize) {
    bytes[at..at + 4].copy_from_slice(&(value as u32).to_le_bytes());
}
fn put64(bytes: &mut [u8], at: usize, value: usize) {
    bytes[at..at + 8].copy_from_slice(&(value as u64).to_le_bytes());
}
fn pad(bytes: &mut Vec<u8>) {
    bytes.resize((bytes.len() + 7) & !7, 0);
}
fn pack(root: &Path, output: &Path) -> Result<()> {
    if !fs::symlink_metadata(root)?.is_dir() {
        return Err(fail("source must be a directory, not a symlink"));
    }
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    if parent.canonicalize()?.starts_with(root.canonicalize()?) {
        return Err(fail("output must be outside source tree"));
    }
    let mut files = Vec::new();
    collect(root, "", &mut files)?;
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let mut bytes = vec![0; initramfs::HEADER + files.len() * initramfs::RECORD];
    let mut string_offset = 0;
    for (i, (path, data)) in files.iter().enumerate() {
        let at = initramfs::HEADER + i * initramfs::RECORD;
        put32(&mut bytes, at, string_offset);
        put32(&mut bytes, at + 4, path.len());
        put32(&mut bytes, at + 8, if data.is_some() { 1 } else { 2 });
        bytes.extend_from_slice(path.as_bytes());
        string_offset += path.len();
    }
    pad(&mut bytes);
    let data_start = bytes.len();
    for (i, (_, data)) in files.iter().enumerate() {
        if let Some(data) = data {
            pad(&mut bytes);
            let offset = bytes.len();
            if offset + data.len() > initramfs::MAX_SIZE {
                return Err(fail("archive exceeds 4 MiB"));
            }
            bytes.extend_from_slice(data);
            let at = initramfs::HEADER + i * initramfs::RECORD;
            put64(&mut bytes, at + 16, offset);
            put64(&mut bytes, at + 24, data.len());
        }
    }
    bytes[..8].copy_from_slice(initramfs::MAGIC);
    put32(&mut bytes, 8, 1);
    put32(&mut bytes, 12, files.len());
    put32(&mut bytes, 16, string_offset);
    put32(&mut bytes, 20, data_start);
    let total = bytes.len();
    put32(&mut bytes, 24, total);
    validate(&bytes)?;
    let name = output
        .file_name()
        .ok_or_else(|| fail("output filename required"))?
        .to_string_lossy();
    let temporary = parent.join(format!(".{name}.{}.tmp", std::process::id()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let result = (|| -> Result<()> {
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, output)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}
fn child(parent: &fs::File, name: &str, flags: i32, mode: u32) -> io::Result<fs::File> {
    let name = std::ffi::CString::new(name)?;
    // SAFETY: valid directory fd and NUL-terminated borrowed name; returned fd is owned.
    let fd = unsafe {
        openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            flags | O_NOFOLLOW | O_CLOEXEC,
            mode,
        )
    };
    if fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { fs::File::from_raw_fd(fd) })
    }
}
fn extract(archive: Archive<'_>, output: &Path) -> Result<()> {
    fs::DirBuilder::new().mode(0o700).create(output)?;
    let result = (|| -> Result<()> {
        let root = fs::OpenOptions::new()
            .read(true)
            .custom_flags(O_DIRECTORY | O_NOFOLLOW)
            .open(output)?;
        for entry in archive.entries() {
            let mut parent = root.try_clone()?;
            let mut components = entry.path.split('/').peekable();
            while let Some(name) = components.next() {
                if components.peek().is_some() {
                    parent = child(&parent, name, O_DIRECTORY, 0)?;
                } else if entry.kind == Kind::Directory {
                    let name = std::ffi::CString::new(name)?;
                    // SAFETY: parent is an opened directory and name is NUL-terminated.
                    if unsafe { mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o755) } != 0 {
                        return Err(io::Error::last_os_error().into());
                    }
                } else {
                    // Linux O_WRONLY | O_CREAT | O_EXCL: never follow or overwrite a leaf.
                    let mut file = child(&parent, name, 1 | 0x40 | 0x80, 0o644)?;
                    file.write_all(entry.data)?;
                }
            }
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(output);
    }
    result
}
fn run() -> Result<()> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    let command = args.first().and_then(|s| s.to_str()).unwrap_or("");
    if command == "pack" && args.len() == 3 {
        return pack(Path::new(&args[1]), Path::new(&args[2]));
    }
    if !matches!(command, "list" | "verify" | "extract")
        || args.len() != if command == "extract" { 3 } else { 2 }
    {
        return Err(fail("usage: initramfs pack ROOT OUTPUT | list ARCHIVE | verify ARCHIVE | extract ARCHIVE NEW_DEST"));
    }
    let bytes = read_bounded(Path::new(&args[1]))?;
    let archive = validate(&bytes)?;
    match command {
        "verify" => println!(
            "initramfs: valid ({} entries, {} bytes)",
            archive.entries().count(),
            bytes.len()
        ),
        "list" => {
            for entry in archive.entries() {
                println!(
                    "{}\t{}\t{}",
                    if entry.kind == Kind::Directory {
                        "dir"
                    } else {
                        "file"
                    },
                    entry.data.len(),
                    entry.path
                );
            }
        }
        "extract" => extract(archive, Path::new(&args[2]))?,
        _ => unreachable!(),
    }
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("initramfs: {error}");
        std::process::exit(1);
    }
}
