mod up;
use crate::up::up;
mod down;
use crate::down::down;
mod namespace;

use anyhow::{bail, Result};
use clap::Parser;
use nix::unistd;
use nix::unistd::ROOT;
use tracing::info;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[derive(Parser)]
struct Args {
    #[clap(subcommand)]
    command: Command,
}

#[derive(clap::Subcommand)]
enum Command {
    /// Start warp in a container
    Up,
    /// Stop warp and cleanup the container
    Down,
}

fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "bubblewarp=info".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let cli = Args::parse();
    ensure_root()?;

    match cli.command {
        Command::Up => {
            up()?;
        }
        Command::Down => {
            down()?;
        }
    }

    Ok(())
}

fn ensure_root() -> Result<()> {
    if !unistd::geteuid().is_root() {
        bail!("We are not running as root!")
    } else if !unistd::getuid().is_root() {
        // We are not root, but we're suid root. Elevate.
        info!("Running as setuid root. Strange, but continuing happily.");
        unistd::setuid(ROOT).expect("Failed to setuid(0), but we have euid 0!");
    }
    Ok(())
}
