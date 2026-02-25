use crate::RuntimeError;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

fn shell_quote(s: &str) -> String {
    // Single-quoting in POSIX shell: replace ' with '\'' then wrap in '
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Shell-escape a Path for safe interpolation.
fn shell_quote_path(p: &Path) -> String {
    shell_quote(&p.to_string_lossy())
}

#[derive(Debug, Clone)]
pub struct BindMount {
    pub source: PathBuf,
    pub target: PathBuf,
    pub read_only: bool,
}

#[derive(Debug, Clone)]
pub struct SandboxConfig {
    pub rootfs: PathBuf,
    pub overlay_lower: PathBuf,
    pub overlay_upper: PathBuf,
    pub overlay_work: PathBuf,
    pub overlay_merged: PathBuf,
    pub hostname: String,
    pub bind_mounts: Vec<BindMount>,
    pub env_vars: Vec<(String, String)>,
    pub isolate_network: bool,
    pub uid: u32,
    pub gid: u32,
    pub username: String,
    pub home_dir: PathBuf,
}

/// Safe wrapper around libc::getuid().
#[allow(unsafe_code)]
fn current_uid() -> u32 {
    // SAFETY: getuid() is always safe — no arguments, no side effects, cannot fail.
    unsafe { libc::getuid() }
}

/// Safe wrapper around libc::getgid().
#[allow(unsafe_code)]
fn current_gid() -> u32 {
    // SAFETY: getgid() is always safe — no arguments, no side effects, cannot fail.
    unsafe { libc::getgid() }
}

impl SandboxConfig {
    pub fn new(rootfs: PathBuf, env_id: &str, env_dir: &Path) -> Self {
        let uid = current_uid();
        let gid = current_gid();
        let username = std::env::var("USER").unwrap_or_else(|_| "user".to_owned());
        let home_dir =
            PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| format!("/home/{username}")));

        Self {
            rootfs,
            overlay_lower: env_dir.join("lower"),
            overlay_upper: env_dir.join("upper"),
            overlay_work: env_dir.join("work"),
            overlay_merged: env_dir.join("merged"),
            hostname: format!("karapace-{}", &env_id[..12.min(env_id.len())]),
            bind_mounts: Vec::new(),
            env_vars: Vec::new(),
            isolate_network: false,
            uid,
            gid,
            username,
            home_dir,
        }
    }
}

pub fn mount_overlay(config: &SandboxConfig) -> Result<(), RuntimeError> {
    let _ = unmount_overlay(config);

    if config.overlay_work.exists() {
        let _ = std::fs::remove_dir_all(&config.overlay_work);
    }

    for dir in [
        &config.overlay_upper,
        &config.overlay_work,
        &config.overlay_merged,
    ] {
        std::fs::create_dir_all(dir)?;
    }

    // Create a symlink to rootfs as lower dir if needed
    if !config.overlay_lower.exists() {
        #[cfg(unix)]
        std::os::unix::fs::symlink(&config.rootfs, &config.overlay_lower)?;
    }

    let status = Command::new("fuse-overlayfs")
        .args([
            "-o",
            &format!(
                "lowerdir={},upperdir={},workdir={}",
                config.rootfs.display(),
                config.overlay_upper.display(),
                config.overlay_work.display()
            ),
            &config.overlay_merged.to_string_lossy(),
        ])
        .status()
        .map_err(|e| {
            RuntimeError::ExecFailed(format!(
                "fuse-overlayfs not found or failed to start: {e}. Install with: sudo zypper install fuse-overlayfs"
            ))
        })?;

    if !status.success() {
        return Err(RuntimeError::ExecFailed(
            "fuse-overlayfs mount failed".to_owned(),
        ));
    }

    Ok(())
}

