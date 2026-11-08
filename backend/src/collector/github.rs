//! Le connecteur GitHub, par l'API GraphQL de GitHub (voir le chantier GitHub, 7 nov. 2026) : un token
//! d'accès personnel classique (`repo` et `read:org`), en lecture seule, en tirage, toutes les 15 min.
//! `contributionsCollection` est écarté : il cache les contributions privées.
//!
//! - **Commits :** la liste des dépôts lisibles est l'union de `viewer.repositories` (propriétaire,
//!   collaborateur, membre d'organisation), des dépôts de chaque organisation de `viewer.organizations`
//!   et de `viewer.repositoriesContributedTo`, par id. Pour chaque dépôt qui n'est pas une fourche,
//!   l'historique de la branche par défaut filtré sur mon compte (`history(author: {id})`), par pages
//!   de 100 : la définition du graphe de contributions de GitHub, qui ne compte pas non plus les
//!   fourches. Le premier passage lit tout l'historique, en notant la page suivante après chaque page :
//!   une coupure reprend là. Ensuite, seuls les dépôts dont `pushedAt` a avancé sont relus, sur 30 jours
//!   (`since`, sur la date de commit, que le rebase et le squash changent : un commit poussé tard est
//!   vu). Un commit se date à sa date d'auteur.
//! - **Pull requests, reviews, issues :** par la recherche (`author:`, `reviewed-by: -author:`), par
//!   tranches d'une année de `created:` au premier passage (la recherche plafonne à 1 000 résultats),
//!   puis par `updated:>=` (le passage précédent moins un jour). Une pull request donne un fait à son
//!   ouverture et un autre à sa fusion ; une review, un fait à sa soumission.
//! - **Commentaires :** `viewer.issueComments`, du plus récent au plus ancien, entièrement la première
//!   fois puis jusqu'au passage précédent moins un jour ; `viewer.commitComments`, relus en entier (il y
//!   en a peu). Jamais le corps : la date, le dépôt, le numéro et le titre de la cible, l'adresse.
//!
//! Chaque fait a une clé stable (sha, id de pull request et action, id de review, d'issue, de
//! commentaire) : relu, il donne `duplicate`. Sous 100 points de la limite horaire, le passage s'arrête
//! et reprend après la remise à zéro. Une organisation à authentification unique (SAML) qui refuse le
//! token est sautée, et l'état rapporté demande de l'autoriser (`sso_required`).
//!
//! Le token est dans l'en-tête de chaque appel : jamais d'URL ni d'erreur de reqwest dans les logs, ni
//! titre, ni nom de dépôt, ni identifiant de compte. Seulement des nombres et des motifs fixes. L'état
//! sur disque ne garde que des ids de dépôts et des curseurs.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;

