//! Explicit delegated slots for console services; independent of startup stack layout.
#![allow(dead_code)]
pub const SIZE: usize = 32;
pub const ECHO: u32 = 1;
pub const SERIAL: u32 = 2;
pub const CONSOLE: u32 = 3;
pub const KEYBOARD: u32 = 4;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Roles {
    pub role: u32,
    pub handles: [u32; 4],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Error;
impl Roles {
    pub fn count(self) -> Result<usize, Error> {
        match self.role {
            ECHO => Ok(1),
            SERIAL | KEYBOARD => Ok(2),
            CONSOLE => Ok(3),
            _ => Err(Error),
        }
    }
    pub fn validate(self) -> Result<(), Error> {
        let count = self.count()?;
        if self.handles[count..].iter().any(|h| *h != 0)
            || self.handles[..count]
                .iter()
                .enumerate()
                .any(|(i, h)| self.handles[..i].contains(h))
        {
            return Err(Error);
        }
        Ok(())
    }
    pub fn encode(self) -> Result<[u8; SIZE], Error> {
        self.validate()?;
        let mut bytes = [0; SIZE];
        bytes[..8].copy_from_slice(b"FRROLE01");
        bytes[8..12].copy_from_slice(&self.role.to_le_bytes());
        bytes[12..16].copy_from_slice(&(self.count()? as u32).to_le_bytes());
        for (i, h) in self.handles.into_iter().enumerate() {
            bytes[16 + i * 4..20 + i * 4].copy_from_slice(&h.to_le_bytes());
        }
        Ok(bytes)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() != SIZE || &bytes[..8] != b"FRROLE01" {
            return Err(Error);
        }
        let word = |n| u32::from_le_bytes(bytes[n..n + 4].try_into().unwrap());
        let roles = Self {
            role: word(8),
            handles: core::array::from_fn(|i| word(16 + i * 4)),
        };
        if word(12) as usize != roles.count()? {
            return Err(Error);
        }
        roles.validate()?;
        Ok(roles)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_roles_allow_slot_zero_without_implicit_slots() {
        for role in [ECHO, SERIAL, CONSOLE, KEYBOARD] {
            let count = Roles {
                role,
                handles: [0; 4],
            }
            .count()
            .unwrap();
            let roles = Roles {
                role,
                handles: core::array::from_fn(|i| if i < count { i as u32 } else { 0 }),
            };
            let bytes = roles.encode().unwrap();
            assert_eq!(Roles::decode(&bytes), Ok(roles));
            for len in 0..SIZE {
                assert!(Roles::decode(&bytes[..len]).is_err());
            }
            let mut extra = [0; SIZE + 1];
            extra[..SIZE].copy_from_slice(&bytes);
            assert!(Roles::decode(&extra).is_err());
            for at in [0, 8, 12] {
                let mut bad = bytes;
                bad[at] ^= 0x80;
                assert!(Roles::decode(&bad).is_err());
            }
        }
        assert!(Roles {
            role: SERIAL,
            handles: [7, 7, 0, 0]
        }
        .encode()
        .is_err());
        assert!(Roles {
            role: ECHO,
            handles: [7, 0, 1, 0]
        }
        .encode()
        .is_err());
        assert!(Roles {
            role: 0,
            handles: [0; 4]
        }
        .encode()
        .is_err());
    }
}
