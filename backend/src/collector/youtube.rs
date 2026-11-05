//! Le connecteur YouTube, par l'API Google Data Portability (voir le journal des décisions de
//! l'architecture, 5 nov. 2026) : un export de `myactivity.youtube` au plus toutes les 24 h, lu en flux
//! (`crate::youtube`), puis le classement de chaque vidéo par l'API YouTube Data v3 : seules les vidéos
//! classiques entrent au journal, avec leur fiche (durée, titre, chaîne) comme enrichissement ; ni les
//! Shorts, ni les vidéos supprimées ou privées.
//!
//! L'accord OAuth (client « Application Web » en mode Testing : un jeton de 7 jours) se donne depuis la
//! web app : le connecteur prépare la demande (état aléatoire et vérificateur PKCE, qu'il garde dans
//! son état), en confie l'URL au backend, et vient y chercher le code que Google a renvoyé à la page
//! `/autorisation`, puis l'échange lui-même. Le jeton de rafraîchissement et le secret client ne
//! quittent jamais le collecteur ; le jeton d'accès reste en mémoire.
//!
//! Ses logs ne donnent que des nombres, des statuts et des motifs fixes : jamais un jeton, un code, une
//! URL signée, un titre ni un identifiant.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use base64::Engine;
use chrono::{DateTime, Days, NaiveDate, SecondsFormat, TimeDelta, Utc};
use reqwest::header::RETRY_AFTER;
use reqwest::{Client, Response, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::watch;
use tracing::{error, info, warn};

use super::lastfm::network_reason;
use super::{
    backoff, cancel_record, load_state, rate_limit_wait, report_status, save_private_state,
};
use crate::ingest_client::{Handover, IngestClient, IngestFailure, RecordOutcome, http_client};
use crate::source_status::{Reason, SourceStatus, State as Health};
use crate::youtube::{self, Activity, Reaction, Skipped, Watch};

pub const DEFAULT_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
pub const DEFAULT_OAUTH_URL: &str = "https://oauth2.googleapis.com";
pub const DEFAULT_PORTABILITY_URL: &str = "https://dataportability.googleapis.com";
pub const DEFAULT_DATA_URL: &str = "https://www.googleapis.com/youtube/v3";
pub const DEFAULT_STORAGE_URL: &str = "https://storage.googleapis.com";

pub const SCOPE: &str = "https://www.googleapis.com/auth/dataportability.myactivity.youtube";
const RESOURCE: &str = "myactivity.youtube";
const STATE_FILE: &str = "youtube-state.json";
/// L'archive téléchargée passe par le disque : le collecteur n'a que quelques dizaines de Mo de mémoire.
const ARCHIVE_FILE: &str = "youtube-archive.zip.tmp";
/// Mon historique tient en quelques Mo compressés ; au-delà, ce n'est pas un export.
pub const MAX_ARCHIVE_BYTES: u64 = 200 * 1024 * 1024;
/// Le fichier d'activité décompressé : 12 Mo pour 31 000 entrées.
const MAX_ACTIVITY_BYTES: u64 = 512 * 1024 * 1024;
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(600);
/// Google n'accepte qu'un export toutes les 24 h.
const EXPORT_EVERY: TimeDelta = TimeDelta::hours(24);
/// `startTime` de l'export suivant : la dernière vue moins 48 h. Son effet n'est pas vérifié ; les clés
/// évitent de toute façon les doublons.
const EXPORT_OVERLAP: TimeDelta = TimeDelta::hours(48);
/// Sans export reçu depuis 48 h (deux exports permis manqués), un passage où Google répond n'est plus
/// `ok` : la page d'état le signale, alors que chaque appel réussit.
pub const EXPORT_STALE_AFTER: TimeDelta = TimeDelta::hours(48);
/// Le jeton de rafraîchissement dure 7 jours : un jour avant, on redemande l'accord, pour que la page
/// d'état propose de le renouveler avant le trou.
pub const RENEW_AHEAD: TimeDelta = TimeDelta::hours(24);
/// Le jeton d'accès dure une heure ; on en redemande un cinq minutes avant.
const ACCESS_MARGIN: TimeDelta = TimeDelta::minutes(5);
const DEFAULT_RATE_LIMIT_DELAY: Duration = Duration::from_secs(300);
/// L'API Data v3 prend 50 ids par appel, pour une unité de quota sur 10 000 par jour.
pub const VIDEOS_PER_CALL: usize = 50;
/// Au plus 5 000 vidéos classées par passage (100 unités de quota) : un historique de 22 000 vidéos en
/// 5 passages, une vingtaine de minutes ; ensuite, quelques vidéos par jour.
pub const CLASSIFY_CALLS_PER_PASS: usize = 100;
/// Une vidéo absente de Data v3 (supprimée, privée) n'est redemandée qu'un mois plus tard, et seulement
/// si une nouvelle vue la cite.
const UNAVAILABLE_RETRY_DAYS: u64 = 30;
/// La version des corrections du journal : 1, les fausses vues tirées des réactions ; 2, les Shorts et
/// les vidéos indisponibles entrés avant la règle des vidéos classiques. Une version de plus refait le
/// tour sur un export complet.
const CORRECTIONS_VERSION: u32 = 2;
/// Un lot reste sous les 500 enregistrements et le Mo du contrat.
const RECORDS_PER_BATCH: usize = 500;
const MAX_BATCH_BYTES: usize = 900_000;

pub struct Config {
    pub client_id: String,
    pub client_secret: String,
    /// L'adresse de retour enregistrée chez Google : la page `/autorisation` de la web app.
    pub redirect_uri: String,
    /// La clé de l'API YouTube Data v3.
    pub api_key: String,
    pub ingest_url: String,
    pub token: String,
    /// Le nom de la source dans le registre, que désignent ses annulations.
    pub source: String,
    pub state_dir: PathBuf,
    /// Le rythme des passages ; les exports eux-mêmes n'ont lieu qu'une fois par 24 h.
    pub interval: Duration,
    /// Le rythme des passages tant qu'un accord est attendu : le code reçu doit être échangé vite.
    pub consent_poll: Duration,
    pub auth_url: String,
    pub oauth_url: String,
    pub portability_url: String,
    pub data_url: String,
    /// Le seul préfixe accepté pour télécharger une archive.
    pub storage_url: String,
    /// Les ids par appel à Data v3 (`VIDEOS_PER_CALL`), et les appels par passage
    /// (`CLASSIFY_CALLS_PER_PASS`) ; plus petits en test.
    pub videos_per_call: usize,
    pub classify_calls_per_pass: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ServiceError {
    /// Google refuse le client OAuth (`invalid_client`) ou la clé Data v3 : à corriger sur le NAS.
    KeyRejected {
        status: u16,
        call: &'static str,
    },
    RateLimited {
        retry_after: Duration,
    },
    /// Réponse d'erreur : son statut et l'appel refusé, jamais son corps.
    Failed {
        status: u16,
        call: &'static str,
    },
    /// Rien d'exploitable : coupure, délai dépassé, réponse ou archive illisible.
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
    /// Appels réussis à Google.
    pub calls: u32,
    pub accepted: u32,
    pub duplicate: u32,
    pub rejected: u32,
    /// Vidéos vues lues dans l'export, et entrées écartées par motif.
    pub watches: u32,
    pub skipped: Skipped,
    /// Vues écartées après classement : Shorts, et vidéos supprimées ou privées.
    pub shorts: u32,
    pub unavailable: u32,
    /// Annulations reçues (`accepted` ou `duplicate`) : fausses vues tirées des réactions, vues et fiches
    /// de Shorts, vues de vidéos indisponibles.
    pub cancelled: u32,
    /// Annulations refusées faute du droit `record.cancel` (`kind_not_declared`).
    pub cancels_refused: u32,
    /// L'export avait des entrées `/watch` mais aucune vue reconnue : rien n'en est tiré.
    pub unrecognized: bool,
    pub exported: bool,
    /// Vidéos classées par Data v3, et fiches de vidéos classiques envoyées.
    pub classified: u32,
    pub enriched: u32,
    /// Un accord est attendu : premier (`consent_required`) ou à renouveler (`consent_expired`).
    pub awaiting: Option<Reason>,
}

/// Une demande d'accord en cours : l'état et le vérificateur PKCE ne quittent jamais le connecteur.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct Authorization {
    state: String,
    verifier: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Job {
    id: String,
    started_at: DateTime<Utc>,
    /// Demandé sans `startTime` : tout l'historique, de quoi faire les corrections.
    #[serde(default)]
    full: bool,
}

/// Ce que Data v3 dit d'une vidéo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Classic,
    Short,
    /// Absente d'une réponse réussie : supprimée ou privée, sa nature ne se sait plus.
    Unavailable,
}

/// Ce qui survit à un redémarrage, dans un fichier lisible par le seul collecteur (0600). Le perdre
/// coûte un nouvel accord et un export complet, en `duplicate`.
#[derive(Debug, Default, Deserialize, Serialize)]
struct State {
    refresh_token: Option<String>,
    refresh_expires_at: Option<DateTime<Utc>>,
    /// Un accord a déjà été donné : en redemander un, c'est le renouveler.
    #[serde(default)]
    consented: bool,
    authorization: Option<Authorization>,
    /// La dernière vue traitée : envoyée, ou écartée après classement.
    last_watched_at: Option<DateTime<Utc>>,
    /// Les corrections du journal faites (`CORRECTIONS_VERSION`). En dessous, l'export suivant reprend
    /// tout l'historique pour les faire ; l'état d'une version d'avant (`reactions_cancelled`) vaut 0.
    #[serde(default)]
    corrections_version: u32,
    /// Les vues en attente viennent d'un export complet à corriger : celles qui ne sont pas classiques
    /// sont annulées, sous la clé que la version d'avant leur donnait.
    #[serde(default)]
    correcting: bool,
    /// Un export lu dont des vues attendent encore leur classement : il ne compte comme reçu
    /// (`last_exported_at`) qu'une fois toutes ses vues classiques envoyées.
    #[serde(default)]
    collecting: bool,
    job: Option<Job>,
    /// Le prochain export que Google accepte.
    next_export_at: Option<DateTime<Utc>>,
    /// Le premier export demandé : sans export reçu 48 h plus tard, la source est en erreur.
    exports_since: Option<DateTime<Utc>>,
    /// Le dernier export téléchargé et remis à l'API d'ingestion : `last_collected_at` de l'état.
    last_exported_at: Option<DateTime<Utc>>,
    /// Le classement des vidéos : classiques (fiche envoyée), Shorts, et indisponibles avec le jour où
    /// les redemander. Quelques centaines de Ko pour 22 000 vidéos.
    #[serde(default)]
    classic: BTreeSet<String>,
    #[serde(default)]
    shorts: BTreeSet<String>,
    #[serde(default)]
    unavailable: BTreeMap<String, NaiveDate>,
    /// Les vues dont la vidéo attend son classement, sans doublon : au plus celles d'un export.
    #[serde(default)]
    pending: Vec<Watch>,
}

impl State {
    /// Le classement d'une vidéo ; `None` s'il reste à faire, ou à refaire pour une vidéo indisponible
    /// depuis un mois.
    fn class_of(&self, id: &str, today: NaiveDate) -> Option<Class> {
        if self.classic.contains(id) {
            Some(Class::Classic)
        } else if self.shorts.contains(id) {
            Some(Class::Short)
        } else {
            self.unavailable
                .get(id)
                .filter(|retry| today < **retry)
                .map(|_| Class::Unavailable)
        }
    }

    fn classify(&mut self, id: String, class: Class, today: NaiveDate) {
        self.unavailable.remove(&id);
        match class {
            Class::Classic => {
                self.classic.insert(id);
            }
            Class::Short => {
                self.shorts.insert(id);
            }
            Class::Unavailable => {
                self.unavailable
                    .insert(id, today + Days::new(UNAVAILABLE_RETRY_DAYS));
            }
        }
    }
}

struct Access {
    token: String,
    refresh_after: DateTime<Utc>,
}

/// Le résultat d'un rafraîchissement : l'accord a expiré ou a été retiré (`invalid_grant`), ou une panne.
enum Refresh {
    ConsentLost,
    Failed(ServiceError),
}

impl From<ServiceError> for Refresh {
    fn from(err: ServiceError) -> Self {
        Self::Failed(err)
    }
}

/// Un état ou un vérificateur PKCE : 32 octets aléatoires en base64url, 43 caractères.
fn random_token() -> Result<String, TickError> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| TickError::State)?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

