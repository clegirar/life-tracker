use std::io::{BufRead, IsTerminal};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use life_tracker::interpreters::INTERPRETERS;
use life_tracker::{admin, api, db, engine, replay};
use tokio::net::TcpListener;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::watch;
use tracing_subscriber::EnvFilter;

const POLL_INTERVAL: Duration = Duration::from_secs(1);

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
    /// Lance l'API et les interprètes, avec le rôle tracker_app
    Serve {
        #[arg(long, env = "DATABASE_URL", hide_env_values = true)]
        database_url: String,
        #[arg(long, env = "LISTEN_ADDR", default_value = "127.0.0.1:8080")]
        listen: String,
        /// Développement seulement : retire `Secure` du cookie de session (Safari sur http://localhost)
        #[arg(long, env = "DEV_INSECURE_COOKIE")]
        dev_insecure_cookie: bool,
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
    /// Crée le compte web ou change son mot de passe (lu au clavier ou sur l'entrée standard),
    /// et ferme toutes les sessions
    SetPassword { username: String },
    /// Construit une génération à côté de l'active, la compare, puis bascule
    Rebuild {
        /// Bascule malgré les différences (une nouvelle version d'interprète, par exemple)
        #[arg(long)]
        allow_differences: bool,
    },
    /// Réinterprète les enregistrements en erreur de la génération active
    RetryErrors,
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Serve {
            database_url,
            listen,
            dev_insecure_cookie,
        } => serve(&database_url, &listen, dev_insecure_cookie).await,
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

async fn serve(database_url: &str, listen: &str, insecure_cookie: bool) -> Result<()> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    if insecure_cookie {
        tracing::warn!("cookie de session sans Secure : développement seulement");
    }
    let pool = db::pool(database_url)?;
    let listener = TcpListener::bind(listen).await.context("écoute")?;
    tracing::info!(addr = %listener.local_addr()?, "listening");
    let (stop, stopped) = watch::channel(false);
    let worker = tokio::spawn(engine::run(
        pool.clone(),
        INTERPRETERS,
        POLL_INTERVAL,
        stopped,
    ));
    let served = axum::serve(
        listener,
        api::router_with(pool.clone(), api::Config { insecure_cookie }),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await;
    // Le worker finit l'enregistrement en cours ; le suivant attendra le prochain démarrage.
    stop.send_replace(true);
    worker.await?;
    pool.close().await;
    served?;
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
        AdminCommand::SetPassword { username } => {
            admin::set_password(&pool, &username, &read_password()?).await?;
            eprintln!("mot de passe de {username} enregistré ; sessions fermées");
        }
        AdminCommand::Rebuild { allow_differences } => {
            let report = replay::rebuild(&pool, INTERPRETERS, allow_differences).await?;
            print_rebuild(&report);
            if !report.switched {
                bail!(
                    "{} différence(s), pas de bascule ; génération {} laissée à examiner, \
                     --allow-differences pour basculer",
                    report.comparison.differences.len(),
                    report.generation
                );
            }
        }
        AdminCommand::RetryErrors => {
            let report = replay::retry_errors(&pool, INTERPRETERS).await?;
            println!(
                "{} levée(s) : {:?} ; {} restante(s) : {:?}",
                report.cleared.len(),
                report.cleared,
                report.kept.len(),
                report.kept
            );
        }
    }
    Ok(())
}

/// Jamais sur la ligne de commande : elle se lit dans `ps` et dans l'historique du shell.
fn read_password() -> Result<String> {
    if std::io::stdin().is_terminal() {
        let password = rpassword::prompt_password("Mot de passe : ")?;
        if password != rpassword::prompt_password("Encore une fois : ")? {
            bail!("les deux saisies diffèrent");
        }
        return Ok(password);
    }
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(line.trim_end_matches(['\r', '\n']).to_owned())
}

/// Des numéros du journal seulement, jamais le contenu des lignes.
fn print_rebuild(report: &replay::RebuildReport) {
    for (kind, last_seq) in &report.comparison.compared_up_to {
        println!("{kind} : comparé jusqu'au numéro {last_seq}");
    }
    for difference in &report.comparison.differences {
        println!(
            "différence : {} numéro {}",
            difference.table, difference.seq
        );
    }
    println!(
        "génération {} : {} différence(s), {}",
        report.generation,
        report.comparison.differences.len(),
        if report.switched {
            "active"
        } else {
            "non basculée"
        }
    );
}