/// Check if a path is currently a mount point by inspecting /proc/mounts.
fn is_mounted(path: &Path) -> bool {
    let canonical = match std::fs::canonicalize(path) {
        Ok(p) => p.to_string_lossy().to_string(),
        Err(_) => path.to_string_lossy().to_string(),
    };
    match std::fs::read_to_string("/proc/mounts") {
        Ok(mounts) => mounts
            .lines()
            .any(|line| line.split_whitespace().nth(1) == Some(&canonical)),
        Err(_) => false,
    }
}

pub fn unmount_overlay(config: &SandboxConfig) -> Result<(), RuntimeError> {
    if !config.overlay_merged.exists() {
        return Ok(());
    }
    if !is_mounted(&config.overlay_merged) {
        return Ok(());
    }
    let _ = Command::new("fusermount3")
        .args(["-u", &config.overlay_merged.to_string_lossy()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    if is_mounted(&config.overlay_merged) {
        let _ = Command::new("fusermount")
            .args(["-u", &config.overlay_merged.to_string_lossy()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    Ok(())
}

pub fn setup_container_rootfs(config: &SandboxConfig) -> Result<PathBuf, RuntimeError> {
    let merged = &config.overlay_merged;

    for subdir in [
        "proc", "sys", "dev", "dev/pts", "dev/shm", "tmp", "run", "run/user", "etc", "var",
        "var/tmp",
    ] {
        std::fs::create_dir_all(merged.join(subdir))?;
    }

    let user_run = merged.join(format!("run/user/{}", config.uid));
    std::fs::create_dir_all(&user_run)?;

    let container_home = merged.join(
        config
            .home_dir
            .strip_prefix("/")
            .unwrap_or(&config.home_dir),
    );
    std::fs::create_dir_all(&container_home)?;

    let _ = std::fs::write(merged.join("etc/hostname"), &config.hostname);

    if !merged.join("etc/resolv.conf").exists() && Path::new("/etc/resolv.conf").exists() {
        let _ = std::fs::copy("/etc/resolv.conf", merged.join("etc/resolv.conf"));
    }

    ensure_user_in_container(config, merged)?;

    Ok(merged.clone())
}

fn ensure_user_in_container(config: &SandboxConfig, merged: &Path) -> Result<(), RuntimeError> {
    let passwd_path = merged.join("etc/passwd");
    let existing = std::fs::read_to_string(&passwd_path).unwrap_or_default();

    let user_entry = format!(
        "{}:x:{}:{}::/{}:/bin/bash\n",
        config.username,
        config.uid,
        config.gid,
        config
            .home_dir
            .strip_prefix("/")
            .unwrap_or(&config.home_dir)
            .display()
    );

    if !existing.contains(&format!("{}:", config.username)) {
        let mut content = existing;
        if !content.contains("root:") {
            content.push_str("root:x:0:0:root:/root:/bin/bash\n");
        }
        content.push_str(&user_entry);
        std::fs::write(&passwd_path, content)?;
    }

    // Ensure group exists
    let group_path = merged.join("etc/group");
    let existing_groups = std::fs::read_to_string(&group_path).unwrap_or_default();
    let group_entry = format!("{}:x:{}:\n", config.username, config.gid);
    if !existing_groups.contains(&format!("{}:", config.username)) {
        let mut content = existing_groups;
        if !content.contains("root:") {
            content.push_str("root:x:0:\n");
        }
        content.push_str(&group_entry);
        std::fs::write(&group_path, content)?;
    }

    Ok(())
}

fn build_unshare_command(config: &SandboxConfig) -> Command {
    let mut cmd = Command::new("unshare");
    cmd.args([
        "--user",
        "--map-root-user",
        "--mount",
        "--pid",
        "--fork",
        "--kill-child=SIGTERM",
    ]);

    if config.isolate_network {
        cmd.arg("--net");
    }

    cmd
}

fn build_setup_script(config: &SandboxConfig) -> String {
    let merged = &config.overlay_merged;
    let qm = shell_quote_path(merged);
    let mut script = String::new();

    let _ = writeln!(script, "mount -t proc proc {qm}/proc 2>/dev/null || true");

    let _ = writeln!(script, "mount --rbind /sys {qm}/sys 2>/dev/null && mount --make-rslave {qm}/sys 2>/dev/null || true");

    let _ = writeln!(script, "mount --rbind /dev {qm}/dev 2>/dev/null && mount --make-rslave {qm}/dev 2>/dev/null || true");

    let container_home = merged.join(
        config
            .home_dir
            .strip_prefix("/")
            .unwrap_or(&config.home_dir),
    );
    let _ = writeln!(
        script,
        "mount --bind {} {} 2>/dev/null || true",
        shell_quote_path(&config.home_dir),
        shell_quote_path(&container_home)
    );

    let _ = writeln!(script, "touch {qm}/etc/resolv.conf 2>/dev/null; mount --bind /etc/resolv.conf {qm}/etc/resolv.conf 2>/dev/null || true");

    let _ = writeln!(script, "mount --bind /tmp {qm}/tmp 2>/dev/null || true");

    for bm in &config.bind_mounts {
        let target = if bm.target.is_absolute() {
            merged.join(bm.target.strip_prefix("/").unwrap_or(&bm.target))
        } else {
            merged.join(&bm.target)
        };
        let qt = shell_quote_path(&target);
        let qs = shell_quote_path(&bm.source);
        let _ = writeln!(
            script,
            "mkdir -p {qt} 2>/dev/null; mount --bind {qs} {qt} 2>/dev/null || true"
        );
        if bm.read_only {
            let _ = writeln!(script, "mount -o remount,ro,bind {qt} 2>/dev/null || true");
        }
    }

    if let Ok(xdg_run) = std::env::var("XDG_RUNTIME_DIR") {
        let container_run = merged.join(format!("run/user/{}", config.uid));
        for socket in &["wayland-0", "pipewire-0", "pulse/native", "bus"] {
            let src = PathBuf::from(&xdg_run).join(socket);
            if src.exists() {
                let dst = container_run.join(socket);
                let qs = shell_quote_path(&src);
                let qd = shell_quote_path(&dst);
                if let Some(parent) = dst.parent() {
                    let _ = writeln!(
                        script,
                        "mkdir -p {} 2>/dev/null || true",
                        shell_quote_path(parent)
                    );
                }
                if src.is_file() || !src.is_dir() {
                    let _ = writeln!(script, "touch {qd} 2>/dev/null || true");
                }
                let _ = writeln!(script, "mount --bind {qs} {qd} 2>/dev/null || true");
            }
        }
    }

    if Path::new("/tmp/.X11-unix").exists() {
        let _ = writeln!(
            script,
            "mount --bind /tmp/.X11-unix {qm}/tmp/.X11-unix 2>/dev/null || true"
        );
    }

    let _ = writeln!(script, "exec chroot {qm} /bin/sh -s <<'__KARAPACE_EOF__'");

    script
}

pub fn enter_interactive(config: &SandboxConfig) -> Result<i32, RuntimeError> {
    let merged = &config.overlay_merged;

    let mut setup = build_setup_script(config);

    let mut env_exports = String::new();
    for (key, val) in &config.env_vars {
        if !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            continue;
        }
        let _ = write!(env_exports, "export {}={}; ", key, shell_quote(val));
    }

    let _ = write!(
        env_exports,
        "export HOME={}; ",
        shell_quote_path(&config.home_dir)
    );
    let _ = write!(
        env_exports,
        "export USER={}; ",
        shell_quote(&config.username)
    );
    let _ = write!(
        env_exports,
        "export HOSTNAME={}; ",
        shell_quote(&config.hostname)
    );
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        let _ = write!(
            env_exports,
            "export XDG_RUNTIME_DIR={}; ",
            shell_quote(&xdg)
        );
    }
    if let Ok(display) = std::env::var("DISPLAY") {
        let _ = write!(env_exports, "export DISPLAY={}; ", shell_quote(&display));
    }
    if let Ok(wayland) = std::env::var("WAYLAND_DISPLAY") {
        let _ = write!(
            env_exports,
            "export WAYLAND_DISPLAY={}; ",
            shell_quote(&wayland)
        );
    }
    env_exports.push_str("export TERM=${TERM:-xterm-256color}; ");
    let _ = write!(
        env_exports,
        "export KARAPACE_ENV=1; export KARAPACE_HOSTNAME={}; ",
        shell_quote(&config.hostname)
    );

    let shell = if merged.join("bin/bash").exists() || merged.join("usr/bin/bash").exists() {
        "/bin/bash"
    } else {
        "/bin/sh"
    };

    let _ = write!(
        setup,
        "{env_exports}cd ~; exec {shell} -l </dev/tty >/dev/tty 2>/dev/tty\n__KARAPACE_EOF__\n"
    );

    let mut cmd = build_unshare_command(config);
    cmd.arg("/bin/sh").arg("-c").arg(&setup);

    cmd.stdin(std::process::Stdio::inherit());
    cmd.stdout(std::process::Stdio::inherit());
    cmd.stderr(std::process::Stdio::inherit());

    let status = cmd
        .status()
        .map_err(|e| RuntimeError::ExecFailed(format!("failed to enter sandbox: {e}")))?;

    Ok(status.code().unwrap_or(1))
}

pub fn spawn_enter_interactive(
    config: &SandboxConfig,
) -> Result<std::process::Child, RuntimeError> {
    let merged = &config.overlay_merged;

    let mut setup = build_setup_script(config);

    let mut env_exports = String::new();
    for (key, val) in &config.env_vars {
        if !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            continue;
        }
        let _ = write!(env_exports, "export {}={}; ", key, shell_quote(val));
    }

    let _ = write!(
        env_exports,
        "export HOME={}; ",
        shell_quote_path(&config.home_dir)
    );
    let _ = write!(
        env_exports,
        "export USER={}; ",
        shell_quote(&config.username)
    );
    let _ = write!(
        env_exports,
        "export HOSTNAME={}; ",
        shell_quote(&config.hostname)
    );
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        let _ = write!(
            env_exports,
            "export XDG_RUNTIME_DIR={}; ",
            shell_quote(&xdg)
        );
    }
    if let Ok(display) = std::env::var("DISPLAY") {
        let _ = write!(env_exports, "export DISPLAY={}; ", shell_quote(&display));
    }
    if let Ok(wayland) = std::env::var("WAYLAND_DISPLAY") {
        let _ = write!(
            env_exports,
            "export WAYLAND_DISPLAY={}; ",
            shell_quote(&wayland)
        );
    }
    env_exports.push_str("export TERM=${TERM:-xterm-256color}; ");
    let _ = write!(
        env_exports,
        "export KARAPACE_ENV=1; export KARAPACE_HOSTNAME={}; ",
        shell_quote(&config.hostname)
    );

    let shell = if merged.join("bin/bash").exists() || merged.join("usr/bin/bash").exists() {
        "/bin/bash"
    } else {
        "/bin/sh"
    };

    let _ = write!(
        setup,
        "{env_exports}cd ~; exec {shell} -l </dev/tty >/dev/tty 2>/dev/tty\n__KARAPACE_EOF__\n"
    );

    let mut cmd = build_unshare_command(config);
    cmd.arg("/bin/sh").arg("-c").arg(&setup);

    cmd.stdin(std::process::Stdio::inherit());
    cmd.stdout(std::process::Stdio::inherit());
    cmd.stderr(std::process::Stdio::inherit());

    cmd.spawn()
        .map_err(|e| RuntimeError::ExecFailed(format!("failed to spawn sandbox: {e}")))
}

pub fn exec_in_container(
    config: &SandboxConfig,
    command: &[String],
) -> Result<std::process::Output, RuntimeError> {
    let mut setup = build_setup_script(config);

    let mut env_exports = String::new();
    for (key, val) in &config.env_vars {
        if !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            continue;
        }
        let _ = write!(env_exports, "export {}={}; ", key, shell_quote(val));
    }
    let _ = write!(
        env_exports,
        "export HOME={}; ",
        shell_quote_path(&config.home_dir)
    );
    let _ = write!(
        env_exports,
        "export USER={}; ",
        shell_quote(&config.username)
    );
    env_exports.push_str("export KARAPACE_ENV=1; ");

    let escaped_cmd: Vec<String> = command.iter().map(|a| shell_quote(a)).collect();
    let _ = write!(
        setup,
        "{env_exports}{}\n__KARAPACE_EOF__\n",
        escaped_cmd.join(" ")
    );

    let mut cmd = build_unshare_command(config);
    cmd.arg("/bin/sh").arg("-c").arg(&setup);

    cmd.output()
        .map_err(|e| RuntimeError::ExecFailed(format!("exec in container failed: {e}")))
}

