//! The per-version install description, `versions/<version>.toml` (embedded at build time).

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;

/// Versions the launcher knows, newest last.
const EMBEDDED: &[(&str, &str)] = &[("0.5.5e", include_str!("../../versions/0.5.5e.toml"))];

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct VersionFile {
    pub version: String,
    pub app_id: u32,
    pub depots: Vec<Depot>,
    #[serde(default)]
    pub verify: VerifySpec,
}

/// One Steam depot. The order in [`VersionFile::depots`] is the install order.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Depot {
    pub id: u32,
    /// Steam manifest id (a u64, kept as text).
    pub manifest: String,
    #[serde(default)]
    pub role: String,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct VerifySpec {
    /// Client executable, relative to the install.
    #[serde(default)]
    pub exe: String,
    /// SHA-256 of the executable; absent or `TODO` = unknown (the check is skipped).
    #[serde(default)]
    pub exe_sha256: Option<String>,
    /// Whether a checksum mismatch fails the verification (otherwise it is a warning).
    #[serde(default)]
    pub exe_sha256_enforce: bool,
    #[serde(default)]
    pub min_files: u64,
    #[serde(default)]
    pub min_bytes: u64,
    #[serde(default)]
    pub files: Vec<KeyFile>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct KeyFile {
    pub path: String,
    pub size: u64,
}

impl VersionFile {
    pub fn parse(text: &str) -> Result<Self> {
        let v: VersionFile = toml::from_str(text).context("version file is not valid TOML")?;
        v.validate()?;
        Ok(v)
    }

    /// The embedded file for `version`.
    pub fn embedded(version: &str) -> Result<Self> {
        let text = EMBEDDED
            .iter()
            .find(|(v, _)| *v == version)
            .map(|(_, t)| *t)
            .with_context(|| format!("no version file for client {version}"))?;
        Self::parse(text)
    }

    /// The client version the launcher installs by default (the newest embedded one).
    pub fn latest() -> Result<Self> {
        let (v, _) = EMBEDDED.last().context("no embedded version files")?;
        Self::embedded(v)
    }

    pub fn known_versions() -> Vec<&'static str> {
        EMBEDDED.iter().map(|(v, _)| *v).collect()
    }

    fn validate(&self) -> Result<()> {
        ensure!(!self.version.trim().is_empty(), "version is empty");
        ensure!(self.app_id != 0, "app_id is missing");
        ensure!(!self.depots.is_empty(), "no depots listed");
        let mut seen = std::collections::HashSet::new();
        for d in &self.depots {
            ensure!(d.id != 0, "a depot has no id");
            ensure!(seen.insert(d.id), "depot {} is listed twice", d.id);
            if d.manifest.is_empty() || !d.manifest.chars().all(|c| c.is_ascii_digit()) {
                bail!("depot {}: manifest {:?} is not a number", d.id, d.manifest);
            }
            d.manifest
                .parse::<u64>()
                .with_context(|| format!("depot {}: manifest does not fit 64 bits", d.id))?;
        }
        let v = &self.verify;
        if let Some(h) = v.exe_sha256_hash()? {
            ensure!(!h.is_empty(), "exe_sha256 is empty");
        }
        for f in &v.files {
            ensure!(
                is_relative_inside(&f.path),
                "verify file {:?} must be a relative path inside the install",
                f.path
            );
        }
        ensure!(
            v.exe.is_empty() || is_relative_inside(&v.exe),
            "verify.exe must be a relative path"
        );
        Ok(())
    }
}

impl VerifySpec {
    /// The checksum as 64 lowercase hex digits, or `None` when unknown (`TODO`, empty, absent).
    pub fn exe_sha256_hash(&self) -> Result<Option<String>> {
        match self.exe_sha256.as_deref().map(str::trim) {
            None | Some("") => Ok(None),
            Some(t) if t.eq_ignore_ascii_case("todo") => Ok(None),
            Some(t) => {
                ensure!(
                    t.len() == 64 && t.chars().all(|c| c.is_ascii_hexdigit()),
                    "exe_sha256 must be 64 hex digits, TODO or absent"
                );
                Ok(Some(t.to_ascii_lowercase()))
            }
        }
    }
}

fn is_relative_inside(p: &str) -> bool {
    !p.is_empty()
        && !p.starts_with(['/', '\\'])
        && !p.contains(':')
        && !p.split(['/', '\\']).any(|c| c == ".." || c.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_0_5_5e_file_matches_the_plan() {
        let v = VersionFile::embedded("0.5.5e").unwrap();
        assert_eq!(v.app_id, 2694490);
        let depots: Vec<_> = v
            .depots
            .iter()
            .map(|d| (d.id, d.manifest.as_str()))
            .collect();
        // Install order: content first, the exe depot over it.
        assert_eq!(
            depots,
            [
                (2694491, "7366357142270402355"),
                (2694492, "246779027001700092")
            ]
        );
        assert_eq!(v.verify.exe, "PathOfExileSteam.exe");
        assert!(v.verify.exe_sha256_hash().unwrap().is_some());
        assert!(v.verify.min_files > 0 && !v.verify.files.is_empty());
        assert_eq!(VersionFile::latest().unwrap(), v);
    }

    fn base(extra: &str) -> String {
        format!("version = \"1\"\napp_id = 5\n[[depots]]\nid = 6\nmanifest = \"123\"\n{extra}")
    }

    #[test]
    fn minimal_file_parses() {
        let v = VersionFile::parse(&base("")).unwrap();
        assert_eq!(v.depots.len(), 1);
        assert_eq!(v.verify.exe_sha256_hash().unwrap(), None);
    }

    #[test]
    fn todo_checksum_means_unknown() {
        let v = VersionFile::parse(&base("[verify]\nexe_sha256 = \"TODO\"\n")).unwrap();
        assert_eq!(v.verify.exe_sha256_hash().unwrap(), None);
        let v = VersionFile::parse(&base(&format!(
            "[verify]\nexe_sha256 = \"{}\"\n",
            "AB".repeat(32)
        )))
        .unwrap();
        assert_eq!(
            v.verify.exe_sha256_hash().unwrap().unwrap(),
            "ab".repeat(32)
        );
    }

    #[test]
    fn bad_files_are_rejected() {
        assert!(
            VersionFile::parse("version = \"1\"").is_err(),
            "no app, no depots"
        );
        assert!(
            VersionFile::parse("version = \"1\"\napp_id = 5\n").is_err(),
            "no depots"
        );
        for bad in [
            "version = \"1\"\napp_id = 5\n[[depots]]\nid = 6\nmanifest = \"12x\"\n",
            "version = \"1\"\napp_id = 5\n[[depots]]\nid = 6\nmanifest = \"\"\n",
            "version = \"1\"\napp_id = 5\n[[depots]]\nid = 6\nmanifest = \"99999999999999999999\"\n",
            "version = \"1\"\napp_id = 5\n[[depots]]\nid = 6\nmanifest = \"1\"\n[[depots]]\nid = 6\nmanifest = \"2\"\n",
        ] {
            assert!(VersionFile::parse(bad).is_err(), "{bad}");
        }
        assert!(VersionFile::parse(&base("[verify]\nexe_sha256 = \"abc\"\n")).is_err());
        assert!(
            VersionFile::parse(&base("[[verify.files]]\npath = \"../x\"\nsize = 1\n")).is_err()
        );
        assert!(
            VersionFile::parse(&base(
                "[[verify.files]]\npath = \"/etc/passwd\"\nsize = 1\n"
            ))
            .is_err()
        );
        assert!(VersionFile::embedded("9.9.9").is_err());
    }
}
