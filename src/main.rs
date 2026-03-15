mod mcp;
mod kakoune;

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
            eprintln!("start: session={session} client={client} cwd={cwd}");
            todo!("daemon start")
        }
        Command::Send { session, msg } => {
            eprintln!("send: session={session}");
            todo!("client send")
        }
    }
}
