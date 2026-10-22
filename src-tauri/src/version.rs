//! The per-version install description, `versions/<version>.toml` (embedded at build time).
//!
//! The files are byte-identical copies of the main repository's `versions/<version>.toml`
//! (Poe2ModdingProject), the single definition of a supported build; `cargo xtask versions`
//! there refreshes and checks them. This launcher reads the depots, the `[exe]` section and
//! `[verify]`; the other sections (protocol, signatures, client hashes, data checks) are for
//! the server and `cargo xtask check-build` and are ignored here. `build.rs` embeds every
//! file, newest `build_revision` last.

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;

include!(concat!(env!("OUT_DIR"), "/embedded_versions.rs"));

/// The pseudo version "Official (read-only)": the current official build from Steam, played on
/// the official servers with the hook in its read-only observation mode. Not a version file.
pub const OFFICIAL: &str = "official";

/// A version file as it is written.
#[derive(Debug, Deserialize)]
struct RawVersionFile {
    version: String,
    app_id: u32,
    depots: Vec<Depot>,
    #[serde(default)]
    exe: RawExe,
    #[serde(default)]
    verify: RawVerify,
}

#[derive(Debug, Default, Deserialize)]
struct RawExe {
    #[serde(default)]
    name: String,
    #[serde(default)]
    sha256: Option<String>,
    #[serde(default)]
    sha256_enforce: bool,
}

