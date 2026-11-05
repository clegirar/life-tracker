//! Le collecteur : un processus, un connecteur par service (Last.fm ici, Garmin dans `garmin`). Chacun
//! a sa planification, son état, ses nouveaux essais, son signal de vie et son token, et passe par
//! l'API d'ingestion comme n'importe quelle source : il ne connaît pas la base. La panne de l'un
//! n'arrête jamais l'autre. Tourne à part de `serve` : lui seul a une sortie réseau et les secrets.
//!
//! Ses logs ne donnent que des nombres et des motifs fixes. Jamais une erreur de reqwest affichée
//! telle quelle, qui citerait l'URL avec la clé et l'utilisateur.
//!
//! Ce module porte le connecteur Last.fm et le commun : délais, état sur disque.

pub mod garmin;
pub mod lastfm;
pub mod youtube;

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context;
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::watch;
use tracing::{error, info, warn};

pub use crate::ingest_client::install_tls;
use crate::ingest_client::{IngestClient, IngestFailure, RecordOutcome, http_client};
use crate::interpreters::lastfm_name;
use crate::music;
use crate::source_status::{Reason, SourceStatus, State as Health};
use lastfm::{Lastfm, RecentPage, ServiceError};

const STATE_FILE: &str = "state.json";
/// Chaque passage relit ces 48 h : le scrobbler rattrape en retard, avec des dates passées (voir
/// l'architecture). Il en analyse l'historique deux fois par jour. Le dédoublonnage fait le reste.
const OVERLAP_SECONDS: i64 = 48 * 3600;
/// L'état s'écrit toutes les N fiches : un arrêt redemande au pire ces fiches-là, en `duplicate`.
const ENRICHMENTS_BETWEEN_SAVES: usize = 25;
/// Bien en dessous du seuil de silence de 6 h, pour qu'un retour de Last.fm se voie vite.
const MAX_BACKOFF: Duration = Duration::from_secs(1800);
const MAX_RATE_LIMIT_WAIT: Duration = Duration::from_secs(3600);
/// Last.fm : clé d'API invalide (10) ou suspendue (26).
const KEY_REJECTED_CODES: [i64; 2] = [10, 26];

pub const DEFAULT_LASTFM_URL: &str = "https://ws.audioscrobbler.com/2.0/";

pub struct Config {
    pub lastfm_url: String,
    pub api_key: String,
    pub user: String,
    pub ingest_url: String,
    pub token: String,
    pub state_dir: PathBuf,
    pub interval: Duration,
    /// Last.fm demande de rester sous quelques requêtes par seconde.
    pub pause: Duration,
}

#[derive(Debug, PartialEq, Eq)]
pub enum TickError {
    Service(ServiceError),
    /// L'API d'ingestion n'a pas accusé réception : le curseur n'avance pas.
    Ingest {
        reason: &'static str,
        status: Option<u16>,
    },
    State,
}

impl From<IngestFailure> for TickError {
    fn from(failure: IngestFailure) -> Self {
        Self::Ingest {
            reason: failure.reason,
            status: failure.status,
        }
    }
}

