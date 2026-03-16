mod mcp;
mod kakoune;
mod lockfile;
mod client;
mod websocket;
mod server;

use clap::{Parser, Subcommand};
use std::sync::atomic::{AtomicBool, Ordering};

pub static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);

extern "C" fn signal_handler(_sig: libc::c_int) {
    SHUTDOWN_REQUESTED.store(true, Ordering::SeqCst);
}

#[derive(Parser)]
#[command(name = "kak-claude", about = "Claude Code IDE integration for Kakoune")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start the kak-claude daemon
    Start {
        /// Kakoune session name
        #[arg(long)]
        session: String,
        /// Kakoune client name
        #[arg(long)]
        client: String,
        /// Working directory
        #[arg(long)]
        cwd: String,
    },
    /// Send a message to a running daemon
    Send {
        /// Kakoune session name
        #[arg(long)]
        session: String,
        #[command(subcommand)]
        msg: SendMessage,
    },
}

#[derive(Subcommand)]
enum SendMessage {
    /// Push editor state (selection, cursor)
    State {
        #[arg(long)]
        file: String,
        #[arg(long)]
        line: u32,
        #[arg(long)]
        col: u32,
        #[arg(long)]
        selection: String,
        /// Kakoune selection_desc (anchor.col,cursor.col format)
        #[arg(long, default_value = "")]
        sel_desc: String,
        /// Kakoune selection_length (1 = cursor only, >1 = real selection)
        #[arg(long, default_value = "1")]
        sel_len: u32,
        /// Read selection from stdin instead of --selection
        #[arg(long, default_value = "false")]
        selection_stdin: bool,
    },
    /// Push buffer list
    Buffers {
        #[arg(long)]
        list: String,
    },
    /// Shutdown the daemon
    Shutdown,
    /// Response to dirty check
    DirtyResponse {
        #[arg(long)]
        file: String,
        #[arg(long)]
        dirty: String,
    },
    /// Response to diff prompt
    DiffResponse {
        #[arg(long)]
        id: String,
        #[arg(long)]
        accepted: bool,
    },
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Start { session, client, cwd } => {
            // Auto-reap child processes (fire-and-forget kak -p calls)
            unsafe {
                libc::signal(libc::SIGCHLD, libc::SIG_IGN);
            }

            // Register signal handlers for graceful shutdown
            unsafe {
                libc::signal(libc::SIGTERM, signal_handler as *const () as libc::sighandler_t);
                libc::signal(libc::SIGINT, signal_handler as *const () as libc::sighandler_t);
                libc::signal(libc::SIGHUP, signal_handler as *const () as libc::sighandler_t);
            }

            // Create server (binds sockets, writes port/pid/lock files)
            let mut server = match server::Server::new(&session, &client, &cwd) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Failed to start server: {e}");
                    std::process::exit(1);
                }
            };

            // Run the event loop (caller is responsible for backgrounding)
            if let Err(e) = server.run() {
                eprintln!("Server error: {e}");
                std::process::exit(1);
            }
        }
        Command::Send { session, msg } => {
            let message = match msg {
                SendMessage::State { file, line, col, selection, sel_desc, sel_len, selection_stdin } => {
                    let actual_selection = if selection_stdin {
                        let mut buf = String::new();
                        std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf).unwrap_or(0);
                        buf
                    } else {
                        selection
                    };
                    client::build_state_message(&file, line, col, &actual_selection, &sel_desc, sel_len)
                }
                SendMessage::Buffers { list } => client::build_buffers_message(&list),
                SendMessage::Shutdown => client::build_shutdown_message(),
                SendMessage::DirtyResponse { file, dirty } => {
                    client::build_dirty_response(&file, &dirty)
                }
                SendMessage::DiffResponse { id, accepted } => {
                    client::build_diff_response(&id, accepted)
                }
            };
            if let Err(e) = client::send_message(&session, &message) {
                eprintln!("Failed to send: {e}");
                std::process::exit(1);
            }
        }
    }
}
