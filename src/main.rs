mod mcp;
mod kakoune;
mod lockfile;
mod client;
mod websocket;
mod server;

use clap::{Parser, Subcommand};

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
            // Bind TCP socket first to get the port (before forking)
            let tcp_addr: std::net::SocketAddr = "127.0.0.1:0".parse().unwrap();
            let std_listener = std::net::TcpListener::bind(tcp_addr).unwrap_or_else(|e| {
                eprintln!("Failed to bind TCP: {e}");
                std::process::exit(1);
            });
            let port = std_listener.local_addr().unwrap().port();

            // Fork: parent prints port and exits, child runs the event loop
            unsafe {
                let pid = libc::fork();
                if pid < 0 {
                    eprintln!("Failed to fork");
                    std::process::exit(1);
                }
                if pid > 0 {
                    // Parent: print port, exit
                    println!("{port}");
                    std::process::exit(0);
                }
                // Child: detach from terminal
                libc::setsid();
            }

            // Child: create server with the pre-bound TCP listener
            std_listener.set_nonblocking(true).unwrap();
            let mut server = match server::Server::with_tcp_listener(&session, &client, &cwd, std_listener) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Failed to start server: {e}");
                    std::process::exit(1);
                }
            };

            if let Err(e) = server.run() {
                eprintln!("Server error: {e}");
                std::process::exit(1);
            }
        }
        Command::Send { session, msg } => {
            let message = match msg {
                SendMessage::State { file, line, col, selection } => {
                    client::build_state_message(&file, line, col, &selection)
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
