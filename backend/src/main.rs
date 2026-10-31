use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use life_tracker::{admin, api, db};
use tokio::net::TcpListener;
use tokio::signal::unix::{SignalKind, signal};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(
    version,
    about = "Life tracker : API d'ingestion, migrations, administration"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

// Chaque commande lit sa propre URL : `serve` ne voit jamais celle du propriétaire.
#[derive(Subcommand)]
enum Command {
    /// Lance l'API, avec le rôle tracker_app
    Serve {
        #[arg(long, env = "DATABASE_URL", hide_env_values = true)]
        database_url: String,
        #[arg(long, env = "LISTEN_ADDR", default_value = "127.0.0.1:8080")]
        listen: String,
    },
    /// Applique les migrations, avec le rôle tracker_owner
    Migrate {
        #[arg(long, env = "MIGRATE_DATABASE_URL", hide_env_values = true)]
        database_url: String,
    },
    /// Opérations rares, avec le rôle tracker_admin
    Admin {
        #[arg(long, env = "ADMIN_DATABASE_URL", hide_env_values = true)]
        database_url: String,
        #[command(subcommand)]
        command: AdminCommand,
    },
}

#[derive(Subcommand)]
enum AdminCommand {
    /// Crée une source ; le token n'est affiché qu'à cette occasion
    CreateSource {
        name: String,
        /// Durée au format PostgreSQL, par exemple « 6 hours » ; sans elle, jamais muette
        #[arg(long)]
        silence_threshold: Option<String>,
    },
    /// Révoque une source et son token
    RevokeSource { name: String },
    /// Déclare un type de donnée pour une source
    GrantKind { source: String, kind: String },
    /// Retire un type de donnée à une source
    RevokeKind { source: String, kind: String },
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Serve {
            database_url,
            listen,
        } => serve(&database_url, &listen).await,
        Command::Migrate { database_url } => {
            let pool = db::pool(&database_url)?;
            db::MIGRATOR.run(&pool).await.context("migrations")?;
            println!("migrations à jour");
            Ok(())
        }
        Command::Admin {
            database_url,
            command,
        } => run_admin(&database_url, command).await,
    }
}

async fn serve(database_url: &str, listen: &str) -> Result<()> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    let pool = db::pool(database_url)?;
    let listener = TcpListener::bind(listen).await.context("écoute")?;
    tracing::info!(addr = %listener.local_addr()?, "listening");
    axum::serve(listener, api::router(pool.clone()))
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    pool.close().await;
    tracing::info!("stopped");
    Ok(())
}

async fn shutdown_signal() {
    let mut term = signal(SignalKind::terminate()).expect("écoute de SIGTERM");
    tokio::select! {
        _ = term.recv() => {}
        _ = tokio::signal::ctrl_c() => {}
    }
}

async fn run_admin(database_url: &str, command: AdminCommand) -> Result<()> {
    let pool = db::pool(database_url)?;
    match command {
        AdminCommand::CreateSource {
            name,
            silence_threshold,
        } => {
            let token = admin::create_source(&pool, &name, silence_threshold.as_deref()).await?;
            // Sur stdout, seul : scriptable. Il ne sera plus jamais affiché.
            println!("{token}");
            eprintln!("source {name} créée ; conservez ce token, il ne sera plus affiché");
        }
        AdminCommand::RevokeSource { name } => {
            admin::revoke_source(&pool, &name).await?;
            eprintln!("source {name} révoquée");
        }
        AdminCommand::GrantKind { source, kind } => {
            admin::grant_kind(&pool, &source, &kind).await?;
            eprintln!("{kind} déclaré pour {source}");
        }
        AdminCommand::RevokeKind { source, kind } => {
            admin::revoke_kind(&pool, &source, &kind).await?;
            eprintln!("{kind} révoqué pour {source}");
        }
    }
    Ok(())
}
