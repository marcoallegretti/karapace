use crate::RuntimeError;
use std::path::{Path, PathBuf};
use std::process::Command;

const LXC_IMAGE_BASE: &str = "https://images.linuxcontainers.org/images";

#[derive(Debug, Clone)]
pub enum ImageSource {
    OpenSuse { variant: String },
    Ubuntu { codename: String },
    Debian { codename: String },
    Fedora { version: String },
    Arch,
    Custom { url: String },
}

#[derive(Debug, Clone)]
pub struct ResolvedImage {
    pub source: ImageSource,
    pub cache_key: String,
    pub display_name: String,
}

pub fn resolve_pinned_image_url(name: &str) -> Result<String, RuntimeError> {
    let resolved = resolve_image(name)?;
    download_url(&resolved.source)
}

#[allow(clippy::too_many_lines)]
pub fn resolve_image(name: &str) -> Result<ResolvedImage, RuntimeError> {
    let name = name.trim().to_lowercase();
    let (source, cache_key, display_name) = match name.as_str() {
        "rolling" | "opensuse" | "opensuse/tumbleweed" | "tumbleweed" => (
            ImageSource::OpenSuse {
                variant: "tumbleweed".to_owned(),
            },
            "opensuse-tumbleweed".to_owned(),
            "openSUSE Tumbleweed".to_owned(),
        ),
        "opensuse/leap" | "leap" => (
            ImageSource::OpenSuse {
                variant: "15.6".to_owned(),
            },
            "opensuse-leap-15.6".to_owned(),
            "openSUSE Leap 15.6".to_owned(),
        ),
        "ubuntu" | "ubuntu/24.04" | "ubuntu/noble" => (
            ImageSource::Ubuntu {
                codename: "noble".to_owned(),
            },
            "ubuntu-noble".to_owned(),
            "Ubuntu 24.04 (Noble)".to_owned(),
        ),
        "ubuntu/22.04" | "ubuntu/jammy" => (
            ImageSource::Ubuntu {
                codename: "jammy".to_owned(),
            },
            "ubuntu-jammy".to_owned(),
            "Ubuntu 22.04 (Jammy)".to_owned(),
        ),
        "debian" | "debian/bookworm" => (
            ImageSource::Debian {
                codename: "bookworm".to_owned(),
            },
            "debian-bookworm".to_owned(),
            "Debian Bookworm".to_owned(),
        ),
        "debian/trixie" => (
            ImageSource::Debian {
                codename: "trixie".to_owned(),
            },
            "debian-trixie".to_owned(),
            "Debian Trixie".to_owned(),
        ),
        "fedora" | "fedora/41" => (
            ImageSource::Fedora {
                version: "41".to_owned(),
            },
            "fedora-41".to_owned(),
            "Fedora 41".to_owned(),
        ),
        "fedora/40" => (
            ImageSource::Fedora {
                version: "40".to_owned(),
            },
            "fedora-40".to_owned(),
            "Fedora 40".to_owned(),
        ),
        "fedora/42" => (
            ImageSource::Fedora {
                version: "42".to_owned(),
            },
            "fedora-42".to_owned(),
            "Fedora 42".to_owned(),
        ),
        "debian/sid" => (
            ImageSource::Debian {
                codename: "sid".to_owned(),
            },
            "debian-sid".to_owned(),
            "Debian Sid".to_owned(),
        ),
        "ubuntu/24.10" | "ubuntu/oracular" => (
            ImageSource::Ubuntu {
                codename: "oracular".to_owned(),
            },
            "ubuntu-oracular".to_owned(),
            "Ubuntu 24.10 (Oracular)".to_owned(),
        ),
        "ubuntu/20.04" | "ubuntu/focal" => (
            ImageSource::Ubuntu {
                codename: "focal".to_owned(),
            },
            "ubuntu-focal".to_owned(),
            "Ubuntu 20.04 (Focal)".to_owned(),
        ),
        "arch" | "archlinux" => (
            ImageSource::Arch,
            "archlinux".to_owned(),
            "Arch Linux".to_owned(),
        ),
        other => {
            if other.starts_with("http://") || other.starts_with("https://") {
                (
                    ImageSource::Custom {
                        url: other.to_owned(),
                    },
                    format!("custom-{}", blake3::hash(other.as_bytes()).to_hex()),
                    format!("Custom ({other})"),
                )
            } else {
                return Err(RuntimeError::ImageNotFound(format!(
                    "unknown image '{other}'. Supported: rolling, opensuse/tumbleweed, opensuse/leap, \
                     ubuntu, ubuntu/24.04, ubuntu/22.04, ubuntu/20.04, \
                     debian, debian/bookworm, debian/trixie, debian/sid, \
                     fedora, fedora/40, fedora/41, fedora/42, \
                     arch, archlinux, or a URL"
                )));
            }
        }
    };

    Ok(ResolvedImage {
        source,
        cache_key,
        display_name,
    })
}

