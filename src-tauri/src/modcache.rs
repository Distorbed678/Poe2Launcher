//! The mod cache: `<data>/mods/cache/<id>/<version>/`, one folder per verified version, so two
//! servers can need different versions of the same mod. Each folder holds `mod/` (the mod's
//! client half, unpacked, `mod.toml` at its root) and `meta.json` (the package's SHA-256, size,
//! where it came from).
//!
//! Everything enters through [`ModCache::add_package`] (a zip, checked with
//! `poe_hook_mods::verify_package` / `verify_package_bytes` first: hash, names, paths,
//! symlinks, limits, CRCs) or [`ModCache::add_folder`] (a developer's mod folder). A mod
//! without a client half is refused; of a `both` mod only the client half is unpacked, so no
//! server file ever lands on the player's disk. Nothing is installed from a package that
//! failed a check: unpacking happens in a scratch folder that is renamed into place at the end.

use std::{
    io::Read,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use anyhow::{Context, Result, anyhow, bail};
use poe_hook_mods::{Limits, PackageReport, package_file_name, sha256_hex};
use semver::Version;
use serde::{Deserialize, Serialize};

use crate::{
    core::now_secs,
    mods::{
        MANIFEST_FILE, Manifest, Side, client_files_missing, ensure_client_half, has_code,
        is_server_path, parse_manifest, server_half_paths, valid_id,
    },
    paths::write_atomic,
};

const META_FILE: &str = "meta.json";
const TREE_DIR: &str = "mod";
const MAX_FILES: usize = 20_000;
static SCRATCH: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheMeta {
    pub id: String,
    pub version: String,
    /// `client` or `both` (a server-only mod is never cached).
    pub side: String,
    /// SHA-256 of the package zip; for a folder, a hash over its client files.
    pub sha256: String,
    /// Size of the package zip; for a folder, the sum of its client files.
    pub size: u64,
    /// `server`, `index`, `zip`, `folder`, `legacy`.
    pub source: String,
    pub added: i64,
    /// The client half runs native code.
    pub has_code: bool,
    /// The package had a server half that was not unpacked.
    pub server_half_left_out: bool,
}

/// A cached version, read back from disk.
#[derive(Debug, Clone)]
pub struct CachedMod {
    pub meta: CacheMeta,
    pub dir: PathBuf,
    pub manifest: Option<Manifest>,
    pub errors: Vec<String>,
    pub unknown_keys: Vec<String>,
}

impl CachedMod {
    pub fn tree(&self) -> PathBuf {
        self.dir.join(TREE_DIR)
    }
}

/// What adding a package or folder did.
#[derive(Debug, Clone, Serialize)]
pub struct Added {
    pub id: String,
    pub version: String,
    pub side: String,
    pub sha256: String,
    pub size: u64,
    /// The same version with the same hash was already cached; nothing changed.
    pub already_cached: bool,
    pub has_code: bool,
    pub server_half_left_out: bool,
    pub unknown_keys: Vec<String>,
    pub warnings: Vec<String>,
}

pub struct ModCache {
    root: PathBuf,
}

fn safe_version(v: &str) -> bool {
    Version::parse(v).is_ok() && !v.contains(['/', '\\'])
}

impl ModCache {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn entry_dir(&self, id: &str, version: &str) -> Result<PathBuf> {
        if !valid_id(id) || !safe_version(version) {
            bail!("{id:?} {version:?} is not a valid mod id and version");
        }
        Ok(self.root.join(id).join(version))
    }

    fn read_entry(&self, dir: PathBuf, id: &str, version: &str) -> Option<CachedMod> {
        let meta: CacheMeta =
            serde_json::from_str(&std::fs::read_to_string(dir.join(META_FILE)).ok()?).ok()?;
        if meta.id != id || meta.version != version {
            return None;
        }
        let tree = dir.join(TREE_DIR);
        let mut item = CachedMod {
            meta,
            dir,
            manifest: None,
            errors: Vec::new(),
            unknown_keys: Vec::new(),
        };
        match std::fs::read_to_string(tree.join(MANIFEST_FILE)) {
            Err(_) => item.errors.push("mod.toml is missing".into()),
            Ok(text) => match parse_manifest(&text) {
                Err(e) => item.errors = e,
                Ok(p) => {
                    item.errors = client_files_missing(&p.manifest, |rel| tree.join(rel).exists());
                    if p.manifest.id != id || p.manifest.version != version {
                        item.errors.push(format!(
                            "the cache entry is {id} {version} but its mod.toml says {} {}",
                            p.manifest.id, p.manifest.version
                        ));
                    }
                    item.unknown_keys = p.unknown_keys;
                    item.manifest = Some(p.manifest);
                }
            },
        }
        Some(item)
    }

    pub fn get(&self, id: &str, version: &str) -> Option<CachedMod> {
        let dir = self.entry_dir(id, version).ok()?;
        self.read_entry(dir, id, version)
    }

    /// Every cached version, sorted by id and then newest version first.
    pub fn list(&self) -> Vec<CachedMod> {
        let mut out = Vec::new();
        let Ok(ids) = std::fs::read_dir(&self.root) else {
            return out;
        };
        for id_entry in ids.flatten() {
            let id = id_entry.file_name().to_string_lossy().into_owned();
            if id.starts_with('.') || !valid_id(&id) || !id_entry.path().is_dir() {
                continue;
            }
            let Ok(versions) = std::fs::read_dir(id_entry.path()) else {
                continue;
            };
            for v in versions.flatten() {
                let version = v.file_name().to_string_lossy().into_owned();
                if version.starts_with('.') || !safe_version(&version) || !v.path().is_dir() {
                    continue;
                }
                if let Some(m) = self.read_entry(v.path(), &id, &version) {
                    out.push(m);
                }
            }
        }
        out.sort_by(|a, b| {
            a.meta.id.cmp(&b.meta.id).then_with(|| {
                match (
                    Version::parse(&b.meta.version),
                    Version::parse(&a.meta.version),
                ) {
                    (Ok(x), Ok(y)) => x.cmp(&y),
                    _ => b.meta.version.cmp(&a.meta.version),
                }
            })
        });
        out
    }

    /// Removes one cached version (and the mod's folder when it was the last one).
    pub fn remove(&self, id: &str, version: &str) -> Result<()> {
        let dir = self.entry_dir(id, version)?;
        if !dir.exists() {
            bail!("{id} {version} is not cached");
        }
        std::fs::remove_dir_all(&dir)?;
        let parent = self.root.join(id);
        if std::fs::read_dir(&parent).is_ok_and(|mut d| d.next().is_none()) {
            let _ = std::fs::remove_dir(&parent);
        }
        Ok(())
    }

    /// Total size of the cache on disk, in bytes.
    pub fn disk_bytes(&self) -> u64 {
        dir_bytes(&self.root)
    }

    /// Size of one cached version on disk.
    pub fn entry_bytes(&self, id: &str, version: &str) -> u64 {
        self.entry_dir(id, version)
            .map(|d| dir_bytes(&d))
            .unwrap_or(0)
    }

    fn scratch(&self) -> Result<PathBuf> {
        std::fs::create_dir_all(&self.root)?;
        let n = SCRATCH.fetch_add(1, Ordering::Relaxed);
        let dir = self
            .root
            .join(format!(".incoming-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    /// Adds a package zip. `expected_sha256` is the hash it must have (an advertisement or the
    /// index pins it); `file_name` the name it must have (`<id>-<version>.zip`). Without an
    /// expected hash, a `.sha256` next to the file is used when there is one (and then the name
    /// must match too); a bare local zip is still checked in full but its identity is unpinned
    /// (a warning says so). `replace`: a cached version with another hash is overwritten.
    pub fn add_package(
        &self,
        zip: &Path,
        expected_sha256: Option<&str>,
        source: &str,
        replace: bool,
    ) -> Result<Added> {
        let limits = Limits::default();
        let mut sidecar = zip.as_os_str().to_owned();
        sidecar.push(".sha256");
        let has_sidecar = Path::new(&sidecar).is_file();
        let mut extra_warnings = Vec::new();
        let report: PackageReport = if expected_sha256.is_none() && has_sidecar {
            poe_hook_mods::verify_package(zip, &limits)
        } else {
            let meta =
                std::fs::metadata(zip).with_context(|| format!("reading {}", zip.display()))?;
            if meta.len() > limits.max_package_bytes {
                bail!(
                    "the package is {} bytes, more than the limit of {}",
                    meta.len(),
                    limits.max_package_bytes
                );
            }
            let bytes = std::fs::read(zip).with_context(|| format!("reading {}", zip.display()))?;
            if expected_sha256.is_none() {
                extra_warnings.push(
                    "no .sha256 file next to the package: its identity is not pinned".to_owned(),
                );
            }
            // The name rule applies when the hash is pinned from outside (a download).
            let name = zip.file_name().and_then(|n| n.to_str());
            let name = if expected_sha256.is_some() {
                name
            } else {
                None
            };
            poe_hook_mods::verify_package_bytes(&bytes, name, expected_sha256, &limits)
        }
        .map_err(|e| anyhow!("the package was refused: {}", e.join("; ")))?;

        let m = report.manifest.clone();
        ensure_client_half(&m).map_err(|e| anyhow!(e))?;
        if let Some(name) = zip.file_name().and_then(|n| n.to_str())
            && expected_sha256.is_some()
            && name != package_file_name(&m.id, &m.version)
        {
            bail!(
                "the package is named {name:?} but contains {} {}",
                m.id,
                m.version
            );
        }
        let mut warnings = report.warnings.clone();
        warnings.extend(extra_warnings);
        self.install_from_zip(zip, &report, source, replace, warnings)
    }

    fn install_from_zip(
        &self,
        zip: &Path,
        report: &PackageReport,
        source: &str,
        replace: bool,
        warnings: Vec<String>,
    ) -> Result<Added> {
        let m = &report.manifest;
        let final_dir = self.entry_dir(&m.id, &m.version)?;
        let mut added = Added {
            id: m.id.clone(),
            version: m.version.clone(),
            side: m.side.as_str().into(),
            sha256: report.sha256.clone(),
            size: report.size,
            already_cached: false,
            has_code: has_code(m),
            server_half_left_out: m.side == Side::Both,
            unknown_keys: report.unknown_keys.clone(),
            warnings,
        };
        if let Some(have) = self.get(&m.id, &m.version)
            && have.errors.is_empty()
        {
            if have.meta.sha256 == report.sha256 {
                added.already_cached = true;
                return Ok(added);
            }
            if !replace {
                bail!(
                    "{} {} is already cached with other content (replace it to use this one)",
                    m.id,
                    m.version
                );
            }
        }
        let scratch = self.scratch()?;
        let result = (|| {
            let tree = scratch.join(TREE_DIR);
            unzip_client_half(zip, &tree, m)?;
            let meta = CacheMeta {
                id: m.id.clone(),
                version: m.version.clone(),
                side: m.side.as_str().into(),
                sha256: report.sha256.clone(),
                size: report.size,
                source: source.into(),
                added: now_secs(),
                has_code: has_code(m),
                server_half_left_out: m.side == Side::Both,
            };
            write_atomic(&scratch.join(META_FILE), &serde_json::to_vec_pretty(&meta)?)?;
            publish(&scratch, &final_dir)
        })();
        let _ = std::fs::remove_dir_all(&scratch);
        result?;
        Ok(added)
    }

    /// Adds a mod from a folder with a `mod.toml` at its root (a developer's mod). Only the
    /// client half is copied; links are refused.
    pub fn add_folder(&self, src: &Path, replace: bool) -> Result<Added> {
        let src = src
            .canonicalize()
            .with_context(|| format!("{} does not exist", src.display()))?;
        if let Ok(own) = self.root.canonicalize()
            && src.starts_with(&own)
        {
            bail!("that folder is already inside the mod cache");
        }
        let text = std::fs::read_to_string(src.join(MANIFEST_FILE))
            .context("mod.toml not found in the mod folder")?;
        let parsed =
            parse_manifest(&text).map_err(|e| anyhow!("invalid mod.toml: {}", e.join("; ")))?;
        let m = parsed.manifest;
        ensure_client_half(&m).map_err(|e| anyhow!(e))?;
        let missing = client_files_missing(&m, |rel| src.join(rel).exists());
        if !missing.is_empty() {
            bail!("the mod is incomplete: {}", missing.join("; "));
        }
        let final_dir = self.entry_dir(&m.id, &m.version)?;
        let scratch = self.scratch()?;
        let result = (|| {
            let tree = scratch.join(TREE_DIR);
            let server = server_half_paths(&m);
            let mut count = 0;
            copy_client_tree(&src, &src, &tree, &server, &mut count)?;
            let (sha256, size) = tree_hash(&tree)?;
            if let Some(have) = self.get(&m.id, &m.version)
                && have.errors.is_empty()
            {
                if have.meta.sha256 == sha256 {
                    return Ok(Added {
                        id: m.id.clone(),
                        version: m.version.clone(),
                        side: m.side.as_str().into(),
                        sha256,
                        size,
                        already_cached: true,
                        has_code: has_code(&m),
                        server_half_left_out: m.side == Side::Both,
                        unknown_keys: parsed.unknown_keys.clone(),
                        warnings: Vec::new(),
                    });
                }
                if !replace {
                    bail!(
                        "{} {} is already cached with other content (replace it to use this one)",
                        m.id,
                        m.version
                    );
                }
            }
            let meta = CacheMeta {
                id: m.id.clone(),
                version: m.version.clone(),
                side: m.side.as_str().into(),
                sha256: sha256.clone(),
                size,
                source: "folder".into(),
                added: now_secs(),
                has_code: has_code(&m),
                server_half_left_out: m.side == Side::Both,
            };
            write_atomic(&scratch.join(META_FILE), &serde_json::to_vec_pretty(&meta)?)?;
            publish(&scratch, &final_dir)?;
            Ok(Added {
                id: m.id.clone(),
                version: m.version.clone(),
                side: m.side.as_str().into(),
                sha256,
                size,
                already_cached: false,
                has_code: has_code(&m),
                server_half_left_out: m.side == Side::Both,
                unknown_keys: parsed.unknown_keys.clone(),
                warnings: vec![
                    "a folder has no package hash: its identity is a hash over its files".into(),
                ],
            })
        })();
        let _ = std::fs::remove_dir_all(&scratch);
        result
    }

    /// Copies the cached client half of `id` `version` to `dest` (which must not exist).
    pub fn materialise(&self, id: &str, version: &str, dest: &Path) -> Result<()> {
        let m = self
            .get(id, version)
            .ok_or_else(|| anyhow!("{id} {version} is not in the mod cache"))?;
        if !m.errors.is_empty() {
            bail!(
                "the cached copy of {id} {version} is damaged: {}",
                m.errors.join("; ")
            );
        }
        let mut count = 0;
        copy_plain_tree(&m.tree(), dest, &mut count)
    }
}

/// Moves the finished scratch folder to `final_dir`, replacing an old version.
fn publish(scratch: &Path, final_dir: &Path) -> Result<()> {
    if let Some(parent) = final_dir.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if final_dir.exists() {
        std::fs::remove_dir_all(final_dir)?;
    }
    if std::fs::rename(scratch, final_dir).is_err() {
        let mut count = 0;
        copy_plain_tree(scratch, final_dir, &mut count)?;
    }
    Ok(())
}

fn dir_bytes(dir: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return 0;
    };
    rd.flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => dir_bytes(&e.path()),
            Ok(t) if t.is_file() => e.metadata().map(|m| m.len()).unwrap_or(0),
            _ => 0,
        })
        .sum()
}

/// Unpacks the client half of a package that already passed verification (so paths, links and
/// sizes are known good); the declared sizes are still enforced while writing.
fn unzip_client_half(zip: &Path, dest: &Path, m: &Manifest) -> Result<()> {
    let server = server_half_paths(m);
    let limits = Limits::default();
    let mut archive = zip::ZipArchive::new(
        std::fs::File::open(zip).with_context(|| format!("opening {}", zip.display()))?,
    )
    .context("not a zip archive")?;
    std::fs::create_dir_all(dest)?;
    let mut total = 0u64;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let name = entry.name().trim_end_matches('/').to_owned();
        let rel = entry
            .enclosed_name()
            .ok_or_else(|| anyhow!("unsafe path in the archive: {name}"))?;
        if is_server_path(&server, &name) {
            continue;
        }
        let out = dest.join(&rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        total = total.saturating_add(entry.size());
        if total > limits.max_unpacked_bytes {
            bail!(
                "the archive unpacks to more than {} bytes",
                limits.max_unpacked_bytes
            );
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut f = std::fs::File::create(&out)?;
        let cap = entry.size() + 1;
        let copied = std::io::copy(&mut (&mut entry).take(cap), &mut f)?;
        if copied >= cap {
            bail!("{name} is longer than the archive declares");
        }
    }
    Ok(())
}

/// Copies a mod folder without the server half; refuses links; bounded.
fn copy_client_tree(
    root: &Path,
    src: &Path,
    dst: &Path,
    server: &[String],
    count: &mut usize,
) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        *count += 1;
        if *count > MAX_FILES {
            bail!("the mod folder has too many files");
        }
        let path = entry.path();
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        if is_server_path(server, &rel) {
            continue;
        }
        let ty = entry.file_type()?;
        let to = dst.join(entry.file_name());
        if ty.is_symlink() {
            bail!(
                "{} is a symbolic link; mods must not contain links",
                path.display()
            );
        } else if ty.is_dir() {
            copy_client_tree(root, &path, &to, server, count)?;
        } else {
            std::fs::copy(&path, &to)?;
        }
    }
    Ok(())
}

