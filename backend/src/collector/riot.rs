//! Le connecteur Riot, par l'API officielle de Riot (voir le chantier Riot et le journal des décisions
//! de l'architecture, 12 nov. 2026) : League of Legends (`match-v5`) et Teamfight Tactics
//! (`tft-match-v1`), routage `europe`. Riot exige un produit par jeu : chaque API a sa clé
//! (`X-Riot-Token`), et le PUUID, chiffré par produit, se redemande avec chaque clé à chaque passage
//! (`account-v1`, depuis le Riot ID). Lecture seule, en tirage, toutes les 15 min.
//!
//! Chaque partie est un enregistrement, à son début, réduit à ma ligne : jamais les autres joueurs, ni
//! mon PUUID. Sa clé est son identifiant (`lol:<id>`, `tft:<id>`) : renvoyée, elle donne `duplicate`,
//! même après la perte de l'état. Le premier passage parcourt toute la liste (deux ans chez Riot, ≈ 930
//! appels) ; ensuite, la première page de 100, et les suivantes tant qu'une page pleine contient une
//! partie inconnue. Une pause sépare deux appels (la limite d'une clé personnelle est de 100 appels par
//! 2 min) ; un 429 arrête le passage, ce qui a été lu part, et le suivant reprend là.
//!
//! Logs : des nombres et le jeu (`lol`, `tft`) seulement ; jamais le Riot ID, le PUUID, un champion,
//! une URL ni une erreur de reqwest.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;

use chrono::{DateTime, SecondsFormat, Utc};
use reqwest::header::{ACCEPT, RETRY_AFTER};
use reqwest::{Client, Response, StatusCode, Url};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::watch;
use tracing::{error, info, warn};

use super::lastfm::network_reason;
use super::{backoff, load_state, rate_limit_wait, report_status, save_state};
use crate::ingest_client::{IngestClient, IngestFailure, RecordOutcome, http_client};
use crate::source_status::{Reason, SourceStatus, State as Health};

pub const DEFAULT_RIOT_URL: &str = "https://europe.api.riotgames.com";
const STATE_FILE: &str = "riot-state.json";
/// Le plus grand `count` que Riot accepte pour une liste.
pub const IDS_PER_PAGE: usize = 100;
/// Les parties lues partent par lots de cette taille : un 429 au milieu n'en perd jamais plus.
const MATCHES_PER_BATCH: usize = 20;
/// Riot donne `Retry-After` ; à défaut, ce que met à retomber la limite de 2 min.
const DEFAULT_RATE_LIMIT_DELAY: Duration = Duration::from_secs(120);

pub struct Config {
    /// Une seule base pour `account-v1` et les deux API de parties (`europe`).
    pub api_url: String,
    pub lol_api_key: String,
    pub tft_api_key: String,
    /// `nom#tag`.
    pub riot_id: String,
    pub ingest_url: String,
    pub token: String,
    pub state_dir: PathBuf,
    pub interval: Duration,
    /// Entre deux appels à Riot.
    pub pause: Duration,
}

/// Un Riot ID `nom#tag`, coupé au dernier `#`. `None` s'il manque l'un des deux.
pub fn split_riot_id(riot_id: &str) -> Option<(String, String)> {
    let (name, tag) = riot_id.trim().rsplit_once('#')?;
    let (name, tag) = (name.trim(), tag.trim());
    (!name.is_empty() && !tag.is_empty()).then(|| (name.to_owned(), tag.to_owned()))
}

/// Une URL de la base suivie de segments encodés un à un : un Riot ID peut contenir des espaces et des
/// lettres accentuées.
pub(crate) fn url_with(base: &str, segments: &[&str]) -> Result<Url, &'static str> {
    let mut url = Url::parse(base.trim_end_matches('/')).map_err(|_| "url")?;
    url.path_segments_mut()
        .map_err(|_| "url")?
        .pop_if_empty()
        .extend(segments);
    Ok(url)
}

