use std::io::{BufRead, IsTerminal};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use life_tracker::interpreters::INTERPRETERS;
use life_tracker::{admin, api, apple, collector, db, engine, replay};
use tokio::net::TcpListener;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::watch;
use tracing_subscriber::EnvFilter;

const POLL_INTERVAL: Duration = Duration::from_secs(1);
const LASTFM_PAUSE: Duration = Duration::from_millis(250);

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
    /// Interroge Last.fm et envoie à l'API d'ingestion, sans jamais connaître la base
    Collect {
        #[arg(long, env = "LASTFM_API_KEY", hide_env_values = true)]
        lastfm_api_key: String,
        #[arg(long, env = "LASTFM_USER", hide_env_values = true)]
        lastfm_user: String,
        #[arg(long, env = "COLLECTOR_TOKEN", hide_env_values = true)]
        token: String,
        #[arg(long, env = "INGEST_URL")]
        ingest_url: String,
        #[arg(long, env = "LASTFM_API_URL", default_value = collector::DEFAULT_LASTFM_URL)]
        lastfm_url: String,
        /// Où garder le curseur ; le perdre ne coûte qu'un rattrapage
        #[arg(
            long,
            env = "COLLECTOR_STATE_DIR",
            default_value = "/var/lib/life-tracker"
        )]
        state_dir: PathBuf,
        #[arg(long, env = "COLLECT_INTERVAL_SECONDS", default_value_t = 300)]
        interval_seconds: u64,
    },
    /// Envoie l'export Apple à l'API d'ingestion, sans jamais connaître la base. Le token se lit sur
    /// l'entrée standard, jamais en argument. Reprendre, c'est relancer : tout revient en duplicate
    ImportApple {
        /// Le dossier des trois fichiers lus, jamais modifié
        #[arg(long, env = "APPLE_ARCHIVE_DIR", default_value = "/archive")]
        archive: PathBuf,
        #[arg(long, env = "INGEST_URL")]
        ingest_url: String,
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
    /// Affiche, source par source, le nombre et les dates des événements d'une métrique
    Coverage { metric: String },
    /// Une source fait foi pour une métrique à partir d'une date ; rien n'est effacé
    SetPrecedence {
        metric: String,
        source: String,
        /// Date RFC 3339 ; sans elle, la première donnée de la source, sauf si elle est isolée
        #[arg(long)]
        from: Option<chrono::DateTime<chrono::Utc>>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Serve {
            database_url,
            listen,
            dev_insecure_cookie,
        } => serve(&database_url, &listen, dev_insecure_cookie).await,
        Command::Collect {
            lastfm_api_key,
            lastfm_user,
            token,
            ingest_url,
            lastfm_url,
            state_dir,
            interval_seconds,
        } => {
            collect(collector::Config {
                lastfm_url,
                api_key: lastfm_api_key,
                user: lastfm_user,
                ingest_url: ingest_url.trim_end_matches('/').to_owned(),
                token,
                state_dir,
                interval: Duration::from_secs(interval_seconds),
                pause: LASTFM_PAUSE,
            })
            .await
        }
        Command::ImportApple {
            archive,
            ingest_url,
        } => import_apple(archive, ingest_url).await,
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

/// Même à `RUST_LOG=trace`, les bibliothèques HTTP se taisent : elles citeraient l'URL, donc la clé.
async fn collect(config: collector::Config) -> Result<()> {
    let filter = [
        "reqwest=off",
        "hyper=off",
        "hyper_util=off",
        "h2=off",
        "rustls=off",
    ]
    .iter()
    .fold(
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        |filter, directive| filter.add_directive(directive.parse().expect("directive valide")),
    );
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(filter)
        .init();
    let collector = collector::Collector::new(config)?;
    let (stop, stopped) = watch::channel(false);
    let worker = tokio::spawn(collector.run(stopped));
    shutdown_signal().await;
    stop.send_replace(true);
    worker.await?;
    tracing::info!("stopped");
    Ok(())
}

/// Aucun journal structuré : la sortie se limite à des nombres, des motifs et des empreintes.
async fn import_apple(archive_dir: PathBuf, ingest_url: String) -> Result<()> {
    let token = read_secret("Token de la source : ")?;
    if token.is_empty() {
        bail!("token vide : il se lit sur l'entrée standard");
    }
    let report = apple::run(apple::Config {
        archive_dir,
        ingest_url,
        token,
        max_attempts: 8,
        retry_base: Duration::from_secs(2),
    })
    .await?;
    println!(
        "empreinte {} : {}",
        apple::PLAY_ACTIVITY,
        report.activity_sha256
    );
    println!(
        "empreinte {} : {}",
        apple::DAILY_TRACKS,
        report.daily_sha256
    );
    println!(
        "empreinte {} : {}",
        apple::LIBRARY_TRACKS,
        report.library_sha256
    );
    println!("lignes lues : {}", report.rows);
    println!("accepted : {}", report.accepted);
    println!("duplicate : {}", report.duplicate);
    println!("rejected : {}", report.rejected.values().sum::<u64>());
    for (reason, count) in &report.rejected {
        println!("  {reason} : {count}");
    }
    println!("ignorées : {}", report.ignored.values().sum::<u64>());
    for (reason, count) in &report.ignored {
        println!("  {reason} : {count}");
    }
    println!("artiste, par méthode, sur les lignes envoyées :");
    for (source, count) in &report.attribution {
        println!("  {source} : {count}");
    }
    if !report.rejected.is_empty() {
        bail!("des lignes ont été rejetées : voir les comptes");
    }
    Ok(())
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
        AdminCommand::Coverage { metric } => print_coverage(&pool, &metric).await?,
        AdminCommand::SetPrecedence {
            metric,
            source,
            from,
        } => {
            let from = admin::set_precedence(&pool, &metric, &source, from).await?;
            println!(
                "{source} fait foi pour {metric} à partir de {}",
                from.to_rfc3339()
            );
            print_coverage(&pool, &metric).await?;
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
    read_secret("")
}

/// Au clavier sans écho, ou la première ligne de l'entrée standard.
fn read_secret(prompt: &str) -> Result<String> {
    if std::io::stdin().is_terminal() {
        return Ok(rpassword::prompt_password(prompt)?);
    }
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(line.trim_end_matches(['\r', '\n']).to_owned())
}

/// Des nombres et des dates. Le dernier mot dit s'il reste un trou entre deux sources successives.
async fn print_coverage(pool: &sqlx::PgPool, metric: &str) -> Result<()> {
    let covered = admin::coverage(pool, metric).await?;
    for source in &covered {
        println!(
            "{} : {} événement(s), du {} au {}{}",
            source.source,
            source.events,
            source.first_at.format("%Y-%m-%d %H:%M UTC"),
            source.last_at.format("%Y-%m-%d %H:%M UTC"),
            source.second_at.map_or(String::new(), |second| format!(
                ", {} jour(s) entre les deux premiers",
                (second - source.first_at).num_days()
            )),
        );
    }
    for pair in covered.windows(2) {
        let days = (pair[1].first_at - pair[0].last_at).num_hours() as f64 / 24.0;
        if days > 0.0 {
            println!(
                "trou de {days:.1} jour(s) entre {} et {}",
                pair[0].source, pair[1].source
            );
        } else {
            println!(
                "chevauchement de {:.1} jour(s) entre {} et {}",
                -days, pair[0].source, pair[1].source
            );
        }
    }
    Ok(())
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