impl From<ServiceError> for TickError {
    fn from(err: ServiceError) -> Self {
        Self::Service(err)
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct TickReport {
    pub pages: u32,
    pub accepted: u32,
    pub duplicate: u32,
    pub rejected: u32,
    pub enriched: u32,
    /// Une écoute tardive a décalé les pages : le passage suivant reprendra au dernier point sûr.
    pub interrupted: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct TrackName {
    artist: String,
    track: String,
}

/// Ce qui survit à un redémarrage. Le perdre ne coûte qu'un rattrapage : tout revient en `duplicate`.
#[derive(Debug, Default, Deserialize, Serialize)]
struct State {
    /// Tout ce qui est plus ancien a été reçu par l'API d'ingestion.
    cursor: Option<i64>,
    /// Les titres dont la fiche a été demandée, par leur clé d'enregistrement.
    enriched: BTreeSet<String>,
    /// Ceux dont elle reste à demander : un écart de plus de 48 h ne les ferait pas revenir.
    pending: BTreeMap<String, TrackName>,
}

fn sha256_hex(text: &str) -> String {
    hex::encode(Sha256::digest(text.as_bytes()))
}

/// Horodatage + artiste + titre, tels que renvoyés : la clé ne doit pas changer si la normalisation
/// change. Le condensat borne sa longueur (512 au plus) et la garde muette dans le journal.
pub fn scrobble_key(uts: i64, artist: &str, track: &str) -> String {
    let identity = sha256_hex(&format!("{artist}\u{0}{track}"));
    format!("{uts}:{identity}")
}

/// Artiste + titre normalisés : une seule fiche par titre, toutes versions confondues.
pub fn track_info_key(artist: &str, track: &str) -> String {
    sha256_hex(&music::track_key(artist, track))
}

fn rfc3339(at: DateTime<Utc>) -> String {
    // `Z` : Last.fm ne donne pas de décalage, il reste inconnu.
    at.to_rfc3339_opts(SecondsFormat::Secs, true)
}

struct Scrobble {
    uts: i64,
    key: String,
    name: TrackName,
    record: Value,
}

/// L'enveloppe d'une écoute : la charge utile reste le scrobble tel que renvoyé. `None` pour le
/// morceau en cours, sans date, ou pour un élément sans date, artiste ou titre lisibles.
fn scrobble(raw: &Value) -> Option<Scrobble> {
    let uts = raw
        .pointer("/date/uts")?
        .as_str()
        .and_then(|uts| uts.trim().parse().ok())
        .or_else(|| raw.pointer("/date/uts")?.as_i64())?;
    let at = DateTime::from_timestamp(uts, 0)?;
    let name = TrackName {
        artist: lastfm_name(raw.get("artist"))?.to_owned(),
        track: lastfm_name(raw.get("name"))?.to_owned(),
    };
    let key = scrobble_key(uts, &name.artist, &name.track);
    let record = json!({
        "key": key,
        "kind": "lastfm.scrobble",
        "schema_version": 1,
        "occurred_at": rfc3339(at),
        "payload": raw,
    });
    Some(Scrobble {
        uts,
        key,
        name,
        record,
    })
}

/// Enveloppes d'une page, sans le morceau en cours. Publique pour tester le contrat.
pub fn scrobble_records(tracks: &[Value]) -> Vec<Value> {
    tracks
        .iter()
        .filter_map(scrobble)
        .map(|s| s.record)
        .collect()
}

pub fn track_info_record(artist: &str, track: &str, info: &Value, now: DateTime<Utc>) -> Value {
    json!({
        "key": track_info_key(artist, track),
        "kind": "lastfm.track_info",
        "schema_version": 1,
        "occurred_at": rfc3339(now),
        "payload": { "artist": artist, "track": track, "info": info },
    })
}

/// Délai avant de réessayer. Pour une limitation, celui demandé, borné pour rester sous le seuil de
/// silence ; sinon 1, 2, 4… minutes jusqu'à 30.
pub fn retry_delay(failures: u32, error: &TickError) -> Duration {
    if let TickError::Service(ServiceError::RateLimited { retry_after }) = error {
        return (*retry_after).min(MAX_RATE_LIMIT_WAIT);
    }
    backoff(failures)
}

/// 1, 2, 4… minutes jusqu'à 30, commun à tous les connecteurs.
pub fn backoff(failures: u32) -> Duration {
    Duration::from_secs(60u64 << failures.saturating_sub(1).min(5)).min(MAX_BACKOFF)
}

/// Une limitation de débit attend ce qu'on lui demande, bornée pour rester sous le seuil de silence.
pub fn rate_limit_wait(retry_after: Duration) -> Duration {
    retry_after.min(MAX_RATE_LIMIT_WAIT)
}

/// L'état rapporté pour la page d'état après un passage en échec (`ok` sinon). Une clé invalide ou
/// suspendue (codes 10 et 26, ou un 403) demande une action ; le reste passe seul. Un état qui ne
/// s'écrit plus sur disque n'a pas de code à lui : `service_unavailable`, le connecteur est en panne.
pub fn failure_status(error: &TickError) -> SourceStatus {
    let (state, reason) = match error {
        TickError::Service(ServiceError::Failed { status, code })
            if *status == 403 || code.is_some_and(|c| KEY_REJECTED_CODES.contains(&c)) =>
        {
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

/// Rapporte l'état d'un connecteur, au mieux : un échec se note et n'arrête rien. `next_run_at` est
/// la prochaine passe prévue, sauf si le connecteur en donne une autre (le prochain export de YouTube).
pub(crate) async fn report_status(
    ingest: &IngestClient,
    mut status: SourceStatus,
    delay: Duration,
    connector: &'static str,
) {
    if status.next_run_at.is_none() {
        status.next_run_at = chrono::TimeDelta::from_std(delay)
            .ok()
            .and_then(|delay| Utc::now().checked_add_signed(delay));
    }
    if let Err(failure) = ingest.report_status(&status).await {
        warn!(
            connector,
            reason = failure.reason,
            status = failure.status,
            "status not reported"
        );
    }
}

/// Une limitation de débit n'est jamais une erreur à signaler. Rien d'autre que des codes.
fn log_failure(error: &TickError, failures: u32) {
    match error {
        TickError::Service(ServiceError::RateLimited { retry_after }) => {
            info!(
                retry_after_s = retry_after.as_secs(),
                "lastfm rate limit, waiting"
            );
        }
        TickError::Service(ServiceError::Failed { status, code }) => {
            warn!(status, code, failures, "lastfm refused");
        }
        TickError::Service(ServiceError::Unavailable { reason }) => {
            warn!(reason, failures, "lastfm unavailable");
        }
        TickError::Ingest { reason, status } => {
            error!(reason, status, failures, "ingestion unavailable");
        }
        TickError::State => error!(failures, "state not saved"),
    }
}

pub struct Collector {
    config: Config,
    lastfm: Lastfm,
    ingest: IngestClient,
    state: State,
    /// Déjà accusés depuis le démarrage : les relire à chaque passage serait sans effet, sauf du
    /// travail pour la base. Limité aux clés de la dernière fenêtre.
    acknowledged: HashSet<String>,
    /// Passages en échec d'affilée, pour espacer les essais.
    failures: u32,
}

impl Collector {
    pub fn new(config: Config) -> anyhow::Result<Self> {
        let http = http_client()?;
        let state = load_state(&config.state_dir, STATE_FILE)?;
        let lastfm = Lastfm {
            http: http.clone(),
            url: config.lastfm_url.clone(),
            api_key: config.api_key.clone(),
            user: config.user.clone(),
        };
        let ingest = IngestClient::new(
            http,
            &config.ingest_url,
            &config.token,
            concat!("collector-", env!("CARGO_PKG_VERSION")),
        );
        Ok(Self {
            config,
            lastfm,
            ingest,
            state,
            acknowledged: HashSet::new(),
            failures: 0,
        })
    }

    pub fn cursor(&self) -> Option<i64> {
        self.state.cursor
    }

    pub fn pending_enrichments(&self) -> usize {
        self.state.pending.len()
    }

    async fn send(&self, records: &[Value]) -> Result<Vec<RecordOutcome>, TickError> {
        let results = self.ingest.send(records).await?;
        Ok(results.iter().map(RecordOutcome::of).collect())
    }

    async fn heartbeat(&self) -> Result<(), TickError> {
        Ok(self.ingest.heartbeat().await?)
    }

    /// Envoie une page puis note le curseur : une coupure après l'accusé n'envoie rien en double,
    /// une coupure avant renvoie la même page.
    async fn store_page(
        &mut self,
        page: &RecentPage,
        report: &mut TickReport,
        seen: &mut HashSet<String>,
    ) -> Result<Option<i64>, TickError> {
        let mut scrobbles: Vec<Scrobble> = page.tracks.iter().filter_map(scrobble).collect();
        // Last.fm va du plus récent au plus ancien : le journal, lui, suit l'ordre des écoutes.
        scrobbles.sort_by_key(|s| s.uts);
        let skipped = page
            .tracks
            .iter()
            .filter(|t| t.pointer("/date/uts").is_some() && scrobble(t).is_none())
            .count();
        if skipped > 0 {
            warn!(skipped, "unreadable tracks set aside");
        }
        let newest = scrobbles.iter().map(|s| s.uts).max();
        seen.extend(scrobbles.iter().map(|s| s.key.clone()));
        let fresh: Vec<&Scrobble> = scrobbles
            .iter()
            .filter(|s| !self.acknowledged.contains(&s.key))
            .collect();
        if !fresh.is_empty() {
            let records: Vec<Value> = fresh.iter().map(|s| s.record.clone()).collect();
            let outcomes = self.send(&records).await?;
            for (scrobble, outcome) in fresh.iter().zip(outcomes) {
                report.count(outcome);
                self.acknowledged.insert(scrobble.key.clone());
                if outcome != RecordOutcome::Rejected {
                    self.queue_enrichment(&scrobble.name);
                }
            }
        }
        Ok(newest)
    }

    fn queue_enrichment(&mut self, name: &TrackName) {
        let key = track_info_key(&name.artist, &name.track);
        if !self.state.enriched.contains(&key) {
            self.state
                .pending
                .entry(key)
                .or_insert_with(|| name.clone());
        }
    }

    /// Du plus ancien au plus récent : le curseur avance à chaque page sans rien sauter. La fenêtre
    /// est fermée par `to`, donc seule une écoute tardive peut décaler les pages, et alors `total`
    /// change : on s'arrête au dernier point sûr.
    async fn sync_listens(&mut self, report: &mut TickReport) -> Result<(), TickError> {
        let to = Utc::now().timestamp();
        let from = self.state.cursor.map(|c| (c - OVERLAP_SECONDS).max(0));
        let first = self.lastfm.recent_tracks(from, to, 1).await?;
        let (total, total_pages) = (first.total, first.total_pages);
        let mut seen = HashSet::new();
        let mut first = Some(first);
        for number in (1..=total_pages.max(1)).rev() {
            let page = if total_pages <= 1 {
                first.take().expect("une seule page, lue une seule fois")
            } else {
                tokio::time::sleep(self.config.pause).await;
                let page = self.lastfm.recent_tracks(from, to, number).await?;
                if page.total != total {
                    report.interrupted = true;
                    return Ok(());
                }
                page
            };
            report.pages += 1;
            let newest = self.store_page(&page, report, &mut seen).await?;
            if let Some(newest) = newest {
                self.state.cursor = self.state.cursor.max(Some(newest));
            }
            self.save()?;
        }
        self.state.cursor = self.state.cursor.max(Some(to));
        self.acknowledged = seen;
        Ok(())
    }

    /// Une fiche par titre. Une fiche inconnue de Last.fm ou refusée pour elle-même n'est plus redemandée.
    async fn enrich(&mut self, report: &mut TickReport) -> Result<(), TickError> {
        let mut unsaved = 0;
        while let Some((key, name)) = self
            .state
            .pending
            .first_key_value()
            .map(|(k, n)| (k.clone(), n.clone()))
        {
            tokio::time::sleep(self.config.pause).await;
            if let Err(err) = self.enrich_one(&name, report).await {
                self.save()?;
                return Err(err);
            }
            self.state.pending.remove(&key);
            self.state.enriched.insert(key);
            unsaved += 1;
            if unsaved == ENRICHMENTS_BETWEEN_SAVES {
                self.save()?;
                unsaved = 0;
            }
        }
        Ok(())
    }

    async fn enrich_one(
        &mut self,
        name: &TrackName,
        report: &mut TickReport,
    ) -> Result<(), TickError> {
        let Some(info) = self.lastfm.track_info(&name.artist, &name.track).await? else {
            return Ok(());
        };
        let record = track_info_record(&name.artist, &name.track, &info, Utc::now());
        let outcomes = self.send(std::slice::from_ref(&record)).await?;
        if outcomes.first() == Some(&RecordOutcome::Rejected) {
            report.rejected += 1;
            warn!("track info rejected, set aside");
        } else {
            report.enriched += 1;
        }
        Ok(())
    }

    /// Un passage complet. Le signal de vie ne part qu'après une interrogation réussie de Last.fm,
    /// et l'ingestion n'est jamais appelée avant : un appel authentifié compte comme un contact.
    pub async fn tick(&mut self) -> Result<TickReport, TickError> {
        let mut report = TickReport::default();
        self.sync_listens(&mut report).await?;
        self.heartbeat().await?;
        self.enrich(&mut report).await?;
        // Toujours réécrit : la date du fichier sert au contrôle de santé du conteneur.
        self.save()?;
        Ok(report)
    }

    fn save(&self) -> Result<(), TickError> {
        save_state(&self.config.state_dir, STATE_FILE, &self.state).map_err(|_| TickError::State)
    }

    /// Un passage, son journal et son état rapporté ; renvoie le délai avant le suivant. Le rapport
    /// d'état vient après le passage et n'en change ni le résultat ni le signal de vie.
    pub async fn pass(&mut self) -> Duration {
        let (delay, status) = match self.tick().await {
            Ok(report) => {
                self.failures = 0;
                info!(
                    pages = report.pages,
                    accepted = report.accepted,
                    duplicate = report.duplicate,
                    rejected = report.rejected,
                    enriched = report.enriched,
                    pending = self.state.pending.len(),
                    interrupted = report.interrupted,
                    "collect pass completed"
                );
                (self.config.interval, SourceStatus::ok())
            }
            Err(err) => {
                self.failures += 1;
                log_failure(&err, self.failures);
                (retry_delay(self.failures, &err), failure_status(&err))
            }
        };
        report_status(&self.ingest, status, delay, "lastfm").await;
        delay
    }

    /// Jusqu'à l'arrêt demandé. Aucune erreur n'arrête le collecteur : il réessaie, son curseur reste
    /// en place, et le seuil de silence de la source signale une panne qui dure.
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

impl TickReport {
    fn count(&mut self, outcome: RecordOutcome) {
        match outcome {
            RecordOutcome::Accepted => self.accepted += 1,
            RecordOutcome::Duplicate => self.duplicate += 1,
            RecordOutcome::Rejected => {
                self.rejected += 1;
                warn!("listen rejected, set aside");
            }
        }
    }
}

/// L'état d'un connecteur, ou l'état vide s'il n'existe pas encore ou ne se lit plus.
pub(crate) fn load_state<T: Default + serde::de::DeserializeOwned>(
    dir: &Path,
    file: &str,
) -> anyhow::Result<T> {
    let path = dir.join(file);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(T::default()),
        Err(err) => return Err(err).context("lecture de l'état du collecteur"),
    };
    Ok(serde_json::from_str(&text).unwrap_or_else(|_| {
        // Sans gravité : tout l'historique revient, en `duplicate`.
        warn!(file, "state unreadable, starting over");
        T::default()
    }))
}

/// Par renommage : un arrêt au milieu laisse l'ancien état, entier.
pub(crate) fn save_state<T: Serialize>(dir: &Path, name: &str, state: &T) -> std::io::Result<()> {
    write_state(dir, name, state, None)
}

/// Comme `save_state`, lisible par le seul propriétaire (0600) : un état qui porte un jeton.
pub(crate) fn save_private_state<T: Serialize>(
    dir: &Path,
    name: &str,
    state: &T,
) -> std::io::Result<()> {
    write_state(dir, name, state, Some(0o600))
}

fn write_state<T: Serialize>(
    dir: &Path,
    name: &str,
    state: &T,
    mode: Option<u32>,
) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let temporary = dir.join(format!("{name}.tmp"));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    if let Some(mode) = mode {
        options.mode(mode);
    }
    let mut file = options.open(&temporary)?;
    if let Some(mode) = mode {
        // Un fichier temporaire laissé par un arrêt garde ses droits : on les reprend.
        file.set_permissions(std::fs::Permissions::from_mode(mode))?;
    }
    file.write_all(&serde_json::to_vec(state)?)?;
    file.sync_all()?;
    std::fs::rename(&temporary, dir.join(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_track_playing_now_has_no_record() {
        let playing = json!({"artist": {"#text": "A"}, "name": "T",
                             "@attr": {"nowplaying": "true"}});
        let dated = json!({"artist": {"#text": "A"}, "name": "T", "date": {"uts": "1000"}});
        let records = scrobble_records(&[playing, dated.clone()]);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["occurred_at"], "1970-01-01T00:16:40Z");
        assert_eq!(records[0]["payload"], dated, "le brut, tel quel");
    }

    #[test]
    fn keys_are_bounded_and_say_nothing() {
        let long = "é".repeat(5000);
        let key = scrobble_key(1_000_000_000, &long, &long);
        assert!(key.len() < 100);
        assert!(!key.contains('é'));
        assert_ne!(scrobble_key(1, "A", "BC"), scrobble_key(1, "AB", "C"));
        assert_ne!(scrobble_key(1, "A", "B"), scrobble_key(2, "A", "B"));
        assert_ne!(scrobble_key(1, "A", "B"), scrobble_key(1, "A", "B (Live)"));
        assert_eq!(
            track_info_key("A", "B"),
            track_info_key("a feat. X", "B (Live)")
        );
        // Un titre à plusieurs artistes n'est demandé qu'une fois, sous la clé de l'artiste principal.
        assert_eq!(
            track_info_key("A & X", "B"),
            track_info_key("A", "B (feat. X)")
        );
    }

    #[test]
    fn retry_delays() {
        let outage = TickError::Ingest {
            reason: "connect",
            status: None,
        };
        let minutes: Vec<u64> = (1..=8)
            .map(|n| retry_delay(n, &outage).as_secs() / 60)
            .collect();
        assert_eq!(minutes, [1, 2, 4, 8, 16, 30, 30, 30]);
        let limited = |s| {
            TickError::Service(ServiceError::RateLimited {
                retry_after: Duration::from_secs(s),
            })
        };
        assert_eq!(retry_delay(1, &limited(90)), Duration::from_secs(90));
        assert_eq!(retry_delay(9, &limited(90)), Duration::from_secs(90));
        assert_eq!(retry_delay(1, &limited(99_999)), MAX_RATE_LIMIT_WAIT);
    }

    #[test]
    fn an_invalid_key_asks_for_an_action_and_the_rest_is_an_error() {
        let status = |error| {
            let s = failure_status(&error);
            assert!(s.is_consistent());
            (s.state, s.reason.unwrap())
        };
        let failed = |status, code| TickError::Service(ServiceError::Failed { status, code });
        for key in [
            failed(403, Some(10)),
            failed(200, Some(26)),
            failed(403, None),
        ] {
            assert_eq!(status(key), (Health::ActionRequired, Reason::KeyRejected));
        }
        for other in [
            failed(200, Some(11)),
            failed(502, None),
            TickError::Service(ServiceError::Unavailable { reason: "timeout" }),
            TickError::State,
        ] {
            assert_eq!(status(other), (Health::Error, Reason::ServiceUnavailable));
        }
        assert_eq!(
            status(TickError::Service(ServiceError::RateLimited {
                retry_after: Duration::from_secs(1)
            })),
            (Health::Error, Reason::RateLimited)
        );
        assert_eq!(
            status(TickError::Ingest {
                reason: "connect",
                status: None
            }),
            (Health::Error, Reason::IngestFailed)
        );
    }
}