use chrono::{DateTime, Datelike, SecondsFormat, TimeDelta, Utc};
use reqwest::header::{ACCEPT, AUTHORIZATION, RETRY_AFTER};
use reqwest::{Client, Response, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::watch;
use tracing::{error, info, warn};

use super::lastfm::network_reason;
use super::{backoff, load_state, rate_limit_wait, report_status, save_state};
use crate::ingest_client::{IngestClient, IngestFailure, RecordOutcome, http_client};
use crate::source_status::{Reason, SourceStatus, State as Health};

pub const DEFAULT_GITHUB_URL: &str = "https://api.github.com/graphql";
const STATE_FILE: &str = "github-state.json";
/// La limite d'un lot de l'API d'ingestion.
const RECORDS_PER_BATCH: usize = 500;
/// GitHub n'annonce pas toujours de délai pour une limite secondaire : au moins une minute, dit-il.
const DEFAULT_RATE_LIMIT_DELAY: Duration = Duration::from_secs(60);
/// Sous ce reste de points de la limite horaire, le passage s'arrête jusqu'à la remise à zéro.
const RATE_LIMIT_FLOOR: i64 = 100;
/// Un dépôt poussé depuis le passage précédent est relu sur cette fenêtre de dates de commit.
const COMMIT_WINDOW: TimeDelta = TimeDelta::days(30);
/// Les relectures par date de mise à jour reprennent un jour avant le passage précédent.
const OVERLAP: TimeDelta = TimeDelta::days(1);
/// La recherche ne rend que les 1 000 premiers résultats d'une requête.
const SEARCH_CAP: i64 = 1000;
/// Un titre plus long est coupé avant de partir : la carte n'en montre pas plus.
const MAX_TITLE_CHARS: usize = 300;

pub struct Config {
    pub api_url: String,
    /// Le token d'accès personnel de GitHub.
    pub api_token: String,
    pub ingest_url: String,
    /// Le token de la source `github` de l'API d'ingestion.
    pub token: String,
    pub state_dir: PathBuf,
    pub interval: Duration,
    /// L'échéance du token d'accès personnel (`GITHUB_TOKEN_EXPIRES_ON`), que GitHub n'expose pas en
    /// GraphQL : rapportée avec chaque état, pour que la page d'état prévienne avant.
    pub token_expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ServiceError {
    /// Le token est refusé (401) : expiré ou révoqué, il faut en créer un.
    KeyRejected {
        status: u16,
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
    /// Appels réussis à GitHub.
    pub calls: u32,
    /// Dépôts lisibles, fourches comprises.
    pub repos: u32,
    /// Dépôts dont l'historique a été lu.
    pub repos_read: u32,
    pub commits: u32,
    /// Faits de pull requests : ouvertures et fusions.
    pub pull_requests: u32,
    pub reviews: u32,
    pub issues: u32,
    pub comments: u32,
    pub accepted: u32,
    pub duplicate: u32,
    pub rejected: u32,
    /// Éléments de GitHub sans enregistrement : sans date, sans dépôt, sans id.
    pub skipped: u32,
    /// Tranches de recherche qui dépassent le plafond de 1 000 résultats.
    pub truncated: u32,
    /// Une organisation SAML a refusé le token : ses dépôts sont sautés.
    pub sso_required: bool,
}

// ─── Les listes blanches ─────────────────────────────────────────────────────

fn utc(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn date(value: Option<&Value>) -> Option<DateTime<Utc>> {
    value
        .and_then(Value::as_str)
        .and_then(|text| DateTime::parse_from_rfc3339(text).ok())
        .map(|at| at.with_timezone(&Utc))
}

fn text(value: Option<&Value>) -> Option<&str> {
    value.and_then(Value::as_str).filter(|t| !t.is_empty())
}

/// Un titre, coupé : jamais plus que ce que la carte montre.
fn title(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(|t| t.chars().take(MAX_TITLE_CHARS).collect())
}

fn count(value: Option<&Value>) -> Option<i64> {
    value.and_then(Value::as_i64).filter(|n| *n >= 0)
}

/// Le dépôt d'un fait, sous son id GraphQL stable, avec son nom `propriétaire/nom` du moment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoRef {
    pub id: String,
    pub name: String,
}

impl RepoRef {
    /// Depuis `{id, nameWithOwner}`.
    pub fn of(node: Option<&Value>) -> Option<Self> {
        let node = node?;
        Some(Self {
            id: text(node.get("id"))?.to_owned(),
            name: text(node.get("nameWithOwner"))?.to_owned(),
        })
    }

    fn payload(&self) -> Value {
        json!({ "id": self.id, "name": self.name })
    }
}

fn record(key: String, kind: &str, at: DateTime<Utc>, payload: Value) -> Value {
    json!({
        "key": key,
        "kind": kind,
        "schema_version": 1,
        "occurred_at": utc(at),
        "payload": payload,
    })
}

/// Un commit de l'historique, à sa date d'auteur. Sa clé est son sha : un même commit vu deux fois
/// (relecture, autre dépôt) est le même fait. Sans le corps du message.
pub fn commit_record(repo: &RepoRef, node: &Value) -> Option<Value> {
    let oid = text(node.get("oid"))?;
    let authored = date(node.get("authoredDate"))?;
    let committed = date(node.get("committedDate"));
    Some(record(
        format!("commit:{oid}"),
        "github.commit",
        authored,
        json!({
            "repo": repo.payload(),
            "oid": oid,
            "authored_at": utc(authored),
            "committed_at": committed.map(utc),
            "title": title(node.get("messageHeadline")),
            "additions": count(node.get("additions")),
            "deletions": count(node.get("deletions")),
            "url": text(node.get("url")),
        }),
    ))
}

/// Une pull request de la recherche : son ouverture, et sa fusion si elle a eu lieu, avec ses lignes.
pub fn pull_request_records(node: &Value) -> Option<Vec<Value>> {
    let id = text(node.get("id"))?;
    let repo = RepoRef::of(node.get("repository"))?;
    let created = date(node.get("createdAt"))?;
    let base = json!({
        "id": id,
        "repo": repo.payload(),
        "number": node.get("number").and_then(Value::as_i64),
        "title": title(node.get("title")),
        "url": text(node.get("url")),
    });
    let mut opened = base.clone();
    opened["action"] = json!("opened");
    let mut records = vec![record(
        format!("pr:{id}:opened"),
        "github.pull_request",
        created,
        opened,
    )];
    if let Some(merged_at) = date(node.get("mergedAt")) {
        let mut merged = base;
        merged["action"] = json!("merged");
        merged["additions"] = json!(count(node.get("additions")));
        merged["deletions"] = json!(count(node.get("deletions")));
        records.push(record(
            format!("pr:{id}:merged"),
            "github.pull_request",
            merged_at,
            merged,
        ));
    }
    Some(records)
}

/// Mes reviews soumises d'une pull request de la recherche, à leur soumission ; une review en attente
/// (sans date de soumission) ne part pas. `skipped` compte les reviews illisibles.
pub fn review_records(node: &Value, skipped: &mut u32) -> Vec<Value> {
    let Some(repo) = RepoRef::of(node.get("repository")) else {
        *skipped += 1;
        return Vec::new();
    };
    let reviews = node
        .pointer("/reviews/nodes")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut records = Vec::new();
    for review in reviews {
        let state = text(review.get("state"));
        if state == Some("PENDING") {
            continue;
        }
        let (Some(id), Some(state), Some(at)) = (
            text(review.get("id")),
            state,
            date(review.get("submittedAt")),
        ) else {
            *skipped += 1;
            continue;
        };
        records.push(record(
            format!("review:{id}"),
            "github.review",
            at,
            json!({
                "id": id,
                "state": state,
                "repo": repo.payload(),
                "number": node.get("number").and_then(Value::as_i64),
                "title": title(node.get("title")),
                "url": text(review.get("url")),
            }),
        ));
    }
    records
}

/// Une issue de la recherche, à sa création.
pub fn issue_record(node: &Value) -> Option<Value> {
    let id = text(node.get("id"))?;
    let repo = RepoRef::of(node.get("repository"))?;
    let created = date(node.get("createdAt"))?;
    Some(record(
        format!("issue:{id}"),
        "github.issue",
        created,
        json!({
            "id": id,
            "repo": repo.payload(),
            "number": node.get("number").and_then(Value::as_i64),
            "title": title(node.get("title")),
            "url": text(node.get("url")),
        }),
    ))
}

/// Un commentaire d'issue ou de pull request (`viewer.issueComments`), à sa création, avec le numéro et
/// le titre de sa cible. Jamais son corps.
pub fn issue_comment_record(node: &Value) -> Option<Value> {
    let id = text(node.get("id"))?;
    let repo = RepoRef::of(node.get("repository"))?;
    let created = date(node.get("createdAt"))?;
    let (target, subject) = match node.get("pullRequest").filter(|p| !p.is_null()) {
        Some(pull) => ("pull_request", pull),
        None => ("issue", node.get("issue").filter(|i| !i.is_null())?),
    };
    Some(record(
        format!("comment:{id}"),
        "github.comment",
        created,
        json!({
            "id": id,
            "repo": repo.payload(),
            "target": target,
            "number": subject.get("number").and_then(Value::as_i64),
            "title": title(subject.get("title")),
            "url": text(node.get("url")),
        }),
    ))
}

/// Un commentaire de commit (`viewer.commitComments`), à sa création, avec pour titre la première ligne
/// du message du commit.
pub fn commit_comment_record(node: &Value) -> Option<Value> {
    let id = text(node.get("id"))?;
    let repo = RepoRef::of(node.get("repository"))?;
    let created = date(node.get("createdAt"))?;
    Some(record(
        format!("comment:{id}"),
        "github.comment",
        created,
        json!({
            "id": id,
            "repo": repo.payload(),
            "target": "commit",
            "number": null,
            "title": title(node.pointer("/commit/messageHeadline")),
            "url": text(node.get("url")),
        }),
    ))
}

// ─── Les requêtes GraphQL ────────────────────────────────────────────────────

const RATE_LIMIT: &str = "rateLimit { remaining resetAt }";
const REPO_FIELDS: &str = "pageInfo { hasNextPage endCursor } \
     nodes { id nameWithOwner isFork pushedAt defaultBranchRef { name } }";

fn viewer_query() -> String {
    format!(
        "query Viewer {{ viewer {{ id login createdAt organizations(first: 100) {{ nodes {{ login }} }} }} \
         {RATE_LIMIT} }}"
    )
}

fn viewer_repositories_query() -> String {
    format!(
        "query ViewerRepositories($after: String) {{ viewer {{ repositories(first: 100, after: $after, \
         affiliations: [OWNER, COLLABORATOR, ORGANIZATION_MEMBER], \
         ownerAffiliations: [OWNER, COLLABORATOR, ORGANIZATION_MEMBER]) {{ {REPO_FIELDS} }} }} {RATE_LIMIT} }}"
    )
}

fn organization_repositories_query() -> String {
    format!(
        "query OrganizationRepositories($login: String!, $after: String) {{ organization(login: $login) \
         {{ repositories(first: 100, after: $after) {{ {REPO_FIELDS} }} }} {RATE_LIMIT} }}"
    )
}

fn contributed_repositories_query() -> String {
    format!(
        "query ContributedRepositories($after: String) {{ viewer {{ repositoriesContributedTo(first: 100, \
         after: $after, includeUserRepositories: true) {{ {REPO_FIELDS} }} }} {RATE_LIMIT} }}"
    )
}

fn history_query() -> String {
    format!(
        "query History($id: ID!, $author: ID!, $since: GitTimestamp, $after: String) {{ \
         node(id: $id) {{ ... on Repository {{ defaultBranchRef {{ target {{ ... on Commit {{ \
         history(first: 100, after: $after, author: {{id: $author}}, since: $since) {{ \
         pageInfo {{ hasNextPage endCursor }} \
         nodes {{ oid authoredDate committedDate messageHeadline additions deletions url }} }} }} }} }} }} }} \
         {RATE_LIMIT} }}"
    )
}

fn search_query() -> String {
    format!(
        "query Search($q: String!, $after: String, $login: String!, $withReviews: Boolean!) {{ \
         search(type: ISSUE, query: $q, first: 100, after: $after) {{ issueCount \
         pageInfo {{ hasNextPage endCursor }} nodes {{ \
         ... on PullRequest {{ id number title url createdAt mergedAt additions deletions \
         repository {{ id nameWithOwner }} \
         reviews(first: 100, author: $login) @include(if: $withReviews) {{ nodes {{ id state submittedAt url }} }} }} \
         ... on Issue {{ id number title url createdAt repository {{ id nameWithOwner }} }} }} }} \
         {RATE_LIMIT} }}"
    )
}

fn issue_comments_query() -> String {
    format!(
        "query IssueComments($after: String) {{ viewer {{ issueComments(first: 100, after: $after, \
         orderBy: {{field: UPDATED_AT, direction: DESC}}) {{ pageInfo {{ hasNextPage endCursor }} \
         nodes {{ id createdAt updatedAt url repository {{ id nameWithOwner }} \
         issue {{ number title }} pullRequest {{ number title }} }} }} }} {RATE_LIMIT} }}"
    )
}

fn commit_comments_query() -> String {
    format!(
        "query CommitComments($after: String) {{ viewer {{ commitComments(first: 100, after: $after) {{ \
         pageInfo {{ hasNextPage endCursor }} \
         nodes {{ id createdAt url repository {{ id nameWithOwner }} commit {{ messageHeadline }} }} }} }} \
         {RATE_LIMIT} }}"
    )
}

// ─── Le client de GitHub ─────────────────────────────────────────────────────

fn unavailable(err: &reqwest::Error) -> ServiceError {
    ServiceError::Unavailable {
        reason: network_reason(err),
    }
}

fn header_number(response: &Response, name: &str) -> Option<i64> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse().ok())
}

/// Le délai demandé : `retry-after`, sinon la remise à zéro de la limite horaire, sinon une minute.
fn rate_limit_delay(response: &Response) -> Duration {
    if let Some(seconds) = header_number(response, RETRY_AFTER.as_str()) {
        return Duration::from_secs(seconds.max(0).unsigned_abs());
    }
    if header_number(response, "x-ratelimit-remaining") == Some(0)
        && let Some(reset) = header_number(response, "x-ratelimit-reset")
    {
        let wait = reset - Utc::now().timestamp();
        return Duration::from_secs(wait.max(1).unsigned_abs());
    }
    DEFAULT_RATE_LIMIT_DELAY
}

/// Une réponse GraphQL exploitable.
pub struct Answer {
    pub data: Value,
    /// Une erreur signale une organisation SAML qui refuse le token : ce qu'elle cache manque.
    pub sso: bool,
    /// Le reste de la limite horaire et sa remise à zéro, si la réponse les donne.
    pub budget: Option<(i64, DateTime<Utc>)>,
}

pub struct Github {
    http: Client,
    url: String,
    token: String,
}

impl Github {
    pub fn new(http: Client, url: &str, token: &str) -> Self {
        Self {
            http,
            url: url.to_owned(),
            token: token.to_owned(),
        }
    }

    /// Une requête. Le statut et les erreurs GraphQL décident ; jamais le texte d'une erreur dans un
    /// log : seule la présence de « SAML » est lue, pour une organisation à autoriser.
    pub async fn query(
        &self,
        call: &'static str,
        query: &str,
        variables: Value,
    ) -> Result<Answer, ServiceError> {
        let response = self
            .http
            .post(&self.url)
            .header(AUTHORIZATION, format!("bearer {}", self.token))
            .header(ACCEPT, "application/json")
            .json(&json!({ "query": query, "variables": variables }))
            .send()
            .await
            .map_err(|err| unavailable(&err))?;
        let status = response.status();
        if status == StatusCode::UNAUTHORIZED {
            return Err(ServiceError::KeyRejected { status: 401 });
        }
        // Une limite secondaire répond 403 ou 429, une limite horaire épuisée 403 avec un reste à 0.
        if matches!(
            status,
            StatusCode::FORBIDDEN | StatusCode::TOO_MANY_REQUESTS
        ) {
            return Err(ServiceError::RateLimited {
                retry_after: rate_limit_delay(&response),
            });
        }
        if !status.is_success() {
            return Err(ServiceError::Failed {
                status: status.as_u16(),
                call,
            });
        }
        let reset_header = header_number(&response, "x-ratelimit-reset");
        let body: Value = response
            .json()
            .await
            .map_err(|_| ServiceError::Unavailable { reason: "body" })?;
        let mut sso = false;
        let mut other = false;
        for error in body
            .get("errors")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            if error.get("type").and_then(Value::as_str) == Some("RATE_LIMITED") {
                let retry_after = reset_header
                    .map(|reset| (reset - Utc::now().timestamp()).max(1).unsigned_abs())
                    .map_or(DEFAULT_RATE_LIMIT_DELAY, Duration::from_secs);
                return Err(ServiceError::RateLimited { retry_after });
            }
            let message = error.get("message").and_then(Value::as_str).unwrap_or("");
            if message.to_ascii_uppercase().contains("SAML") {
                sso = true;
            } else {
                other = true;
            }
        }
        let data = body.get("data").cloned().unwrap_or(Value::Null);
        if !data.is_object() {
            return Err(if other || sso {
                ServiceError::Failed { status: 200, call }
            } else {
                ServiceError::Unavailable { reason: "shape" }
            });
        }
        let budget = data
            .pointer("/rateLimit/remaining")
            .and_then(Value::as_i64)
            .zip(date(data.pointer("/rateLimit/resetAt")));
        Ok(Answer { data, sso, budget })
    }
}