/// Les deux jeux de l'API officielle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Game {
    Lol,
    Tft,
}

impl Game {
    pub fn code(self) -> &'static str {
        match self {
            Self::Lol => "lol",
            Self::Tft => "tft",
        }
    }

    fn api(self) -> &'static [&'static str] {
        match self {
            Self::Lol => &["lol", "match", "v5", "matches"],
            Self::Tft => &["tft", "match", "v1", "matches"],
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ServiceError {
    /// La clé d'un jeu est refusée (401 ou 403) : expirée, révoquée, ou pas encore active.
    KeyRejected {
        status: u16,
        game: &'static str,
    },
    RateLimited {
        retry_after: Duration,
    },
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
    /// Appels réussis à Riot.
    pub calls: u32,
    /// Parties envoyées.
    pub matches: u32,
    pub accepted: u32,
    pub duplicate: u32,
    pub rejected: u32,
    /// Parties sans enregistrement : introuvables (404), ou sans ma ligne, ou illisibles.
    pub skipped: u32,
}

impl TickReport {
    fn count(&mut self, outcome: RecordOutcome) {
        self.matches += 1;
        match outcome {
            RecordOutcome::Accepted => self.accepted += 1,
            RecordOutcome::Duplicate => self.duplicate += 1,
            RecordOutcome::Rejected => {
                self.rejected += 1;
                warn!("riot record rejected, set aside");
            }
        }
    }
}

// ─── Les listes blanches ─────────────────────────────────────────────────────

fn millis(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn instant_ms(value: Option<&Value>) -> Option<DateTime<Utc>> {
    value
        .and_then(Value::as_i64)
        .filter(|ms| *ms > 0)
        .and_then(DateTime::from_timestamp_millis)
}

fn text(value: Option<&Value>) -> Option<&str> {
    value.and_then(Value::as_str).filter(|t| !t.is_empty())
}

/// Ma ligne parmi les participants, par mon PUUID.
fn my_line<'a>(info: &'a Value, puuid: &str) -> Option<&'a Value> {
    info.get("participants")?
        .as_array()?
        .iter()
        .find(|p| p.get("puuid").and_then(Value::as_str) == Some(puuid))
}

/// Une partie de League of Legends, réduite à ma ligne. `None` sans début, sans durée ou sans moi.
/// `gameDuration` est en secondes quand `gameEndTimestamp` existe, en millisecondes avant (parties
/// d'avant 2021, hors des deux ans que Riot garde, mais le cas est documenté).
pub fn lol_record(match_id: &str, raw: &Value, puuid: &str) -> Option<Value> {
    let info = raw.get("info")?;
    let started = instant_ms(info.get("gameStartTimestamp"))?;
    let ended = instant_ms(info.get("gameEndTimestamp"));
    let duration = info.get("gameDuration").and_then(Value::as_i64)?;
    let duration_s = if ended.is_some() {
        duration
    } else {
        duration / 1000
    };
    if duration_s <= 0 {
        return None;
    }
    let me = my_line(info, puuid)?;
    Some(json!({
        "key": format!("lol:{match_id}"),
        "kind": "riot.lol_match",
        "schema_version": 1,
        "occurred_at": millis(started),
        "payload": {
            "match_id": match_id,
            "started_at": millis(started),
            "ended_at": ended.map(millis),
            "duration_s": duration_s,
            "queue_id": info.get("queueId").and_then(Value::as_i64)?,
            "game_mode": text(info.get("gameMode")),
            "end_of_game_result": text(info.get("endOfGameResult")),
            "champion": text(me.get("championName")),
            "win": me.get("win").and_then(Value::as_bool).unwrap_or(false),
            "position": text(me.get("teamPosition")),
            "remake": me.get("gameEndedInEarlySurrender").and_then(Value::as_bool).unwrap_or(false),
        },
    }))
}

/// Une partie de Teamfight Tactics, réduite à ma ligne. Le début est `gameCreation` (`game_datetime`
/// est la fin) ; mon temps de jeu, `time_eliminated`. `None` sans début, sans temps ou sans moi.
pub fn tft_record(match_id: &str, raw: &Value, puuid: &str) -> Option<Value> {
    let info = raw.get("info")?;
    let started = instant_ms(info.get("gameCreation"))?;
    let me = my_line(info, puuid)?;
    let eliminated = me
        .get("time_eliminated")
        .and_then(Value::as_f64)
        .filter(|s| *s > 0.0)?;
    Some(json!({
        "key": format!("tft:{match_id}"),
        "kind": "riot.tft_match",
        "schema_version": 1,
        "occurred_at": millis(started),
        "payload": {
            "match_id": match_id,
            "started_at": millis(started),
            "ended_at": instant_ms(info.get("game_datetime")).map(millis),
            "game_length_s": info.get("game_length").and_then(Value::as_f64),
            "eliminated_s": eliminated,
            "queue_id": info.get("queue_id").and_then(Value::as_i64)?,
            "game_type": text(info.get("tft_game_type")),
            "set_number": info.get("tft_set_number").and_then(Value::as_i64),
            "placement": me.get("placement").and_then(Value::as_i64)?,
            "partner_group_id": me.get("partner_group_id").and_then(Value::as_i64),
        },
    }))
}

fn record(game: Game, match_id: &str, raw: &Value, puuid: &str) -> Option<Value> {
    match game {
        Game::Lol => lol_record(match_id, raw, puuid),
        Game::Tft => tft_record(match_id, raw, puuid),
    }
}

// ─── Le client de Riot ───────────────────────────────────────────────────────

fn retry_after(response: &Response) -> Duration {
    response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse().ok())
        .map_or(DEFAULT_RATE_LIMIT_DELAY, Duration::from_secs)
}