/// Le défi PKCE de la méthode S256.
pub fn code_challenge(verifier: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// L'URL de consentement : accès hors ligne et consentement redemandé, pour recevoir un jeton de
/// rafraîchissement à chaque accord.
pub fn authorization_url(
    auth_url: &str,
    client_id: &str,
    redirect_uri: &str,
    state: &str,
    verifier: &str,
) -> Option<String> {
    let challenge = code_challenge(verifier);
    reqwest::Url::parse_with_params(
        auth_url,
        [
            ("client_id", client_id),
            ("redirect_uri", redirect_uri),
            ("response_type", "code"),
            ("scope", SCOPE),
            ("access_type", "offline"),
            ("prompt", "consent"),
            ("state", state),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
        ],
    )
    .ok()
    .map(String::from)
}

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

/// Le `timestamp_after_24hrs` d'un refus `RESOURCE_EXHAUSTED_TIME_BASED` : l'heure du prochain export
/// permis.
pub fn next_export_after_refusal(body: &Value) -> Option<DateTime<Utc>> {
    body.pointer("/error/details")?
        .as_array()?
        .iter()
        .filter(|d| {
            d.get("reason").and_then(Value::as_str) == Some("RESOURCE_EXHAUSTED_TIME_BASED")
        })
        .find_map(|d| {
            let text = d.pointer("/metadata/timestamp_after_24hrs")?.as_str()?;
            DateTime::parse_from_rfc3339(text).ok()
        })
        .map(|at| at.with_timezone(&Utc))
}

/// Les motifs d'une erreur de l'API Data v3 (`error.errors[].reason`, `error.details[].reason`).
fn data_reasons(body: &Value) -> Vec<String> {
    let mut reasons = Vec::new();
    for pointer in ["/error/errors", "/error/details"] {
        if let Some(list) = body.pointer(pointer).and_then(Value::as_array) {
            reasons.extend(
                list.iter()
                    .filter_map(|e| e.get("reason")?.as_str())
                    .map(str::to_owned),
            );
        }
    }
    reasons
}

/// Un identifiant de tâche d'export, avant de le mettre dans un chemin.
fn valid_job_id(id: &str) -> bool {
    (1..=128).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

struct ArchiveState {
    state: String,
    urls: Vec<String>,
    export_time: Option<DateTime<Utc>>,
}

struct Google {
    http: Client,
    client_id: String,
    client_secret: String,
    redirect_uri: String,
    api_key: String,
    oauth_url: String,
    portability_url: String,
    data_url: String,
    storage_url: String,
    access: Option<Access>,
}

/// La réponse du point de jeton.
#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: Option<i64>,
    refresh_token: Option<String>,
    refresh_token_expires_in: Option<i64>,
}

impl Google {
    /// Un appel au point de jeton. `invalid_grant` : l'accord a expiré ou a été retiré ;
    /// `invalid_client` ou `unauthorized_client` : le client OAuth est refusé.
    async fn token(&self, form: &[(&str, &str)]) -> Result<TokenResponse, Refresh> {
        let response = self
            .http
            .post(format!("{}/token", self.oauth_url))
            .form(form)
            .send()
            .await
            .map_err(|err| unavailable(&err))?;
        let status = response.status();
        if status == StatusCode::TOO_MANY_REQUESTS {
            return Err(ServiceError::RateLimited {
                retry_after: retry_after(&response),
            }
            .into());
        }
        if status.is_client_error() {
            let body: Value = response.json().await.unwrap_or(Value::Null);
            return Err(match body.get("error").and_then(Value::as_str) {
                Some("invalid_grant") => Refresh::ConsentLost,
                Some("invalid_client" | "unauthorized_client") => ServiceError::KeyRejected {
                    status: status.as_u16(),
                    call: "token",
                }
                .into(),
                _ => ServiceError::Failed {
                    status: status.as_u16(),
                    call: "token",
                }
                .into(),
            });
        }
        if !status.is_success() {
            return Err(ServiceError::Failed {
                status: status.as_u16(),
                call: "token",
            }
            .into());
        }
        let token: TokenResponse = response
            .json()
            .await
            .map_err(|_| ServiceError::Unavailable { reason: "token" })?;
        if token.access_token.is_empty() {
            return Err(ServiceError::Unavailable { reason: "token" }.into());
        }
        Ok(token)
    }

    fn keep_access(&mut self, token: &TokenResponse) {
        let lifetime = TimeDelta::seconds(token.expires_in.unwrap_or(3600).clamp(60, 86_400));
        self.access = Some(Access {
            token: token.access_token.clone(),
            refresh_after: Utc::now() + lifetime - ACCESS_MARGIN,
        });
    }

    async fn exchange(&mut self, code: &str, verifier: &str) -> Result<TokenResponse, Refresh> {
        let form = [
            ("grant_type", "authorization_code"),
            ("code", code),
            ("code_verifier", verifier),
            ("client_id", self.client_id.as_str()),
            ("client_secret", self.client_secret.as_str()),
            ("redirect_uri", self.redirect_uri.as_str()),
        ];
        let token = self.token(&form).await?;
        self.keep_access(&token);
        Ok(token)
    }

    /// Un jeton d'accès valide, rafraîchi au besoin. `true` dans le second élément : un appel à
    /// Google a eu lieu.
    async fn access_token(&mut self, refresh_token: &str) -> Result<(String, bool), Refresh> {
        if let Some(access) = &self.access
            && Utc::now() < access.refresh_after
        {
            return Ok((access.token.clone(), false));
        }
        let form = [
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", self.client_id.as_str()),
            ("client_secret", self.client_secret.as_str()),
        ];
        let token = self.token(&form).await?;
        self.keep_access(&token);
        Ok((token.access_token, true))
    }

    /// Un appel à Data Portability. Un 401 : le jeton d'accès est jeté, le passage suivant en redemande un.
    async fn portability(
        &mut self,
        token: &str,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
        call: &'static str,
    ) -> Result<(StatusCode, Value), ServiceError> {
        let mut request = self
            .http
            .request(method, format!("{}{path}", self.portability_url))
            .bearer_auth(token);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.map_err(|err| unavailable(&err))?;
        let status = response.status();
        if status == StatusCode::UNAUTHORIZED {
            self.access = None;
        }
        let body: Value = response.json().await.unwrap_or(Value::Null);
        if status.is_success() || status == StatusCode::TOO_MANY_REQUESTS {
            return Ok((status, body));
        }
        Err(ServiceError::Failed {
            status: status.as_u16(),
            call,
        })
    }

    /// Lance un export. `Err(Some(heure))` : Google n'en accepte pas d'autre avant cette heure.
    async fn initiate(
        &mut self,
        token: &str,
        start: Option<DateTime<Utc>>,
    ) -> Result<Result<String, DateTime<Utc>>, ServiceError> {
        let mut body = json!({ "resources": [RESOURCE] });
        if let Some(start) = start {
            body["startTime"] = json!(start.to_rfc3339_opts(SecondsFormat::Secs, true));
        }
        let (status, body) = self
            .portability(
                token,
                reqwest::Method::POST,
                "/v1/portabilityArchive:initiate",
                Some(body),
                "initiate",
            )
            .await?;
        if status == StatusCode::TOO_MANY_REQUESTS {
            return match next_export_after_refusal(&body) {
                Some(next) => Ok(Err(next)),
                None => Err(ServiceError::RateLimited {
                    retry_after: DEFAULT_RATE_LIMIT_DELAY,
                }),
            };
        }
        body.get("archiveJobId")
            .and_then(Value::as_str)
            .filter(|id| valid_job_id(id))
            .map(|id| Ok(id.to_owned()))
            .ok_or(ServiceError::Unavailable { reason: "shape" })
    }

    async fn archive_state(
        &mut self,
        token: &str,
        job: &str,
    ) -> Result<ArchiveState, ServiceError> {
        let (status, body) = self
            .portability(
                token,
                reqwest::Method::GET,
                &format!("/v1/archiveJobs/{job}/portabilityArchiveState"),
                None,
                "archive_state",
            )
            .await?;
        if status == StatusCode::TOO_MANY_REQUESTS {
            return Err(ServiceError::RateLimited {
                retry_after: DEFAULT_RATE_LIMIT_DELAY,
            });
        }
        let state = body
            .get("state")
            .and_then(Value::as_str)
            .ok_or(ServiceError::Unavailable { reason: "shape" })?
            .to_owned();
        let urls = body
            .get("urls")
            .and_then(Value::as_array)
            .map(|urls| {
                urls.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let export_time = body
            .get("exportTime")
            .and_then(Value::as_str)
            .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
            .map(|t| t.with_timezone(&Utc));
        Ok(ArchiveState {
            state,
            urls,
            export_time,
        })
    }

    /// Retire l'accord en cours : Google refuse un nouvel accord tant que le précédent est actif.
    async fn reset(&mut self, token: &str) -> Result<(), ServiceError> {
        let (status, _) = self
            .portability(
                token,
                reqwest::Method::POST,
                "/v1/authorization:reset",
                Some(json!({})),
                "reset",
            )
            .await?;
        if status == StatusCode::TOO_MANY_REQUESTS {
            return Err(ServiceError::RateLimited {
                retry_after: DEFAULT_RATE_LIMIT_DELAY,
            });
        }
        self.access = None;
        Ok(())
    }

    /// Une archive, sur le disque, bornée. Seules les URL du stockage de Google sont suivies, sans
    /// redirection ni jeton : elles sont signées.
    async fn download(&self, url: &str, path: &std::path::Path) -> Result<(), ServiceError> {
        if !url.starts_with(&format!("{}/", self.storage_url)) {
            return Err(ServiceError::Unavailable {
                reason: "download host",
            });
        }
        let mut response = self
            .http
            .get(url)
            .timeout(DOWNLOAD_TIMEOUT)
            .send()
            .await
            .map_err(|err| unavailable(&err))?;
        let status = response.status();
        if !status.is_success() {
            return Err(ServiceError::Failed {
                status: status.as_u16(),
                call: "download",
            });
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_ARCHIVE_BYTES)
        {
            return Err(ServiceError::Unavailable {
                reason: "too large",
            });
        }
        let mut file = std::fs::File::create(path).map_err(|_| ServiceError::Unavailable {
            reason: "archive file",
        })?;
        let mut written: u64 = 0;
        while let Some(chunk) = response.chunk().await.map_err(|err| unavailable(&err))? {
            written += chunk.len() as u64;
            if written > MAX_ARCHIVE_BYTES {
                return Err(ServiceError::Unavailable {
                    reason: "too large",
                });
            }
            file.write_all(&chunk)
                .map_err(|_| ServiceError::Unavailable {
                    reason: "archive file",
                })?;
        }
        Ok(())
    }

    /// Les fiches de vidéos de l'API Data v3, 50 au plus, avec la taille du lecteur (`maxWidth` la fait
    /// rendre, au rapport de la vidéo ; même quota). Les vidéos absentes d'une réponse réussie n'existent
    /// plus ; une réponse sans `items` ni `pageInfo` n'est pas une réponse réussie.
    async fn videos(&self, ids: &[String]) -> Result<Vec<Value>, ServiceError> {
        let response = self
            .http
            .get(format!("{}/videos", self.data_url))
            .query(&[
                ("part", "contentDetails,snippet,player"),
                ("id", &ids.join(",")),
                ("maxResults", "50"),
                ("maxWidth", "1000"),
                (
                    "fields",
                    "items(id,contentDetails(duration),snippet(title,channelId,channelTitle),\
                     player(embedWidth,embedHeight)),pageInfo(totalResults)",
                ),
                ("key", &self.api_key),
            ])
            .send()
            .await
            .map_err(|err| unavailable(&err))?;
        let status = response.status();
        if status == StatusCode::TOO_MANY_REQUESTS {
            return Err(ServiceError::RateLimited {
                retry_after: retry_after(&response),
            });
        }
        if !status.is_success() {
            let body: Value = response.json().await.unwrap_or(Value::Null);
            let reasons = data_reasons(&body);
            let any = |names: &[&str]| reasons.iter().any(|r| names.contains(&r.as_str()));
            if any(&["quotaExceeded", "rateLimitExceeded", "dailyLimitExceeded"]) {
                return Err(ServiceError::RateLimited {
                    retry_after: Duration::from_secs(3600),
                });
            }
            if status == StatusCode::FORBIDDEN
                || any(&[
                    "keyInvalid",
                    "keyExpired",
                    "API_KEY_INVALID",
                    "API_KEY_EXPIRED",
                ])
            {
                return Err(ServiceError::KeyRejected {
                    status: status.as_u16(),
                    call: "videos",
                });
            }
            return Err(ServiceError::Failed {
                status: status.as_u16(),
                call: "videos",
            });
        }
        let body: Value = response
            .json()
            .await
            .map_err(|_| ServiceError::Unavailable { reason: "body" })?;
        match (body.get("items"), body.get("pageInfo")) {
            (Some(Value::Array(items)), _) => Ok(items.clone()),
            (None, Some(Value::Object(_))) => Ok(Vec::new()),
            _ => Err(ServiceError::Unavailable { reason: "shape" }),
        }
    }
}

/// Les vidéos vues de chaque `MyActivity.json` (ou `watch-history.json`) d'une archive.
pub fn activity_from_zip(path: &std::path::Path) -> Result<Activity, ServiceError> {
    use std::io::Read;
    fn unreadable<E>(_: E) -> ServiceError {
        ServiceError::Unavailable { reason: "archive" }
    }
    let file = std::fs::File::open(path).map_err(|_| ServiceError::Unavailable {
        reason: "archive file",
    })?;
    let mut archive = zip::ZipArchive::new(file).map_err(unreadable)?;
    let mut activity = Activity::default();
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(unreadable)?;
        let wanted = entry.is_file()
            && entry.name().is_ok_and(|name| {
                let name = name.to_ascii_lowercase();
                name.ends_with("myactivity.json") || name.ends_with("watch-history.json")
            });
        if !wanted {
            continue;
        }
        let found = youtube::parse_activity(entry.take(MAX_ACTIVITY_BYTES)).map_err(unreadable)?;
        activity.extend(found);
    }
    Ok(activity)
}

/// Une taille du lecteur : un entier, que les API de Google rendent souvent en texte.
fn embed_size(item: &Value, name: &str) -> Option<u64> {
    let value = item.pointer(&format!("/player/{name}"))?;
    value
        .as_u64()
        .or_else(|| value.as_str()?.trim().parse().ok())
}

/// Le classement d'une vidéo rendue par Data v3, et sa durée.
fn item_class(item: &Value) -> (Class, Option<u64>) {
    let duration = item
        .pointer("/contentDetails/duration")
        .and_then(Value::as_str)
        .and_then(|d| youtube::iso8601_seconds(d.trim()));
    let embed = embed_size(item, "embedWidth").zip(embed_size(item, "embedHeight"));
    let class = if youtube::is_short(duration, embed) {
        Class::Short
    } else {
        Class::Classic
    };
    (class, duration)
}

/// L'état rapporté après un passage en échec. Un client OAuth ou une clé Data v3 refusés demandent une
/// action ; le reste passe seul.
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
        TickError::Service(ServiceError::KeyRejected { status, call }) => {
            error!(
                status,
                call, failures, "client OAuth ou clé YouTube refusés"
            );
        }
        TickError::Service(ServiceError::RateLimited { retry_after }) => {
            info!(
                retry_after_s = retry_after.as_secs(),
                "youtube rate limit, waiting"
            );
        }
        TickError::Service(ServiceError::Failed { status, call }) => {
            warn!(status, call, failures, "youtube refused");
        }
        TickError::Service(ServiceError::Unavailable { reason }) => {
            warn!(reason, failures, "youtube unavailable");
        }
        TickError::Ingest { reason, status } => {
            error!(reason, status, failures, "youtube ingestion unavailable");
        }
        TickError::State => error!(failures, "youtube state not saved"),
    }
}