// ─── Le connecteur ───────────────────────────────────────────────────────────

/// Ce qu'on sait de l'historique d'un dépôt.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
struct Repo {
    /// Tout l'historique a été lu au moins une fois.
    #[serde(default)]
    complete: bool,
    /// La page suivante du premier passage, pour reprendre après une coupure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    after: Option<String>,
    /// `pushedAt` à la dernière lecture complète : un dépôt n'est relu que s'il a avancé.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pushed_at: Option<DateTime<Utc>>,
}

/// L'avancement d'une liste lue en entier une fois, puis par date de mise à jour.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
struct Sweep {
    /// Le début de la lecture entière en cours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    started_at: Option<DateTime<Utc>>,
    /// L'année de `created:` en cours de lecture (recherches seulement).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    year: Option<i32>,
    /// La page suivante de la lecture entière en cours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    after: Option<String>,
    /// Tout ce qui a changé avant cette heure a été reçu.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    synced_at: Option<DateTime<Utc>>,
}

/// Ce qui survit à un redémarrage : des ids de dépôts, des curseurs et des heures, jamais un nom. Le
/// perdre ne coûte qu'une relecture entière, en `duplicate`.
#[derive(Debug, Default, Deserialize, Serialize)]
struct State {
    /// Le dernier passage réussi.
    #[serde(default)]
    last_pass_at: Option<DateTime<Utc>>,
    #[serde(default)]
    repos: BTreeMap<String, Repo>,
    #[serde(default)]
    pull_requests: Sweep,
    #[serde(default)]
    reviews: Sweep,
    #[serde(default)]
    issues: Sweep,
    #[serde(default)]
    issue_comments: Sweep,
}

