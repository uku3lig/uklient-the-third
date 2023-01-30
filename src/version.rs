use std::fmt::Display;

use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MinecraftVersion {
    pub minor: u8,
    pub patch: u8,
}

#[derive(Debug, thiserror::Error)]
pub enum VersionError {
    #[error("Invalid {0} version")]
    InvalidVersion(&'static str),
    #[error("Snapshots are unsupported")]
    SnapshotsAreUnsupported,
}

impl MinecraftVersion {
    pub fn parse(source: &str) -> Result<Self, VersionError> {
        let snapshot_regex = Regex::new(r"\d+w\d{2}[a-z]").unwrap();
        if snapshot_regex.find(source).is_some() {
            return Err(VersionError::SnapshotsAreUnsupported);
        }

        let mut parts = source.split('.');

        if parts
            .next()
            .and_then(|s| s.parse::<u8>().ok())
            .filter(|&n| n == 1)
            .is_none()
        {
            return Err(VersionError::InvalidVersion("major"));
        }

        let minor: u8 = match parts.next().and_then(|s| s.parse::<u8>().ok()) {
            Some(n) => n,
            None => return Err(VersionError::InvalidVersion("minor")),
        };

        let patch: u8 = match parts.next().map(|s| s.parse::<u8>().ok()) {
            Some(Some(n)) => n,
            Some(None) => return Err(VersionError::InvalidVersion("patch")),
            None => 0, // no patch version specified, so 0
        };

        Ok(MinecraftVersion { minor, patch })
    }
}

impl Display for MinecraftVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.patch != 0 {
            write!(f, "1.{}.{}", self.minor, self.patch)
        } else {
            write!(f, "1.{}", self.minor)
        }
    }
}

impl PartialEq for MinecraftVersion {
    fn eq(&self, other: &Self) -> bool {
        self.minor == other.minor && self.patch == other.patch
    }
}

impl PartialOrd for MinecraftVersion {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        match self.minor.partial_cmp(&other.minor) {
            Some(core::cmp::Ordering::Equal) => {}
            ord => return ord,
        }
        self.patch.partial_cmp(&other.patch)
    }
}