pub struct Riot {
    http: Client,
    url: String,
}

impl Riot {
    pub fn new(http: Client, url: &str) -> Self {
        Self {
            http,
            url: url.trim_end_matches('/').to_owned(),
        }
    }

    /// Un appel : `None` pour un 404 (une partie que Riot ne garde plus).
    async fn get(
        &self,
        key: &str,
        game: Game,
        segments: &[&str],
        query: &[(&str, String)],
        call: &'static str,
    ) -> Result<Option<Value>, ServiceError> {
        let url =
            url_with(&self.url, segments).map_err(|reason| ServiceError::Unavailable { reason })?;
        let response = self
            .http
            .get(url)
            .query(query)
            .header("X-Riot-Token", key)
            .header(ACCEPT, "application/json")
            .send()
            .await
            .map_err(|err| ServiceError::Unavailable {
                reason: network_reason(&err),
            })?;
        let status = response.status();
        if status == StatusCode::TOO_MANY_REQUESTS {
            return Err(ServiceError::RateLimited {
                retry_after: retry_after(&response),
            });
        }
        if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
            return Err(ServiceError::KeyRejected {
                status: status.as_u16(),
                game: game.code(),
            });
        }
        if status == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !status.is_success() {
            return Err(ServiceError::Failed {
                status: status.as_u16(),
                call,
            });
        }
        response
            .json()
            .await
            .map(Some)
            .map_err(|_| ServiceError::Unavailable { reason: "body" })
    }

    /// Le PUUID du Riot ID, chiffré pour la clé donnée. Un Riot ID inconnu (404) est un refus.
    pub async fn puuid(
        &self,
        key: &str,
        game: Game,
        name: &str,
        tag: &str,
    ) -> Result<String, ServiceError> {
        let segments = ["riot", "account", "v1", "accounts", "by-riot-id", name, tag];
        let body = self
            .get(key, game, &segments, &[], "account")
            .await?
            .ok_or(ServiceError::Failed {
                status: 404,
                call: "account",
            })?;
        body.get("puuid")
            .and_then(Value::as_str)
            .filter(|p| !p.is_empty())
            .map(str::to_owned)
            .ok_or(ServiceError::Unavailable { reason: "shape" })
    }

    /// Une page d'identifiants, des plus récentes aux plus anciennes.
    pub async fn ids(
        &self,
        key: &str,
        game: Game,
        puuid: &str,
        start: usize,
    ) -> Result<Vec<String>, ServiceError> {
        let mut segments = game.api().to_vec();
        segments.extend(["by-puuid", puuid, "ids"]);
        let query = [
            ("start", start.to_string()),
            ("count", IDS_PER_PAGE.to_string()),
        ];
        let body =
            self.get(key, game, &segments, &query, "ids")
                .await?
                .ok_or(ServiceError::Failed {
                    status: 404,
                    call: "ids",
                })?;
        let ids = body
            .as_array()
            .ok_or(ServiceError::Unavailable { reason: "shape" })?;
        Ok(ids
            .iter()
            .filter_map(Value::as_str)
            .filter(|id| !id.is_empty())
            .map(str::to_owned)
            .collect())
    }

    /// Le détail d'une partie ; `None` si Riot ne la connaît plus.
    pub async fn detail(
        &self,
        key: &str,
        game: Game,
        match_id: &str,
    ) -> Result<Option<Value>, ServiceError> {
        let mut segments = game.api().to_vec();
        segments.push(match_id);
        self.get(key, game, &segments, &[], "match").await
    }
}