/// Le compte du token.
struct Viewer {
    id: String,
    login: String,
    created_year: i32,
    organizations: Vec<String>,
}

/// Un dépôt de la liste.
struct Listed {
    repo: RepoRef,
    fork: bool,
    has_branch: bool,
    pushed_at: Option<DateTime<Utc>>,
}

/// Les trois recherches : la requête de base et ce qu'elles donnent.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Search {
    PullRequests,
    Reviews,
    Issues,
}

impl Search {
    fn base(self, login: &str) -> String {
        match self {
            Self::PullRequests => format!("author:{login} is:pr"),
            Self::Reviews => format!("reviewed-by:{login} is:pr -author:{login}"),
            Self::Issues => format!("author:{login} is:issue"),
        }
    }
}

pub struct Collector {
    config: Config,
    github: Github,
    ingest: IngestClient,
    state: State,
    /// Les clés déjà accusées depuis le démarrage : les commentaires de commit, relus en entier, et les
    /// fenêtres glissantes ne repartent pas à chaque passage.
    acknowledged: HashSet<String>,
    /// Le reste de la limite horaire et sa remise à zéro, selon la dernière réponse.
    budget: Option<(i64, DateTime<Utc>)>,
    /// Passages en échec d'affilée, pour espacer les essais.
    failures: u32,
}

/// L'état rapporté pour la page d'état après un passage en échec (`ok` sinon). Un token refusé demande
/// une action ; le reste passe seul. Un état qui ne s'écrit plus sur disque n'a pas de code à lui :
/// `service_unavailable`, le connecteur est en panne.
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

