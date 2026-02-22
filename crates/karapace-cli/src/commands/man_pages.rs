use super::EXIT_SUCCESS;
use clap::CommandFactory;
use std::path::Path;

pub fn run<C: CommandFactory>(dir: &Path) -> Result<u8, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("failed to create dir: {e}"))?;
    let cmd = C::command();
    let man = clap_mangen::Man::new(cmd.clone());
    let mut buf = Vec::new();
    man.render(&mut buf)
        .map_err(|e| format!("man page render failed: {e}"))?;
    let path = dir.join("karapace.1");
    std::fs::write(&path, &buf).map_err(|e| format!("failed to write {}: {e}", path.display()))?;
    for sub in cmd.get_subcommands() {
        let sub_name = format!("karapace-{}", sub.get_name());
        let man = clap_mangen::Man::new(sub.clone());
        let mut buf = Vec::new();
        man.render(&mut buf)
            .map_err(|e| format!("man page render failed: {e}"))?;
        let path = dir.join(format!("{sub_name}.1"));
        std::fs::write(&path, &buf)
            .map_err(|e| format!("failed to write {}: {e}", path.display()))?;
    }
    println!("man pages written to {}", dir.display());
    Ok(EXIT_SUCCESS)
}
