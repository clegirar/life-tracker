//! Le connecteur VALORANT, par l'API non officielle HenrikDev (voir le chantier Riot et le journal des
//! décisions de l'architecture, 12 nov. 2026) : Riot ne donne pas l'historique de VALORANT à une clé
//! personnelle. Une clé HenrikDev (`Authorization`), le Riot ID et la région. Lecture seule, en tirage,
//! toutes les 15 min.
//!
//! `GET /valorant/v4/matches/{région}/pc/{nom}/{tag}?start&size=10` rend les parties récentes, dix par
//! page, dans la fenêtre que Riot garde (≈ 36 parties) ; au-delà, une page vide. Le premier passage lit
//! toutes les pages jusqu'à la page vide ; ensuite, la première page, et les suivantes tant qu'elles
//! contiennent une partie inconnue. Une partie non terminée (`is_completed` faux) est ignorée. Les
//! « stored matches » (sans durée) sont écartés. Ma ligne se trouve par mon nom et mon tag.
//!
//! La limite est de 30 unités par minute, et une page en coûte jusqu'à 10 : avant chaque page, le
//! connecteur lit ce qui reste (`x-ratelimit-remaining`) et attend la remise à zéro
//! (`x-ratelimit-reset`) si elle est proche ; sinon, ou sur un 429, le passage s'arrête et le suivant
//! reprend. Logs : des nombres seulement, jamais le Riot ID, un agent, une carte ni une URL.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;

use chrono::{DateTime, SecondsFormat, Utc};
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, RETRY_AFTER};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::watch;
use tracing::{error, info, warn};

use super::lastfm::network_reason;
use super::riot::{split_riot_id, url_with};
use super::{backoff, load_state, rate_limit_wait, report_status, save_state};
use crate::ingest_client::{IngestClient, IngestFailure, RecordOutcome, http_client};
use crate::source_status::{Reason, SourceStatus, State as Health};

pub const DEFAULT_HENRIK_URL: &str = "https://api.henrikdev.xyz";
const STATE_FILE: &str = "valorant-state.json";
/// Le plus grand `size` que HenrikDev accepte.
pub const MATCHES_PER_PAGE: usize = 10;
/// Ce qu'une page non cachée peut coûter, en unités de la limite.
const PAGE_COST: i64 = 10;
/// Une remise à zéro plus lointaine n'est pas attendue : le passage suivant reprendra.
const MAX_WAIT: Duration = Duration::from_secs(60);
const DEFAULT_RATE_LIMIT_DELAY: Duration = Duration::from_secs(60);

pub struct Config {
    pub api_url: String,
    pub api_key: String,
    /// `nom#tag`.
    pub riot_id: String,
    /// `eu`, `na`, `ap`, `kr`, `latam`, `br`.
    pub region: String,
    pub ingest_url: String,
    pub token: String,
    pub state_dir: PathBuf,
    pub interval: Duration,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ServiceError {
    /// La clé est refusée (401 ou 403).
    KeyRejected {
        status: u16,
    },
    RateLimited {
        retry_after: Duration,
    },
    Failed {
        status: u16,
    },
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
    /// Pages reçues de HenrikDev.
    pub pages: u32,
    pub matches: u32,
    pub accepted: u32,
    pub duplicate: u32,
    pub rejected: u32,
    /// Parties sans enregistrement : sans ma ligne, ou illisibles.
    pub skipped: u32,
    /// Parties non terminées, ignorées.
    pub incomplete: u32,
}

impl TickReport {
    fn count(&mut self, outcome: RecordOutcome) {
        self.matches += 1;
        match outcome {
            RecordOutcome::Accepted => self.accepted += 1,
            RecordOutcome::Duplicate => self.duplicate += 1,
            RecordOutcome::Rejected => {
                self.rejected += 1;
                warn!("valorant record rejected, set aside");
            }
        }
    }
}

// ─── La liste blanche ────────────────────────────────────────────────────────

fn text<'a>(value: &'a Value, pointer: &str) -> Option<&'a str> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
}

/// L'identifiant d'une partie, si elle est terminée ; `None` pour une partie à ignorer.
pub fn completed_id(raw: &Value) -> Option<&str> {
    if raw
        .pointer("/metadata/is_completed")
        .and_then(Value::as_bool)
        == Some(false)
    {
        return None;
    }
    text(raw, "/metadata/match_id")
}