/// Le motif de log d'un échec : jamais de secret ni de donnée.
fn log_failure(error: &TickError, failures: u32) {
    match error {
        TickError::Service(ServiceError::KeyRejected { status }) => {
            error!(status, failures, "token GitHub refusé : révoqué ou expiré");
        }
        TickError::Service(ServiceError::RateLimited { retry_after }) => {
            info!(
                retry_after_s = retry_after.as_secs(),
                "github rate limit, waiting"
            );
        }
        TickError::Service(ServiceError::Failed { status, call }) => {
            warn!(status, call, failures, "github refused");
        }
        TickError::Service(ServiceError::Unavailable { reason }) => {
            warn!(reason, failures, "github unavailable");
        }
        TickError::Ingest { reason, status } => {
            error!(reason, status, failures, "github ingestion unavailable");
        }
        TickError::State => error!(failures, "github state not saved"),
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
                warn!("github record rejected, set aside");
            }
        }
    }
}

/// Une page d'une connexion GraphQL : ses nœuds et la suivante.
fn page(connection: Option<&Value>) -> (Vec<Value>, Option<String>) {
    let Some(connection) = connection.filter(|c| c.is_object()) else {
        return (Vec::new(), None);
    };
    let nodes = connection
        .get("nodes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let next = (connection.pointer("/pageInfo/hasNextPage") == Some(&Value::Bool(true)))
        .then(|| text(connection.pointer("/pageInfo/endCursor")).map(str::to_owned))
        .flatten();
    (nodes, next)
}

impl Collector {
    pub fn new(config: Config) -> anyhow::Result<Self> {
        let http = http_client()?;
        let state = load_state(&config.state_dir, STATE_FILE)?;
        let github = Github::new(http.clone(), &config.api_url, &config.api_token);
        let ingest = IngestClient::new(
            http,
            &config.ingest_url,
            &config.token,
            concat!("collector-", env!("CARGO_PKG_VERSION")),
        );
        Ok(Self {
            config,
            github,
            ingest,
            state,
            acknowledged: HashSet::new(),
            budget: None,
            failures: 0,
        })
    }

    /// Le dernier passage réussi.
    pub fn last_pass_at(&self) -> Option<DateTime<Utc>> {
        self.state.last_pass_at
    }

    /// Le nombre de dépôts dont tout l'historique a été lu.
    pub fn complete_repos(&self) -> usize {
        self.state.repos.values().filter(|r| r.complete).count()
    }

    fn save(&self) -> Result<(), TickError> {
        save_state(&self.config.state_dir, STATE_FILE, &self.state).map_err(|_| TickError::State)
    }

    /// Une requête, si la limite horaire le permet : sous `RATE_LIMIT_FLOOR` points, le passage
    /// s'arrête jusqu'à la remise à zéro, l'état gardé.
    async fn ask(
        &mut self,
        call: &'static str,
        query: &str,
        variables: Value,
        report: &mut TickReport,
    ) -> Result<Answer, TickError> {
        if let Some((remaining, reset)) = self.budget
            && remaining < RATE_LIMIT_FLOOR
        {
            let wait = (reset - Utc::now()).num_seconds().max(1).unsigned_abs();
            if reset > Utc::now() {
                return Err(ServiceError::RateLimited {
                    retry_after: Duration::from_secs(wait),
                }
                .into());
            }
        }
        let answer = self.github.query(call, query, variables).await?;
        report.calls += 1;
        if answer.budget.is_some() {
            self.budget = answer.budget;
        }
        if answer.sso {
            report.sso_required = true;
        }
        Ok(answer)
    }

    /// Envoie ce qui n'a pas déjà été accusé depuis le démarrage, par lots.
    async fn send(
        &mut self,
        records: Vec<Value>,
        report: &mut TickReport,
    ) -> Result<(), TickError> {
        let fresh: Vec<Value> = records
            .into_iter()
            .filter(|r| {
                r["key"]
                    .as_str()
                    .is_some_and(|k| !self.acknowledged.contains(k))
            })
            .collect();
        for batch in fresh.chunks(RECORDS_PER_BATCH) {
            let results = self.ingest.send(batch).await?;
            for (record, result) in batch.iter().zip(&results) {
                let outcome = RecordOutcome::of(result);
                report.count(outcome);
                if outcome != RecordOutcome::Rejected
                    && let Some(key) = record["key"].as_str()
                {
                    self.acknowledged.insert(key.to_owned());
                }
            }
        }
        Ok(())
    }

    async fn viewer(&mut self, report: &mut TickReport) -> Result<Viewer, TickError> {
        let answer = self
            .ask("viewer", &viewer_query(), json!({}), report)
            .await?;
        let viewer = answer.data.get("viewer");
        let field = |name: &str| text(viewer.and_then(|v| v.get(name))).map(str::to_owned);
        let (Some(id), Some(login)) = (field("id"), field("login")) else {
            return Err(ServiceError::Unavailable { reason: "shape" }.into());
        };
        let created_year =
            date(viewer.and_then(|v| v.get("createdAt"))).map_or(2008, |at| at.year());
        let organizations = viewer
            .and_then(|v| v.pointer("/organizations/nodes"))
            .and_then(Value::as_array)
            .map(|nodes| {
                nodes
                    .iter()
                    .filter_map(|n| text(n.get("login")).map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        Ok(Viewer {
            id,
            login,
            created_year,
            organizations,
        })
    }

    /// Toutes les pages d'une liste de dépôts.
    async fn repositories(
        &mut self,
        call: &'static str,
        query: &str,
        variables: Value,
        pointer: &str,
        listed: &mut BTreeMap<String, Listed>,
        report: &mut TickReport,
    ) -> Result<(), TickError> {
        let mut after: Option<String> = None;
        loop {
            let mut vars = variables.clone();
            vars["after"] = json!(after);
            let answer = self.ask(call, query, vars, report).await?;
            let (nodes, next) = page(answer.data.pointer(pointer));
            for node in nodes {
                let Some(repo) = RepoRef::of(Some(&node)) else {
                    report.skipped += 1;
                    continue;
                };
                listed.entry(repo.id.clone()).or_insert(Listed {
                    fork: node.get("isFork") == Some(&Value::Bool(true)),
                    has_branch: node.get("defaultBranchRef").is_some_and(|b| b.is_object()),
                    pushed_at: date(node.get("pushedAt")),
                    repo,
                });
            }
            match next {
                Some(next) => after = Some(next),
                None => return Ok(()),
            }
        }
    }

    /// L'union des dépôts lisibles, par id.
    async fn listed(
        &mut self,
        viewer: &Viewer,
        report: &mut TickReport,
    ) -> Result<BTreeMap<String, Listed>, TickError> {
        let mut listed = BTreeMap::new();
        self.repositories(
            "viewer_repositories",
            &viewer_repositories_query(),
            json!({}),
            "/viewer/repositories",
            &mut listed,
            report,
        )
        .await?;
        for login in &viewer.organizations {
            self.repositories(
                "organization_repositories",
                &organization_repositories_query(),
                json!({ "login": login }),
                "/organization/repositories",
                &mut listed,
                report,
            )
            .await?;
        }
        self.repositories(
            "contributed_repositories",
            &contributed_repositories_query(),
            json!({}),
            "/viewer/repositoriesContributedTo",
            &mut listed,
            report,
        )
        .await?;
        report.repos = u32::try_from(listed.len()).unwrap_or(u32::MAX);
        Ok(listed)
    }

    /// L'historique d'un dépôt : entier (en reprenant à la page notée) tant qu'il n'a jamais été lu
    /// jusqu'au bout, sinon les 30 derniers jours s'il a été poussé depuis.
    async fn history(
        &mut self,
        viewer: &Viewer,
        listed: &Listed,
        now: DateTime<Utc>,
        report: &mut TickReport,
    ) -> Result<(), TickError> {
        let id = listed.repo.id.clone();
        let known = self.state.repos.get(&id).cloned().unwrap_or_default();
        if !listed.has_branch {
            // Un dépôt vide : rien à lire, jusqu'à ce qu'il soit poussé.
            return Ok(());
        }
        let (since, mut after) = if !known.complete {
            (None, known.after.clone())
        } else if listed.pushed_at > known.pushed_at {
            (Some(now - COMMIT_WINDOW), None)
        } else {
            return Ok(());
        };
        report.repos_read += 1;
        loop {
            let variables = json!({
                "id": id, "author": viewer.id, "after": after, "since": since.map(utc),
            });
            let answer = self
                .ask("history", &history_query(), variables, report)
                .await?;
            if answer.data.get("node").is_none_or(Value::is_null) {
                // Dépôt disparu ou caché par une organisation SAML : relu au passage suivant.
                return Ok(());
            }
            let (nodes, next) = page(answer.data.pointer("/node/defaultBranchRef/target/history"));
            let mut records = Vec::new();
            for node in &nodes {
                match commit_record(&listed.repo, node) {
                    Some(record) => records.push(record),
                    None => report.skipped += 1,
                }
            }
            report.commits += u32::try_from(records.len()).unwrap_or(u32::MAX);
            self.send(records, report).await?;
            let repo = self.state.repos.entry(id.clone()).or_default();
            match next {
                Some(next) => {
                    if !repo.complete {
                        repo.after = Some(next.clone());
                        self.save()?;
                    }
                    after = Some(next);
                }
                None => {
                    repo.complete = true;
                    repo.after = None;
                    repo.pushed_at = listed.pushed_at;
                    return self.save();
                }
            }
        }
    }

    fn sweep(&mut self, search: Search) -> &mut Sweep {
        match search {
            Search::PullRequests => &mut self.state.pull_requests,
            Search::Reviews => &mut self.state.reviews,
            Search::Issues => &mut self.state.issues,
        }
    }

    /// Les faits d'une page de recherche.
    fn search_records(search: Search, nodes: &[Value], report: &mut TickReport) -> Vec<Value> {
        let mut records = Vec::new();
        for node in nodes {
            match search {
                Search::PullRequests => match pull_request_records(node) {
                    Some(found) => {
                        report.pull_requests += u32::try_from(found.len()).unwrap_or(u32::MAX);
                        records.extend(found);
                    }
                    None => report.skipped += 1,
                },
                Search::Reviews => {
                    let found = review_records(node, &mut report.skipped);
                    report.reviews += u32::try_from(found.len()).unwrap_or(u32::MAX);
                    records.extend(found);
                }
                Search::Issues => match issue_record(node) {
                    Some(record) => {
                        report.issues += 1;
                        records.push(record);
                    }
                    None => report.skipped += 1,
                },
            }
        }
        records
    }

    /// Une recherche : par années de `created:` tant qu'elle n'a jamais été lue en entier (en reprenant
    /// à l'année et à la page notées), puis par `updated:>=` depuis le passage précédent moins un jour.
    async fn search(
        &mut self,
        search: Search,
        viewer: &Viewer,
        now: DateTime<Utc>,
        report: &mut TickReport,
    ) -> Result<(), TickError> {
        let base = search.base(&viewer.login);
        let with_reviews = search == Search::Reviews;
        if let Some(synced) = self.sweep(search).synced_at {
            let query = format!(
                "{base} updated:>={}",
                (synced - OVERLAP).format("%Y-%m-%dT%H:%M:%SZ")
            );
            let mut after: Option<String> = None;
            loop {
                let variables = json!({
                    "q": query, "after": after, "login": viewer.login, "withReviews": with_reviews,
                });
                let answer = self
                    .ask("search", &search_query(), variables, report)
                    .await?;
                let (nodes, next) = page(answer.data.get("search"));
                let records = Self::search_records(search, &nodes, report);
                self.send(records, report).await?;
                match next {
                    Some(next) => after = Some(next),
                    None => break,
                }
            }
            self.sweep(search).synced_at = Some(now);
            return self.save();
        }

        let sweep = self.sweep(search);
        let started = *sweep.started_at.get_or_insert(now);
        let mut year = sweep.year.unwrap_or(viewer.created_year);
        let mut after = sweep.after.clone();
        while year <= now.year() {
            let query = format!("{base} created:{year}-01-01..{year}-12-31");
            loop {
                let variables = json!({
                    "q": query, "after": after, "login": viewer.login, "withReviews": with_reviews,
                });
                let answer = self
                    .ask("search", &search_query(), variables, report)
                    .await?;
                if after.is_none()
                    && answer
                        .data
                        .pointer("/search/issueCount")
                        .and_then(Value::as_i64)
                        .is_some_and(|n| n > SEARCH_CAP)
                {
                    report.truncated += 1;
                }
                let (nodes, next) = page(answer.data.get("search"));
                let records = Self::search_records(search, &nodes, report);
                self.send(records, report).await?;
                let sweep = self.sweep(search);
                sweep.year = Some(year);
                sweep.after = next.clone();
                self.save()?;
                match next {
                    Some(next) => after = Some(next),
                    None => break,
                }
            }
            year += 1;
            after = None;
            let sweep = self.sweep(search);
            sweep.year = Some(year);
            sweep.after = None;
            self.save()?;
        }
        *self.sweep(search) = Sweep {
            synced_at: Some(started),
            ..Sweep::default()
        };
        self.save()
    }

    /// Les commentaires d'issues et de pull requests, du plus récent au plus ancien par mise à jour :
    /// tous au premier passage (en reprenant à la page notée), puis jusqu'au passage précédent moins un
    /// jour.
    async fn issue_comments(
        &mut self,
        now: DateTime<Utc>,
        report: &mut TickReport,
    ) -> Result<(), TickError> {
        let threshold = self.state.issue_comments.synced_at.map(|at| at - OVERLAP);
        let backfill = threshold.is_none();
        let started = if backfill {
            *self.state.issue_comments.started_at.get_or_insert(now)
        } else {
            now
        };
        let mut after = if backfill {
            self.state.issue_comments.after.clone()
        } else {
            None
        };
        loop {
            let answer = self
                .ask(
                    "issue_comments",
                    &issue_comments_query(),
                    json!({ "after": after }),
                    report,
                )
                .await?;
            let (nodes, next) = page(answer.data.pointer("/viewer/issueComments"));
            let mut records = Vec::new();
            let mut older = false;
            for node in &nodes {
                if let Some(threshold) = threshold
                    && date(node.get("updatedAt")).is_some_and(|at| at < threshold)
                {
                    older = true;
                    continue;
                }
                match issue_comment_record(node) {
                    Some(record) => records.push(record),
                    None => report.skipped += 1,
                }
            }
            report.comments += u32::try_from(records.len()).unwrap_or(u32::MAX);
            self.send(records, report).await?;
            if backfill {
                self.state.issue_comments.after = next.clone();
                self.save()?;
            }
            match next {
                Some(next) if !older => after = Some(next),
                _ => break,
            }
        }
        self.state.issue_comments = Sweep {
            synced_at: Some(started),
            ..Sweep::default()
        };
        self.save()
    }

    /// Les commentaires de commits, relus en entier : il y en a peu, et ceux déjà accusés ne repartent
    /// pas.
    async fn commit_comments(&mut self, report: &mut TickReport) -> Result<(), TickError> {
        let mut after: Option<String> = None;
        loop {
            let answer = self
                .ask(
                    "commit_comments",
                    &commit_comments_query(),
                    json!({ "after": after }),
                    report,
                )
                .await?;
            let (nodes, next) = page(answer.data.pointer("/viewer/commitComments"));
            let mut records = Vec::new();
            for node in &nodes {
                match commit_comment_record(node) {
                    Some(record) => records.push(record),
                    None => report.skipped += 1,
                }
            }
            report.comments += u32::try_from(records.len()).unwrap_or(u32::MAX);
            self.send(records, report).await?;
            match next {
                Some(next) => after = Some(next),
                None => return Ok(()),
            }
        }
    }

    /// Un passage. GitHub d'abord, l'ingestion ensuite : le signal de vie ne part qu'après une réponse
    /// de GitHub. Un token refusé n'en envoie donc aucun, et le seuil de silence de la source alerte.
    pub async fn tick(&mut self) -> Result<TickReport, TickError> {
        let mut report = TickReport::default();
        let now = Utc::now();
        let viewer = self.viewer(&mut report).await?;
        self.ingest.heartbeat().await?;
        let listed = self.listed(&viewer, &mut report).await?;
        for repo in listed.values().filter(|r| !r.fork) {
            self.history(&viewer, repo, now, &mut report).await?;
        }
        for search in [Search::PullRequests, Search::Reviews, Search::Issues] {
            self.search(search, &viewer, now, &mut report).await?;
        }
        self.issue_comments(now, &mut report).await?;
        self.commit_comments(&mut report).await?;
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
                    repos = report.repos,
                    repos_read = report.repos_read,
                    commits = report.commits,
                    pull_requests = report.pull_requests,
                    reviews = report.reviews,
                    issues = report.issues,
                    comments = report.comments,
                    accepted = report.accepted,
                    duplicate = report.duplicate,
                    rejected = report.rejected,
                    skipped = report.skipped,
                    truncated = report.truncated,
                    sso_required = report.sso_required,
                    "github pass completed"
                );
                let status = if report.sso_required {
                    SourceStatus::failed(Health::ActionRequired, Reason::SsoRequired)
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
        status.expires_at = self.config.token_expires_at;
        status.last_collected_at = self.state.last_pass_at;
        report_status(&self.ingest, status, delay, "github").await;
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

    fn repo() -> Value {
        json!({"id": "R_1", "nameWithOwner": "moi/depot", "description": "écartée"})
    }

    #[test]
    fn the_whitelists_are_frozen() {
        let commit = commit_record(
            &RepoRef::of(Some(&repo())).unwrap(),
            &json!({"oid": "abcdef0", "authoredDate": "2026-11-06T21:15:00+01:00",
                    "committedDate": "2026-11-07T09:00:00Z", "messageHeadline": "Titre",
                    "message": "Titre\n\ncorps jamais recopié", "additions": 3, "deletions": 1,
                    "url": "https://github.com/moi/depot/commit/abcdef0", "author": {"email": "x"}}),
        )
        .unwrap();
        assert_eq!(commit["key"], "commit:abcdef0");
        assert_eq!(
            commit["occurred_at"], "2026-11-06T20:15:00Z",
            "date d'auteur"
        );
        assert_eq!(
            keys(&commit["payload"]),
            [
                "additions",
                "authored_at",
                "committed_at",
                "deletions",
                "oid",
                "repo",
                "title",
                "url"
            ]
        );
        assert_eq!(
            commit["payload"]["repo"],
            json!({"id": "R_1", "name": "moi/depot"})
        );

        let pr = json!({"id": "PR_1", "number": 4, "title": "T", "url": "https://x",
                        "body": "jamais", "createdAt": "2026-11-01T10:00:00Z",
                        "mergedAt": "2026-11-03T10:00:00Z", "additions": 10, "deletions": 2,
                        "repository": repo()});
        let records = pull_request_records(&pr).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0]["key"], "pr:PR_1:opened");
        assert_eq!(records[0]["occurred_at"], "2026-11-01T10:00:00Z");
        assert_eq!(
            keys(&records[0]["payload"]),
            ["action", "id", "number", "repo", "title", "url"]
        );
        assert_eq!(records[1]["key"], "pr:PR_1:merged");
        assert_eq!(records[1]["occurred_at"], "2026-11-03T10:00:00Z");
        assert_eq!(records[1]["payload"]["additions"], 10);
        let open = json!({"id": "PR_2", "number": 5, "createdAt": "2026-11-01T10:00:00Z",
                          "mergedAt": null, "repository": repo()});
        assert_eq!(pull_request_records(&open).unwrap().len(), 1);

        let mut skipped = 0;
        let reviewed = json!({"id": "PR_3", "number": 6, "title": "T", "repository": repo(),
        "reviews": {"nodes": [
            {"id": "PRR_1", "state": "APPROVED", "submittedAt": "2026-11-02T10:00:00Z",
             "url": "https://x", "body": "jamais"},
            {"id": "PRR_2", "state": "PENDING", "submittedAt": null},
            {"id": "PRR_3", "state": "COMMENTED"},
        ]}});
        let reviews = review_records(&reviewed, &mut skipped);
        assert_eq!(skipped, 1, "sans date de soumission");
        assert_eq!(reviews.len(), 1);
        assert_eq!(reviews[0]["key"], "review:PRR_1");
        assert_eq!(
            keys(&reviews[0]["payload"]),
            ["id", "number", "repo", "state", "title", "url"]
        );

        let comment =
            issue_comment_record(&json!({"id": "IC_1", "createdAt": "2026-11-02T10:00:00Z",
            "updatedAt": "2026-11-02T10:00:00Z", "url": "https://x", "body": "jamais",
            "repository": repo(), "issue": {"number": 6, "title": "T"},
            "pullRequest": {"number": 6, "title": "T"}}))
            .unwrap();
        assert_eq!(comment["payload"]["target"], "pull_request");
        assert_eq!(
            keys(&comment["payload"]),
            ["id", "number", "repo", "target", "title", "url"]
        );
        let on_issue =
            issue_comment_record(&json!({"id": "IC_2", "createdAt": "2026-11-02T10:00:00Z",
            "repository": repo(), "issue": {"number": 7, "title": "T"}, "pullRequest": null}))
            .unwrap();
        assert_eq!(on_issue["payload"]["target"], "issue");
        let on_commit = commit_comment_record(&json!({"id": "CC_1",
            "createdAt": "2026-11-02T10:00:00Z", "repository": repo(),
            "commit": {"messageHeadline": "Titre", "message": "jamais"}}))
        .unwrap();
        assert_eq!(on_commit["payload"]["number"], Value::Null);
        assert_eq!(on_commit["payload"]["title"], "Titre");

        let long = commit_record(
            &RepoRef::of(Some(&repo())).unwrap(),
            &json!({"oid": "abcdef0", "authoredDate": "2026-11-06T21:15:00Z",
                    "messageHeadline": "x".repeat(1000)}),
        )
        .unwrap();
        assert_eq!(
            long["payload"]["title"].as_str().unwrap().len(),
            MAX_TITLE_CHARS
        );
    }

    #[test]
    fn unreadable_items_are_skipped() {
        assert_eq!(
            commit_record(
                &RepoRef::of(Some(&repo())).unwrap(),
                &json!({"oid": "abcdef0"})
            ),
            None
        );
        assert_eq!(
            pull_request_records(&json!({"id": "PR_1", "createdAt": "2026-11-01T10:00:00Z"})),
            None
        );
        assert_eq!(
            issue_record(&json!({"id": "I_1", "repository": repo()})),
            None
        );
        assert_eq!(RepoRef::of(Some(&json!({"id": "R_1"}))), None);
    }

    #[test]
    fn a_rejected_token_asks_for_an_action_and_the_rest_is_an_error() {
        let status = |error| {
            let s = failure_status(&error);
            assert!(s.is_consistent());
            (s.state, s.reason.unwrap())
        };
        assert_eq!(
            status(TickError::Service(ServiceError::KeyRejected {
                status: 401
            })),
            (Health::ActionRequired, Reason::KeyRejected)
        );
        assert_eq!(
            status(TickError::Service(ServiceError::RateLimited {
                retry_after: Duration::from_secs(1)
            })),
            (Health::Error, Reason::RateLimited)
        );
        for other in [
            TickError::Service(ServiceError::Failed {
                status: 502,
                call: "history",
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