/// Aucun export reçu depuis 48 h : depuis le dernier, ou, s'il n'y en a jamais eu, depuis le premier
/// demandé. Les appels à Google réussissent, mais les exports échouent ou n'aboutissent pas.
pub fn exports_stale(
    last_exported_at: Option<DateTime<Utc>>,
    exports_since: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> bool {
    last_exported_at
        .or(exports_since)
        .is_some_and(|since| now - since > EXPORT_STALE_AFTER)
}

pub fn retry_delay(failures: u32, error: &TickError) -> Duration {
    match error {
        TickError::Service(ServiceError::RateLimited { retry_after }) => {
            rate_limit_wait(*retry_after)
        }
        _ => backoff(failures),
    }
}

pub struct Collector {
    config: Config,
    google: Google,
    ingest: IngestClient,
    state: State,
    failures: u32,
}

impl Collector {
    pub fn new(config: Config) -> anyhow::Result<Self> {
        let http = http_client()?;
        let state = load_state(&config.state_dir, STATE_FILE)?;
        let trim = |url: &str| url.trim_end_matches('/').to_owned();
        let google = Google {
            http: http.clone(),
            client_id: config.client_id.clone(),
            client_secret: config.client_secret.clone(),
            redirect_uri: config.redirect_uri.clone(),
            api_key: config.api_key.clone(),
            oauth_url: trim(&config.oauth_url),
            portability_url: trim(&config.portability_url),
            data_url: trim(&config.data_url),
            storage_url: trim(&config.storage_url),
            access: None,
        };
        let ingest = IngestClient::new(
            http,
            &config.ingest_url,
            &config.token,
            concat!("collector-", env!("CARGO_PKG_VERSION")),
        );
        Ok(Self {
            config,
            google,
            ingest,
            state,
            failures: 0,
        })
    }

    /// La dernière vue reçue par l'API d'ingestion.
    pub fn last_watched_at(&self) -> Option<DateTime<Utc>> {
        self.state.last_watched_at
    }

    pub fn next_export_at(&self) -> Option<DateTime<Utc>> {
        self.state.next_export_at
    }

    /// Le dernier export reçu.
    pub fn last_exported_at(&self) -> Option<DateTime<Utc>> {
        self.state.last_exported_at
    }

    pub fn refresh_expires_at(&self) -> Option<DateTime<Utc>> {
        self.state.refresh_expires_at
    }

    /// Les vues dont la vidéo attend son classement.
    pub fn awaiting_classification(&self) -> usize {
        self.state.pending.len()
    }

    fn save(&self) -> Result<(), TickError> {
        save_private_state(&self.config.state_dir, STATE_FILE, &self.state)
            .map_err(|_| TickError::State)
    }

    fn consent_reason(&self) -> Reason {
        if self.state.consented {
            Reason::ConsentExpired
        } else {
            Reason::ConsentRequired
        }
    }

    /// Confie la demande en cours au backend, en en créant une s'il n'y en a pas. Renvoyée à chaque
    /// passage : un redémarrage du backend la perd, et sa durée de vie y est d'une heure.
    async fn offer_authorization(&mut self) -> Result<Reason, TickError> {
        if self.state.authorization.is_none() {
            self.state.authorization = Some(Authorization {
                state: random_token()?,
                verifier: random_token()?,
            });
            self.save()?;
        }
        let authorization = self.state.authorization.as_ref().expect("créée ci-dessus");
        let url = authorization_url(
            &self.config.auth_url,
            &self.config.client_id,
            &self.config.redirect_uri,
            &authorization.state,
            &authorization.verifier,
        )
        .ok_or(TickError::Service(ServiceError::Unavailable {
            reason: "auth url",
        }))?;
        self.ingest
            .request_authorization(&url, &authorization.state)
            .await?;
        Ok(self.consent_reason())
    }

    /// Le jeton n'est plus : on oublie tout ce qui en dépendait, et on demande un nouvel accord.
    async fn consent_lost(&mut self) -> Result<Reason, TickError> {
        self.state.refresh_token = None;
        self.google.access = None;
        self.save()?;
        self.offer_authorization().await
    }

    /// Le code d'un accord, échangé contre un jeton. Le code ne sert qu'une fois : en cas d'échec, la
    /// demande est oubliée, et le passage suivant en prépare une autre.
    async fn complete_authorization(
        &mut self,
        code: &str,
        report: &mut TickReport,
    ) -> Result<bool, TickError> {
        let authorization = self
            .state
            .authorization
            .take()
            .expect("une demande est en cours");
        self.save()?;
        match self.google.exchange(code, &authorization.verifier).await {
            Ok(token) => {
                report.calls += 1;
                let Some(refresh) = token.refresh_token.filter(|t| !t.is_empty()) else {
                    return Err(ServiceError::Unavailable {
                        reason: "no refresh token",
                    }
                    .into());
                };
                self.state.refresh_token = Some(refresh);
                self.state.refresh_expires_at = token
                    .refresh_token_expires_in
                    .filter(|s| *s > 0)
                    .map(|s| Utc::now() + TimeDelta::seconds(s));
                self.state.consented = true;
                self.save()?;
                info!("youtube consent received");
                Ok(true)
            }
            Err(Refresh::ConsentLost) => {
                warn!("youtube authorization code refused");
                Ok(false)
            }
            Err(Refresh::Failed(err)) => Err(err.into()),
        }
    }

    /// L'export : la tâche en cours si elle existe, sinon une nouvelle si Google l'accepte.
    async fn export(&mut self, token: &str, report: &mut TickReport) -> Result<(), TickError> {
        let now = Utc::now();
        if let Some(job) = self.state.job.clone() {
            let archive = self.google.archive_state(token, &job.id).await?;
            report.calls += 1;
            match archive.state.as_str() {
                "COMPLETE" => {
                    let mut activity = self.download(&archive.urls, report).await?;
                    let base = archive.export_time.unwrap_or(job.started_at);
                    if activity.unrecognized() {
                        // Une langue inconnue : chaque vue passerait pour une réaction. Rien n'entre,
                        // rien n'est annulé, et ni la dernière vue ni le dernier export n'avancent.
                        report.unrecognized = true;
                        report.skipped.add(&activity.skipped);
                        warn!(
                            reactions = activity.reactions.len(),
                            skipped_ads = activity.skipped.ad,
                            skipped_other = activity.skipped.total() - activity.skipped.reaction,
                            "youtube export without any recognized watch: unknown language?"
                        );
                    } else {
                        // Les vues attendent leur classement ; l'export compte comme reçu quand toutes
                        // ses vues classiques sont envoyées (`resolve`).
                        let reactions = std::mem::take(&mut activity.reactions);
                        report.watches += activity.watches.len() as u32;
                        report.skipped.add(&activity.skipped);
                        self.queue(activity.watches);
                        if job.full && self.state.corrections_version < CORRECTIONS_VERSION {
                            let keys = reactions.iter().map(Reaction::watch_key).collect();
                            self.state.correcting = self.cancel(&keys, report).await?;
                        }
                        self.state.collecting = true;
                        report.exported = true;
                    }
                    self.state.next_export_at = Some(base + EXPORT_EVERY);
                    self.state.job = None;
                    self.save()?;
                }
                "FAILED" | "CANCELLED" => {
                    warn!(
                        state = archive.state.as_str(),
                        "youtube export ended without archive"
                    );
                    // Google dira, par un refus, quand un nouvel export sera permis.
                    self.state.job = None;
                    self.state.next_export_at = Some(now);
                    self.save()?;
                }
                _ => {}
            }
            return Ok(());
        }
        if self.state.next_export_at.is_some_and(|next| now < next) {
            return Ok(());
        }
        // Tant que les corrections ne sont pas faites, tout l'historique : les vues à annuler sont
        // anciennes.
        let start = self
            .state
            .last_watched_at
            .filter(|_| self.state.corrections_version >= CORRECTIONS_VERSION)
            .map(|at| at - EXPORT_OVERLAP);
        let initiated = self.google.initiate(token, start).await?;
        self.state.exports_since.get_or_insert(now);
        match initiated {
            Ok(id) => {
                report.calls += 1;
                info!("youtube export started");
                self.state.job = Some(Job {
                    id,
                    started_at: now,
                    full: start.is_none(),
                });
                self.state.next_export_at = Some(now + EXPORT_EVERY);
            }
            Err(next) => {
                report.calls += 1;
                info!("youtube export refused until the next allowed time");
                self.state.next_export_at = Some(next);
            }
        }
        self.save()
    }

    /// Chaque archive, une à une, par le disque, puis effacée.
    async fn download(
        &mut self,
        urls: &[String],
        report: &mut TickReport,
    ) -> Result<Activity, TickError> {
        let path = self.config.state_dir.join(ARCHIVE_FILE);
        let mut activity = Activity::default();
        for url in urls {
            let result = self.google.download(url, &path).await;
            let found = result.and_then(|()| activity_from_zip(&path));
            let _ = std::fs::remove_file(&path);
            let found = found?;
            report.calls += 1;
            activity.extend(found);
        }
        Ok(activity)
    }

    /// Ajoute les vues d'un export à celles qui attendent leur classement, sans doublon.
    fn queue(&mut self, watches: Vec<Watch>) {
        let mut keys: BTreeSet<String> = self.state.pending.iter().map(Watch::key).collect();
        for watch in watches {
            if keys.insert(watch.key()) {
                self.state.pending.push(watch);
            }
        }
    }

    /// Les vues de vidéos classiques, de la plus ancienne à la plus récente, par lots.
    async fn store(
        &mut self,
        mut watches: Vec<Watch>,
        report: &mut TickReport,
    ) -> Result<(), TickError> {
        watches.sort_by_key(|w| w.watched_at);
        let mut start = 0;
        while start < watches.len() {
            let mut records = Vec::new();
            let mut bytes = 0;
            let mut end = start;
            while end < watches.len() && records.len() < RECORDS_PER_BATCH {
                let record = watches[end].record();
                let size = record.to_string().len() + 1;
                if !records.is_empty() && bytes + size > MAX_BATCH_BYTES {
                    break;
                }
                bytes += size;
                records.push(record);
                end += 1;
            }
            let results = self.ingest.send(&records).await?;
            for result in &results {
                match RecordOutcome::of(result) {
                    RecordOutcome::Accepted => report.accepted += 1,
                    RecordOutcome::Duplicate => report.duplicate += 1,
                    RecordOutcome::Rejected => {
                        report.rejected += 1;
                        warn!("youtube watch rejected, set aside");
                    }
                }
            }
            start = end;
        }
        Ok(())
    }

    /// Annule des enregistrements de la source, sous la clé que la version d'avant leur donnait (une vue
    /// `<id>:<instant en ms>`, une fiche `<id>`). Annuler une clé qui n'a jamais existé ne fait rien ;
    /// renvoyer une annulation ne fait qu'un `duplicate`. Sans le droit `record.cancel`, `false` : les
    /// annulations sont mises de côté, la correction reste à faire, et l'export complet suivant les
    /// renvoie.
    async fn cancel(
        &mut self,
        keys: &BTreeSet<String>,
        report: &mut TickReport,
    ) -> Result<bool, TickError> {
        let now = Utc::now();
        let records: Vec<Value> = keys
            .iter()
            .map(|key| cancel_record(&self.config.source, key, now))
            .collect();
        for (index, batch) in records.chunks(RECORDS_PER_BATCH).enumerate() {
            let results = match self.ingest.send(batch).await {
                Ok(results) => results,
                Err(failure) if failure.reason == "kind_not_declared" => {
                    let refused = records.len() - index * RECORDS_PER_BATCH;
                    report.cancels_refused += refused as u32;
                    report.rejected += refused as u32;
                    warn!(
                        reason = "kind_not_declared",
                        refused, "youtube cancellations refused: grant record.cancel to the source"
                    );
                    return Ok(false);
                }
                Err(failure) => return Err(failure.into()),
            };
            for result in &results {
                match RecordOutcome::of(result) {
                    RecordOutcome::Rejected => {
                        report.rejected += 1;
                        warn!("youtube cancellation rejected, set aside");
                    }
                    _ => report.cancelled += 1,
                }
            }
        }
        Ok(true)
    }

    /// Classe les vidéos des vues en attente par Data v3, quelques appels par passage. Une vidéo
    /// classique a sa fiche envoyée ; une vidéo absente d'une réponse réussie est indisponible. Un appel
    /// en échec (HTTP, quota, réponse illisible) ne classe rien : le passage suivant reprend.
    async fn classify(&mut self, report: &mut TickReport) -> Result<(), TickError> {
        let today = Utc::now().date_naive();
        let mut wanted: BTreeSet<String> = self
            .state
            .pending
            .iter()
            .filter(|w| self.state.class_of(&w.video_id, today).is_none())
            .map(|w| w.video_id.clone())
            .collect();
        for _ in 0..self.config.classify_calls_per_pass {
            let ids: Vec<String> = std::iter::from_fn(|| wanted.pop_first())
                .take(self.config.videos_per_call.max(1))
                .collect();
            if ids.is_empty() {
                break;
            }
            let items = self.google.videos(&ids).await?;
            report.calls += 1;
            let now = Utc::now();
            let mut records = Vec::new();
            let mut found = BTreeMap::new();
            for item in &items {
                let Some(id) = item
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|id| ids.iter().any(|wanted| wanted == id))
                else {
                    continue;
                };
                let (class, duration) = item_class(item);
                if class == Class::Classic {
                    let text = |pointer: &str| {
                        item.pointer(pointer)
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|t| !t.is_empty())
                    };
                    records.push(youtube::video_info_record(
                        id,
                        duration,
                        text("/snippet/title"),
                        text("/snippet/channelId"),
                        text("/snippet/channelTitle"),
                        now,
                    ));
                }
                found.insert(id.to_owned(), class);
            }
            if !records.is_empty() {
                let results = self.ingest.send(&records).await?;
                for result in &results {
                    match RecordOutcome::of(result) {
                        RecordOutcome::Rejected => {
                            report.rejected += 1;
                            warn!("youtube video info rejected, set aside");
                        }
                        _ => report.enriched += 1,
                    }
                }
            }
            for id in ids {
                let class = found.get(&id).copied().unwrap_or(Class::Unavailable);
                report.classified += 1;
                self.state.classify(id, class, today);
            }
        }
        Ok(())
    }

    /// Les vues en attente dont la vidéo est classée : les classiques sont envoyées, les autres
    /// écartées, et annulées pendant une correction (la vue, et la fiche d'un Short). Quand il n'en reste
    /// plus, l'export compte comme reçu, et la correction comme faite.
    async fn resolve(&mut self, report: &mut TickReport) -> Result<(), TickError> {
        let today = Utc::now().date_naive();
        let (ready, waiting): (Vec<Watch>, Vec<Watch>) = self
            .state
            .pending
            .iter()
            .cloned()
            .partition(|w| self.state.class_of(&w.video_id, today).is_some());
        let mut classic = Vec::new();
        let mut cancels = BTreeSet::new();
        let (mut shorts, mut unavailable) = (0, 0);
        let latest = ready.iter().map(|w| w.watched_at).max();
        for watch in ready {
            match self.state.class_of(&watch.video_id, today) {
                Some(Class::Classic) => classic.push(watch),
                Some(Class::Short) => {
                    shorts += 1;
                    cancels.insert(watch.key());
                    cancels.insert(watch.video_id);
                }
                _ => {
                    unavailable += 1;
                    cancels.insert(watch.key());
                }
            }
        }
        self.store(classic, report).await?;
        if self.state.correcting && !self.cancel(&cancels, report).await? {
            self.state.correcting = false;
        }
        report.shorts += shorts;
        report.unavailable += unavailable;
        self.state.pending = waiting;
        self.state.last_watched_at = self.state.last_watched_at.max(latest);
        if self.state.collecting && self.state.pending.is_empty() {
            self.state.collecting = false;
            self.state.last_exported_at = Some(Utc::now());
            if self.state.correcting {
                self.state.correcting = false;
                self.state.corrections_version = CORRECTIONS_VERSION;
                info!("youtube journal corrections done");
            }
        }
        Ok(())
    }

    /// Un passage. Un accord attendu d'abord ; puis le jeton, l'export et les fiches. Le signal de vie
    /// ne part qu'après un appel réussi à Google : un accord expiré n'en envoie donc aucun, et le seuil
    /// de silence de la source alerte.
    pub async fn tick(&mut self) -> Result<TickReport, TickError> {
        let mut report = TickReport::default();
        if self.state.authorization.is_some() {
            match self.ingest.authorization_code().await? {
                Handover::Code(code) => {
                    if !self.complete_authorization(&code, &mut report).await? {
                        report.awaiting = Some(self.offer_authorization().await?);
                        return Ok(report);
                    }
                }
                Handover::Refused => {
                    info!("youtube consent refused, asking again");
                    self.state.authorization = None;
                    self.save()?;
                    report.awaiting = Some(self.offer_authorization().await?);
                    return Ok(report);
                }
                Handover::Nothing => {
                    report.awaiting = Some(self.offer_authorization().await?);
                    return Ok(report);
                }
            }
        }
        let Some(refresh) = self.state.refresh_token.clone() else {
            report.awaiting = Some(self.offer_authorization().await?);
            return Ok(report);
        };
        let token = match self.google.access_token(&refresh).await {
            Ok((token, called)) => {
                if called {
                    report.calls += 1;
                }
                token
            }
            Err(Refresh::ConsentLost) => {
                info!("youtube consent expired");
                report.awaiting = Some(self.consent_lost().await?);
                return Ok(report);
            }
            Err(Refresh::Failed(err)) => return Err(err.into()),
        };
        // Un jour avant l'échéance, et jamais au milieu d'un export : l'accord en cours est retiré
        // (Google refuserait le nouveau), et la page d'état propose de le renouveler.
        let renew = self
            .state
            .refresh_expires_at
            .is_some_and(|at| at - Utc::now() < RENEW_AHEAD);
        if renew && self.state.job.is_none() {
            self.google.reset(&token).await?;
            report.calls += 1;
            info!("youtube consent reset before renewal");
            self.ingest.heartbeat().await?;
            report.awaiting = Some(self.consent_lost().await?);
            return Ok(report);
        }
        self.export(&token, &mut report).await?;
        if report.calls > 0 {
            self.ingest.heartbeat().await?;
        }
        // Ce qui est classé avant une panne de Data v3 part quand même ; la panne est rapportée ensuite.
        let classified = self.classify(&mut report).await;
        self.resolve(&mut report).await?;
        // Toujours réécrit : la date du fichier sert au contrôle de santé du conteneur.
        self.save()?;
        classified?;
        Ok(report)
    }

    /// Un passage, son journal et son état rapporté : l'échéance du jeton, et le prochain export
    /// permis. Tant qu'un accord est attendu, les passages se rapprochent.
    pub async fn pass(&mut self) -> Duration {
        let (delay, mut status) = match self.tick().await {
            Ok(report) => {
                self.failures = 0;
                let s = report.skipped;
                info!(
                    calls = report.calls,
                    exported = report.exported,
                    watches = report.watches,
                    accepted = report.accepted,
                    duplicate = report.duplicate,
                    rejected = report.rejected,
                    skipped_ads = s.ad,
                    skipped_searches = s.search,
                    skipped_posts = s.post,
                    skipped_subscriptions = s.subscription,
                    skipped_music = s.music,
                    skipped_reactions = s.reaction,
                    skipped_other = s.other,
                    skipped_shorts = report.shorts,
                    skipped_unavailable = report.unavailable,
                    cancelled = report.cancelled,
                    classified = report.classified,
                    enriched = report.enriched,
                    awaiting_classification = self.state.pending.len(),
                    awaiting_consent = report.awaiting.is_some(),
                    "youtube pass completed"
                );
                match report.awaiting {
                    Some(reason) => (
                        self.config.consent_poll,
                        SourceStatus::failed(Health::ActionRequired, reason),
                    ),
                    None if report.unrecognized => (
                        self.config.interval,
                        SourceStatus::failed(Health::Error, Reason::ServiceUnavailable),
                    ),
                    None if report.cancels_refused > 0 => (
                        self.config.interval,
                        SourceStatus::failed(Health::Error, Reason::IngestFailed),
                    ),
                    None if exports_stale(
                        self.state.last_exported_at,
                        self.state.exports_since,
                        Utc::now(),
                    ) =>
                    {
                        warn!("youtube: no export received for 48 hours");
                        (
                            self.config.interval,
                            SourceStatus::failed(Health::Error, Reason::ServiceUnavailable),
                        )
                    }
                    None => (self.config.interval, SourceStatus::ok()),
                }
            }
            Err(err) => {
                self.failures += 1;
                log_failure(&err, self.failures);
                (retry_delay(self.failures, &err), failure_status(&err))
            }
        };
        status.expires_at = self.state.refresh_expires_at;
        status.last_collected_at = self.state.last_exported_at;
        // Le prochain export permis ; tant qu'il est passé, la prochaine passe.
        let next_pass = TimeDelta::from_std(delay)
            .ok()
            .and_then(|d| Utc::now().checked_add_signed(d));
        status.next_run_at = match (self.state.next_export_at, next_pass) {
            (Some(export), Some(pass)) => Some(export.max(pass)),
            (export, pass) => export.or(pass),
        };
        report_status(&self.ingest, status, delay, "youtube").await;
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

    #[test]
    fn the_consent_url_asks_for_offline_access_with_pkce() {
        let url = authorization_url(
            DEFAULT_AUTH_URL,
            "client",
            "https://nas.example/autorisation",
            "etat",
            "verificateur",
        )
        .unwrap();
        let parsed = reqwest::Url::parse(&url).unwrap();
        let query: BTreeMap<String, String> = parsed.query_pairs().into_owned().collect();
        assert_eq!(parsed.host_str(), Some("accounts.google.com"));
        assert_eq!(query["scope"], SCOPE);
        assert_eq!(query["access_type"], "offline");
        assert_eq!(query["prompt"], "consent");
        assert_eq!(query["state"], "etat");
        assert_eq!(query["code_challenge_method"], "S256");
        assert_eq!(query["code_challenge"], code_challenge("verificateur"));
        assert!(
            !url.contains("verificateur"),
            "le vérificateur reste dans le connecteur"
        );
        // RFC 7636, annexe B.
        assert_eq!(
            code_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        let state = random_token().unwrap();
        assert_eq!(state.len(), 43);
        assert!(crate::authorization::valid_state(&state));
        assert!(crate::authorization::valid_url(
            &authorization_url(DEFAULT_AUTH_URL, "c", "https://x/autorisation", &state, "v")
                .unwrap(),
            &state
        ));
    }

    #[test]
    fn a_time_based_refusal_gives_the_next_allowed_export() {
        let body = json!({"error": {"code": 429, "status": "RESOURCE_EXHAUSTED", "details": [
            {"@type": "type.googleapis.com/google.rpc.ErrorInfo",
             "reason": "RESOURCE_EXHAUSTED_TIME_BASED",
             "metadata": {"timestamp_after_24hrs": "2026-11-06T12:53:00Z"}}]}});
        assert_eq!(
            next_export_after_refusal(&body),
            Some("2026-11-06T12:53:00Z".parse().unwrap())
        );
        assert_eq!(next_export_after_refusal(&json!({"error": {}})), None);
    }

    #[test]
    fn exports_are_stale_48_hours_after_the_last_or_the_first_asked() {
        let now: DateTime<Utc> = "2026-11-05T12:00:00Z".parse().unwrap();
        let ago = |hours| Some(now - TimeDelta::hours(hours));
        assert!(!exports_stale(None, None, now), "aucun export demandé");
        assert!(!exports_stale(None, ago(47), now));
        assert!(exports_stale(None, ago(49), now), "jamais reçu");
        assert!(!exports_stale(ago(47), ago(500), now));
        assert!(exports_stale(ago(49), ago(500), now));
    }

    #[test]
    fn job_ids_are_checked_before_entering_a_path() {
        assert!(valid_job_id("a1-B_2"));
        assert!(!valid_job_id("../x"));
        assert!(!valid_job_id(""));
    }

    #[test]
    fn a_rejected_client_asks_for_an_action_and_the_rest_is_an_error() {
        let status = |error| {
            let s = failure_status(&error);
            assert!(s.is_consistent());
            (s.state, s.reason.unwrap())
        };
        assert_eq!(
            status(TickError::Service(ServiceError::KeyRejected {
                status: 401,
                call: "token"
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
                call: "initiate"
            })),
            (Health::Error, Reason::ServiceUnavailable)
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