// ─── Le connecteur ───────────────────────────────────────────────────────────

/// Ce qu'on sait d'un jeu.
#[derive(Debug, Default, Deserialize, Serialize)]
struct Track {
    /// Les parties traitées : reçues par l'API d'ingestion, rejetées, ou sans enregistrement.
    #[serde(default)]
    done: BTreeSet<String>,
    /// Toute la liste a été parcourue une fois : la suite ne lit que le début.
    #[serde(default)]
    complete: bool,
}

/// Ce qui survit à un redémarrage. Le perdre ne coûte qu'un nouveau parcours, en `duplicate`.
#[derive(Debug, Default, Deserialize, Serialize)]
struct State {
    last_pass_at: Option<DateTime<Utc>>,
    #[serde(default)]
    lol: Track,
    #[serde(default)]
    tft: Track,
}

impl State {
    fn track(&mut self, game: Game) -> &mut Track {
        match game {
            Game::Lol => &mut self.lol,
            Game::Tft => &mut self.tft,
        }
    }
}

pub struct Collector {
    config: Config,
    name: String,
    tag: String,
    riot: Riot,
    ingest: IngestClient,
    state: State,
    /// Le signal de vie de ce passage est parti.
    alive: bool,
    /// Un appel à Riot a déjà eu lieu dans ce passage : le suivant attend la pause.
    called: bool,
    failures: u32,
}

/// L'état rapporté après un passage en échec (`ok` sinon). Une clé refusée demande une action ; le
/// reste passe seul.
pub fn failure_status(error: &TickError) -> SourceStatus {
    let (state, reason) = match error {
        TickError::Service(ServiceError::KeyRejected { .. }) => {
            (Health::ActionRequired, Reason::KeyRejected)
        }
        TickError::Service(ServiceError::RateLimited { .. }) => {
            (Health::Error, Reason::RateLimited)
        }
        TickError::Service(_) | TickError::State => (Health::Error, Reason::ServiceUnavailable),
        TickError::Ingest { .. } => (Health::Error, Reason::IngestFailed),
    };
    SourceStatus::failed(state, reason)
}