/// Copies a tree as it is (cache to game folder, scratch to final); refuses links; bounded.
pub fn copy_plain_tree(src: &Path, dst: &Path, count: &mut usize) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        *count += 1;
        if *count > MAX_FILES {
            bail!("the mod has too many files");
        }
        let ty = entry.file_type()?;
        let to = dst.join(entry.file_name());
        if ty.is_symlink() {
            bail!("{} is a symbolic link", entry.path().display());
        } else if ty.is_dir() {
            copy_plain_tree(&entry.path(), &to, count)?;
        } else {
            std::fs::copy(entry.path(), &to)?;
        }
    }
    Ok(())
}

/// A hash over a tree: sorted relative paths and contents. Returns the hash and the byte total.
fn tree_hash(root: &Path) -> Result<(String, u64)> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) -> Result<()> {
        for e in std::fs::read_dir(dir)? {
            let e = e?;
            if e.file_type()?.is_dir() {
                walk(base, &e.path(), out)?;
            } else {
                let rel = e
                    .path()
                    .strip_prefix(base)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push((rel, e.path()));
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(root, root, &mut files)?;
    files.sort();
    let mut all = Vec::new();
    let mut size = 0u64;
    for (rel, path) in files {
        let bytes = std::fs::read(&path)?;
        size += bytes.len() as u64;
        all.extend_from_slice(rel.as_bytes());
        all.push(0);
        all.extend_from_slice(sha256_hex(&bytes).as_bytes());
        all.push(b'\n');
    }
    Ok((sha256_hex(&all), size))
}

#[cfg(test)]
pub(crate) mod testutil {
    use std::io::Write;

    use super::*;

    /// Builds `<id>-<version>.zip` (+ `.sha256`) in `dir` from `(path, content)` files.
    pub fn package(dir: &Path, id: &str, version: &str, files: &[(&str, &[u8])]) -> PathBuf {
        let name = package_file_name(id, version);
        let path = dir.join(&name);
        let mut w = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (p, c) in files {
            w.start_file(*p, opts).unwrap();
            w.write_all(c).unwrap();
        }
        w.finish().unwrap();
        let hash = sha256_hex(&std::fs::read(&path).unwrap());
        std::fs::write(
            format!("{}.sha256", path.display()),
            poe_hook_mods::sha256_file_text(&hash, &name),
        )
        .unwrap();
        path
    }

    pub fn hash_of(path: &Path) -> String {
        sha256_hex(&std::fs::read(path).unwrap())
    }

    pub fn v1_manifest(id: &str, version: &str) -> String {
        format!(
            "manifest_version = 1\nid = \"{id}\"\nname = \"{id}\"\nversion = \"{version}\"\n[game]\nversions = [\"0.5.5e\"]\nhook_api = 1\n[entry]\ndll = \"{id}.dll\"\n"
        )
    }

    pub fn both_manifest(id: &str, version: &str) -> String {
        format!(
            "manifest_version = 2\nid = \"{id}\"\nname = \"{id}\"\nversion = \"{version}\"\nside = \"both\"\n[game]\nversions = [\"0.5.5e\"]\n[client]\ndll = \"client/{id}.dll\"\nhook_api = 1\n[server]\nserver_api = 1\n[server.library]\nlinux = \"server/lib{id}.so\"\nwindows = \"server/{id}.dll\"\n"
        )
    }

    pub fn server_manifest(id: &str, version: &str) -> String {
        format!(
            "manifest_version = 2\nid = \"{id}\"\nname = \"{id}\"\nversion = \"{version}\"\nside = \"server\"\n[server]\nserver_api = 1\n[server.library]\nlinux = \"server/lib{id}.so\"\n"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{testutil::*, *};

    fn cache() -> (tempfile::TempDir, ModCache) {
        let t = tempfile::tempdir().unwrap();
        let c = ModCache::new(t.path().join("cache"));
        (t, c)
    }

    #[test]
    fn a_client_package_is_verified_unpacked_and_listed() {
        let (t, c) = cache();
        let zip = package(
            t.path(),
            "hello-mod",
            "1.0.0",
            &[
                ("mod.toml", v1_manifest("hello-mod", "1.0.0").as_bytes()),
                ("hello-mod.dll", b"MZ-dll"),
                ("README.md", b"hi"),
            ],
        );
        let a = c.add_package(&zip, None, "zip", false).unwrap();
        assert!(!a.already_cached && a.has_code && a.side == "client" && !a.server_half_left_out);
        assert_eq!(a.sha256, hash_of(&zip));
        let list = c.list();
        assert_eq!(list.len(), 1);
        assert!(list[0].errors.is_empty(), "{:?}", list[0].errors);
        assert!(list[0].tree().join("hello-mod.dll").is_file());
        assert_eq!(list[0].meta.source, "zip");
        // The same package again changes nothing.
        assert!(
            c.add_package(&zip, None, "zip", false)
                .unwrap()
                .already_cached
        );
    }

    #[test]
    fn a_both_mod_is_unpacked_without_its_server_half() {
        let (t, c) = cache();
        let zip = package(
            t.path(),
            "dual-mod",
            "1.2.0",
            &[
                ("mod.toml", both_manifest("dual-mod", "1.2.0").as_bytes()),
                ("client/dual-mod.dll", b"MZ"),
                ("server/libdual-mod.so", b"ELF"),
                ("server/dual-mod.dll", b"MZ-server"),
                ("server/data/x.txt", b"x"),
            ],
        );
        let a = c.add_package(&zip, None, "server", false).unwrap();
        assert!(a.server_half_left_out && a.side == "both");
        let m = c.get("dual-mod", "1.2.0").unwrap();
        assert!(m.errors.is_empty(), "{:?}", m.errors);
        assert!(m.tree().join("client/dual-mod.dll").is_file());
        assert!(
            !m.tree().join("server").exists(),
            "no server file may land on the player's disk"
        );
        // The game folder gets the same.
        let dest = t.path().join("game-copy");
        c.materialise("dual-mod", "1.2.0", &dest).unwrap();
        assert!(dest.join("client/dual-mod.dll").is_file() && !dest.join("server").exists());
    }

    #[test]
    fn a_server_only_mod_is_refused_with_a_clear_message_and_nothing_is_stored() {
        let (t, c) = cache();
        let zip = package(
            t.path(),
            "srv-mod",
            "1.0.0",
            &[
                ("mod.toml", server_manifest("srv-mod", "1.0.0").as_bytes()),
                ("server/libsrv-mod.so", b"ELF"),
            ],
        );
        let e = c
            .add_package(&zip, None, "zip", false)
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("server-only") && e.contains("poe-server mod install"),
            "{e}"
        );
        assert!(c.list().is_empty());
        assert!(!c.root().join("srv-mod").exists());
        // A server-only folder too.
        let dir = t.path().join("srvdir");
        std::fs::create_dir_all(dir.join("server")).unwrap();
        std::fs::write(dir.join("mod.toml"), server_manifest("srv-mod", "1.0.0")).unwrap();
        std::fs::write(dir.join("server/libsrv-mod.so"), b"ELF").unwrap();
        assert!(
            c.add_folder(&dir, false)
                .unwrap_err()
                .to_string()
                .contains("server-only")
        );
    }

    #[test]
    fn a_failed_verification_installs_nothing() {
        let (t, c) = cache();
        let good = package(
            t.path(),
            "hello-mod",
            "1.0.0",
            &[
                ("mod.toml", v1_manifest("hello-mod", "1.0.0").as_bytes()),
                ("hello-mod.dll", b"MZ"),
            ],
        );
        // A wrong pinned hash.
        let e = c
            .add_package(&good, Some(&"0".repeat(64)), "server", false)
            .unwrap_err()
            .to_string();
        assert!(e.contains("SHA-256 mismatch"), "{e}");
        // A wrong checksum file.
        std::fs::write(
            format!("{}.sha256", good.display()),
            format!("{}  hello-mod-1.0.0.zip\n", "1".repeat(64)),
        )
        .unwrap();
        assert!(c.add_package(&good, None, "zip", false).is_err());
        // A manifest naming a file the zip lacks.
        let incomplete = package(
            t.path(),
            "lacking",
            "1.0.0",
            &[("mod.toml", v1_manifest("lacking", "1.0.0").as_bytes())],
        );
        assert!(c.add_package(&incomplete, None, "zip", false).is_err());
        // Not a zip at all.
        let junk = t.path().join("junk.zip");
        std::fs::write(&junk, b"not a zip").unwrap();
        assert!(c.add_package(&junk, None, "zip", false).is_err());
        assert!(c.list().is_empty());
        let leftovers: Vec<_> = std::fs::read_dir(c.root())
            .map(|d| d.flatten().collect())
            .unwrap_or_default();
        assert!(
            leftovers.is_empty(),
            "no scratch or half-installed folder may remain: {leftovers:?}"
        );
    }

    #[test]
    fn a_download_must_be_named_for_its_content() {
        let (t, c) = cache();
        let zip = package(
            t.path(),
            "hello-mod",
            "1.0.0",
            &[
                ("mod.toml", v1_manifest("hello-mod", "1.0.0").as_bytes()),
                ("hello-mod.dll", b"MZ"),
            ],
        );
        let renamed = t.path().join("other-name.zip");
        std::fs::copy(&zip, &renamed).unwrap();
        let h = hash_of(&zip);
        assert!(c.add_package(&renamed, Some(&h), "server", false).is_err());
        assert!(c.add_package(&zip, Some(&h), "server", false).is_ok());
    }

    #[test]
    fn several_versions_live_side_by_side_and_a_changed_hash_needs_replace() {
        let (t, c) = cache();
        for v in ["1.0.0", "1.10.0", "1.2.0"] {
            let z = package(
                t.path(),
                "hello-mod",
                v,
                &[
                    ("mod.toml", v1_manifest("hello-mod", v).as_bytes()),
                    ("hello-mod.dll", b"MZ"),
                ],
            );
            c.add_package(&z, None, "zip", false).unwrap();
        }
        let versions: Vec<String> = c.list().iter().map(|m| m.meta.version.clone()).collect();
        assert_eq!(
            versions,
            ["1.10.0", "1.2.0", "1.0.0"],
            "newest first, by semver"
        );
        // Same version, other content.
        let other_dir = t.path().join("o");
        std::fs::create_dir_all(&other_dir).unwrap();
        let z2 = package(
            &other_dir,
            "hello-mod",
            "1.0.0",
            &[
                ("mod.toml", v1_manifest("hello-mod", "1.0.0").as_bytes()),
                ("hello-mod.dll", b"MZ-changed"),
            ],
        );
        let e = c
            .add_package(&z2, None, "zip", false)
            .unwrap_err()
            .to_string();
        assert!(e.contains("already cached"), "{e}");
        c.add_package(&z2, None, "zip", true).unwrap();
        assert_eq!(
            c.get("hello-mod", "1.0.0").unwrap().meta.sha256,
            hash_of(&z2)
        );
        c.remove("hello-mod", "1.2.0").unwrap();
        assert_eq!(c.list().len(), 2);
        assert!(c.disk_bytes() > 0);
        c.remove("hello-mod", "1.0.0").unwrap();
        c.remove("hello-mod", "1.10.0").unwrap();
        assert!(!c.root().join("hello-mod").exists());
    }

    #[test]
    fn a_folder_is_cached_by_its_client_files() {
        let (t, c) = cache();
        let dir = t.path().join("devmod");
        std::fs::create_dir_all(dir.join("client")).unwrap();
        std::fs::create_dir_all(dir.join("server")).unwrap();
        std::fs::write(dir.join("mod.toml"), both_manifest("dev-mod", "0.1.0")).unwrap();
        std::fs::write(dir.join("client/dev-mod.dll"), b"MZ").unwrap();
        std::fs::write(dir.join("server/libdev-mod.so"), b"ELF").unwrap();
        let a = c.add_folder(&dir, false).unwrap();
        assert!(a.server_half_left_out);
        let m = c.get("dev-mod", "0.1.0").unwrap();
        assert!(m.errors.is_empty() && !m.tree().join("server").exists());
        assert!(c.add_folder(&dir, false).unwrap().already_cached);
        std::fs::write(dir.join("client/dev-mod.dll"), b"MZ2").unwrap();
        assert!(c.add_folder(&dir, false).is_err());
        assert!(c.add_folder(&dir, true).is_ok());
        // A missing DLL is refused.
        std::fs::remove_file(dir.join("client/dev-mod.dll")).unwrap();
        assert!(
            c.add_folder(&dir, true)
                .unwrap_err()
                .to_string()
                .contains("incomplete")
        );
    }

    #[test]
    fn a_damaged_cache_entry_is_reported_and_not_materialised() {
        let (t, c) = cache();
        let z = package(
            t.path(),
            "hello-mod",
            "1.0.0",
            &[
                ("mod.toml", v1_manifest("hello-mod", "1.0.0").as_bytes()),
                ("hello-mod.dll", b"MZ"),
            ],
        );
        c.add_package(&z, None, "zip", false).unwrap();
        let m = c.get("hello-mod", "1.0.0").unwrap();
        std::fs::remove_file(m.tree().join("hello-mod.dll")).unwrap();
        assert!(!c.get("hello-mod", "1.0.0").unwrap().errors.is_empty());
        assert!(
            c.materialise("hello-mod", "1.0.0", &t.path().join("out"))
                .unwrap_err()
                .to_string()
                .contains("damaged")
        );
        assert!(c.entry_dir("../x", "1.0.0").is_err());
        assert!(c.entry_dir("ok-mod", "../1").is_err());
    }
}
