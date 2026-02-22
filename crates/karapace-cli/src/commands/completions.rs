use super::EXIT_SUCCESS;
use clap::CommandFactory;
use clap_complete::Shell;

#[allow(clippy::unnecessary_wraps)]
pub fn run<C: CommandFactory>(shell: Shell) -> Result<u8, String> {
    clap_complete::generate(shell, &mut C::command(), "karapace", &mut std::io::stdout());
    Ok(EXIT_SUCCESS)
}