#[derive(Debug, Default, Deserialize)]
struct RawVerify {
    #[serde(default)]
    min_files: u64,
    #[serde(default)]
    min_bytes: u64,
    #[serde(default)]
    files: Vec<KeyFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionFile {
    pub version: String,
    pub app_id: u32,
    pub depots: Vec<Depot>,
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

/// `[exe]` and `[verify]` of the version file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VerifySpec {
    /// Client executable (`[exe] name`), relative to the install.
    pub exe: String,
    /// SHA-256 of the executable (`[exe] sha256`); absent or `TODO` = unknown (the check is
    /// skipped).
    pub exe_sha256: Option<String>,
    /// Whether a checksum mismatch fails the verification (otherwise it is a warning).
    pub exe_sha256_enforce: bool,
    pub min_files: u64,
    pub min_bytes: u64,
    pub files: Vec<KeyFile>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct KeyFile {
    pub path: String,
    pub size: u64,
}

impl VersionFile {
    pub fn parse(text: &str) -> Result<Self> {
        let raw: RawVersionFile = toml::from_str(text).context("version file is not valid TOML")?;
        let v = VersionFile {
            version: raw.version,
            app_id: raw.app_id,
            depots: raw.depots,
            verify: VerifySpec {
                exe: raw.exe.name,
                exe_sha256: raw.exe.sha256,
                exe_sha256_enforce: raw.exe.sha256_enforce,
                min_files: raw.verify.min_files,
                min_bytes: raw.verify.min_bytes,
                files: raw.verify.files,
            },
        };
        v.validate()?;
        Ok(v)
    }

    /// The embedded file for `version`.
    pub fn embedded(version: &str) -> Result<Self> {
        let text = EMBEDDED_VERSIONS
            .iter()
            .find(|(v, _)| *v == version)
            .map(|(_, t)| *t)
            .with_context(|| format!("no version file for client {version}"))?;
        Self::parse(text)
    }

    /// The client version the launcher installs by default (the newest embedded one).
    pub fn latest() -> Result<Self> {
        let (v, _) = EMBEDDED_VERSIONS
            .last()
            .context("no embedded version files")?;
        Self::embedded(v)
    }

    /// The "Official (read-only)" target: the newest version file's depots and executable, but
    /// no pinned manifests (DepotDownloader then fetches the current build) and no per-file size
    /// or checksum expectations (they belong to the pinned build). Its `version` is
    /// [`OFFICIAL`].
    pub fn official() -> Result<Self> {
        let mut v = Self::latest()?;
        v.version = OFFICIAL.to_owned();
        for d in &mut v.depots {
            d.manifest.clear();
        }
        v.verify.files.clear();
        v.verify.exe_sha256 = None;
        v.verify.exe_sha256_enforce = false;
        Ok(v)
    }

    pub fn is_official(&self) -> bool {
        self.version == OFFICIAL
    }

    /// A value the `game_version` setting may hold: a supported version or [`OFFICIAL`].
    pub fn is_selectable(version: &str) -> bool {
        version == OFFICIAL || Self::known_versions().contains(&version)
    }

    /// The version the user picked, or the newest embedded one when none is picked (or the
    /// picked one is no longer supported by this build).
    pub fn resolve(requested: Option<&str>) -> Result<Self> {
        match requested.map(str::trim).filter(|v| !v.is_empty()) {
            Some(OFFICIAL) => Self::official(),
            Some(v) if Self::known_versions().contains(&v) => Self::embedded(v),
            _ => Self::latest(),
        }
    }

    /// Every supported client version, oldest first (`build_revision` order).
    pub fn known_versions() -> Vec<&'static str> {
        EMBEDDED_VERSIONS.iter().map(|(v, _)| *v).collect()
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
    fn every_embedded_file_parses_and_the_newest_is_the_default() {
        let known = VersionFile::known_versions();
        assert!(!known.is_empty(), "versions/ has no version file");
        for version in &known {
            let v = VersionFile::embedded(version).unwrap();
            assert_eq!(&v.version, version);
            assert!(v.app_id != 0);
            // Install order: content first, the exe depot over it.
            assert!(v.depots.len() >= 2, "content depot, then the exe depot");
            assert!(!v.verify.exe.is_empty());
            assert!(v.verify.exe_sha256_hash().unwrap().is_some());
            assert!(v.verify.min_files > 0 && !v.verify.files.is_empty());
            // The exe is among the key files, with the size the main repo records.
            assert!(v.verify.files.iter().any(|f| f.path == v.verify.exe));
        }
        let latest = VersionFile::latest().unwrap();
        assert_eq!(&latest.version, known.last().unwrap());
    }

    #[test]
    fn resolve_falls_back_to_the_newest() {
        let latest = VersionFile::latest().unwrap().version;
        for req in [None, Some(""), Some("  "), Some("0.0.1-gone")] {
            assert_eq!(VersionFile::resolve(req).unwrap().version, latest);
        }
        for v in VersionFile::known_versions() {
            assert_eq!(VersionFile::resolve(Some(v)).unwrap().version, v);
        }
    }

    #[test]
    fn official_has_the_depots_but_no_pins() {
        let o = VersionFile::official().unwrap();
        let l = VersionFile::latest().unwrap();
        assert!(o.is_official() && !l.is_official());
        assert_eq!(o.app_id, l.app_id);
        assert_eq!(
            o.depots.iter().map(|d| d.id).collect::<Vec<_>>(),
            l.depots.iter().map(|d| d.id).collect::<Vec<_>>()
        );
        assert!(o.depots.iter().all(|d| d.manifest.is_empty()));
        assert!(o.verify.files.is_empty() && o.verify.exe_sha256.is_none());
        assert_eq!(o.verify.exe, l.verify.exe);
        assert_eq!(VersionFile::resolve(Some("official")).unwrap(), o);
        assert!(VersionFile::is_selectable("official"));
        assert!(!VersionFile::known_versions().contains(&"official"));
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
        let v = VersionFile::parse(&base("[exe]\nsha256 = \"TODO\"\n")).unwrap();
        assert_eq!(v.verify.exe_sha256_hash().unwrap(), None);
        let v = VersionFile::parse(&base(&format!("[exe]\nsha256 = \"{}\"\n", "AB".repeat(32))))
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
        assert!(VersionFile::parse(&base("[exe]\nsha256 = \"abc\"\n")).is_err());
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