/// Une partie, réduite à ma ligne (trouvée par mon nom et mon tag, sans tenir compte de la casse) et
/// au score de mon équipe. `None` sans début, sans durée ou sans moi.
pub fn match_record(raw: &Value, name: &str, tag: &str) -> Option<Value> {
    let id = text(raw, "/metadata/match_id")?;
    let started = text(raw, "/metadata/started_at")
        .and_then(|t| DateTime::parse_from_rfc3339(t).ok())?
        .with_timezone(&Utc);
    let duration = raw
        .pointer("/metadata/game_length_in_ms")
        .and_then(Value::as_i64)
        .filter(|ms| *ms > 0)?;
    let me = raw.get("players")?.as_array()?.iter().find(|p| {
        p.get("name")
            .and_then(Value::as_str)
            .is_some_and(|n| n.to_lowercase() == name.to_lowercase())
            && p.get("tag")
                .and_then(Value::as_str)
                .is_some_and(|t| t.to_lowercase() == tag.to_lowercase())
    })?;
    let team = text(me, "/team_id");
    let mine = team.and_then(|team| {
        raw.get("teams")?
            .as_array()?
            .iter()
            .find(|t| t.get("team_id").and_then(Value::as_str) == Some(team))
    });
    let at = started.to_rfc3339_opts(SecondsFormat::Millis, true);
    Some(json!({
        "key": format!("valorant:{id}"),
        "kind": "valorant.match",
        "schema_version": 1,
        "occurred_at": at,
        "payload": {
            "match_id": id,
            "started_at": at,
            "duration_ms": duration,
            "queue_id": text(raw, "/metadata/queue/id"),
            "queue_name": text(raw, "/metadata/queue/name"),
            "map": text(raw, "/metadata/map/name"),
            "agent": text(me, "/agent/name"),
            "team": team,
            "won": mine.and_then(|t| t.get("won")).and_then(Value::as_bool),
            "rounds_won": mine.and_then(|t| t.pointer("/rounds/won")).and_then(Value::as_i64),
            "rounds_lost": mine.and_then(|t| t.pointer("/rounds/lost")).and_then(Value::as_i64),
        },
    }))
}

// ─── Le client de HenrikDev ──────────────────────────────────────────────────

/// Ce que dit la limite après un appel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Budget {
    remaining: Option<i64>,
    reset: Option<Duration>,
}

fn header_number(headers: &HeaderMap, name: &str) -> Option<i64> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<f64>().ok())
        .map(|n| n.ceil() as i64)
}

fn budget(headers: &HeaderMap) -> Budget {
    Budget {
        remaining: header_number(headers, "x-ratelimit-remaining"),
        reset: header_number(headers, "x-ratelimit-reset")
            .filter(|s| *s >= 0)
            .map(|s| Duration::from_secs(s as u64)),
    }
}

pub struct Henrik {
    http: Client,
    url: String,
    api_key: String,
}

impl Henrik {
    pub fn new(http: Client, url: &str, api_key: &str) -> Self {
        Self {
            http,
            url: url.trim_end_matches('/').to_owned(),
            api_key: api_key.to_owned(),
        }
    }

    async fn page(
        &self,
        region: &str,
        name: &str,
        tag: &str,
        start: usize,
    ) -> Result<(Vec<Value>, Budget), ServiceError> {
        let url = url_with(
            &self.url,
            &["valorant", "v4", "matches", region, "pc", name, tag],
        )
        .map_err(|reason| ServiceError::Unavailable { reason })?;
        let response = self
            .http
            .get(url)
            .query(&[
                ("start", start.to_string()),
                ("size", MATCHES_PER_PAGE.to_string()),
            ])
            .header(AUTHORIZATION, &self.api_key)
            .header(ACCEPT, "application/json")
            .send()
            .await
            .map_err(|err| ServiceError::Unavailable {
                reason: network_reason(&err),
            })?;
        let status = response.status();
        let budget = budget(response.headers());
        if status == StatusCode::TOO_MANY_REQUESTS {
            let retry_after = budget
                .reset
                .filter(|d| !d.is_zero())
                .or_else(|| {
                    header_number(response.headers(), RETRY_AFTER.as_str())
                        .filter(|s| *s > 0)
                        .map(|s| Duration::from_secs(s as u64))
                })
                .unwrap_or(DEFAULT_RATE_LIMIT_DELAY);
            return Err(ServiceError::RateLimited { retry_after });
        }
        if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
            return Err(ServiceError::KeyRejected {
                status: status.as_u16(),
            });
        }
        if !status.is_success() {
            return Err(ServiceError::Failed {
                status: status.as_u16(),
            });
        }
        let body: Value = response
            .json()
            .await
            .map_err(|_| ServiceError::Unavailable { reason: "body" })?;
        match body.get("data") {
            Some(Value::Array(matches)) => Ok((matches.clone(), budget)),
            Some(Value::Null) | None => Ok((Vec::new(), budget)),
            Some(_) => Err(ServiceError::Unavailable { reason: "shape" }),
        }
    }
}

