//! Assignment lifecycle. No state transition treats a failed reset as quiescence.
#![allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum State {
    Isolated = 1,
    Running = 2,
    Stopped = 3,
    Quarantined = 4,
    Revoked = 5,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid,
    Permission,
    Busy,
    Quota,
    Unsupported,
    Hardware,
    Quarantined,
    Revoked,
}
impl Error {
    pub const fn errno(self) -> isize {
        match self {
            Self::Invalid => -22,
            Self::Permission => -13,
            Self::Busy => -16,
            Self::Quota => -28,
            Self::Unsupported => -95,
            Self::Hardware => -5,
            Self::Quarantined => -5,
            Self::Revoked => -116,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Lifecycle {
    state: State,
}
impl Lifecycle {
    pub const fn isolated() -> Self {
        Self {
            state: State::Isolated,
        }
    }
    pub const fn state(&self) -> State {
        self.state
    }
    pub fn configurable(&self) -> Result<(), Error> {
        match self.state {
            State::Isolated | State::Stopped => Ok(()),
            State::Running => Err(Error::Busy),
            State::Quarantined => Err(Error::Quarantined),
            State::Revoked => Err(Error::Revoked),
        }
    }
    pub fn started(&mut self) -> Result<(), Error> {
        self.configurable()?;
        self.state = State::Running;
        Ok(())
    }
    pub fn stopped(&mut self) -> Result<(), Error> {
        if matches!(self.state, State::Quarantined | State::Revoked) {
            return self.configurable();
        }
        self.state = State::Stopped;
        Ok(())
    }
    pub fn quarantine(&mut self) {
        if self.state != State::Revoked {
            self.state = State::Quarantined;
        }
    }
    pub fn revoke(&mut self) -> Result<(), Error> {
        self.configurable()?;
        self.state = State::Revoked;
        Ok(())
    }
}
