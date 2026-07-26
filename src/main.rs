mod audio;
mod daemon;
mod db;
mod hypr;
mod idle;
mod report;
mod suspend;
mod tui;

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser)]
#[command(name = "selftrack", about = "Focus-based time tracker for Hyprland")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Daemon {
        #[arg(long, default_value = "5")]
        idle_threshold: u64,
    },
    Report {
        #[arg(short, long)]
        date: Option<String>,
        #[arg(short, long)]
        app: Option<String>,
    },
    Today,
    Timeline {
        #[arg(short, long)]
        date: Option<String>,
    },
    Tui,
}

fn default_data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("selftrack")
}

fn db_path() -> PathBuf {
    default_data_dir().join("track.db")
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "selftrack=info".into()),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Daemon { idle_threshold } => {
            let path = db_path();
            tracing::info!("database: {}", path.display());
            let db = Arc::new(
                db::Database::open(&path).expect("failed to open database"),
            );
            daemon::run(db, idle_threshold).await;
        }
        Commands::Report { date, app } => {
            let date = date.unwrap_or_else(today);
            let path = db_path();
            let db = db::Database::open(&path).expect("failed to open database");
            report::print_report(&db, &date, app.as_deref());
        }
        Commands::Today => {
            let date = today();
            let path = db_path();
            let db = db::Database::open(&path).expect("failed to open database");
            report::print_report(&db, &date, None);
        }
        Commands::Timeline { date } => {
            let date = date.unwrap_or_else(today);
            let path = db_path();
            let db = db::Database::open(&path).expect("failed to open database");
            report::print_timeline(&db, &date);
        }
        Commands::Tui => {
            let path = db_path();
            let db = db::Database::open(&path).expect("failed to open database");
            tui::run(db).expect("TUI error");
        }
    }
}

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}
