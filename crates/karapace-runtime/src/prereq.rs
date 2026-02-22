use std::fmt;
use std::process::Command;

/// A missing prerequisite with actionable install instructions.
#[derive(Debug)]
pub struct MissingPrereq {
    pub name: &'static str,
    pub purpose: &'static str,
    pub install_hint: &'static str,
}

impl fmt::Display for MissingPrereq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "  - {}: {} (install: {})",
            self.name, self.purpose, self.install_hint
        )
    }
}

fn command_exists(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn user_namespaces_work() -> bool {
    Command::new("unshare")
        .args(["--user", "--map-root-user", "--fork", "true"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Check all prerequisites for the namespace backend.
/// Returns a list of missing items. Empty list means all prerequisites are met.
pub fn check_namespace_prereqs() -> Vec<MissingPrereq> {
    let mut missing = Vec::new();

    if !command_exists("unshare") {
        missing.push(MissingPrereq {
            name: "unshare",
            purpose: "user namespace isolation",
            install_hint: "part of util-linux (usually pre-installed)",
        });
    } else if !user_namespaces_work() {
        missing.push(MissingPrereq {
            name: "user namespaces",
            purpose: "unprivileged container isolation",
            install_hint:
                "enable CONFIG_USER_NS=y in kernel, or: sysctl kernel.unprivileged_userns_clone=1",
        });
    }

    if !command_exists("fuse-overlayfs") {
        missing.push(MissingPrereq {
            name: "fuse-overlayfs",
            purpose: "overlay filesystem for writable container layers",
            install_hint: "zypper install fuse-overlayfs | apt install fuse-overlayfs | dnf install fuse-overlayfs | pacman -S fuse-overlayfs",
        });
    }

    if !command_exists("curl") {
        missing.push(MissingPrereq {
            name: "curl",
            purpose: "downloading container images",
            install_hint:
                "zypper install curl | apt install curl | dnf install curl | pacman -S curl",
        });
    }

    missing
}

/// Check prerequisites for the OCI backend.
pub fn check_oci_prereqs() -> Vec<MissingPrereq> {
    let mut missing = Vec::new();

    let has_runtime = command_exists("crun") || command_exists("runc") || command_exists("youki");

    if !has_runtime {
        missing.push(MissingPrereq {
            name: "OCI runtime",
            purpose: "OCI container execution",
            install_hint: "install one of: crun, runc, or youki",
        });
    }

    if !command_exists("curl") {
        missing.push(MissingPrereq {
            name: "curl",
            purpose: "downloading container images",
            install_hint:
                "zypper install curl | apt install curl | dnf install curl | pacman -S curl",
        });
    }

    missing
}

/// Format a list of missing prerequisites into a user-friendly error message.
pub fn format_missing(missing: &[MissingPrereq]) -> String {
    use std::fmt::Write as _;
    let mut msg = String::from("missing prerequisites:\n");
    for m in missing {
        let _ = writeln!(msg, "{m}");
    }
    msg.push_str("\nKarapace requires these tools to create container environments.");
    msg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_prereq_display() {
        let m = MissingPrereq {
            name: "curl",
            purpose: "downloading images",
            install_hint: "apt install curl",
        };
        let s = format!("{m}");
        assert!(s.contains("curl"));
        assert!(s.contains("downloading images"));
        assert!(s.contains("apt install curl"));
    }

    #[test]
    fn format_missing_produces_readable_output() {
        let items = vec![
            MissingPrereq {
                name: "curl",
                purpose: "downloads",
                install_hint: "apt install curl",
            },
            MissingPrereq {
                name: "fuse-overlayfs",
                purpose: "overlay",
                install_hint: "apt install fuse-overlayfs",
            },
        ];
        let output = format_missing(&items);
        assert!(output.contains("missing prerequisites:"));
        assert!(output.contains("curl"));
        assert!(output.contains("fuse-overlayfs"));
    }
}
