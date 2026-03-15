pub mod dwl_ipc {
    pub use wayland_client;
    pub use wayland_client::protocol::wl_output;

    pub mod __interfaces {
        pub use wayland_client;
        use wayland_client::backend as wayland_backend;
        use wayland_client::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("protocols/dwl-ipc-unstable-v2.xml");
    }
    use self::__interfaces::*;
    wayland_scanner::generate_client_code!("protocols/dwl-ipc-unstable-v2.xml");
}

pub mod config;
pub mod event;
pub mod lua;
pub mod manifest;
pub mod socket;
pub mod wayland;

use crate::event::{WaylandEvent, WaylandRequest};
use anyhow::Result;
use clap::{Parser, Subcommand};
use std::sync::mpsc::channel;
use std::thread;

#[derive(Parser)]
#[command(
    author,
    version,
    about,
    long_about = "\
mplug manages Lua plugins for MangoWM / MangoWC Wayland compositors.\n\
Plugins are installed from git repositories containing a mplug.toml manifest.\n\
\n\
Quick start:\n  \
mplug add https://github.com/user/plugin   # install a plugin\n  \
mplug enable plugin-name                    # activate it\n  \
mplug list                                  # see all plugins\
"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    #[command(
        after_help = "Examples:\n  mplug daemon\n\nTypically started by your compositor or session manager, not run directly."
    )]
    Daemon,
    #[command(after_help = "Examples:\n  mplug enable plugin-name\n  mplug enable dynamic-tiling")]
    Enable { plugin: String },
    #[command(
        after_help = "Examples:\n  mplug disable plugin-name\n  mplug disable dynamic-tiling"
    )]
    Disable { plugin: String },
    #[command(
        after_help = "Examples:\n  mplug list\n\nShows a table of all installed plugins with their status (enabled/disabled) and the source git URL."
    )]
    List,
    #[command(
        after_help = "Examples:\n  mplug add https://github.com/user/plugin\n  mplug add https://github.com/user/plugin.git"
    )]
    Add { repo: String },
    #[command(after_help = "Examples:\n  mplug update plugin-name\n  mplug update dynamic-tiling")]
    Update { name: String },
    #[command(
        after_help = "Examples:\n  mplug outdated\n\nCompares installed plugins against their git remotes and lists any with upstream commits not yet pulled."
    )]
    Outdated,
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            for cause in e.chain().skip(1) {
                eprintln!("  caused by: {cause}");
            }
            std::process::ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    match &cli.command {
        Commands::Daemon => {
            let (event_tx, event_rx) = channel::<WaylandEvent>();
            let (req_tx, req_rx) = channel::<WaylandRequest>();

            let event_tx_socket = event_tx.clone();
            let wayland_handle = thread::spawn(move || {
                wayland::run_wayland(event_tx, req_rx);
            });

            let req_tx_socket = req_tx.clone();
            let lua_handle = thread::spawn(move || {
                if let Err(e) = lua::run_lua(event_rx, req_tx) {
                    eprintln!("Lua thread error: {}", e);
                }
            });

            let socket_handle = thread::spawn(move || {
                socket::run_socket(req_tx_socket, event_tx_socket);
            });

            let _ = wayland_handle.join();
            let _ = lua_handle.join();
            let _ = socket_handle.join();
        }
        Commands::Enable { plugin } => config::enable_plugin(plugin)?,
        Commands::Disable { plugin } => config::disable_plugin(plugin)?,
        Commands::List => config::list_plugins()?,
        Commands::Add { repo } => config::add_plugin(repo)?,
        Commands::Update { name } => config::update_plugin(name)?,
        Commands::Outdated => config::outdated_plugins()?,
    }
    Ok(())
}
