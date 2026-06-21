use std::fmt;
use std::str::FromStr;

/// Version of a service contract.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ContractVersion {
    /// Major version number.
    pub major: u32,
    /// Minor version number.
    pub minor: u32,
    /// Patch version number.
    pub patch: u32,
}

impl ContractVersion {
    /// Creates a new contract version.
    pub fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// Returns the semver representation of this version.
    pub fn to_semver(&self) -> semver::Version {
        semver::Version::new(self.major as u64, self.minor as u64, self.patch as u64)
    }
}

impl fmt::Display for ContractVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl FromStr for ContractVersion {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() != 3 {
            return Err("Invalid version format. Expected format: major.minor.patch".to_string());
        }

        let major = parts[0]
            .parse::<u32>()
            .map_err(|_| "Invalid major version")?;
        let minor = parts[1]
            .parse::<u32>()
            .map_err(|_| "Invalid minor version")?;
        let patch = parts[2]
            .parse::<u32>()
            .map_err(|_| "Invalid patch version")?;

        Ok(ContractVersion::new(major, minor, patch))
    }
}

impl PartialOrd for ContractVersion {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ContractVersion {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.major
            .cmp(&other.major)
            .then_with(|| self.minor.cmp(&other.minor))
            .then_with(|| self.patch.cmp(&other.patch))
    }
}

/// A version constraint for resolving service implementations.
#[derive(Debug, Clone)]
pub enum VersionConstraint {
    /// Exact version match.
    Exact(ContractVersion),
    /// Semver range match — picks the highest satisfying version.
    LatestCompatible(semver::VersionReq),
}

impl VersionConstraint {
    /// Creates an exact version constraint from a string.
    pub fn exact(s: &str) -> Result<Self, String> {
        let v = s.parse::<ContractVersion>()?;
        Ok(VersionConstraint::Exact(v))
    }

    /// Creates a semver range constraint from a string.
    pub fn range(s: &str) -> Result<Self, String> {
        semver::VersionReq::parse(s)
            .map(VersionConstraint::LatestCompatible)
            .map_err(|e| e.to_string())
    }

    /// Checks whether the given `ContractVersion` satisfies this constraint.
    pub fn matches(&self, version: &ContractVersion) -> bool {
        match self {
            VersionConstraint::Exact(v) => v == version,
            VersionConstraint::LatestCompatible(req) => req.matches(&version.to_semver()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_contract_version_new() {
        let version = ContractVersion::new(1, 2, 3);
        assert_eq!(version.major, 1);
        assert_eq!(version.minor, 2);
        assert_eq!(version.patch, 3);
    }

    #[test]
    fn test_contract_version_display() {
        let version = ContractVersion::new(1, 2, 3);
        assert_eq!(format!("{}", version), "1.2.3");
    }

    #[test]
    fn test_version_constraint_exact_matches() {
        let v = ContractVersion::new(1, 0, 0);
        let constraint = VersionConstraint::Exact(v.clone());
        assert!(constraint.matches(&v));
        assert!(!constraint.matches(&ContractVersion::new(1, 0, 1)));
    }

    #[test]
    fn test_version_constraint_range_matches() {
        let constraint = VersionConstraint::range("^1").unwrap();
        assert!(constraint.matches(&ContractVersion::new(1, 0, 0)));
        assert!(constraint.matches(&ContractVersion::new(1, 5, 3)));
        assert!(!constraint.matches(&ContractVersion::new(2, 0, 0)));
    }
}
