//! Required boot services, deliberately bounded and without dependency/restart policy.
#![allow(dead_code)]
#[derive(Clone, Copy, Debug)]
pub struct Service<'a> {
    pub name: &'a str,
    pub path: &'a str,
    pub filesystem: bool,
}
#[derive(Debug)]
pub struct ConfigError;
pub fn parse<'a>(b: &'a [u8], out: &mut [Option<Service<'a>>; 8]) -> Result<usize, ConfigError> {
    if b.len() > 4096 {
        return Err(ConfigError);
    }
    let text = core::str::from_utf8(b).map_err(|_| ConfigError)?;
    let mut version = false;
    let mut count = 0;
    for line in text.lines() {
        if line.len() > 320 {
            return Err(ConfigError);
        }
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut words = line.split_ascii_whitespace();
        if !version {
            if words.next() != Some("version")
                || words.next() != Some("1")
                || words.next().is_some()
            {
                return Err(ConfigError);
            }
            version = true;
            continue;
        }
        if words.next() != Some("service") {
            return Err(ConfigError);
        }
        let name = words.next().ok_or(ConfigError)?;
        let path = words.next().ok_or(ConfigError)?;
        let mode = words.next().ok_or(ConfigError)?;
        if words.next().is_some()
            || count == 8
            || name.is_empty()
            || name.len() > 31
            || !name
                .bytes()
                .all(|x| x.is_ascii_alphanumeric() || matches!(x, b'-' | b'_'))
            || !super::initramfs::valid_path(path)
            || !matches!(mode, "fs" | "none")
            || out[..count].iter().flatten().any(|s| s.name == name)
        {
            return Err(ConfigError);
        }
        out[count] = Some(Service {
            name,
            path,
            filesystem: mode == "fs",
        });
        count += 1;
    }
    if !version || count == 0 {
        return Err(ConfigError);
    }
    Ok(count)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn config() {
        let mut out = [None; 8];
        assert_eq!(parse(b"version 1\nservice alpha libexec/services/alpha fs\nservice beta libexec/services/beta none\n",&mut out).unwrap(),2);
        for b in [
            &b"version 2\n"[..],
            b"version 1\nservice a ../x fs\n",
            b"version 1\nservice a x fs\nservice a y none\n",
            b"version 1\nservice a x all\n",
        ] {
            assert!(parse(b, &mut [None; 8]).is_err());
        }
    }
}