fn log_failure(error: &TickError, failures: u32) {
    match error {
        TickError::Service(ServiceError::KeyRejected { status, game }) => {
            error!(
                status,
                game, failures, "clé Riot refusée : expirée, révoquée ou invalide"
            );
        }
        TickError::Service(ServiceError::RateLimited { retry_after }) => {
            info!(
                retry_after_s = retry_after.as_secs(),
                "riot rate limit, waiting"
            );
        }
        TickError::Service(ServiceError::Failed { status, call }) => {
            warn!(status, call, failures, "riot refused");
        }
        TickError::Service(ServiceError::Unavailable { reason }) => {
            warn!(reason, failures, "riot unavailable");
        }
        TickError::Ingest { reason, status } => {
            error!(reason, status, failures, "riot ingestion unavailable");
        }
        TickError::State => error!(failures, "riot state not saved"),
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

impl Collector {
    pub fn new(config: Config) -> anyhow::Result<Self> {
        let (name, tag) = split_riot_id(&config.riot_id)
            .ok_or_else(|| anyhow::anyhow!("RIOT_ID illisible : `nom#tag` est attendu"))?;
        let http = http_client()?;
        let state = load_state(&config.state_dir, STATE_FILE)?;
        let riot = Riot::new(http.clone(), &config.api_url);
        let ingest = IngestClient::new(
            http,
            &config.ingest_url,
            &config.token,
            concat!("collector-", env!("CARGO_PKG_VERSION")),
        );
        Ok(Self {
            config,
            name,
            tag,
            riot,
            ingest,
            state,
            alive: false,
            called: false,
            failures: 0,
        })
    }

    pub fn last_pass_at(&self) -> Option<DateTime<Utc>> {
        self.state.last_pass_at
    }

    /// Les parties traitées d'un jeu.
    pub fn done(&self, game: Game) -> usize {
        match game {
            Game::Lol => self.state.lol.done.len(),
            Game::Tft => self.state.tft.done.len(),
        }
    }

    fn save(&self) -> Result<(), TickError> {
        save_state(&self.config.state_dir, STATE_FILE, &self.state).map_err(|_| TickError::State)
    }

    fn key(&self, game: Game) -> &str {
        match game {
            Game::Lol => &self.config.lol_api_key,
            Game::Tft => &self.config.tft_api_key,
        }
    }

    /// La pause entre deux appels à Riot, sauf avant le premier du passage.
    async fn pace(&mut self) {
        if self.called {
            tokio::time::sleep(self.config.pause).await;
        }
        self.called = true;
    }

    /// Le signal de vie, une fois par passage, après le premier appel réussi à Riot.
    async fn alive(&mut self) -> Result<(), TickError> {
        if !self.alive {
            self.ingest.heartbeat().await?;
            self.alive = true;
        }
        Ok(())
    }

    /// Envoie les parties lues ; elles sont alors traitées, même rejetées.
    async fn flush(
        &mut self,
        game: Game,
        pending: &mut Vec<(String, Value)>,
        report: &mut TickReport,
    ) -> Result<(), TickError> {
        if pending.is_empty() {
            return Ok(());
        }
        let records: Vec<Value> = pending.iter().map(|(_, r)| r.clone()).collect();
        let results = self.ingest.send(&records).await?;
        for ((id, _), result) in pending.drain(..).zip(&results) {
            report.count(RecordOutcome::of(result));
            self.state.track(game).done.insert(id);
        }
        self.save()
    }

    /// Les identifiants à lire : toute la liste au premier parcours ; ensuite les pages tant qu'une
    /// page pleine contient une partie inconnue. Des plus anciennes aux plus récentes.
    async fn unknown_ids(
        &mut self,
        game: Game,
        puuid: &str,
        report: &mut TickReport,
    ) -> Result<Vec<String>, TickError> {
        let complete = self.state.track(game).complete;
        let mut fresh = Vec::new();
        let mut start = 0;
        loop {
            self.pace().await;
            let key = self.key(game).to_owned();
            let ids = self.riot.ids(&key, game, puuid, start).await?;
            report.calls += 1;
            let done = &self.state.track(game).done;
            let unknown: Vec<String> = ids
                .iter()
                .filter(|id| !done.contains(*id))
                .cloned()
                .collect();
            let any_unknown = !unknown.is_empty();
            fresh.extend(unknown);
            if ids.len() < IDS_PER_PAGE || (complete && !any_unknown) {
                break;
            }
            start += IDS_PER_PAGE;
        }
        fresh.reverse();
        Ok(fresh)
    }

    /// Un jeu : son PUUID, ses parties inconnues, leur détail, l'envoi par lots. Une erreur au milieu
    /// envoie d'abord ce qui a été lu.
    async fn sync(&mut self, game: Game, report: &mut TickReport) -> Result<(), TickError> {
        self.pace().await;
        let key = self.key(game).to_owned();
        let puuid = self.riot.puuid(&key, game, &self.name, &self.tag).await?;
        report.calls += 1;
        self.alive().await?;
        let fresh = self.unknown_ids(game, &puuid, report).await?;
        let mut pending = Vec::new();
        for id in fresh {
            self.pace().await;
            let detail = match self.riot.detail(&key, game, &id).await {
                Ok(detail) => detail,
                Err(err) => {
                    self.flush(game, &mut pending, report).await?;
                    return Err(err.into());
                }
            };
            report.calls += 1;
            match detail.and_then(|raw| record(game, &id, &raw, &puuid)) {
                Some(record) => pending.push((id, record)),
                None => {
                    report.skipped += 1;
                    self.state.track(game).done.insert(id);
                }
            }
            if pending.len() >= MATCHES_PER_BATCH {
                self.flush(game, &mut pending, report).await?;
            }
        }
        self.flush(game, &mut pending, report).await?;
        self.state.track(game).complete = true;
        self.save()
    }

    /// Un passage : LoL puis TFT, chacun avec sa clé. La panne de l'un n'empêche pas l'autre ; la
    /// première erreur est rendue. Riot d'abord, l'ingestion ensuite : une clé refusée n'envoie aucun
    /// signal de vie, et le seuil de silence de la source alerte.
    pub async fn tick(&mut self) -> Result<TickReport, TickError> {
        let mut report = TickReport::default();
        let now = Utc::now();
        self.alive = false;
        self.called = false;
        let mut first = None;
        for game in [Game::Lol, Game::Tft] {
            match self.sync(game, &mut report).await {
                Ok(()) => {}
                // Sans ingestion ni état, l'autre jeu n'irait pas plus loin.
                Err(err @ (TickError::Ingest { .. } | TickError::State)) => return Err(err),
                Err(err) => {
                    first.get_or_insert(err);
                }
            }
        }
        if let Some(err) = first {
            return Err(err);
        }
        self.state.last_pass_at = Some(now);
        // Toujours réécrit : la date du fichier sert au contrôle de santé du conteneur.
        self.save()?;
        Ok(report)
    }

    pub async fn pass(&mut self) -> Duration {
        let (delay, mut status) = match self.tick().await {
            Ok(report) => {
                self.failures = 0;
                info!(
                    calls = report.calls,
                    matches = report.matches,
                    accepted = report.accepted,
                    duplicate = report.duplicate,
                    rejected = report.rejected,
                    skipped = report.skipped,
                    lol = self.state.lol.done.len(),
                    tft = self.state.tft.done.len(),
                    "riot pass completed"
                );
                (self.config.interval, SourceStatus::ok())
            }
            Err(err) => {
                self.failures += 1;
                log_failure(&err, self.failures);
                (retry_delay(self.failures, &err), failure_status(&err))
            }
        };
        status.last_collected_at = self.state.last_pass_at;
        report_status(&self.ingest, status, delay, "riot").await;
        delay
    }

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

    #[test]
    fn a_riot_id_is_cut_at_its_last_hash() {
        assert_eq!(
            split_riot_id(" Nom Fictif#EUW "),
            Some(("Nom Fictif".to_owned(), "EUW".to_owned()))
        );
        assert_eq!(
            split_riot_id("a#b#c"),
            Some(("a#b".to_owned(), "c".to_owned()))
        );
        for bad in ["sans-tag", "#tag", "nom#", ""] {
            assert_eq!(split_riot_id(bad), None, "{bad}");
        }
        let url = url_with("http://h:1/", &["x", "Nom Fictif", "é#"]).unwrap();
        assert_eq!(url.as_str(), "http://h:1/x/Nom%20Fictif/%C3%A9%23");
    }

    #[test]
    fn the_whitelists_keep_my_line_only() {
        let raw = json!({
            "metadata": {"matchId": "EUW1_1", "participants": ["moi", "autre"]},
            "info": {
                "gameStartTimestamp": 1_700_000_000_000_i64, "gameEndTimestamp": 1_700_001_800_000_i64,
                "gameDuration": 1790, "queueId": 420, "gameMode": "CLASSIC",
                "endOfGameResult": "GameComplete",
                "participants": [
                    {"puuid": "autre", "riotIdGameName": "Autre Joueur", "championName": "Zed",
                     "win": false},
                    {"puuid": "moi", "riotIdGameName": "Moi", "championName": "Ahri", "win": true,
                     "teamPosition": "", "gameEndedInEarlySurrender": false, "kills": 9},
                ],
            },
        });
        let record = lol_record("EUW1_1", &raw, "moi").unwrap();
        let payload = record["payload"].as_object().unwrap();
        let keys: Vec<&str> = payload.keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            [
                "champion",
                "duration_s",
                "end_of_game_result",
                "ended_at",
                "game_mode",
                "match_id",
                "position",
                "queue_id",
                "remake",
                "started_at",
                "win"
            ]
        );
        assert_eq!(record["key"], "lol:EUW1_1");
        assert_eq!(record["occurred_at"], "2023-11-14T22:13:20.000Z");
        assert_eq!(payload["position"], Value::Null, "un poste vide");
        assert!(!record.to_string().contains("Autre") && !record.to_string().contains("moi\""));
        assert_eq!(lol_record("EUW1_1", &raw, "inconnu"), None);

        let raw = json!({"info": {
            "gameCreation": 1_700_000_000_000_i64, "game_datetime": 1_700_002_000_000_i64,
            "game_length": 2000.5, "queue_id": 1100, "tft_game_type": "standard",
            "tft_set_number": 15,
            "participants": [{"puuid": "moi", "placement": 2, "time_eliminated": 1900.25,
                              "units": [{"character_id": "x"}]}],
        }});
        let record = tft_record("EUW1_2", &raw, "moi").unwrap();
        assert_eq!(
            record["occurred_at"], "2023-11-14T22:13:20.000Z",
            "le début : gameCreation"
        );
        assert_eq!(record["payload"]["ended_at"], "2023-11-14T22:46:40.000Z");
        assert_eq!(record["payload"]["eliminated_s"], 1900.25);
        assert_eq!(record["payload"].get("units"), None);
    }

    #[test]
    fn a_rejected_key_asks_for_an_action() {
        let status = |error| {
            let s = failure_status(&error);
            assert!(s.is_consistent());
            (s.state, s.reason.unwrap())
        };
        assert_eq!(
            status(TickError::Service(ServiceError::KeyRejected {
                status: 401,
                game: "tft"
            })),
            (Health::ActionRequired, Reason::KeyRejected)
        );
        assert_eq!(
            status(TickError::Service(ServiceError::RateLimited {
                retry_after: Duration::from_secs(1)
            })),
            (Health::Error, Reason::RateLimited)
        );
        assert_eq!(
            status(TickError::Service(ServiceError::Failed {
                status: 500,
                call: "ids"
            })),
            (Health::Error, Reason::ServiceUnavailable)
        );
    }
}
