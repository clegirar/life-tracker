//! Le connecteur Steam, par l'API Web de Steam (voir le journal des décisions de l'architecture, 6 nov.
//! 2026) : une clé personnelle (steamcommunity.com/dev/apikey) et mon identifiant Steam 64 bits.
//! Lecture seule, en tirage, toutes les 15 min.
//!
//! Steam ne donne que des compteurs cumulés : `GetOwnedGames` rend, pour chaque jeu, le total joué en
//! minutes depuis toujours. Le temps d'un jour se déduit de l'écart entre deux relevés ; l'état garde
//! donc le dernier relevé de chaque jeu, et chaque relevé envoyé porte le précédent, pour que
//! l'interprète calcule l'écart sans mémoire. Un relevé ne part qu'à la première vue d'un jeu déjà
//! joué, ou quand son compteur change. Sa clé ne dépend que du jeu et du compteur : renvoyé, il donne
//! `duplicate`, même après la perte de l'état.
//!
//! Les succès (`GetPlayerAchievements`) arrivent avec leurs vraies dates. Ils sont demandés pour chaque
//! jeu dont le relevé vient de partir, avec une pause entre deux appels ; un jeu sans succès est noté et
//! n'est plus redemandé. Steam ne les donne que d'un profil public : sinon, les relevés partent quand
//! même, et l'état rapporté demande d'ouvrir le profil (`profile_private`).
//!
//! La clé figure dans l'URL de chaque appel : jamais d'URL ni d'erreur de reqwest dans les logs, ni de
//! nom de jeu. Seulement des nombres et des motifs fixes.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::Duration;