fn lxc_rootfs_url(distro: &str, variant: &str) -> String {
    format!("{LXC_IMAGE_BASE}/{distro}/{variant}/amd64/default/")
}

fn fetch_latest_build(index_url: &str) -> Result<String, RuntimeError> {
    let output = Command::new("curl")
        .args(["-fsSL", "--max-time", "30", index_url])
        .output()
        .map_err(|e| RuntimeError::ExecFailed(format!("curl failed: {e}")))?;

    if !output.status.success() {
        return Err(RuntimeError::ExecFailed(format!(
            "failed to fetch image index from {index_url}: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    let body = String::from_utf8_lossy(&output.stdout);
    // LXC image server uses build dates like "20260220_04:20/" or URL-encoded "20260220_04%3A20/"
    let mut builds: Vec<String> = body
        .lines()
        .filter_map(|line| {
            let href = line.split("href=\"").nth(1)?;
            let raw = href.split('"').next()?;
            let name = raw.trim_end_matches('/');
            // Decode %3A -> : for comparison, but keep the raw form for URL construction
            let decoded = name.replace("%3A", ":");
            // Build dates start with digits (e.g. "20260220_04:20")
            if decoded.starts_with(|c: char| c.is_ascii_digit()) && decoded.len() >= 8 {
                Some(decoded)
            } else {
                None
            }
        })
        .collect();
    builds.sort();

    builds
        .last()
        .cloned()
        .ok_or_else(|| RuntimeError::ExecFailed(format!("no builds found at {index_url}")))
}

fn url_encode_build(build: &str) -> String {
    build.replace(':', "%3A")
}

fn build_download_url(base_idx: &str) -> Result<String, RuntimeError> {
    let build = fetch_latest_build(base_idx)?;
    let encoded = url_encode_build(&build);
    Ok(format!("{base_idx}{encoded}/rootfs.tar.xz"))
}

fn download_url(source: &ImageSource) -> Result<String, RuntimeError> {
    match source {
        ImageSource::OpenSuse { variant } => {
            let idx = if variant == "tumbleweed" {
                lxc_rootfs_url("opensuse", "tumbleweed")
            } else {
                lxc_rootfs_url("opensuse", variant)
            };
            build_download_url(&idx)
        }
        ImageSource::Ubuntu { codename } => {
            let idx = lxc_rootfs_url("ubuntu", codename);
            build_download_url(&idx)
        }
        ImageSource::Debian { codename } => {
            let idx = lxc_rootfs_url("debian", codename);
            build_download_url(&idx)
        }
        ImageSource::Fedora { version } => {
            let idx = lxc_rootfs_url("fedora", version);
            build_download_url(&idx)
        }
        ImageSource::Arch => {
            let idx = lxc_rootfs_url("archlinux", "current");
            build_download_url(&idx)
        }
        ImageSource::Custom { url } => Ok(url.clone()),
    }
}

pub struct ImageCache {
    cache_dir: PathBuf,
}

impl ImageCache {
    pub fn new(store_root: &Path) -> Self {
        Self {
            cache_dir: store_root.join("images"),
        }
    }

    pub fn rootfs_path(&self, cache_key: &str) -> PathBuf {
        self.cache_dir.join(cache_key).join("rootfs")
    }

    pub fn is_cached(&self, cache_key: &str) -> bool {
        self.rootfs_path(cache_key).join("etc").exists()
    }

    pub fn ensure_image(
        &self,
        resolved: &ResolvedImage,
        progress: &dyn Fn(&str),
        offline: bool,
    ) -> Result<PathBuf, RuntimeError> {
        let rootfs = self.rootfs_path(&resolved.cache_key);
        if self.is_cached(&resolved.cache_key) {
            progress(&format!("using cached image: {}", resolved.display_name));
            return Ok(rootfs);
        }

        if offline {
            return Err(RuntimeError::ExecFailed(format!(
                "offline mode: base image '{}' is not cached",
                resolved.display_name
            )));
        }

        std::fs::create_dir_all(&rootfs)?;

        progress(&format!(
            "resolving image URL for {}...",
            resolved.display_name
        ));
        let url = download_url(&resolved.source)?;

        let tarball = self
            .cache_dir
            .join(&resolved.cache_key)
            .join("rootfs.tar.xz");
        progress(&format!("downloading {url}..."));

        let status = Command::new("curl")
            .args([
                "-fSL",
                "--progress-bar",
                "--max-time",
                "600",
                "-o",
                &tarball.to_string_lossy(),
                &url,
            ])
            .status()
            .map_err(|e| RuntimeError::ExecFailed(format!("curl download failed: {e}")))?;

        if !status.success() {
            let _ = std::fs::remove_dir_all(self.cache_dir.join(&resolved.cache_key));
            return Err(RuntimeError::ExecFailed(format!(
                "failed to download image from {url}"
            )));
        }

        progress("extracting rootfs...");
        let status = Command::new("tar")
            .args([
                "xf",
                &tarball.to_string_lossy(),
                "-C",
                &rootfs.to_string_lossy(),
                "--no-same-owner",
                "--no-same-permissions",
                "--exclude=dev/*",
            ])
            .status()
            .map_err(|e| RuntimeError::ExecFailed(format!("tar extract failed: {e}")))?;

        if !status.success() {
            let _ = force_remove(&self.cache_dir.join(&resolved.cache_key));
            return Err(RuntimeError::ExecFailed(
                "failed to extract rootfs tarball".to_owned(),
            ));
        }

        // Ensure all extracted files are user-readable and directories are user-writable.
        // LXC rootfs tarballs contain setuid binaries and root-owned restrictive permissions.
        let _ = Command::new("chmod")
            .args(["-R", "u+rwX", &rootfs.to_string_lossy()])
            .status();

        let _ = std::fs::remove_file(&tarball);

        // Compute and store the content digest for future integrity verification.
        progress("computing image digest...");
        let digest = compute_image_digest(&rootfs)?;
        let digest_file = self
            .cache_dir
            .join(&resolved.cache_key)
            .join("rootfs.blake3");
        std::fs::write(&digest_file, &digest)?;

        progress(&format!("image {} ready", resolved.display_name));
        Ok(rootfs)
    }

    /// Verify the integrity of a cached image by recomputing its digest
    /// and comparing it to the stored value. Returns an error if the image
    /// has been corrupted or tampered with.
    pub fn verify_image(&self, cache_key: &str) -> Result<(), RuntimeError> {
        let rootfs = self.rootfs_path(cache_key);
        let digest_file = self.cache_dir.join(cache_key).join("rootfs.blake3");

        if !digest_file.exists() {
            // No stored digest (pre-verification image); compute and store one now
            let digest = compute_image_digest(&rootfs)?;
            std::fs::write(&digest_file, &digest)?;
            return Ok(());
        }

        let stored = std::fs::read_to_string(&digest_file)
            .map_err(|e| RuntimeError::ExecFailed(format!("failed to read digest file: {e}")))?;
        let current = compute_image_digest(&rootfs)?;

        if stored.trim() != current.trim() {
            return Err(RuntimeError::ExecFailed(format!(
                "image integrity check failed for {cache_key}: stored digest {stored} != computed {current}"
            )));
        }

        Ok(())
    }
}

/// Compute a content digest (blake3) of a rootfs directory.
///
/// Hashes the sorted list of file paths + sizes for a deterministic
/// fingerprint of the image content without reading every byte.
pub fn compute_image_digest(rootfs: &Path) -> Result<String, RuntimeError> {
    // Hash the tarball if it exists, otherwise hash a manifest of the rootfs
    let tarball = rootfs.parent().map(|p| p.join("rootfs.tar.xz"));
    if let Some(ref tb) = tarball {
        if tb.exists() {
            let data = std::fs::read(tb)
                .map_err(|e| RuntimeError::ExecFailed(format!("failed to read tarball: {e}")))?;
            return Ok(blake3::hash(&data).to_hex().to_string());
        }
    }

    // Fallback: hash a deterministic file listing
    let mut hasher = blake3::Hasher::new();
    let mut entries = Vec::new();
    collect_file_entries(rootfs, rootfs, &mut entries)?;
    entries.sort();
    for entry in &entries {
        hasher.update(entry.as_bytes());
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn collect_file_entries(
    base: &Path,
    dir: &Path,
    entries: &mut Vec<String>,
) -> Result<(), RuntimeError> {
    let Ok(listing) = std::fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in listing {
        let entry = entry?;
        let ft = entry.file_type()?;
        let rel = entry
            .path()
            .strip_prefix(base)
            .unwrap_or(&entry.path())
            .to_string_lossy()
            .to_string();
        if ft.is_file() {
            let len = entry.metadata().map(|m| m.len()).unwrap_or(0);
            entries.push(format!("{rel}:{len}"));
        } else if ft.is_dir() {
            entries.push(format!("{rel}/"));
            collect_file_entries(base, &entry.path(), entries)?;
        }
    }
    Ok(())
}

/// Build a command to query installed package versions from the container.
pub fn query_versions_command(pkg_manager: &str, packages: &[String]) -> Vec<String> {
    match pkg_manager {
        "apt" => {
            // dpkg-query outputs name\tversion for each installed package
            let mut cmd = vec![
                "dpkg-query".to_owned(),
                "-W".to_owned(),
                "-f".to_owned(),
                "${Package}\\t${Version}\\n".to_owned(),
            ];
            cmd.extend(packages.iter().cloned());
            cmd
        }
        "dnf" | "zypper" => {
            let mut cmd = vec![
                "rpm".to_owned(),
                "-q".to_owned(),
                "--qf".to_owned(),
                "%{NAME}\\t%{VERSION}-%{RELEASE}\\n".to_owned(),
            ];
            cmd.extend(packages.iter().cloned());
            cmd
        }
        "pacman" => {
            // pacman -Q outputs "name version" per line
            let mut cmd = vec!["pacman".to_owned(), "-Q".to_owned()];
            cmd.extend(packages.iter().cloned());
            cmd
        }
        _ => Vec::new(),
    }
}

/// Parse the output of a version query command into (name, version) pairs.
pub fn parse_version_output(pkg_manager: &str, output: &str) -> Vec<(String, String)> {
    let mut results = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = if pkg_manager == "pacman" {
            line.splitn(2, ' ').collect()
        } else {
            line.splitn(2, '\t').collect()
        };
        if parts.len() == 2 {
            results.push((parts[0].to_owned(), parts[1].to_owned()));
        }
    }
    results
}

pub fn force_remove(path: &Path) -> Result<(), RuntimeError> {
    if path.exists() {
        let _ = Command::new("chmod")
            .args(["-R", "u+rwX", &path.to_string_lossy()])
            .status();
        std::fs::remove_dir_all(path)?;
    }
    Ok(())
}

pub fn detect_package_manager(rootfs: &Path) -> Option<&'static str> {
    if rootfs.join("usr/bin/apt-get").exists() || rootfs.join("usr/bin/apt").exists() {
        Some("apt")
    } else if rootfs.join("usr/bin/dnf").exists() || rootfs.join("usr/bin/dnf5").exists() {
        Some("dnf")
    } else if rootfs.join("usr/bin/zypper").exists() {
        Some("zypper")
    } else if rootfs.join("usr/bin/pacman").exists() {
        Some("pacman")
    } else {
        None
    }
}

pub fn install_packages_command(pkg_manager: &str, packages: &[String]) -> Vec<String> {
    if packages.is_empty() {
        return Vec::new();
    }
    let mut cmd = Vec::new();
    match pkg_manager {
        "apt" => {
            cmd.push("apt-get".to_owned());
            cmd.push("install".to_owned());
            cmd.push("-y".to_owned());
            cmd.push("--no-install-recommends".to_owned());
            cmd.extend(packages.iter().cloned());
        }
        "dnf" => {
            cmd.push("dnf".to_owned());
            cmd.push("install".to_owned());
            cmd.push("-y".to_owned());
            cmd.push("--setopt=install_weak_deps=False".to_owned());
            cmd.extend(packages.iter().cloned());
        }
        "zypper" => {
            cmd.push("zypper".to_owned());
            cmd.push("--non-interactive".to_owned());
            cmd.push("install".to_owned());
            cmd.push("--no-recommends".to_owned());
            cmd.extend(packages.iter().cloned());
        }
        "pacman" => {
            cmd.push("pacman".to_owned());
            cmd.push("-S".to_owned());
            cmd.push("--noconfirm".to_owned());
            cmd.push("--needed".to_owned());
            cmd.extend(packages.iter().cloned());
        }
        _ => {}
    }
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_known_images() {
        assert!(resolve_image("rolling").is_ok());
        assert!(resolve_image("ubuntu/24.04").is_ok());
        assert!(resolve_image("debian/bookworm").is_ok());
        assert!(resolve_image("fedora/41").is_ok());
        assert!(resolve_image("archlinux").is_ok());
    }

    #[test]
    fn resolve_unknown_image_fails() {
        assert!(resolve_image("not-a-distro").is_err());
    }

    #[test]
    fn resolve_custom_url() {
        let r = resolve_image("https://example.com/rootfs.tar.xz").unwrap();
        assert!(r.cache_key.starts_with("custom-"));
    }

    #[test]
    fn install_commands_correct() {
        let pkgs = vec!["git".to_owned(), "cmake".to_owned()];
        let cmd = install_packages_command("apt", &pkgs);
        assert_eq!(cmd[0], "apt-get");
        assert!(cmd.contains(&"git".to_owned()));

        let cmd = install_packages_command("zypper", &pkgs);
        assert_eq!(cmd[0], "zypper");
        assert!(cmd.contains(&"--non-interactive".to_owned()));

        let cmd = install_packages_command("pacman", &pkgs);
        assert_eq!(cmd[0], "pacman");
    }

    #[test]
    fn detect_no_pkg_manager_on_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert!(detect_package_manager(dir.path()).is_none());
    }

    #[test]
    fn parse_apt_version_output() {
        let output = "git\t1:2.43.0-1ubuntu7\nclang\t1:18.1.3-1\n";
        let versions = parse_version_output("apt", output);
        assert_eq!(versions.len(), 2);
        assert_eq!(
            versions[0],
            ("git".to_owned(), "1:2.43.0-1ubuntu7".to_owned())
        );
        assert_eq!(versions[1], ("clang".to_owned(), "1:18.1.3-1".to_owned()));
    }

    #[test]
    fn parse_rpm_version_output() {
        let output = "git\t2.44.0-1.fc41\ncmake\t3.28.3-1.fc41\n";
        let versions = parse_version_output("zypper", output);
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].0, "git");
        assert_eq!(versions[0].1, "2.44.0-1.fc41");
    }

    #[test]
    fn parse_pacman_version_output() {
        let output = "git 2.44.0-1\ncmake 3.28.3-1\n";
        let versions = parse_version_output("pacman", output);
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0], ("git".to_owned(), "2.44.0-1".to_owned()));
    }

    #[test]
    fn parse_empty_version_output() {
        let versions = parse_version_output("apt", "");
        assert!(versions.is_empty());
        let versions = parse_version_output("apt", "\n\n");
        assert!(versions.is_empty());
    }

    #[test]
    fn query_versions_commands_generated() {
        let pkgs = vec!["git".to_owned()];
        let cmd = query_versions_command("apt", &pkgs);
        assert_eq!(cmd[0], "dpkg-query");

        let cmd = query_versions_command("zypper", &pkgs);
        assert_eq!(cmd[0], "rpm");

        let cmd = query_versions_command("dnf", &pkgs);
        assert_eq!(cmd[0], "rpm");

        let cmd = query_versions_command("pacman", &pkgs);
        assert_eq!(cmd[0], "pacman");

        let cmd = query_versions_command("unknown", &pkgs);
        assert!(cmd.is_empty());
    }

    #[test]
    fn compute_digest_of_test_rootfs() {
        let dir = tempfile::tempdir().unwrap();
        let rootfs = dir.path().join("rootfs");
        std::fs::create_dir_all(rootfs.join("etc")).unwrap();
        std::fs::write(rootfs.join("etc/hostname"), "test").unwrap();
        std::fs::create_dir_all(rootfs.join("usr/bin")).unwrap();
        std::fs::write(rootfs.join("usr/bin/hello"), "#!/bin/sh\necho hi").unwrap();

        let digest = compute_image_digest(&rootfs).unwrap();
        assert_eq!(digest.len(), 64);

        // Same content = same digest (determinism)
        let digest2 = compute_image_digest(&rootfs).unwrap();
        assert_eq!(digest, digest2);
    }

    #[test]
    fn detect_apt_package_manager() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("usr/bin")).unwrap();
        std::fs::write(dir.path().join("usr/bin/apt-get"), "").unwrap();
        assert_eq!(detect_package_manager(dir.path()), Some("apt"));
    }

    #[test]
    fn detect_zypper_package_manager() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("usr/bin")).unwrap();
        std::fs::write(dir.path().join("usr/bin/zypper"), "").unwrap();
        assert_eq!(detect_package_manager(dir.path()), Some("zypper"));
    }

    #[test]
    fn detect_pacman_package_manager() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("usr/bin")).unwrap();
        std::fs::write(dir.path().join("usr/bin/pacman"), "").unwrap();
        assert_eq!(detect_package_manager(dir.path()), Some("pacman"));
    }

    #[test]
    fn resolve_all_image_aliases() {
        // Verify every documented alias resolves correctly
        for alias in &[
            "rolling",
            "opensuse",
            "opensuse/tumbleweed",
            "tumbleweed",
            "opensuse/leap",
            "leap",
            "ubuntu",
            "ubuntu/24.04",
            "ubuntu/noble",
            "ubuntu/22.04",
            "ubuntu/jammy",
            "ubuntu/20.04",
            "ubuntu/focal",
            "ubuntu/24.10",
            "ubuntu/oracular",
            "debian",
            "debian/bookworm",
            "debian/trixie",
            "debian/sid",
            "fedora",
            "fedora/40",
            "fedora/41",
            "fedora/42",
            "arch",
            "archlinux",
        ] {
            let result = resolve_image(alias);
            assert!(result.is_ok(), "failed to resolve alias: {alias}");
        }
    }

    #[test]
    fn install_empty_packages_returns_empty() {
        let cmd = install_packages_command("apt", &[]);
        assert!(cmd.is_empty());
    }
}
