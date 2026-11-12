use std::io::{BufRead, IsTerminal};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use life_tracker::interpreters::INTERPRETERS;
use life_tracker::{admin, api, apple, collector, db, engine, privacy, replay};
use tokio::net::TcpListener;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::watch;
use tracing_subscriber::EnvFilter;

const POLL_INTERVAL: Duration = Duration::from_secs(1);
const LASTFM_PAUSE: Duration = Duration::from_millis(250);
/// Garmin n'annonce aucune limite : une seconde entre deux appels, le rattrapage reste de quelques minutes.
const GARMIN_PAUSE: Duration = Duration::from_secs(1);
/// Tant qu'un accord YouTube est attendu : le code reçu par la web app s'échange dans la minute.
const YOUTUBE_CONSENT_POLL: Duration = Duration::from_secs(60);
/// Steam n'annonce pas de limite par seconde (100 000 appels par jour) : une seconde entre deux
/// demandes de succès, le premier passage reste de quelques minutes.
const STEAM_PAUSE: Duration = Duration::from_secs(1);
/// Une clé personnelle de Riot a droit à 100 appels par 2 min : un appel toutes les 1,25 s y reste.
/// Le premier passage (≈ 930 appels) dure ainsi une vingtaine de minutes.
const RIOT_PAUSE: Duration = Duration::from_millis(1250);

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
// Lue une fois au démarrage : la taille de `Collect` ne coûte rien.
#[allow(clippy::large_enum_variant)]
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
        /// Les zones de confidentialité du MCP, `lat,lon,rayon_m` séparées par `;`, sur le NAS seulement
        #[arg(long, env = "PRIVACY_ZONES", hide_env_values = true)]
        privacy_zones: Option<String>,
    },
    /// Interroge Last.fm, Garmin, YouTube, Steam et GitHub, chacun si sa clé est fournie, et envoie à
    /// l'API d'ingestion, sans jamais connaître la base. Chaque connecteur a son token, son état et ses
    /// nouveaux essais
    Collect {
        #[arg(long, env = "LASTFM_API_KEY", hide_env_values = true)]
        lastfm_api_key: Option<String>,
        #[arg(long, env = "LASTFM_USER", hide_env_values = true)]
        lastfm_user: Option<String>,
        /// Le token de la source Last.fm
        #[arg(long, env = "COLLECTOR_TOKEN", hide_env_values = true)]
        token: Option<String>,
        #[arg(long, env = "LASTFM_API_URL", default_value = collector::DEFAULT_LASTFM_URL)]
        lastfm_url: String,
        #[arg(long, env = "COLLECT_INTERVAL_SECONDS", default_value_t = 300)]
        interval_seconds: u64,
        /// La clé de l'API personnelle de Garmin, créée dans Garmin Connect
        #[arg(long, env = "GARMIN_API_KEY", hide_env_values = true)]
        garmin_api_key: Option<String>,
        /// Le token de la source Garmin
        #[arg(long, env = "GARMIN_TOKEN", hide_env_values = true)]
        garmin_token: Option<String>,
        #[arg(long, env = "GARMIN_API_URL", default_value = collector::garmin::DEFAULT_GARMIN_URL)]
        garmin_url: String,
        /// Le nom de la source Garmin dans le registre, que désignent ses annulations
        #[arg(long, env = "GARMIN_SOURCE", default_value = "garmin")]
        garmin_source: String,
        #[arg(long, env = "GARMIN_INTERVAL_SECONDS", default_value_t = 900)]
        garmin_interval_seconds: u64,
        /// L'échéance de la clé Garmin, `AAAA-MM-JJ`, telle que Garmin l'affiche à sa création
        #[arg(long, env = "GARMIN_KEY_EXPIRES_ON")]
        garmin_key_expires_on: Option<String>,
        /// Le client OAuth « Application Web » de Google Cloud, pour Data Portability
        #[arg(long, env = "YOUTUBE_CLIENT_ID", hide_env_values = true)]
        youtube_client_id: Option<String>,
        #[arg(long, env = "YOUTUBE_CLIENT_SECRET", hide_env_values = true)]
        youtube_client_secret: Option<String>,
        /// L'adresse de retour enregistrée chez Google : la page `/autorisation` de la web app
        #[arg(long, env = "YOUTUBE_REDIRECT_URI")]
        youtube_redirect_uri: Option<String>,
        /// La clé de l'API YouTube Data v3 (durées des vidéos)
        #[arg(long, env = "YOUTUBE_API_KEY", hide_env_values = true)]
        youtube_api_key: Option<String>,
        /// Le token de la source YouTube
        #[arg(long, env = "YOUTUBE_TOKEN", hide_env_values = true)]
        youtube_token: Option<String>,
        /// Le nom de la source YouTube dans le registre, que désignent ses annulations
        #[arg(long, env = "YOUTUBE_SOURCE", default_value = "youtube")]
        youtube_source: String,
        #[arg(long, env = "YOUTUBE_INTERVAL_SECONDS", default_value_t = 300)]
        youtube_interval_seconds: u64,
        #[arg(long, env = "YOUTUBE_OAUTH_URL", default_value = collector::youtube::DEFAULT_OAUTH_URL)]
        youtube_oauth_url: String,
        #[arg(long, env = "YOUTUBE_PORTABILITY_URL", default_value = collector::youtube::DEFAULT_PORTABILITY_URL)]
        youtube_portability_url: String,
        #[arg(long, env = "YOUTUBE_DATA_URL", default_value = collector::youtube::DEFAULT_DATA_URL)]
        youtube_data_url: String,
        /// Le seul préfixe accepté pour télécharger une archive d'export
        #[arg(long, env = "YOUTUBE_STORAGE_URL", default_value = collector::youtube::DEFAULT_STORAGE_URL)]
        youtube_storage_url: String,
        /// La clé de l'API Web de Steam (steamcommunity.com/dev/apikey)
        #[arg(long, env = "STEAM_API_KEY", hide_env_values = true)]
        steam_api_key: Option<String>,
        /// L'identifiant Steam 64 bits du compte
        #[arg(long, env = "STEAM_ID", hide_env_values = true)]
        steam_id: Option<String>,
        /// Le token de la source Steam
        #[arg(long, env = "STEAM_TOKEN", hide_env_values = true)]
        steam_token: Option<String>,
        #[arg(long, env = "STEAM_API_URL", default_value = collector::steam::DEFAULT_STEAM_URL)]
        steam_url: String,
        #[arg(long, env = "STEAM_INTERVAL_SECONDS", default_value_t = 900)]
        steam_interval_seconds: u64,
        /// Le token d'accès personnel de GitHub (classique, `repo` et `read:org`)
        #[arg(long, env = "GITHUB_API_TOKEN", hide_env_values = true)]
        github_api_token: Option<String>,
        /// Le token de la source GitHub
        #[arg(long, env = "GITHUB_INGEST_TOKEN", hide_env_values = true)]
        github_token: Option<String>,
        #[arg(long, env = "GITHUB_API_URL", default_value = collector::github::DEFAULT_GITHUB_URL)]
        github_url: String,
        #[arg(long, env = "GITHUB_INTERVAL_SECONDS", default_value_t = 900)]
        github_interval_seconds: u64,
        /// L'échéance du token d'accès personnel, `AAAA-MM-JJ`, telle que GitHub l'affiche à sa création
        #[arg(long, env = "GITHUB_TOKEN_EXPIRES_ON")]
        github_token_expires_on: Option<String>,
        /// La clé de l'API de Riot du produit League of Legends
        #[arg(long, env = "RIOT_LOL_API_KEY", hide_env_values = true)]
        riot_lol_api_key: Option<String>,
        /// La clé de l'API de Riot du produit Teamfight Tactics
        #[arg(long, env = "RIOT_TFT_API_KEY", hide_env_values = true)]
        riot_tft_api_key: Option<String>,
        /// Le Riot ID, `nom#tag`
        #[arg(long, env = "RIOT_ID", hide_env_values = true)]
        riot_id: Option<String>,
        /// Le token de la source Riot
        #[arg(long, env = "RIOT_TOKEN", hide_env_values = true)]
        riot_token: Option<String>,
        #[arg(long, env = "RIOT_API_URL", default_value = collector::riot::DEFAULT_RIOT_URL)]
        riot_url: String,
        #[arg(long, env = "RIOT_INTERVAL_SECONDS", default_value_t = 900)]
        riot_interval_seconds: u64,
        /// La clé de l'API non officielle HenrikDev (VALORANT)
        #[arg(long, env = "HENRIK_API_KEY", hide_env_values = true)]
        henrik_api_key: Option<String>,
        /// Le Riot ID du compte VALORANT, `nom#tag`
        #[arg(long, env = "VALORANT_RIOT_ID", hide_env_values = true)]
        valorant_riot_id: Option<String>,
        /// Le token de la source VALORANT
        #[arg(long, env = "VALORANT_TOKEN", hide_env_values = true)]
        valorant_token: Option<String>,
        #[arg(long, env = "VALORANT_REGION", default_value = "eu")]
        valorant_region: String,
        #[arg(long, env = "VALORANT_INTERVAL_SECONDS", default_value_t = 900)]
        valorant_interval_seconds: u64,
        #[arg(long, env = "HENRIK_API_URL", default_value = collector::valorant::DEFAULT_HENRIK_URL)]
        henrik_url: String,
        #[arg(long, env = "INGEST_URL")]
        ingest_url: String,
        /// Où garder l'état de chaque connecteur ; le perdre ne coûte qu'un rattrapage
        #[arg(
            long,
            env = "COLLECTOR_STATE_DIR",
            default_value = "/var/lib/life-tracker"
        )]
        state_dir: PathBuf,
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
        /// Le nom affiché par la page d'état, par exemple « Garmin Connect »
        #[arg(long)]
        label: Option<String>,
    },
    /// Fixe le nom affiché d'une source par la page d'état
    SetLabel { source: String, label: String },
    /// Révoque une source et son token
    RevokeSource { name: String },
    /// Crée un jeton de service de l'API de lecture (serveur MCP, niveau pseudonymes, sans écriture) ;
    /// il n'est affiché qu'à cette occasion
    CreateServiceToken { name: String },
    /// Révoque un jeton de service
    RevokeServiceToken { name: String },
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
            privacy_zones,
        } => {
            serve(
                &database_url,
                &listen,
                dev_insecure_cookie,
                privacy_zones.as_deref(),
            )
            .await
        }
        Command::Collect {
            lastfm_api_key,
            lastfm_user,
            token,
            lastfm_url,
            interval_seconds,
            garmin_api_key,
            garmin_token,
            garmin_url,
            garmin_source,
            garmin_interval_seconds,
            garmin_key_expires_on,
            youtube_client_id,
            youtube_client_secret,
            youtube_redirect_uri,
            youtube_api_key,
            youtube_token,
            youtube_source,
            youtube_interval_seconds,
            youtube_oauth_url,
            youtube_portability_url,
            youtube_data_url,
            youtube_storage_url,
            steam_api_key,
            steam_id,
            steam_token,
            steam_url,
            steam_interval_seconds,
            github_api_token,
            github_token,
            github_url,
            github_interval_seconds,
            github_token_expires_on,
            riot_lol_api_key,
            riot_tft_api_key,
            riot_id,
            riot_token,
            riot_url,
            riot_interval_seconds,
            henrik_api_key,
            valorant_riot_id,
            valorant_token,
            valorant_region,
            valorant_interval_seconds,
            henrik_url,
            ingest_url,
            state_dir,
        } => {
            let ingest_url = ingest_url.trim_end_matches('/').to_owned();
            let set = |value: Option<String>| value.filter(|v| !v.is_empty());
            let lastfm = match (set(lastfm_api_key), set(lastfm_user), set(token)) {
                (None, None, None) => None,
                (Some(api_key), Some(user), Some(token)) => Some(collector::Config {
                    lastfm_url,
                    api_key,
                    user,
                    ingest_url: ingest_url.clone(),
                    token,
                    state_dir: state_dir.clone(),
                    interval: Duration::from_secs(interval_seconds),
                    pause: LASTFM_PAUSE,
                }),
                _ => {
                    bail!("Last.fm demande LASTFM_API_KEY, LASTFM_USER et COLLECTOR_TOKEN ensemble")
                }
            };
            let key_expires_at = set(garmin_key_expires_on)
                .map(|text| {
                    collector::garmin::key_expiry(&text).context(
                        "GARMIN_KEY_EXPIRES_ON illisible : une date AAAA-MM-JJ est attendue",
                    )
                })
                .transpose()?;
            let garmin = match (set(garmin_api_key), set(garmin_token)) {
                (None, None) => None,
                (Some(api_key), Some(token)) => Some(collector::garmin::Config {
                    api_url: garmin_url,
                    api_key,
                    ingest_url: ingest_url.clone(),
                    token,
                    source: garmin_source,
                    state_dir: state_dir.clone(),
                    interval: Duration::from_secs(garmin_interval_seconds),
                    pause: GARMIN_PAUSE,
                    key_expires_at,
                }),
                _ => bail!("Garmin demande GARMIN_API_KEY et GARMIN_TOKEN ensemble"),
            };
            let youtube = match (
                set(youtube_client_id),
                set(youtube_client_secret),
                set(youtube_redirect_uri),
                set(youtube_api_key),
                set(youtube_token),
            ) {
                (None, None, None, None, None) => None,
                (
                    Some(client_id),
                    Some(client_secret),
                    Some(redirect_uri),
                    Some(api_key),
                    Some(token),
                ) => Some(collector::youtube::Config {
                    client_id,
                    client_secret,
                    redirect_uri,
                    api_key,
                    ingest_url: ingest_url.clone(),
                    token,
                    source: youtube_source,
                    state_dir: state_dir.clone(),
                    interval: Duration::from_secs(youtube_interval_seconds),
                    consent_poll: YOUTUBE_CONSENT_POLL,
                    auth_url: collector::youtube::DEFAULT_AUTH_URL.to_owned(),
                    oauth_url: youtube_oauth_url,
                    portability_url: youtube_portability_url,
                    data_url: youtube_data_url,
                    storage_url: youtube_storage_url,
                    videos_per_call: collector::youtube::VIDEOS_PER_CALL,
                    classify_calls_per_pass: collector::youtube::CLASSIFY_CALLS_PER_PASS,
                }),
                _ => bail!(
                    "YouTube demande YOUTUBE_CLIENT_ID, YOUTUBE_CLIENT_SECRET, YOUTUBE_REDIRECT_URI, \
                     YOUTUBE_API_KEY et YOUTUBE_TOKEN ensemble"
                ),
            };
            let steam = match (set(steam_api_key), set(steam_id), set(steam_token)) {
                (None, None, None) => None,
                (Some(api_key), Some(steam_id), Some(token)) => Some(collector::steam::Config {
                    api_url: steam_url,
                    api_key,
                    steam_id,
                    ingest_url: ingest_url.clone(),
                    token,
                    state_dir: state_dir.clone(),
                    interval: Duration::from_secs(steam_interval_seconds),
                    pause: STEAM_PAUSE,
                }),
                _ => bail!("Steam demande STEAM_API_KEY, STEAM_ID et STEAM_TOKEN ensemble"),
            };
            let token_expires_at = set(github_token_expires_on)
                .map(|text| {
                    collector::garmin::key_expiry(&text).context(
                        "GITHUB_TOKEN_EXPIRES_ON illisible : une date AAAA-MM-JJ est attendue",
                    )
                })
                .transpose()?;
            let riot = match (
                set(riot_lol_api_key),
                set(riot_tft_api_key),
                set(riot_id),
                set(riot_token),
            ) {
                (None, None, None, None) => None,
                (Some(lol_api_key), Some(tft_api_key), Some(riot_id), Some(token)) => {
                    Some(collector::riot::Config {
                        api_url: riot_url,
                        lol_api_key,
                        tft_api_key,
                        riot_id,
                        ingest_url: ingest_url.clone(),
                        token,
                        state_dir: state_dir.clone(),
                        interval: Duration::from_secs(riot_interval_seconds),
                        pause: RIOT_PAUSE,
                    })
                }
                _ => bail!(
                    "Riot demande RIOT_LOL_API_KEY, RIOT_TFT_API_KEY, RIOT_ID et RIOT_TOKEN ensemble"
                ),
            };
            let valorant = match (
                set(henrik_api_key),
                set(valorant_riot_id),
                set(valorant_token),
            ) {
                (None, None, None) => None,
                (Some(api_key), Some(riot_id), Some(token)) => Some(collector::valorant::Config {
                    api_url: henrik_url,
                    api_key,
                    riot_id,
                    region: valorant_region,
                    ingest_url: ingest_url.clone(),
                    token,
                    state_dir: state_dir.clone(),
                    interval: Duration::from_secs(valorant_interval_seconds),
                }),
                _ => bail!(
                    "VALORANT demande HENRIK_API_KEY, VALORANT_RIOT_ID et VALORANT_TOKEN ensemble"
                ),
            };
            let github = match (set(github_api_token), set(github_token)) {
                (None, None) => None,
                (Some(api_token), Some(token)) => Some(collector::github::Config {
                    api_url: github_url,
                    api_token,
                    ingest_url,
                    token,
                    state_dir,
                    interval: Duration::from_secs(github_interval_seconds),
                    token_expires_at,
                }),
                _ => bail!("GitHub demande GITHUB_API_TOKEN et GITHUB_INGEST_TOKEN ensemble"),
            };
            if lastfm.is_none()
                && garmin.is_none()
                && youtube.is_none()
                && steam.is_none()
                && github.is_none()
                && riot.is_none()
                && valorant.is_none()
            {
                bail!(
                    "aucun connecteur : LASTFM_API_KEY (Last.fm), GARMIN_API_KEY (Garmin), \
                     YOUTUBE_CLIENT_ID (YouTube), STEAM_API_KEY (Steam), GITHUB_API_TOKEN \
                     (GitHub), RIOT_LOL_API_KEY (Riot) ou HENRIK_API_KEY (VALORANT) est requis"
                );
            }
            collect(lastfm, garmin, youtube, steam, github, riot, valorant).await
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
/// Un connecteur par tâche : la panne de l'un n'arrête jamais l'autre.
async fn collect(
    lastfm: Option<collector::Config>,
    garmin: Option<collector::garmin::Config>,
    youtube: Option<collector::youtube::Config>,
    steam: Option<collector::steam::Config>,
    github: Option<collector::github::Config>,
    riot: Option<collector::riot::Config>,
    valorant: Option<collector::valorant::Config>,
) -> Result<()> {
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
    let (stop, stopped) = watch::channel(false);
    let mut workers = Vec::new();
    if let Some(config) = lastfm {
        let connector = collector::Collector::new(config)?;
        workers.push(tokio::spawn(connector.run(stopped.clone())));
    }
    if let Some(config) = garmin {
        let connector = collector::garmin::Collector::new(config)?;
        workers.push(tokio::spawn(connector.run(stopped.clone())));
    }
    if let Some(config) = youtube {
        let connector = collector::youtube::Collector::new(config)?;
        workers.push(tokio::spawn(connector.run(stopped.clone())));
    }
    if let Some(config) = steam {
        let connector = collector::steam::Collector::new(config)?;
        workers.push(tokio::spawn(connector.run(stopped.clone())));
    }
    if let Some(config) = github {
        let connector = collector::github::Collector::new(config)?;
        workers.push(tokio::spawn(connector.run(stopped.clone())));
    }
    if let Some(config) = riot {
        let connector = collector::riot::Collector::new(config)?;
        workers.push(tokio::spawn(connector.run(stopped.clone())));
    }
    if let Some(config) = valorant {
        let connector = collector::valorant::Collector::new(config)?;
        workers.push(tokio::spawn(connector.run(stopped.clone())));
    }
    shutdown_signal().await;
    stop.send_replace(true);
    for worker in workers {
        worker.await?;
    }
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

async fn serve(
    database_url: &str,
    listen: &str,
    insecure_cookie: bool,
    privacy_zones: Option<&str>,
) -> Result<()> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    if insecure_cookie {
        tracing::warn!("cookie de session sans Secure : développement seulement");
    }
    // Le nombre de zones seulement, jamais leurs valeurs.
    let (zones, status) = privacy::Zones::parse(privacy_zones);
    match status {
        privacy::Status::Zones(count) => {
            tracing::info!("zone de confidentialité : {count} zone(s)")
        }
        privacy::Status::Absent => {
            tracing::warn!("zone de confidentialité absente : coordonnées masquées pour le MCP");
        }
        privacy::Status::Invalid => {
            tracing::warn!("zone de confidentialité illisible : coordonnées masquées pour le MCP");
        }
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
        api::router_with(
            pool.clone(),
            api::Config {
                insecure_cookie,
                zones,
            },
        ),
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
            label,
        } => {
            let token =
                admin::create_source(&pool, &name, silence_threshold.as_deref(), label.as_deref())
                    .await?;
            // Sur stdout, seul : scriptable. Il ne sera plus jamais affiché.
            println!("{token}");
            eprintln!("source {name} créée ; conservez ce token, il ne sera plus affiché");
        }
        AdminCommand::SetLabel { source, label } => {
            admin::set_label(&pool, &source, &label).await?;
            eprintln!("libellé de {source} enregistré");
        }
        AdminCommand::RevokeSource { name } => {
            admin::revoke_source(&pool, &name).await?;
            eprintln!("source {name} révoquée");
        }
        AdminCommand::CreateServiceToken { name } => {
            let token = admin::create_service_token(&pool, &name).await?;
            println!("{token}");
            eprintln!("jeton de service {name} créé ; conservez-le, il ne sera plus affiché");
        }
        AdminCommand::RevokeServiceToken { name } => {
            admin::revoke_service_token(&pool, &name).await?;
            eprintln!("jeton de service {name} révoqué");
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