use chrono::{DateTime, SecondsFormat, Utc};
use reqwest::header::{ACCEPT, RETRY_AFTER};
use reqwest::{Client, Response, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::watch;
use tracing::{error, info, warn};

use super::lastfm::network_reason;
use super::{backoff, load_state, rate_limit_wait, report_status, save_state};
use crate::ingest_client::{IngestClient, IngestFailure, RecordOutcome, http_client};
use crate::source_status::{Reason, SourceStatus, State as Health};

pub const DEFAULT_STEAM_URL: &str = "https://api.steampowered.com";
const STATE_FILE: &str = "steam-state.json";
const OWNED_GAMES_PATH: &str = "/IPlayerService/GetOwnedGames/v1/";
const ACHIEVEMENTS_PATH: &str = "/ISteamUserStats/GetPlayerAchievements/v1/";
/// La limite d'un lot de l'API d'ingestion.
const RECORDS_PER_BATCH: usize = 500;
/// Steam n'annonce pas toujours de délai : on s'en tient à ce qu'une limite met à retomber.
const DEFAULT_RATE_LIMIT_DELAY: Duration = Duration::from_secs(300);

pub struct Config {
    pub api_url: String,
    pub api_key: String,
    /// L'identifiant Steam 64 bits du compte.
    pub steam_id: String,
    pub ingest_url: String,
    pub token: String,
    pub state_dir: PathBuf,
    pub interval: Duration,
    /// Entre deux demandes de succès.
    pub pause: Duration,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ServiceError {
    /// La clé est refusée (401 ou 403) : révoquée, il faut en créer une.
    KeyRejected {
        status: u16,
    },
    RateLimited {
        retry_after: Duration,
    },
    /// La liste des jeux arrive vide de jeux : le profil ne les montre pas.
    ProfilePrivate,
    /// Réponse d'erreur : son statut et l'appel refusé, jamais son corps.
    Failed {
        status: u16,
        call: &'static str,
    },
    /// Rien d'exploitable : coupure, délai dépassé, réponse illisible.
    Unavailable {
        reason: &'static str,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub enum TickError {
    Service(ServiceError),
    Ingest {
        reason: &'static str,
        status: Option<u16>,
    },
    State,
}

impl From<ServiceError> for TickError {
    fn from(err: ServiceError) -> Self {
        Self::Service(err)
    }
}

impl From<IngestFailure> for TickError {
    fn from(failure: IngestFailure) -> Self {
        Self::Ingest {
            reason: failure.reason,
            status: failure.status,
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct TickReport {
    /// Appels réussis à Steam.
    pub calls: u32,
    /// Jeux de la bibliothèque.
    pub games: u32,
    /// Relevés envoyés (compteur changé, ou jeu vu pour la première fois).
    pub readings: u32,
    /// Succès débloqués envoyés.
    pub achievements: u32,
    pub accepted: u32,
    pub duplicate: u32,
    pub rejected: u32,
    /// Éléments de Steam sans enregistrement : jeu ou succès illisible, succès sans date.
    pub skipped: u32,
    /// Steam a refusé les succès : le profil n'est pas public. Les relevés sont partis.
    pub profile_private: bool,
    /// Jeux dont Steam a refusé les succès (erreur 500…), sans arrêter le passage.
    pub achievement_failures: u32,
}

// ─── Les listes blanches ─────────────────────────────────────────────────────

fn utc(at: DateTime<Utc>) -> String {
    // `Z` : Steam ne donne pas de décalage, il reste inconnu.
    at.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// L'heure d'un relevé, à la milliseconde : l'interprète exige que le précédent soit strictement avant.
fn observed(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Un jeu de la bibliothèque, tel que `GetOwnedGames` le rend, réduit à ce qui sert.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnedGame {
    pub appid: u64,
    pub name: Option<String>,
    /// Minutes, depuis toujours.
    pub playtime_forever: i64,
    /// Minutes jouées hors ligne.
    pub playtime_disconnected: i64,
    /// Secondes Unix de la dernière partie ; `None` pour 0 (jamais).
    pub rtime_last_played: Option<i64>,
    /// `has_community_visible_stats` : le jeu a des statistiques, donc peut-être des succès. Sans lui,
    /// `GetPlayerAchievements` n'a rien à dire (et répond parfois une erreur 500) : on ne demande pas.
    pub stats: bool,
}

/// Un entier positif ou nul.
fn count(value: Option<&Value>) -> Option<i64> {
    value.and_then(Value::as_i64).filter(|n| *n >= 0)
}

/// `None` pour un jeu sans appid ni compteur lisibles.
pub fn owned_game(raw: &Value) -> Option<OwnedGame> {
    Some(OwnedGame {
        appid: raw.get("appid")?.as_u64().filter(|id| *id > 0)?,
        name: raw.get("name").and_then(Value::as_str).map(str::to_owned),
        playtime_forever: count(raw.get("playtime_forever"))?,
        playtime_disconnected: count(raw.get("playtime_disconnected")).unwrap_or(0),
        rtime_last_played: count(raw.get("rtime_last_played")).filter(|s| *s > 0),
        stats: raw
            .get("has_community_visible_stats")
            .and_then(Value::as_bool)
            == Some(true),
    })
}

/// Le relevé précédent d'un jeu, porté par le suivant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Previous {
    pub playtime_forever: i64,
    pub observed_at: DateTime<Utc>,
}

/// La clé d'un relevé : le jeu et son compteur. Deux relevés au même compteur sont le même fait.
pub fn playtime_key(appid: u64, playtime_forever: i64) -> String {
    format!("playtime:{appid}:{playtime_forever}")
}

/// Un relevé, à l'heure du relevé, avec le précédent du même jeu (`null` pour un point de départ).
pub fn playtime_record(
    game: &OwnedGame,
    previous: Option<Previous>,
    observed_at: DateTime<Utc>,
) -> Value {
    json!({
        "key": playtime_key(game.appid, game.playtime_forever),
        "kind": "steam.playtime",
        "schema_version": 1,
        "occurred_at": observed(observed_at),
        "payload": {
            "appid": game.appid,
            "name": game.name,
            "playtime_forever": game.playtime_forever,
            "playtime_disconnected": game.playtime_disconnected,
            "rtime_last_played": game.rtime_last_played,
            "previous": previous.map(|p| json!({
                "playtime_forever": p.playtime_forever,
                "observed_at": observed(p.observed_at),
            })),
        },
    })
}

/// Un succès débloqué, prêt à partir.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unlocked {
    pub apiname: String,
    pub record: Value,
}

/// Les succès débloqués d'une réponse, à leur date de déblocage ; la liste blanche écarte la
/// description. Un succès verrouillé (`achieved` 0) ne part pas ; un succès sans date ni identifiant
/// lisibles est compté dans `skipped`.
pub fn unlocked(
    appid: u64,
    game_name: Option<&str>,
    achievements: &[Value],
    skipped: &mut u32,
) -> Vec<Unlocked> {
    let mut found = Vec::new();
    for raw in achievements {
        if raw.get("achieved").and_then(Value::as_i64) != Some(1) {
            continue;
        }
        let apiname = raw
            .get("apiname")
            .and_then(Value::as_str)
            .filter(|a| !a.is_empty());
        let at = count(raw.get("unlocktime"))
            .filter(|s| *s > 0)
            .and_then(|s| DateTime::from_timestamp(s, 0));
        let (Some(apiname), Some(at)) = (apiname, at) else {
            *skipped += 1;
            continue;
        };
        found.push(Unlocked {
            apiname: apiname.to_owned(),
            record: json!({
                "key": format!("achievement:{appid}:{apiname}"),
                "kind": "steam.achievement",
                "schema_version": 1,
                "occurred_at": utc(at),
                "payload": {
                    "appid": appid,
                    "game_name": game_name,
                    "apiname": apiname,
                    "name": raw.get("name").and_then(Value::as_str),
                },
            }),
        });
    }
    found
}

// ─── Le client de Steam ──────────────────────────────────────────────────────

fn retry_after(response: &Response) -> Duration {
    response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse().ok())
        .map_or(DEFAULT_RATE_LIMIT_DELAY, Duration::from_secs)
}

fn unavailable(err: &reqwest::Error) -> ServiceError {
    ServiceError::Unavailable {
        reason: network_reason(err),
    }
}

/// Ce que Steam dit des succès d'un jeu.
#[derive(Debug, PartialEq)]
pub enum Achievements {
    /// La liste du jeu, débloqués et verrouillés.
    List(Vec<Value>),
    /// Un jeu sans succès, ou sans statistiques : à ne plus redemander.
    None,
    /// Le profil n'est pas public : rien à lire, pour aucun jeu.
    ProfilePrivate,
}

pub struct Steam {
    http: Client,
    url: String,
    api_key: String,
    steam_id: String,
}

impl Steam {
    pub fn new(http: Client, url: &str, api_key: &str, steam_id: &str) -> Self {
        Self {
            http,
            url: url.trim_end_matches('/').to_owned(),
            api_key: api_key.to_owned(),
            steam_id: steam_id.to_owned(),
        }
    }

    async fn call(&self, path: &str, query: &[(&str, String)]) -> Result<Response, ServiceError> {
        let response = self
            .http
            .get(format!("{}{path}", self.url))
            .query(&[("key", self.api_key.as_str()), ("steamid", &self.steam_id)])
            .query(query)
            .header(ACCEPT, "application/json")
            .send()
            .await
            .map_err(|err| unavailable(&err))?;
        if response.status() == StatusCode::TOO_MANY_REQUESTS {
            return Err(ServiceError::RateLimited {
                retry_after: retry_after(&response),
            });
        }
        Ok(response)
    }

    /// La bibliothèque, jeux gratuits joués compris, avec leurs noms.
    pub async fn owned_games(&self) -> Result<Vec<Value>, ServiceError> {
        let query = [
            ("include_appinfo", "1".to_owned()),
            ("include_played_free_games", "1".to_owned()),
            ("format", "json".to_owned()),
        ];
        let response = self.call(OWNED_GAMES_PATH, &query).await?;
        let status = response.status();
        if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
            return Err(ServiceError::KeyRejected {
                status: status.as_u16(),
            });
        }
        if !status.is_success() {
            return Err(ServiceError::Failed {
                status: status.as_u16(),
                call: "owned_games",
            });
        }
        let body: Value = response
            .json()
            .await
            .map_err(|_| ServiceError::Unavailable { reason: "body" })?;
        let response = body
            .get("response")
            .and_then(Value::as_object)
            .ok_or(ServiceError::Unavailable { reason: "shape" })?;
        match response.get("games") {
            Some(Value::Array(games)) => Ok(games.clone()),
            // `{"response": {}}` : Steam ne montre pas la bibliothèque.
            None => Err(ServiceError::ProfilePrivate),
            Some(_) => Err(ServiceError::Unavailable { reason: "shape" }),
        }
    }

    /// Les succès d'un jeu, avec leurs noms en français quand le jeu les traduit. Steam répond par
    /// `playerstats.success` et un message d'erreur, parfois avec un statut d'erreur (403 pour un profil
    /// privé, 400 pour un jeu sans statistiques) : le corps décide, le statut seulement à défaut.
    pub async fn achievements(&self, appid: u64) -> Result<Achievements, ServiceError> {
        let query = [("appid", appid.to_string()), ("l", "french".to_owned())];
        let response = self.call(ACHIEVEMENTS_PATH, &query).await?;
        let status = response.status();
        let body: Option<Value> = response.json().await.ok();
        if let Some(stats) = body.as_ref().and_then(|b| b.get("playerstats")) {
            if stats.get("success").and_then(Value::as_bool) == Some(true) {
                return Ok(match stats.get("achievements") {
                    Some(Value::Array(list)) => Achievements::List(list.clone()),
                    _ => Achievements::None,
                });
            }
            let message = stats
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_ascii_lowercase();
            if message.contains("not public") {
                return Ok(Achievements::ProfilePrivate);
            }
            if message.contains("no stats") {
                return Ok(Achievements::None);
            }
        }
        if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
            return Err(ServiceError::KeyRejected {
                status: status.as_u16(),
            });
        }
        if !status.is_success() {
            return Err(ServiceError::Failed {
                status: status.as_u16(),
                call: "achievements",
            });
        }
        Err(ServiceError::Unavailable { reason: "shape" })
    }
}

// ─── Le connecteur ───────────────────────────────────────────────────────────

/// Le dernier relevé reçu d'un jeu, et ce qu'on sait de ses succès.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct Game {
    playtime: i64,
    observed_at: DateTime<Utc>,
    /// Les succès déjà reçus par l'API d'ingestion, par leur identifiant.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    achievements: BTreeSet<String>,
    /// Le jeu n'a pas de succès : ne plus demander.
    #[serde(default)]
    no_achievements: bool,
    /// Ses succès restent à demander : son compteur a changé, ou la demande a échoué.
    #[serde(default)]
    achievements_pending: bool,
    /// Demandes de succès refusées d'affilée par Steam pour ce jeu (erreur 500…) ; au-delà de
    /// `MAX_ACHIEVEMENT_FAILURES`, le jeu est tenu pour sans succès.
    #[serde(default, skip_serializing_if = "is_zero")]
    achievement_failures: u32,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// Au-delà, un jeu dont Steam refuse toujours les succès n'est plus demandé.
const MAX_ACHIEVEMENT_FAILURES: u32 = 3;

/// Ce qui survit à un redémarrage. Le perdre ne coûte qu'un point de départ par jeu joué (le temps
/// joué depuis le dernier relevé n'est pas compté) et des `duplicate`.
#[derive(Debug, Default, Deserialize, Serialize)]
struct State {
    /// Le dernier passage réussi : l'heure du relevé précédent d'un jeu apparu depuis.
    last_pass_at: Option<DateTime<Utc>>,
    games: BTreeMap<u64, Game>,
}

/// Un relevé à envoyer.
struct Reading {
    appid: u64,
    playtime: i64,
    stats: bool,
    record: Value,
}

pub struct Collector {
    config: Config,
    steam: Steam,
    ingest: IngestClient,
    state: State,
    /// Passages en échec d'affilée, pour espacer les essais.
    failures: u32,
}

/// L'état rapporté pour la page d'état après un passage en échec (`ok` sinon). Une clé refusée ou un
/// profil fermé demandent une action ; le reste passe seul. Un état qui ne s'écrit plus sur disque n'a
/// pas de code à lui : `service_unavailable`, le connecteur est en panne.
pub fn failure_status(error: &TickError) -> SourceStatus {
    let (state, reason) = match error {
        TickError::Service(ServiceError::KeyRejected { .. }) => {
            (Health::ActionRequired, Reason::KeyRejected)
        }
        TickError::Service(ServiceError::ProfilePrivate) => {
            (Health::ActionRequired, Reason::ProfilePrivate)
        }
        TickError::Service(ServiceError::RateLimited { .. }) => {
            (Health::Error, Reason::RateLimited)
        }
        TickError::Service(_) | TickError::State => (Health::Error, Reason::ServiceUnavailable),
        TickError::Ingest { .. } => (Health::Error, Reason::IngestFailed),
    };
    SourceStatus::failed(state, reason)
}

/// Le motif de log d'un échec : jamais de secret ni de donnée.
fn log_failure(error: &TickError, failures: u32) {
    match error {
        TickError::Service(ServiceError::KeyRejected { status }) => {
            error!(status, failures, "clé Steam refusée : révoquée ou invalide");
        }
        TickError::Service(ServiceError::ProfilePrivate) => {
            warn!(failures, "steam: library not visible, profile not public");
        }
        TickError::Service(ServiceError::RateLimited { retry_after }) => {
            info!(
                retry_after_s = retry_after.as_secs(),
                "steam rate limit, waiting"
            );
        }
        TickError::Service(ServiceError::Failed { status, call }) => {
            warn!(status, call, failures, "steam refused");
        }
        TickError::Service(ServiceError::Unavailable { reason }) => {
            warn!(reason, failures, "steam unavailable");
        }
        TickError::Ingest { reason, status } => {
            error!(reason, status, failures, "steam ingestion unavailable");
        }
        TickError::State => error!(failures, "steam state not saved"),
    }
}

pub fn retry_delay(failures: u32, error: &TickError) -> Duration {
    match error {
        TickError::Service(ServiceError::RateLimited { retry_after }) => {
            rate_limit_wait(*retry_after)
        }
        _ => backoff(failures),
    }
}

impl TickReport {
    fn count(&mut self, outcome: RecordOutcome) {
        match outcome {
            RecordOutcome::Accepted => self.accepted += 1,
            RecordOutcome::Duplicate => self.duplicate += 1,
            RecordOutcome::Rejected => {
                self.rejected += 1;
                warn!("steam record rejected, set aside");
            }
        }
    }
}

impl Collector {
    pub fn new(config: Config) -> anyhow::Result<Self> {
        let http = http_client()?;
        let state = load_state(&config.state_dir, STATE_FILE)?;
        let steam = Steam::new(
            http.clone(),
            &config.api_url,
            &config.api_key,
            &config.steam_id,
        );
        let ingest = IngestClient::new(
            http,
            &config.ingest_url,
            &config.token,
            concat!("collector-", env!("CARGO_PKG_VERSION")),
        );
        Ok(Self {
            config,
            steam,
            ingest,
            state,
            failures: 0,
        })
    }

    /// Le nombre de jeux dont un relevé est gardé.
    pub fn games(&self) -> usize {
        self.state.games.len()
    }

    /// Le dernier passage réussi.
    pub fn last_pass_at(&self) -> Option<DateTime<Utc>> {
        self.state.last_pass_at
    }

    fn save(&self) -> Result<(), TickError> {
        save_state(&self.config.state_dir, STATE_FILE, &self.state).map_err(|_| TickError::State)
    }

    /// Les relevés à envoyer : à la première vue d'un jeu déjà joué, ou quand son compteur change. Au
    /// premier passage, un jeu joué part sans précédent (point de départ) ; ensuite, un jeu apparu
    /// depuis (acheté et joué) part avec un précédent à zéro, à l'heure du dernier passage. Un jeu
    /// jamais joué est seulement noté.
    fn readings(&mut self, games: &[OwnedGame], now: DateTime<Utc>) -> Vec<Reading> {
        let mut readings = Vec::new();
        for game in games {
            let previous = match self.state.games.get(&game.appid) {
                // Une horloge revenue en arrière : le relevé attend le passage suivant.
                Some(known)
                    if known.playtime == game.playtime_forever || known.observed_at >= now =>
                {
                    continue;
                }
                Some(known) => Some(Previous {
                    playtime_forever: known.playtime,
                    observed_at: known.observed_at,
                }),
                None if game.playtime_forever == 0 => {
                    self.state.games.insert(
                        game.appid,
                        Game {
                            playtime: 0,
                            observed_at: now,
                            achievements: BTreeSet::new(),
                            no_achievements: false,
                            achievements_pending: false,
                            achievement_failures: 0,
                        },
                    );
                    continue;
                }
                None => self.state.last_pass_at.map(|at| Previous {
                    playtime_forever: 0,
                    observed_at: at,
                }),
            };
            readings.push(Reading {
                appid: game.appid,
                playtime: game.playtime_forever,
                stats: game.stats,
                record: playtime_record(game, previous, now),
            });
        }
        readings
    }

    /// Par lots ; l'état d'un jeu n'avance que quand son relevé est reçu. Ses succès sont alors à
    /// demander.
    async fn send_readings(
        &mut self,
        readings: Vec<Reading>,
        now: DateTime<Utc>,
        report: &mut TickReport,
    ) -> Result<(), TickError> {
        for batch in readings.chunks(RECORDS_PER_BATCH) {
            let records: Vec<Value> = batch.iter().map(|r| r.record.clone()).collect();
            let results = self.ingest.send(&records).await?;
            for (reading, result) in batch.iter().zip(&results) {
                let outcome = RecordOutcome::of(result);
                report.count(outcome);
                report.readings += 1;
                if outcome == RecordOutcome::Rejected {
                    continue;
                }
                let game = self.state.games.entry(reading.appid).or_insert(Game {
                    playtime: reading.playtime,
                    observed_at: now,
                    achievements: BTreeSet::new(),
                    no_achievements: false,
                    achievements_pending: false,
                    achievement_failures: 0,
                });
                game.playtime = reading.playtime;
                game.observed_at = now;
                game.achievements_pending = reading.stats && !game.no_achievements;
            }
            self.save()?;
        }
        Ok(())
    }

    /// Les succès de chaque jeu qui les attend, un appel par jeu, une pause entre deux. Un profil fermé
    /// arrête la tournée : les jeux restent en attente pour le passage suivant.
    async fn achievements(
        &mut self,
        names: &BTreeMap<u64, Option<String>>,
        report: &mut TickReport,
    ) -> Result<(), TickError> {
        let pending: Vec<u64> = self
            .state
            .games
            .iter()
            .filter(|(_, g)| g.achievements_pending)
            .map(|(appid, _)| *appid)
            .collect();
        for (i, appid) in pending.into_iter().enumerate() {
            if i > 0 {
                tokio::time::sleep(self.config.pause).await;
            }
            let answer = match self.steam.achievements(appid).await {
                // Un refus propre à ce jeu (erreur 500 de Steam…) n'arrête pas la tournée : le jeu reste
                // en attente, et n'est plus demandé après quelques refus d'affilée.
                Err(ServiceError::Failed { status, .. }) => {
                    report.achievement_failures += 1;
                    let game = self.state.games.get_mut(&appid).expect("jeu de l'état");
                    game.achievement_failures += 1;
                    if game.achievement_failures >= MAX_ACHIEVEMENT_FAILURES {
                        game.no_achievements = true;
                        game.achievements_pending = false;
                    }
                    warn!(status, "steam achievements refused for one game");
                    self.save()?;
                    continue;
                }
                other => other?,
            };
            let list = match answer {
                Achievements::ProfilePrivate => {
                    report.calls += 1;
                    report.profile_private = true;
                    return Ok(());
                }
                Achievements::None => {
                    report.calls += 1;
                    let game = self.state.games.get_mut(&appid).expect("jeu de l'état");
                    game.no_achievements = true;
                    game.achievements_pending = false;
                    self.save()?;
                    continue;
                }
                Achievements::List(list) => list,
            };
            report.calls += 1;
            let name = names.get(&appid).cloned().flatten();
            let sent = &self.state.games[&appid].achievements;
            let fresh: Vec<Unlocked> = unlocked(appid, name.as_deref(), &list, &mut report.skipped)
                .into_iter()
                .filter(|u| !sent.contains(&u.apiname))
                .collect();
            for batch in fresh.chunks(RECORDS_PER_BATCH) {
                let records: Vec<Value> = batch.iter().map(|u| u.record.clone()).collect();
                let results = self.ingest.send(&records).await?;
                let game = self.state.games.get_mut(&appid).expect("jeu de l'état");
                for (unlocked, result) in batch.iter().zip(&results) {
                    let outcome = RecordOutcome::of(result);
                    report.count(outcome);
                    report.achievements += 1;
                    if outcome != RecordOutcome::Rejected {
                        game.achievements.insert(unlocked.apiname.clone());
                    }
                }
                self.save()?;
            }
            let game = self.state.games.get_mut(&appid).expect("jeu de l'état");
            game.achievements_pending = false;
            game.achievement_failures = 0;
            self.save()?;
        }
        Ok(())
    }

    /// Un passage. Steam d'abord, l'ingestion ensuite : un appel authentifié compte comme un contact, et
    /// le signal de vie ne part qu'après une liste reçue de Steam. Une clé refusée n'en envoie donc
    /// aucun, et le seuil de silence de la source alerte.
    pub async fn tick(&mut self) -> Result<TickReport, TickError> {
        let mut report = TickReport::default();
        let now = Utc::now();
        let raw = self.steam.owned_games().await?;
        report.calls += 1;
        self.ingest.heartbeat().await?;
        let mut games = Vec::new();
        for raw in &raw {
            match owned_game(raw) {
                Some(game) => games.push(game),
                None => report.skipped += 1,
            }
        }
        report.games = u32::try_from(games.len()).unwrap_or(u32::MAX);
        let readings = self.readings(&games, now);
        self.send_readings(readings, now, &mut report).await?;
        // Un jeu sans statistiques n'a pas de succès à demander, même s'il était en attente.
        for game in games.iter().filter(|g| !g.stats) {
            if let Some(known) = self.state.games.get_mut(&game.appid) {
                known.achievements_pending = false;
            }
        }
        let names = games.iter().map(|g| (g.appid, g.name.clone())).collect();
        self.achievements(&names, &mut report).await?;
        self.state.last_pass_at = Some(now);
        // Toujours réécrit : la date du fichier sert au contrôle de santé du conteneur.
        self.save()?;
        Ok(report)
    }

    /// Un passage, son journal et son état rapporté ; renvoie le délai avant le suivant. Le rapport
    /// d'état vient après le passage et n'en change ni le résultat ni le signal de vie.
    pub async fn pass(&mut self) -> Duration {
        let (delay, mut status) = match self.tick().await {
            Ok(report) => {
                self.failures = 0;
                info!(
                    calls = report.calls,
                    games = report.games,
                    readings = report.readings,
                    achievements = report.achievements,
                    accepted = report.accepted,
                    duplicate = report.duplicate,
                    rejected = report.rejected,
                    skipped = report.skipped,
                    profile_private = report.profile_private,
                    achievement_failures = report.achievement_failures,
                    "steam pass completed"
                );
                let status = if report.profile_private {
                    SourceStatus::failed(Health::ActionRequired, Reason::ProfilePrivate)
                } else {
                    SourceStatus::ok()
                };
                (self.config.interval, status)
            }
            Err(err) => {
                self.failures += 1;
                log_failure(&err, self.failures);
                (retry_delay(self.failures, &err), failure_status(&err))
            }
        };
        status.last_collected_at = self.state.last_pass_at;
        report_status(&self.ingest, status, delay, "steam").await;
        delay
    }

    /// Jusqu'à l'arrêt demandé, comme les autres connecteurs : aucune erreur ne l'arrête.
    pub async fn run(mut self, mut stop: watch::Receiver<bool>) {
        while !*stop.borrow() {
            let delay = self.pass().await;
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                _ = stop.changed() => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(value: &Value) -> Vec<&str> {
        value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect()
    }

    #[test]
    fn the_whitelists_are_frozen() {
        let game = owned_game(&json!({
            "appid": 4242, "name": "Jeu Fictif", "playtime_forever": 55, "img_icon_url": "abc",
            "has_community_visible_stats": true, "playtime_windows_forever": 54,
            "playtime_mac_forever": 0, "playtime_linux_forever": 0, "playtime_deck_forever": 0,
            "rtime_last_played": 1_700_000_000, "content_descriptorids": [5],
            "playtime_disconnected": 3,
        }))
        .unwrap();
        let at: DateTime<Utc> = "2026-11-06T20:00:00Z".parse().unwrap();
        let record = playtime_record(&game, None, at);
        assert_eq!(record["key"], "playtime:4242:55");
        assert_eq!(record["occurred_at"], "2026-11-06T20:00:00.000Z");
        assert_eq!(
            keys(&record["payload"]),
            [
                "appid",
                "name",
                "playtime_disconnected",
                "playtime_forever",
                "previous",
                "rtime_last_played"
            ]
        );
        assert_eq!(record["payload"]["previous"], Value::Null);
        let previous = Previous {
            playtime_forever: 15,
            observed_at: "2026-11-06T19:45:00Z".parse().unwrap(),
        };
        let record = playtime_record(&game, Some(previous), at);
        assert_eq!(
            record["payload"]["previous"],
            json!({"playtime_forever": 15, "observed_at": "2026-11-06T19:45:00.000Z"})
        );

        let mut skipped = 0;
        let found = unlocked(
            4242,
            Some("Jeu Fictif"),
            &[
                json!({"apiname": "A1", "achieved": 1, "unlocktime": 1_700_000_000,
                       "name": "Premier", "description": "écartée"}),
                json!({"apiname": "A2", "achieved": 0, "unlocktime": 0, "name": "Verrouillé"}),
                json!({"apiname": "A3", "achieved": 1, "unlocktime": 0, "name": "Sans date"}),
            ],
            &mut skipped,
        );
        assert_eq!(skipped, 1);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].apiname, "A1");
        assert_eq!(found[0].record["key"], "achievement:4242:A1");
        assert_eq!(found[0].record["occurred_at"], "2023-11-14T22:13:20Z");
        assert_eq!(
            keys(&found[0].record["payload"]),
            ["apiname", "appid", "game_name", "name"]
        );
    }

    #[test]
    fn an_unreadable_game_is_skipped_and_never_played_means_no_last_time() {
        assert_eq!(
            owned_game(&json!({"name": "x", "playtime_forever": 1})),
            None
        );
        assert_eq!(
            owned_game(&json!({"appid": 1, "playtime_forever": -1})),
            None
        );
        let game = owned_game(&json!({"appid": 1, "playtime_forever": 0,
                                      "rtime_last_played": 0}))
        .unwrap();
        assert_eq!(game.rtime_last_played, None);
        assert_eq!(game.playtime_disconnected, 0);
        assert_eq!(game.name, None);
    }

    #[test]
    fn a_rejected_key_or_a_closed_profile_asks_for_an_action() {
        let status = |error| {
            let s = failure_status(&error);
            assert!(s.is_consistent());
            (s.state, s.reason.unwrap())
        };
        assert_eq!(
            status(TickError::Service(ServiceError::KeyRejected {
                status: 403
            })),
            (Health::ActionRequired, Reason::KeyRejected)
        );
        assert_eq!(
            status(TickError::Service(ServiceError::ProfilePrivate)),
            (Health::ActionRequired, Reason::ProfilePrivate)
        );
        assert_eq!(
            status(TickError::Service(ServiceError::RateLimited {
                retry_after: Duration::from_secs(1)
            })),
            (Health::Error, Reason::RateLimited)
        );
        for other in [
            TickError::Service(ServiceError::Failed {
                status: 500,
                call: "owned_games",
            }),
            TickError::Service(ServiceError::Unavailable { reason: "timeout" }),
            TickError::State,
        ] {
            assert_eq!(status(other), (Health::Error, Reason::ServiceUnavailable));
        }
        assert_eq!(
            status(TickError::Ingest {
                reason: "connect",
                status: None
            }),
            (Health::Error, Reason::IngestFailed)
        );
    }
}
