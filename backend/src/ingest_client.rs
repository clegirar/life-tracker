//! Le client de l'API d'ingestion, commun au collecteur et à l'outil d'import : un lot, un résultat par
//! enregistrement. Jamais une erreur de reqwest affichée telle quelle : elle citerait l'URL.

use std::time::Duration;

use reqwest::Client;
use reqwest::redirect::Policy;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::collector::lastfm::network_reason;
use crate::source_status::SourceStatus;

const TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Un rapport d'état ne doit jamais retarder la passe suivante de plus que cela.
const STATUS_TIMEOUT: Duration = Duration::from_secs(5);

/// reqwest sans fournisseur de cryptographie imposé : c'est `ring`, installé une fois par processus.
pub fn install_tls() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // Une erreur dirait qu'un fournisseur est déjà en place : il convient aussi.
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// Une redirection emporterait les secrets ailleurs ; aucun proxy implicite non plus.
pub fn http_client() -> anyhow::Result<Client> {
    install_tls();
    Client::builder()
        .user_agent(concat!("life-tracker/", env!("CARGO_PKG_VERSION")))
        .timeout(TIMEOUT)
        .connect_timeout(CONNECT_TIMEOUT)
        .redirect(Policy::none())
        .no_proxy()
        .build()
        .map_err(|_| anyhow::anyhow!("client HTTP impossible à construire"))
}

/// L'API n'a pas accusé réception : le lot reste à renvoyer entier, l'idempotence fait le reste.
#[derive(Debug, PartialEq, Eq)]
pub struct IngestFailure {
    pub reason: &'static str,
    pub status: Option<u16>,
}

#[derive(Deserialize)]
struct IngestResponse {
    results: Vec<IngestResult>,
}

#[derive(Deserialize)]
pub struct IngestResult {
    pub status: String,
    pub reason: Option<String>,
}

/// Un rejet est définitif, sauf type non déclaré (voir `send`) : il est mis de côté, compté par le
/// serveur, et la suite continue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordOutcome {
    Accepted,
    Duplicate,
    Rejected,
}

impl RecordOutcome {
    pub fn of(result: &IngestResult) -> Self {
        match result.status.as_str() {
            "accepted" => Self::Accepted,
            "duplicate" => Self::Duplicate,
            _ => Self::Rejected,
        }
    }
}

pub struct IngestClient {
    http: Client,
    url: String,
    token: String,
    client_version: &'static str,
}

impl IngestClient {
    pub fn new(http: Client, url: &str, token: &str, client_version: &'static str) -> Self {
        Self {
            http,
            url: url.trim_end_matches('/').to_owned(),
            token: token.to_owned(),
            client_version,
        }
    }

    fn post(&self, path: &str) -> reqwest::RequestBuilder {
        self.http
            .post(format!("{}{path}", self.url))
            .bearer_auth(&self.token)
            .header("X-Client-Version", self.client_version)
    }

    /// Le résultat de chaque enregistrement. Toute autre réponse qu'un 200 complet laisse le lot à
    /// renvoyer entier. Un type non déclaré est une erreur de configuration, pas un défaut de
    /// l'enregistrement : rien n'est mis de côté, et tout repart une fois le droit accordé.
    pub async fn send(&self, records: &[Value]) -> Result<Vec<IngestResult>, IngestFailure> {
        let response = self
            .post("/v1/records")
            .json(&json!({ "records": records }))
            .send()
            .await
            .map_err(|err| IngestFailure {
                reason: network_reason(&err),
                status: None,
            })?;
        let status = response.status();
        if !status.is_success() {
            return Err(IngestFailure {
                reason: "status",
                status: Some(status.as_u16()),
            });
        }
        let body: Option<IngestResponse> = response.json().await.ok();
        let body = body
            .filter(|body| body.results.len() == records.len())
            .ok_or(IngestFailure {
                reason: "body",
                status: None,
            })?;
        if body
            .results
            .iter()
            .any(|r| r.reason.as_deref() == Some("kind_not_declared"))
        {
            return Err(IngestFailure {
                reason: "kind_not_declared",
                status: None,
            });
        }
        Ok(body.results)
    }

    /// Le dernier état de la source, pour la page d'état. Ne compte pas comme un contact : le serveur ne
    /// touche pas au dernier contact (voir `PUT /v1/sources/me/status`).
    pub async fn report_status(&self, status: &SourceStatus) -> Result<(), IngestFailure> {
        let response = self
            .http
            .put(format!("{}/v1/sources/me/status", self.url))
            .bearer_auth(&self.token)
            .header("X-Client-Version", self.client_version)
            .timeout(STATUS_TIMEOUT)
            .json(status)
            .send()
            .await
            .map_err(|err| IngestFailure {
                reason: network_reason(&err),
                status: None,
            })?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(IngestFailure {
                reason: "status",
                status: Some(response.status().as_u16()),
            })
        }
    }

    pub async fn heartbeat(&self) -> Result<(), IngestFailure> {
        let response = self
            .post("/v1/heartbeat")
            .send()
            .await
            .map_err(|err| IngestFailure {
                reason: network_reason(&err),
                status: None,
            })?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(IngestFailure {
                reason: "status",
                status: Some(response.status().as_u16()),
            })
        }
    }
}
