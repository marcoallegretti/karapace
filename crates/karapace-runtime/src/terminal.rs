use std::io::Write;

const OSC_START: &str = "\x1b]777;";
const OSC_END: &str = "\x1b\\";

pub fn emit_container_push(env_id: &str, hostname: &str) {
    if is_interactive_terminal() {
        let marker = format!("{OSC_START}container;push;{hostname};karapace;{env_id}{OSC_END}");
        let _ = std::io::stderr().write_all(marker.as_bytes());
        let _ = std::io::stderr().flush();
    }
}

pub fn emit_container_pop() {
    if is_interactive_terminal() {
        let marker = format!("{OSC_START}container;pop;;{OSC_END}");
        let _ = std::io::stderr().write_all(marker.as_bytes());
        let _ = std::io::stderr().flush();
    }
}

pub fn print_container_banner(env_id: &str, image: &str, hostname: &str) {
    if is_interactive_terminal() {
        let short_id = &env_id[..12.min(env_id.len())];
        eprintln!(
            "\x1b[1;36m[karapace]\x1b[0m entering \x1b[1m{image}\x1b[0m ({short_id}) as \x1b[1m{hostname}\x1b[0m"
        );
    }
}

pub fn print_container_exit(env_id: &str) {
    if is_interactive_terminal() {
        let short_id = &env_id[..12.min(env_id.len())];
        eprintln!("\x1b[1;36m[karapace]\x1b[0m exited environment {short_id}");
    }
}

#[allow(unsafe_code)]
fn is_interactive_terminal() -> bool {
    // SAFETY: isatty() is always safe — checks if fd is a terminal, no side effects.
    unsafe { libc::isatty(libc::STDERR_FILENO) != 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc_markers_dont_panic() {
        // Just ensure these don't crash; output depends on terminal
        emit_container_push("abc123def456", "karapace-abc123def456");
        emit_container_pop();
    }
}
