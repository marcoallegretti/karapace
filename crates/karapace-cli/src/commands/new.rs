use super::{json_pretty, EXIT_SUCCESS};
use dialoguer::{Confirm, Input, Select};
use karapace_schema::manifest::{
    parse_manifest_str, BaseSection, GuiSection, HardwareSection, ManifestV1, MountsSection,
    RuntimeSection, SystemSection,
};
use std::io::{stderr, stdin, IsTerminal};
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;

const DEST_MANIFEST: &str = "karapace.toml";

fn template_source(name: &str) -> Option<&'static str> {
    match name {
        "minimal" => Some(include_str!("../../../../examples/minimal.toml")),
        "dev" => Some(include_str!("../../../../examples/dev.toml")),
        "gui-dev" => Some(include_str!("../../../../examples/gui-dev.toml")),
        "rust-dev" => Some(include_str!("../../../../examples/rust-dev.toml")),
        "ubuntu-dev" => Some(include_str!("../../../../examples/ubuntu-dev.toml")),
        _ => None,
    }
}

fn load_template(name: &str) -> Result<ManifestV1, String> {
    let src = template_source(name).ok_or_else(|| {
        format!("unknown template '{name}' (expected: minimal, dev, gui-dev, rust-dev, ubuntu-dev)")
    })?;
    parse_manifest_str(src).map_err(|e| format!("template parse error: {e}"))
}

fn write_atomic(dest: &Path, content: &str) -> Result<(), String> {
    let dir = dest
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    let mut tmp = NamedTempFile::new_in(&dir).map_err(|e| format!("write temp file: {e}"))?;
    use std::io::Write;
    tmp.write_all(content.as_bytes())
        .map_err(|e| format!("write temp file: {e}"))?;
    tmp.as_file()
        .sync_all()
        .map_err(|e| format!("fsync temp file: {e}"))?;
    tmp.persist(dest)
        .map_err(|e| format!("persist manifest: {}", e.error))?;
    Ok(())
}

fn ensure_can_write(dest: &Path, force: bool, is_tty: bool) -> Result<(), String> {
    if !dest.exists() || force {
        return Ok(());
    }
    if !is_tty {
        return Err(format!(
            "refusing to overwrite existing ./{DEST_MANIFEST} (pass --force)"
        ));
    }
    let overwrite = Confirm::new()
        .with_prompt(format!("overwrite ./{DEST_MANIFEST}?"))
        .default(false)
        .interact()
        .map_err(|e| format!("prompt failed: {e}"))?;
    if overwrite {
        Ok(())
    } else {
        Err(format!(
            "refusing to overwrite existing ./{DEST_MANIFEST} (pass --force)"
        ))
    }
}

fn print_result(name: &str, template: Option<&str>, json: bool) -> Result<(), String> {
    if json {
        let payload = serde_json::json!({
            "status": "written",
            "path": format!("./{DEST_MANIFEST}"),
            "name": name,
            "template": template,
        });
        println!("{}", json_pretty(&payload)?);
    } else {
        println!("wrote ./{DEST_MANIFEST} for '{name}'");
        if let Some(tpl) = template {
            println!("template: {tpl}");
        }
    }
    Ok(())
}

pub fn run(name: &str, template: Option<&str>, force: bool, json: bool) -> Result<u8, String> {
    let dest = Path::new(DEST_MANIFEST);
    let is_tty = stdin().is_terminal() && stderr().is_terminal();

    let mut manifest = if let Some(tpl) = template {
        let m = load_template(tpl)?;
        ensure_can_write(dest, force, is_tty)?;
        m
    } else {
        ensure_can_write(dest, force, is_tty)?;
        if !is_tty {
            return Err("no --template provided and stdin is not a TTY".to_owned());
        }
        let image: String = Input::new()
            .with_prompt("base image")
            .default("rolling".to_owned())
            .interact_text()
            .map_err(|e| format!("prompt failed: {e}"))?;
        ManifestV1 {
            manifest_version: 1,
            base: BaseSection { image },
            system: SystemSection::default(),
            gui: GuiSection::default(),
            hardware: HardwareSection::default(),
            mounts: MountsSection::default(),
            runtime: RuntimeSection::default(),
        }
    };
    if is_tty {
        let packages: String = Input::new()
            .with_prompt("packages (space-separated, empty to skip)")
            .allow_empty(true)
            .interact_text()
            .map_err(|e| format!("prompt failed: {e}"))?;
        if !packages.trim().is_empty() {
            manifest
                .system
                .packages
                .extend(packages.split_whitespace().map(str::to_owned));
        }

        let mount: String = Input::new()
            .with_prompt("mount (format '<host>:<container>', empty to skip)")
            .allow_empty(true)
            .interact_text()
            .map_err(|e| format!("prompt failed: {e}"))?;
        if !mount.trim().is_empty() {
            manifest
                .mounts
                .entries
                .insert("workspace".to_owned(), mount);
        }
        let backends = ["namespace", "oci", "mock"];
        let default_idx = backends
            .iter()
            .position(|b| *b == manifest.runtime.backend.as_str())
            .unwrap_or(0);
        let idx = Select::new()
            .with_prompt("runtime backend")
            .items(&backends)
            .default(default_idx)
            .interact()
            .map_err(|e| format!("prompt failed: {e}"))?;
        manifest.runtime.backend.clear();
        manifest.runtime.backend.push_str(backends[idx]);
        let isolated = Confirm::new()
            .with_prompt("enable network isolation?")
            .default(manifest.runtime.network_isolation)
            .interact()
            .map_err(|e| format!("prompt failed: {e}"))?;
        manifest.runtime.network_isolation = isolated;
    } else if template.is_none() {
        return Err("interactive prompts require a TTY".to_owned());
    }
    let toml =
        toml::to_string_pretty(&manifest).map_err(|e| format!("TOML serialization failed: {e}"))?;
    write_atomic(dest, &toml)?;
    print_result(name, template, json)?;
    Ok(EXIT_SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_parse() {
        for tpl in ["minimal", "dev", "gui-dev", "rust-dev", "ubuntu-dev"] {
            let m = load_template(tpl).unwrap();
            assert_eq!(m.manifest_version, 1);
            assert!(!m.base.image.is_empty());
        }
    }
}