// ─── Le connecteur ───────────────────────────────────────────────────────────

#[derive(Debug, Default, Deserialize, Serialize)]
struct State {
    last_pass_at: Option<DateTime<Utc>>,
    /// Les parties traitées : reçues, rejetées, ou sans enregistrement.
    #[serde(default)]
    done: BTreeSet<String>,
    /// Toutes les pages ont été lues une fois, jusqu'à la page vide.
    #[serde(default)]
    complete: bool,
}

pub struct Collector {
    config: Config,
    name: String,
    tag: String,
    henrik: Henrik,
    ingest: IngestClient,
    state: State,
    failures: u32,
}

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
        TickError::Service(ServiceError::KeyRejected { status }) => {
            error!(
                status,
                failures, "clé HenrikDev refusée : révoquée ou invalide"
            );
        }
        TickError::Service(ServiceError::RateLimited { retry_after }) => {
            info!(
                retry_after_s = retry_after.as_secs(),
                "valorant rate limit, waiting"
            );
        }
        TickError::Service(ServiceError::Failed { status }) => {
            warn!(status, failures, "valorant refused");
        }
        TickError::Service(ServiceError::Unavailable { reason }) => {
            warn!(reason, failures, "valorant unavailable");
        }
        TickError::Ingest { reason, status } => {
            error!(reason, status, failures, "valorant ingestion unavailable");
        }
        TickError::State => error!(failures, "valorant state not saved"),
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
            .ok_or_else(|| anyhow::anyhow!("VALORANT_RIOT_ID illisible : `nom#tag` est attendu"))?;
        let http = http_client()?;
        let state = load_state(&config.state_dir, STATE_FILE)?;
        let henrik = Henrik::new(http.clone(), &config.api_url, &config.api_key);
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
            henrik,
            ingest,
            state,
            failures: 0,
        })
    }

    pub fn last_pass_at(&self) -> Option<DateTime<Utc>> {
        self.state.last_pass_at
    }

    pub fn done(&self) -> usize {
        self.state.done.len()
    }

    fn save(&self) -> Result<(), TickError> {
        save_state(&self.config.state_dir, STATE_FILE, &self.state).map_err(|_| TickError::State)
    }

    /// Les parties inconnues d'une page, envoyées ; renvoie s'il y en avait.
    async fn store(&mut self, page: &[Value], report: &mut TickReport) -> Result<bool, TickError> {
        let mut records = Vec::new();
        let mut unknown = false;
        for raw in page {
            let Some(id) = completed_id(raw) else {
                report.incomplete += 1;
                continue;
            };
            if self.state.done.contains(id) {
                continue;
            }
            unknown = true;
            match match_record(raw, &self.name, &self.tag) {
                Some(record) => records.push((id.to_owned(), record)),
                None => {
                    report.skipped += 1;
                    self.state.done.insert(id.to_owned());
                }
            }
        }
        if !records.is_empty() {
            let batch: Vec<Value> = records.iter().map(|(_, r)| r.clone()).collect();
            let results = self.ingest.send(&batch).await?;
            for ((id, _), result) in records.into_iter().zip(&results) {
                report.count(RecordOutcome::of(result));
                self.state.done.insert(id);
            }
        }
        self.save()?;
        Ok(unknown)
    }

    /// Un passage. HenrikDev d'abord, l'ingestion ensuite : une clé refusée n'envoie aucun signal de
    /// vie.
    pub async fn tick(&mut self) -> Result<TickReport, TickError> {
        let mut report = TickReport::default();
        let now = Utc::now();
        let complete = self.state.complete;
        let mut budget = Budget::default();
        let mut start = 0;
        loop {
            if report.pages > 0 && budget.remaining.is_some_and(|r| r < PAGE_COST) {
                let wait = budget.reset.unwrap_or(DEFAULT_RATE_LIMIT_DELAY);
                if wait > MAX_WAIT {
                    return Err(ServiceError::RateLimited { retry_after: wait }.into());
                }
                tokio::time::sleep(wait).await;
            }
            let (page, next) = self
                .henrik
                .page(&self.config.region, &self.name, &self.tag, start)
                .await?;
            budget = next;
            report.pages += 1;
            if report.pages == 1 {
                self.ingest.heartbeat().await?;
            }
            if page.is_empty() {
                self.state.complete = true;
                break;
            }
            let unknown = self.store(&page, &mut report).await?;
            if complete && !unknown {
                break;
            }
            start += page.len();
        }
        self.state.last_pass_at = Some(now);
        self.save()?;
        Ok(report)
    }

    pub async fn pass(&mut self) -> Duration {
        let (delay, mut status) = match self.tick().await {
            Ok(report) => {
                self.failures = 0;
                info!(
                    pages = report.pages,
                    matches = report.matches,
                    accepted = report.accepted,
                    duplicate = report.duplicate,
                    rejected = report.rejected,
                    skipped = report.skipped,
                    incomplete = report.incomplete,
                    done = self.state.done.len(),
                    "valorant pass completed"
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
        report_status(&self.ingest, status, delay, "valorant").await;
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

    fn raw() -> Value {
        json!({
            "metadata": {
                "match_id": "0d3c6a4e-0000-4000-8000-000000000001",
                "map": {"id": "m", "name": "Ascent"}, "game_length_in_ms": 2_100_000,
                "started_at": "2026-10-13T19:42:10.250Z", "is_completed": true,
                "queue": {"id": "competitive", "name": "Competitive", "mode_type": "Standard"},
            },
            "players": [
                {"puuid": "p1", "name": "Autre", "tag": "0000", "team_id": "Blue",
                 "agent": {"name": "Sova"}},
                {"puuid": "p2", "name": "Nom Fictif", "tag": "EUW", "team_id": "Red",
                 "agent": {"id": "a", "name": "Jett"}, "stats": {"score": 4000}},
            ],
            "teams": [
                {"team_id": "Blue", "won": false, "rounds": {"won": 7, "lost": 13}},
                {"team_id": "Red", "won": true, "rounds": {"won": 13, "lost": 7}},
            ],
            "kills": [{"killer": {"name": "Autre"}}],
        })
    }

    #[test]
    fn the_whitelist_keeps_my_line_and_my_team() {
        let record = match_record(&raw(), "nom fictif", "euw").unwrap();
        assert_eq!(
            record["key"],
            "valorant:0d3c6a4e-0000-4000-8000-000000000001"
        );
        assert_eq!(record["occurred_at"], "2026-10-13T19:42:10.250Z");
        let payload = record["payload"].as_object().unwrap();
        let keys: Vec<&str> = payload.keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            [
                "agent",
                "duration_ms",
                "map",
                "match_id",
                "queue_id",
                "queue_name",
                "rounds_lost",
                "rounds_won",
                "started_at",
                "team",
                "won"
            ]
        );
        assert_eq!(payload["agent"], "Jett");
        assert_eq!(payload["won"], true);
        assert_eq!(payload["rounds_won"], 13);
        let text = record.to_string();
        assert!(!text.contains("Autre") && !text.contains("Sova") && !text.contains("Nom Fictif"));
        assert_eq!(match_record(&raw(), "Quelqu'un", "EUW"), None);
        let mut incomplete = raw();
        incomplete["metadata"]["is_completed"] = json!(false);
        assert_eq!(completed_id(&incomplete), None);
        assert!(completed_id(&raw()).is_some());
    }

    #[test]
    fn the_budget_reads_the_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("x-ratelimit-remaining", "8".parse().unwrap());
        headers.insert("x-ratelimit-reset", "42".parse().unwrap());
        assert_eq!(
            budget(&headers),
            Budget {
                remaining: Some(8),
                reset: Some(Duration::from_secs(42))
            }
        );
        assert_eq!(budget(&HeaderMap::new()), Budget::default());
    }
}