pub fn install_packages_in_container(
    config: &SandboxConfig,
    install_cmd: &[String],
) -> Result<(), RuntimeError> {
    if install_cmd.is_empty() {
        return Ok(());
    }

    let output = exec_in_container(config, install_cmd)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(RuntimeError::ExecFailed(format!(
            "package installation failed:\nstdout: {stdout}\nstderr: {stderr}"
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_config_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let rootfs = dir.path().join("rootfs");
        std::fs::create_dir_all(&rootfs).unwrap();
        let config = SandboxConfig::new(rootfs, "abc123def456", dir.path());
        assert!(config.hostname.starts_with("karapace-"));
        assert!(!config.isolate_network);
    }

    #[test]
    fn shell_quote_escapes_single_quotes() {
        assert_eq!(shell_quote("hello"), "'hello'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn shell_quote_prevents_injection() {
        // Command substitution is safely wrapped in single quotes
        let malicious = "$(rm -rf /)";
        let quoted = shell_quote(malicious);
        assert_eq!(quoted, "'$(rm -rf /)'");
        assert!(quoted.starts_with('\'') && quoted.ends_with('\''));

        // Backtick injection is also safely quoted
        let backtick = "`whoami`";
        let quoted = shell_quote(backtick);
        assert_eq!(quoted, "'`whoami`'");

        // Newline injection
        let newline = "value\n; rm -rf /";
        let quoted = shell_quote(newline);
        assert!(quoted.starts_with('\'') && quoted.ends_with('\''));
    }

    #[test]
    fn shell_quote_path_handles_spaces() {
        let p = PathBuf::from("/home/user/my project/dir");
        let quoted = shell_quote_path(&p);
        assert_eq!(quoted, "'/home/user/my project/dir'");
    }

    #[test]
    fn build_setup_script_contains_essential_mounts() {
        let dir = tempfile::tempdir().unwrap();
        let rootfs = dir.path().join("rootfs");
        std::fs::create_dir_all(&rootfs).unwrap();
        let config = SandboxConfig::new(rootfs, "abc123def456", dir.path());
        let script = build_setup_script(&config);
        assert!(script.contains("mount -t proc"));
        assert!(script.contains("mount --rbind /sys"));
        assert!(script.contains("mount --rbind /dev"));
        assert!(script.contains("chroot"));
    }

    #[test]
    fn is_mounted_returns_false_for_regular_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!is_mounted(dir.path()));
    }

    #[test]
    fn unmount_overlay_noop_on_non_mounted() {
        let dir = tempfile::tempdir().unwrap();
        let rootfs = dir.path().join("rootfs");
        std::fs::create_dir_all(&rootfs).unwrap();
        let config = SandboxConfig::new(rootfs, "abc123def456", dir.path());
        // Create the merged dir but don't mount anything
        std::fs::create_dir_all(&config.overlay_merged).unwrap();
        // Should not error — just returns Ok because nothing is mounted
        assert!(unmount_overlay(&config).is_ok());
    }
}
