//! L'API : ingestion v1 (contrat dans contract/ingestion-v1.schema.json), compte web et lecture.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::{DefaultBodyLimit, FromRequestParts, MatchedPath, Path, Query, Request, State};
use axum::http::header::{AUTHORIZATION, CACHE_CONTROL, SET_COOKIE, WWW_AUTHENTICATE};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::sync::Semaphore;
use tracing::{Instrument, error, info, info_span, warn};
use uuid::Uuid;

use crate::activity;
use crate::admin::{generate_token, token_hash};
use crate::auth;
use crate::calendar;
use crate::ingest::{Envelope, RejectReason, Rejection, parse_envelope};
use crate::music::fold;
use crate::read::{self, Period, ReadError};

pub const MAX_RECORDS: usize = 500;
pub const MAX_BODY_BYTES: usize = 1_000_000;

const LOGIN: &str = "/v1/auth/login";
const RECORDS: &str = "/v1/records";
/// Le mot de passe et l'identifiant n'ont pas besoin de plus.
const MAX_LOGIN_BYTES: usize = 4096;
/// Argon2 prend 19 Mio par calcul : le NAS n'en fait pas plus de deux à la fois.
const MAX_CONCURRENT_LOGINS: usize = 2;

/// Une route exige la session par défaut (voir `guard`) : seules celles-ci en sont dispensées.
/// `/v1/records` est à part : le token, ou sinon la session.
pub const NO_SESSION_ROUTES: [&str; 3] = ["/health", LOGIN, "/v1/heartbeat"];

/// Toute écriture par cookie doit porter cet en-tête. Un autre site ne peut pas l'ajouter sans
/// une autorisation CORS préalable, et le serveur n'en donne jamais.
pub const CSRF_HEADER: &str = "x-csrf-protection";

#[derive(Clone, Copy, Default)]
pub struct Config {
    pub insecure_cookie: bool,
}

#[derive(Clone)]
pub struct AppState {
    pool: PgPool,
    config: Config,
    logins: Arc<Semaphore>,
}

#[derive(Clone, Copy)]
struct RequestId(Uuid);

/// Posée par `guard` sur une requête authentifiée par sa session.
#[derive(Clone, Copy)]
struct SessionAuth;

/// Les routes que le jeton de service peut lire, en GET seulement : une liste fermée. Toute autre route,
/// et toute écriture, le refuse (`guard`), y compris celles qu'on ajoutera.
pub const READ_ROUTES: [&str; 11] = [
    "/v1/days/{date}",
    "/v1/days/{date}/music",
    "/v1/months/{month}",
    "/v1/sources/health",
    "/v1/inventory",
    "/v1/search",
    "/v1/occurrences",
    "/v1/durations",
    "/v1/item",
    "/v1/top",
    "/v1/discoveries",
];

/// Les deux niveaux de visibilité de l'API de lecture (voir l'architecture), posés par `guard` : les
/// noms de l'annuaire pour la session de la web app, les pseudonymes seuls pour le jeton de service.
/// Aucune réponse du POC ne lit l'annuaire : les deux niveaux rendent aujourd'hui la même chose, et
/// une route qui nommera un contact devra s'en tenir au pseudonyme sous `Pseudonyms`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visibility {
    Names,
    Pseudonyms,
}

pub fn router(pool: PgPool) -> Router {
    router_with(pool, Config::default())
}

pub fn router_with(pool: PgPool, config: Config) -> Router {
    harden(routes(), pool, config)
}

fn routes() -> Router<AppState> {
    Router::new()
        .route(
            RECORDS,
            post(post_records).layer(DefaultBodyLimit::max(MAX_BODY_BYTES)),
        )
        .route("/v1/heartbeat", post(post_heartbeat))
        .route(
            LOGIN,
            post(login).layer(DefaultBodyLimit::max(MAX_LOGIN_BYTES)),
        )
        .route("/v1/auth/logout", post(logout))
        .route("/v1/days/{date}", get(get_day))
        .route("/v1/days/{date}/music", get(get_day_music))
        // Pas dans `READ_ROUTES` : des coordonnées, que le MCP ne voit jamais.
        .route("/v1/days/{date}/activity", get(get_day_activity))
        .route("/v1/activities/{id}/track", get(get_activity_track))
        .route("/v1/months/{month}", get(get_month))
        // Pas dans `READ_ROUTES` : la vue de l'année ne sert qu'à la web app.
        .route("/v1/years/{year}", get(get_year))
        .route("/v1/sources/health", get(get_sources_health))
        .route("/v1/inventory", get(get_inventory))
        .route("/v1/search", get(get_search))
        .route("/v1/occurrences", get(get_occurrences))
        .route("/v1/durations", get(get_durations))
        .route("/v1/item", get(get_item))
        .route("/v1/top", get(get_top))
        .route("/v1/discoveries", get(get_discoveries))
        .route("/health", get(health))
}

/// Pose les protections sur toutes les routes, y compris celles qu'on ajoutera et les chemins inconnus.
pub fn harden(routes: Router<AppState>, pool: PgPool, config: Config) -> Router {
    let state = AppState {
        pool,
        config,
        logins: Arc::new(Semaphore::new(MAX_CONCURRENT_LOGINS)),
    };
    routes
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .layer(middleware::from_fn(request_context))
        .with_state(state)
}

/// Le seul contrôle d'accès, refus par défaut.
async fn guard(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(|path| path.as_str().to_owned());
    let route = route.as_deref().unwrap_or("");
    let headers = request.headers();
    let writes = !matches!(*request.method(), Method::GET | Method::HEAD);
    let csrf_ok = !writes || headers.contains_key(CSRF_HEADER);

    // Pas de CORS, donc pas d'Authorization venue d'un autre site : le token n'a rien à craindre
    // du CSRF. Un mauvais token ne se rattrape jamais par le cookie.
    if route == RECORDS && headers.contains_key(AUTHORIZATION) {
        return Ok(next.run(request).await);
    }
    if NO_SESSION_ROUTES.contains(&route) {
        if route == LOGIN && !csrf_ok {
            return Err(ApiError::CsrfRejected);
        }
        return Ok(next.run(request).await);
    }

    // Le jeton de service du MCP : une lecture, sur une route de lecture, et rien d'autre. Ailleurs,
    // un `Authorization` est refusé sans repli sur le cookie, comme pour l'ingestion.
    if headers.contains_key(AUTHORIZATION) {
        let reads = matches!(*request.method(), Method::GET | Method::HEAD);
        if !reads || !READ_ROUTES.contains(&route) {
            return Err(ApiError::Unauthorized);
        }
        let token = bearer_token(headers).ok_or(ApiError::Unauthorized)?;
        if !service_token_is_valid(&state.pool, token).await? {
            return Err(ApiError::Unauthorized);
        }
        request.extensions_mut().insert(Visibility::Pseudonyms);
        return Ok(next.run(request).await);
    }

    let token = auth::session_token(headers).ok_or(ApiError::Unauthorized)?;
    if !auth::session_is_valid(&state.pool, token).await? {
        return Err(ApiError::Unauthorized);
    }
    if !csrf_ok {
        return Err(ApiError::CsrfRejected);
    }
    request.extensions_mut().insert(SessionAuth);
    request.extensions_mut().insert(Visibility::Names);
    Ok(next.run(request).await)
}

async fn service_token_is_valid(pool: &PgPool, token: &str) -> Result<bool, sqlx::Error> {
    let hash = token_hash(token);
    sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM core.service_tokens
                          WHERE token_hash = $1 AND revoked_at IS NULL) AS "valid!""#,
        hash,
    )
    .fetch_one(pool)
    .await
}

/// Identifiant de requête, et le seul journal d'accès : jamais d'en-tête ni de corps.
async fn request_context(mut request: Request, next: Next) -> Response {
    let id = Uuid::now_v7();
    request.extensions_mut().insert(RequestId(id));
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map_or("unmatched", MatchedPath::as_str)
        .to_owned();
    let span = info_span!("request", request_id = %id, route = %route);
    async move {
        let started = Instant::now();
        let mut response = next.run(request).await;
        info!(
            status = response.status().as_u16(),
            duration_ms = started.elapsed().as_millis() as u64,
            "request completed"
        );
        // Rien ici ne se garde : le passé peut changer d'apparence (règle 7).
        response
            .headers_mut()
            .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
        response.headers_mut().insert(
            "x-request-id",
            HeaderValue::from_str(&id.to_string()).expect("un UUID est un en-tête valide"),
        );
        response
    }
    .instrument(span)
    .await
}

enum ApiError {
    Unauthorized,
    InvalidCredentials,
    CsrfRejected,
    InvalidDate,
    InvalidParameter(&'static str),
    NotFound,
    InvalidJson,
    BatchTooLarge,
    Internal,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        if let Self::InvalidParameter(field) = self {
            let body = json!({ "error": "invalid_parameter", "field": field });
            return (StatusCode::BAD_REQUEST, Json(body)).into_response();
        }
        let (status, code) = match &self {
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            // Le même refus quelle qu'en soit la cause : mauvais identifiant, mauvais mot de passe, pas de compte.
            Self::InvalidCredentials => (StatusCode::UNAUTHORIZED, "invalid_credentials"),
            Self::CsrfRejected => (StatusCode::FORBIDDEN, "csrf_rejected"),
            Self::InvalidDate => (StatusCode::BAD_REQUEST, "invalid_date"),
            Self::InvalidParameter(_) => (StatusCode::BAD_REQUEST, "invalid_parameter"),
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found"),
            Self::InvalidJson => (StatusCode::BAD_REQUEST, "invalid_json"),
            Self::BatchTooLarge => (StatusCode::PAYLOAD_TOO_LARGE, "batch_too_large"),
            Self::Internal => (StatusCode::INTERNAL_SERVER_ERROR, "internal"),
        };
        let mut response = (status, Json(json!({ "error": code }))).into_response();
        if matches!(self, Self::Unauthorized) {
            response
                .headers_mut()
                .insert(WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        }
        response
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(err: sqlx::Error) -> Self {
        // Seulement le code : le détail d'une erreur PostgreSQL peut citer des valeurs.
        match err.as_database_error() {
            Some(db) => error!(code = db.code().as_deref(), "database error"),
            None => error!(error = %err, "database unavailable"),
        }
        Self::Internal
    }
}

impl From<ReadError> for ApiError {
    fn from(err: ReadError) -> Self {
        match err {
            ReadError::Invalid(field) => Self::InvalidParameter(field),
            ReadError::NotFound => Self::NotFound,
            ReadError::Db(err) => err.into(),
        }
    }
}

/// Source authentifiée par son token, ou `webapp` si `guard` a validé la session : jamais une source
/// choisie par le client. Tout appel authentifié met à jour son dernier contact.
struct Source {
    id: i16,
    client_version: Option<String>,
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let (scheme, token) = headers.get(AUTHORIZATION)?.to_str().ok()?.split_once(' ')?;
    let token = token.trim();
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then_some(token)
}

impl FromRequestParts<AppState> for Source {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let client_version = parts
            .headers
            .get("x-client-version")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let id = if parts.extensions.get::<SessionAuth>().is_some() {
            sqlx::query_scalar!(
                "UPDATE core.sources
                 SET last_seen_at = now(), client_version = COALESCE($1, client_version)
                 WHERE name = 'webapp' AND auth = 'session' AND revoked_at IS NULL
                 RETURNING id",
                client_version,
            )
            .fetch_optional(&state.pool)
            .await?
        } else {
            let token = bearer_token(&parts.headers).ok_or(ApiError::Unauthorized)?;
            let hash = token_hash(token);
            // auth = 'token' : la source webapp passe par la session et n'a pas de token.
            sqlx::query_scalar!(
                "UPDATE core.sources
                 SET last_seen_at = now(), client_version = COALESCE($2, client_version)
                 WHERE token_hash = $1 AND auth = 'token' AND revoked_at IS NULL
                 RETURNING id",
                hash,
                client_version,
            )
            .fetch_optional(&state.pool)
            .await?
        }
        .ok_or(ApiError::Unauthorized)?;
        Ok(Self { id, client_version })
    }
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum Status {
    Accepted,
    Duplicate,
    Rejected,
}

#[derive(Serialize)]
struct RecordResult {
    key: Option<String>,
    status: Status,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<RejectReason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    field: Option<&'static str>,
}

#[derive(Serialize)]
struct RecordsResponse {
    request_id: Uuid,
    results: Vec<RecordResult>,
}

async fn post_records(
    State(state): State<AppState>,
    Extension(RequestId(request_id)): Extension<RequestId>,
    source: Source,
    // Dernier extracteur : le corps n'est lu qu'après l'authentification.
    body: Result<Bytes, BytesRejection>,
) -> Result<Json<RecordsResponse>, ApiError> {
    let body = body.map_err(|rejection| {
        if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
            ApiError::BatchTooLarge
        } else {
            ApiError::InvalidJson
        }
    })?;
    // Lecture souple : un enregistrement mal formé ne doit pas faire échouer tout le lot.
    let document: Value = serde_json::from_slice(&body).map_err(|_| ApiError::InvalidJson)?;
    let records = document
        .get("records")
        .and_then(Value::as_array)
        .ok_or(ApiError::InvalidJson)?;
    if records.len() > MAX_RECORDS {
        return Err(ApiError::BatchTooLarge);
    }

    let parsed = check_envelopes(&state.pool, source.id, records, Utc::now()).await?;
    let inserted = store(&state.pool, &source, request_id, &parsed).await?;
    let results = build_results(records, parsed, inserted);
    Ok(Json(RecordsResponse {
        request_id,
        results,
    }))
}

async fn check_envelopes(
    pool: &PgPool,
    source_id: i16,
    records: &[Value],
    now: DateTime<Utc>,
) -> Result<Vec<Result<Envelope, Rejection>>, sqlx::Error> {
    let granted: HashSet<String> = sqlx::query_scalar!(
        "SELECT kind FROM core.source_kinds WHERE source_id = $1 AND revoked_at IS NULL",
        source_id,
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .collect();

    let mut parsed: Vec<_> = records
        .iter()
        .map(|record| parse_envelope(record, &granted, now))
        .collect();

    // Seule limite à la charge utile : ce que jsonb ne sait pas stocker (\u0000, nombre hors bornes)
    // ferait échouer tout le lot à chaque nouvel essai. La base en juge, sans dupliquer ses règles.
    let payloads: Vec<&str> = parsed
        .iter()
        .flatten()
        .map(|e| e.payload.as_str())
        .collect();
    if !payloads.is_empty() {
        let storable = sqlx::query_scalar!(
            r#"SELECT pg_input_is_valid(p, 'jsonb') AS "storable!"
               FROM unnest($1::text[]) WITH ORDINALITY AS u(p, ord) ORDER BY ord"#,
            &payloads as &[&str],
        )
        .fetch_all(pool)
        .await?;
        let mut storable = storable.into_iter();
        for entry in &mut parsed {
            if entry.is_ok() && storable.next() == Some(false) {
                *entry = Err(Rejection::invalid("payload"));
            }
        }
    }
    Ok(parsed)
}

/// Une transaction par lot, une seule instruction d'ajout : le verrou du journal n'est tenu que le
/// temps de cet ajout et de la mise à jour du compteur. Renvoie les clés réellement écrites.
async fn store(
    pool: &PgPool,
    source: &Source,
    request_id: Uuid,
    parsed: &[Result<Envelope, Rejection>],
) -> Result<HashSet<String>, sqlx::Error> {
    let valid: Vec<&Envelope> = parsed.iter().flatten().collect();
    let rejected = (parsed.len() - valid.len()) as i64;
    if valid.is_empty() && rejected == 0 {
        return Ok(HashSet::new());
    }

    let mut tx = pool.begin().await?;
    let mut inserted = HashSet::new();
    if !valid.is_empty() {
        let kinds: Vec<&str> = valid.iter().map(|e| e.kind.as_str()).collect();
        let versions: Vec<i16> = valid.iter().map(|e| e.schema_version).collect();
        let keys: Vec<&str> = valid.iter().map(|e| e.key.as_str()).collect();
        let dates: Vec<DateTime<Utc>> = valid.iter().map(|e| e.occurred_at).collect();
        let offsets: Vec<Option<i16>> = valid.iter().map(|e| e.utc_offset_minutes).collect();
        let payloads: Vec<&str> = valid.iter().map(|e| e.payload.as_str()).collect();
        // ORDER BY : les numéros suivent l'ordre du lot, et le premier d'une clé répétée l'emporte.
        inserted = sqlx::query_scalar!(
            "INSERT INTO core.records (source_id, kind, schema_version, key, occurred_at,
                                       utc_offset_minutes, client_version, request_id, payload)
             SELECT $1, r.kind, r.schema_version, r.key, r.occurred_at,
                    r.utc_offset_minutes, $2, $3, r.payload::jsonb
             FROM unnest($4::text[], $5::smallint[], $6::text[], $7::timestamptz[],
                         $8::smallint[], $9::text[])
                  WITH ORDINALITY AS r(kind, schema_version, key, occurred_at,
                                       utc_offset_minutes, payload, ord)
             ORDER BY r.ord
             ON CONFLICT (source_id, key) DO NOTHING
             RETURNING key",
            source.id,
            source.client_version,
            request_id,
            &kinds as &[&str],
            &versions,
            &keys as &[&str],
            &dates,
            &offsets as _,
            &payloads as &[&str],
        )
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .collect();
    }
    if rejected > 0 {
        // Après l'ajout : toujours le verrou puis la ligne, donc pas d'interblocage entre lots.
        sqlx::query!(
            "UPDATE core.sources
             SET rejected_count = rejected_count + $2, last_rejected_at = now()
             WHERE id = $1",
            source.id,
            rejected,
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(inserted)
}

fn build_results(
    records: &[Value],
    parsed: Vec<Result<Envelope, Rejection>>,
    mut inserted: HashSet<String>,
) -> Vec<RecordResult> {
    let (mut accepted, mut duplicate, mut rejected) = (0u32, 0u32, 0u32);
    let results = records
        .iter()
        .zip(parsed)
        .enumerate()
        .map(|(index, (record, entry))| match entry {
            Ok(envelope) => {
                // remove : dans un même lot, seule la première occurrence d'une clé est « accepted ».
                let status = if inserted.remove(&envelope.key) {
                    accepted += 1;
                    Status::Accepted
                } else {
                    duplicate += 1;
                    Status::Duplicate
                };
                RecordResult {
                    key: Some(envelope.key),
                    status,
                    reason: None,
                    field: None,
                }
            }
            Err(rejection) => {
                rejected += 1;
                // La position dans le lot, jamais la clé ni la valeur fautive.
                warn!(index, reason = ?rejection.reason, field = rejection.field, "record rejected");
                RecordResult {
                    key: record.get("key").and_then(Value::as_str).map(str::to_owned),
                    status: Status::Rejected,
                    reason: Some(rejection.reason),
                    field: Some(rejection.field),
                }
            }
        })
        .collect();
    info!(accepted, duplicate, rejected, "batch processed");
    results
}

async fn post_heartbeat(
    Extension(RequestId(request_id)): Extension<RequestId>,
    _source: Source,
) -> Json<Value> {
    Json(json!({ "request_id": request_id }))
}

async fn health(State(state): State<AppState>) -> StatusCode {
    match sqlx::query_scalar!("SELECT 1 AS one")
        .fetch_one(&state.pool)
        .await
    {
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}

#[derive(Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
}

/// Le mot de passe est toujours vérifié contre le compte, quel que soit l'identifiant donné :
/// le temps de réponse ne dit pas si l'identifiant est bon.
async fn login(
    State(state): State<AppState>,
    body: Result<Bytes, BytesRejection>,
) -> Result<Response, ApiError> {
    let body = body.map_err(|_| ApiError::InvalidJson)?;
    let credentials: LoginRequest =
        serde_json::from_slice(&body).map_err(|_| ApiError::InvalidJson)?;
    let account = sqlx::query!("SELECT username, password_hash FROM core.account")
        .fetch_optional(&state.pool)
        .await?
        .ok_or(ApiError::InvalidCredentials)?;
    let _permit = state
        .logins
        .acquire()
        .await
        .map_err(|_| ApiError::Internal)?;
    let password = credentials.password;
    let stored_hash = account.password_hash;
    let password_ok =
        tokio::task::spawn_blocking(move || auth::verify_password(&password, &stored_hash))
            .await
            .map_err(|_| ApiError::Internal)?;
    if !password_ok || credentials.username != account.username {
        warn!("login refused");
        return Err(ApiError::InvalidCredentials);
    }
    let token = generate_token().map_err(|_| ApiError::Internal)?;
    auth::create_session(&state.pool, &token).await?;
    let cookie = auth::session_cookie(&token, auth::SESSION_SECONDS, state.config.insecure_cookie);
    info!("login succeeded");
    Ok(with_cookie(cookie))
}

async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, ApiError> {
    if let Some(token) = auth::session_token(&headers) {
        auth::delete_session(&state.pool, token).await?;
    }
    Ok(with_cookie(auth::session_cookie(
        "",
        0,
        state.config.insecure_cookie,
    )))
}

fn with_cookie(cookie: String) -> Response {
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(
        SET_COOKIE,
        HeaderValue::from_str(&cookie).expect("un token hexadécimal est un en-tête valide"),
    );
    response
}

#[derive(Serialize)]
struct Cup {
    /// Heure de Paris, `HH:MM`.
    time: String,
    occurred_at: String,
    /// Source et clé : ce qu'il faut pour annuler ce café, quelle que soit sa source.
    source: String,
    key: String,
}

#[derive(Serialize)]
struct DayResponse {
    date: NaiveDate,
    count: usize,
    cups: Vec<Cup>,
}

/// `AAAA-MM-JJ` strict : chrono seul accepterait aussi `2026-1-5`.
fn parse_day(value: &str) -> Option<NaiveDate> {
    (value.len() == 10)
        .then(|| NaiveDate::parse_from_str(value, "%Y-%m-%d").ok())
        .flatten()
}

/// La vue choisit les événements ; le journal et le registre ne donnent que leur clé et leur source.
/// Le fuseau en sous-requête et non en jointure : `core.settings`, jamais analysée, passe pour 1 250
/// lignes, et le plan parcourrait alors tout le journal à chaque lecture (voir l'architecture).
/// Le numéro de génération, redondant avec la vue, est ce qui ouvre l'index : sans lui, toute lecture
/// d'un jour parcourt tous les événements (6 s pour 300 écoutes sur 60 000, voir l'architecture).
/// Le jour demandé devient un intervalle UTC : minuit à minuit, 23 ou 25 heures au changement d'heure.
async fn get_day(
    State(state): State<AppState>,
    Path(date): Path<String>,
) -> Result<Json<DayResponse>, ApiError> {
    let date = parse_day(&date).ok_or(ApiError::InvalidDate)?;
    let cups = sqlx::query!(
        r#"SELECT to_char(e.occurred_at AT TIME ZONE (SELECT timezone FROM core.settings), 'HH24:MI') AS "time!",
                  e.occurred_at AS "occurred_at!",
                  src.name AS source,
                  r.key
           FROM derived.live_events e
           JOIN core.records r ON r.seq = e.seq
           JOIN core.sources src ON src.id = e.source_id
           WHERE e.generation = (SELECT id FROM derived.generations WHERE state = 'active')
             AND e.metric = 'coffee'
             AND e.occurred_at >= ($1::date::timestamp AT TIME ZONE (SELECT timezone FROM core.settings))
             AND e.occurred_at < (($1::date + 1)::timestamp AT TIME ZONE (SELECT timezone FROM core.settings))
           ORDER BY e.occurred_at, e.seq"#,
        date,
    )
    .fetch_all(&state.pool)
    .await?;
    let cups: Vec<Cup> = cups
        .into_iter()
        .map(|cup| Cup {
            time: cup.time,
            occurred_at: cup.occurred_at.to_rfc3339(),
            source: cup.source,
            key: cup.key,
        })
        .collect();
    Ok(Json(DayResponse {
        date,
        count: cups.len(),
        cups,
    }))
}

#[derive(Serialize)]
struct MusicTrack {
    name: String,
    /// `null` : l'export Apple ne permet pas de dire de qui est ce titre, et rien n'est inventé.
    artist: Option<String>,
    listens: i64,
}

#[derive(Serialize)]
struct MusicArtist {
    name: String,
    /// Titres différents écoutés ce jour-là.
    tracks: usize,
    listens: i64,
}

#[derive(Serialize)]
struct MusicAlbum {
    name: String,
    artist: String,
    listens: i64,
}

#[derive(Serialize)]
struct ArtistOfDay {
    name: String,
    listens: i64,
}

/// Ce qu'il faut pour annuler un marquage, quelle que soit sa source.
#[derive(Serialize)]
struct KnownMark {
    source: String,
    key: String,
}

/// Un élément écouté pour la première fois ce jour-là. C'est une découverte tant que `known_marks` est
/// vide. Marqué, il reste dans la liste pour que le marquage puisse s'annuler depuis la même ligne.
#[derive(Serialize)]
struct FirstListen {
    level: String,
    item_key: String,
    name: String,
    artist: String,
    /// Plusieurs si plusieurs appareils ont marqué : les annuler tous rétablit la découverte.
    known_marks: Vec<KnownMark>,
}

#[derive(Serialize)]
struct DayMusic {
    date: NaiveDate,
    listens: usize,
    /// Minutes estimées, sur les écoutes dont la durée est connue. `null` si aucune ne l'est.
    minutes: Option<i64>,
    /// Écoutes sans durée connue : le temps affiché les ignore, et cela doit se voir.
    listens_without_duration: usize,
    /// Écoutes sans artiste attribuable : absentes des artistes, des albums et des découvertes.
    listens_without_artist: usize,
    artist_of_day: Option<ArtistOfDay>,
    tracks: Vec<MusicTrack>,
    artists: Vec<MusicArtist>,
    albums: Vec<MusicAlbum>,
    /// Nombre d'éléments de `first_listens` qui ne sont pas marqués.
    discoveries: usize,
    first_listens: Vec<FirstListen>,
}

/// Une découverte dépend de tout l'historique : elle se calcule à la lecture, jamais dans un interprète
/// (principe 8). Pour chaque élément écouté ce jour-là, à son niveau : aucune écoute avant minuit, heure
/// de Paris. `live_*` écarte d'office les écoutes annulées et les sources qui ne font pas foi, et les
/// marquages annulés. Le coût suit le jour demandé, pas l'historique : une recherche d'index par élément,
/// d'où le `LIMIT 1` latéral, qu'un `NOT EXISTS` ferait devenir un parcours de tous les éléments.
async fn first_listens(pool: &PgPool, date: NaiveDate) -> Result<Vec<FirstListen>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"WITH listened AS (
             SELECT DISTINCT ON (i.level, i.item_key)
                    e.generation, i.level, i.item_key, i.display,
                    -- L'album porte la clé de l'artiste principal : c'est lui qu'on affiche, et que
                    -- le marquage renvoie.
                    CASE WHEN i.level = 'album'
                         THEN COALESCE(e.details->'artists'->0->>'name', e.details->>'artist')
                         ELSE e.details->>'artist' END AS artist,
                    min(e.occurred_at) OVER (PARTITION BY i.level, i.item_key) AS first_at,
                    ($1::date::timestamp AT TIME ZONE (SELECT timezone FROM core.settings)) AS day_start
             FROM derived.live_events e
               JOIN derived.event_items i USING (generation, seq, ordinal)
             WHERE e.generation = (SELECT id FROM derived.generations WHERE state = 'active')
               AND e.metric = 'music.listen'
               AND e.occurred_at >= ($1::date::timestamp AT TIME ZONE (SELECT timezone FROM core.settings))
               AND e.occurred_at < (($1::date + 1)::timestamp AT TIME ZONE (SELECT timezone FROM core.settings))
             ORDER BY i.level, i.item_key, e.occurred_at DESC, e.seq DESC
           )
           SELECT l.level, l.item_key, l.display AS name, l.artist AS "artist!",
                  COALESCE(m.sources, '{}') AS "mark_sources!: Vec<String>",
                  COALESCE(m.keys, '{}') AS "mark_keys!: Vec<String>"
           FROM listened l
           LEFT JOIN LATERAL (
             SELECT array_agg(src.name ORDER BY k.seq) AS sources,
                    array_agg(r.key ORDER BY k.seq) AS keys
             FROM derived.live_event_items k
             JOIN core.records r ON r.seq = k.seq
             JOIN core.sources src ON src.id = r.source_id
             WHERE k.generation = l.generation AND k.metric = 'music.known'
               AND k.level = l.level AND k.item_key = l.item_key
           ) m ON true
           LEFT JOIN LATERAL (
             SELECT 1 AS found FROM derived.live_event_items x
             WHERE x.generation = l.generation AND x.metric = 'music.listen'
               AND x.level = l.level AND x.item_key = l.item_key
               AND x.occurred_at < l.day_start
             LIMIT 1
           ) earlier ON true
           WHERE earlier.found IS NULL
           ORDER BY array_position(ARRAY['artist', 'album', 'track'], l.level), l.first_at,
                    l.item_key"#,
        date,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| FirstListen {
            level: row.level,
            item_key: row.item_key,
            name: row.name,
            artist: row.artist,
            known_marks: row
                .mark_sources
                .into_iter()
                .zip(row.mark_keys)
                .map(|(source, key)| KnownMark { source, key })
                .collect(),
        })
        .collect())
}

/// Une ligne de liste en cours de calcul. Les écoutes arrivent de la plus ancienne à la plus récente :
/// le nom gardé est le dernier vu, car une même clé peut couvrir plusieurs orthographes.
struct Tally {
    name: String,
    artist: Option<String>,
    listens: i64,
    tracks: HashSet<String>,
}

fn tally<'a>(
    lists: &'a mut HashMap<String, Tally>,
    key: String,
    name: &str,
    artist: Option<&str>,
) -> &'a mut Tally {
    let entry = lists.entry(key).or_insert_with(|| Tally {
        name: String::new(),
        artist: None,
        listens: 0,
        tracks: HashSet::new(),
    });
    entry.name = name.to_owned();
    entry.artist = artist.map(str::to_owned);
    entry.listens += 1;
    entry
}

/// Les plus écoutés d'abord, puis par nom : le même jour donne toujours le même ordre.
fn ranked(lists: HashMap<String, Tally>) -> Vec<Tally> {
    let mut rows: Vec<Tally> = lists.into_values().collect();
    rows.sort_by(|a, b| {
        b.listens
            .cmp(&a.listens)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.artist.cmp(&b.artist))
    });
    rows
}

/// Les minutes croisent deux enregistrements, donc se calculent ici et jamais dans un interprète :
/// la durée réellement écoutée si la source la donne, sinon la dernière durée connue du titre.
async fn get_day_music(
    State(state): State<AppState>,
    Path(date): Path<String>,
) -> Result<Json<DayMusic>, ApiError> {
    let date = parse_day(&date).ok_or(ApiError::InvalidDate)?;
    let rows = sqlx::query!(
        r#"SELECT e.details->>'artist' AS artist,
                  e.details->>'artist_key' AS artist_key,
                  e.details->'artists' AS artists,
                  e.details->>'album' AS album,
                  e.details->>'album_key' AS album_key,
                  e.details->>'track' AS "track!",
                  e.details->>'track_key' AS track_key,
                  COALESCE((e.details->>'played_ms')::numeric / 1000, d.value)::float8 AS seconds
           FROM derived.live_events e
           LEFT JOIN LATERAL (
             SELECT known.value
             FROM derived.event_items i
             JOIN derived.live_events known USING (generation, seq, ordinal)
             WHERE i.generation = e.generation
               AND i.metric = 'music.track_duration' AND i.level = 'track'
               AND i.item_key = e.details->>'track_key'
             ORDER BY i.occurred_at DESC, i.seq DESC
             LIMIT 1
           ) d ON true
           WHERE e.generation = (SELECT id FROM derived.generations WHERE state = 'active')
             AND e.metric = 'music.listen'
             AND e.occurred_at >= ($1::date::timestamp AT TIME ZONE (SELECT timezone FROM core.settings))
             AND e.occurred_at < (($1::date + 1)::timestamp AT TIME ZONE (SELECT timezone FROM core.settings))
           ORDER BY e.occurred_at, e.seq"#,
        date,
    )
    .fetch_all(&state.pool)
    .await?;

    let first_listens = first_listens(&state.pool, date).await?;
    let listens = rows.len();
    let known: Vec<f64> = rows.iter().filter_map(|row| row.seconds).collect();
    let minutes = (!known.is_empty()).then(|| (known.iter().sum::<f64>() / 60.0).round() as i64);
    let listens_without_artist = rows.iter().filter(|row| row.artist.is_none()).count();
    let (mut tracks, mut artists, mut albums) = (HashMap::new(), HashMap::new(), HashMap::new());
    for row in rows {
        let artist = row.artist.as_deref();
        // Sans artiste, pas de clé : le même nom se regroupe, sans se mêler à un titre attribué.
        let track_key = row
            .track_key
            .clone()
            .unwrap_or_else(|| format!("\u{0}{}", fold(&row.track)));
        tally(&mut tracks, track_key, &row.track, artist);
        let Some(artist) = artist else { continue };
        // Un artiste par crédit, l'artiste principal d'abord ; une écoute interprétée avant les
        // artistes crédités n'a que son artiste entier.
        let credits: Vec<(String, String)> = match row.artists.as_ref().and_then(Value::as_array) {
            Some(list) => list
                .iter()
                .filter_map(|c| {
                    Some((
                        c["key"].as_str()?.to_owned(),
                        c["name"].as_str()?.to_owned(),
                    ))
                })
                .collect(),
            None => row
                .artist_key
                .clone()
                .map(|key| (key, artist.to_owned()))
                .into_iter()
                .collect(),
        };
        if let Some(track_key) = &row.track_key {
            for (key, name) in &credits {
                tally(&mut artists, key.clone(), name, Some(name))
                    .tracks
                    .insert(track_key.clone());
            }
        }
        if let (Some(key), Some(album)) = (row.album_key, row.album.as_deref()) {
            let lead = credits.first().map_or(artist, |(_, name)| name.as_str());
            tally(&mut albums, key, album, Some(lead));
        }
    }
    let artists: Vec<MusicArtist> = ranked(artists)
        .into_iter()
        .map(|t| MusicArtist {
            tracks: t.tracks.len(),
            name: t.name,
            listens: t.listens,
        })
        .collect();
    Ok(Json(DayMusic {
        date,
        listens,
        minutes,
        listens_without_duration: listens - known.len(),
        listens_without_artist,
        artist_of_day: artists.first().map(|a| ArtistOfDay {
            name: a.name.clone(),
            listens: a.listens,
        }),
        tracks: ranked(tracks)
            .into_iter()
            .map(|t| MusicTrack {
                name: t.name,
                artist: t.artist,
                listens: t.listens,
            })
            .collect(),
        artists,
        albums: ranked(albums)
            .into_iter()
            .map(|t| MusicAlbum {
                name: t.name,
                artist: t.artist.unwrap_or_default(),
                listens: t.listens,
            })
            .collect(),
        discoveries: first_listens
            .iter()
            .filter(|f| f.known_marks.is_empty())
            .count(),
        first_listens,
    }))
}

/// Le mois, jour par jour : valeurs, erreurs, journées en attente ou à vérifier (voir `calendar`).
/// Pas, calories, sommeil et séances d'un jour, avec leurs détails (voir `activity`).
async fn get_day_activity(
    State(state): State<AppState>,
    Path(date): Path<String>,
) -> Result<Json<activity::DayActivity>, ApiError> {
    let date = parse_day(&date).ok_or(ApiError::InvalidDate)?;
    Ok(Json(activity::day(&state.pool, date).await?))
}

/// La trace simplifiée d'une séance, pour sa carte.
async fn get_activity_track(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<activity::Track>, ApiError> {
    let id: u64 = id.parse().map_err(|_| ApiError::InvalidParameter("id"))?;
    activity::track(&state.pool, id)
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

async fn get_month(
    State(state): State<AppState>,
    Path(month): Path<String>,
) -> Result<Json<calendar::Month>, ApiError> {
    let start = calendar::parse_month(&month).ok_or(ApiError::InvalidDate)?;
    Ok(Json(calendar::month(&state.pool, start).await?))
}

/// L'année, jour par jour, de la même forme que le mois. Une année future est refusée.
async fn get_year(
    State(state): State<AppState>,
    Path(year): Path<String>,
) -> Result<Json<calendar::Year>, ApiError> {
    let start = calendar::parse_year(&year).ok_or(ApiError::InvalidDate)?;
    calendar::year(&state.pool, start)
        .await?
        .map(Json)
        .ok_or(ApiError::InvalidDate)
}

#[derive(Serialize)]
struct SourceHealth {
    name: String,
    last_seen_at: Option<DateTime<Utc>>,
    silence_threshold_seconds: f64,
    silent: bool,
}

#[derive(Serialize)]
struct MetricHealth {
    metric: String,
    first_event_at: Option<DateTime<Utc>>,
    last_event_at: Option<DateTime<Utc>>,
    freshness_threshold_seconds: f64,
    /// Aucune nouvelle donnée depuis le seuil : à vérifier, pas une erreur.
    stale: bool,
    /// Ce que chaque source couvre : un jour entre deux couvertures n'a pas de données, ce n'est pas
    /// un jour sans activité.
    coverage: Vec<SourceCoverage>,
}

#[derive(Serialize)]
struct SourceCoverage {
    source: String,
    first_at: DateTime<Utc>,
    last_at: DateTime<Utc>,
}

#[derive(Serialize)]
struct SourcesHealth {
    sources: Vec<SourceHealth>,
    metrics: Vec<MetricHealth>,
    /// Le compteur global : enregistrements en erreur d'interprétation dans la génération active.
    errors: i64,
}

/// Générique : tout vient des seuils des registres. Une source est jugée sur son dernier contact
/// réussi, ou sur sa création si elle n'en a jamais eu. Une métrique sans aucune donnée n'est pas
/// « à vérifier » : elle n'a jamais commencé, et l'interface le dit autrement.
async fn get_sources_health(
    State(state): State<AppState>,
) -> Result<Json<SourcesHealth>, ApiError> {
    let sources = sqlx::query!(
        r#"SELECT name, last_seen_at,
                  extract(epoch FROM silence_threshold)::float8 AS "threshold!",
                  now() - COALESCE(last_seen_at, created_at) > silence_threshold AS "silent!"
           FROM core.sources
           WHERE silence_threshold IS NOT NULL AND revoked_at IS NULL
           ORDER BY name"#,
    )
    .fetch_all(&state.pool)
    .await?;
    let metrics = sqlx::query!(
        r#"SELECT m.metric, seen.first_at, seen.last_at,
                  extract(epoch FROM m.freshness_threshold)::float8 AS "threshold!",
                  COALESCE(now() - seen.last_at > m.freshness_threshold, false) AS "stale!"
           FROM core.metrics m
           CROSS JOIN LATERAL (
             SELECT min(e.occurred_at) AS first_at, max(e.occurred_at) AS last_at
             FROM derived.live_events e
             WHERE e.generation = (SELECT id FROM derived.generations WHERE state = 'active')
               AND e.metric = m.metric
           ) seen
           WHERE m.freshness_threshold IS NOT NULL
           ORDER BY m.metric"#,
    )
    .fetch_all(&state.pool)
    .await?;
    // La vue, donc ce que la lecture montre vraiment : une source qui ne fait plus foi s'arrête là où
    // la suivante commence, sans faux trou.
    let coverage = sqlx::query!(
        r#"SELECT e.metric AS "metric!", src.name AS source,
                  min(e.occurred_at) AS "first_at!", max(e.occurred_at) AS "last_at!"
           FROM derived.live_events e
           JOIN core.sources src ON src.id = e.source_id
           JOIN core.metrics m ON m.metric = e.metric
           WHERE e.generation = (SELECT id FROM derived.generations WHERE state = 'active')
             AND m.freshness_threshold IS NOT NULL
           GROUP BY e.metric, src.name
           ORDER BY 3"#,
    )
    .fetch_all(&state.pool)
    .await?;
    let errors = sqlx::query_scalar!(
        r#"SELECT count(*) AS "errors!" FROM derived.interpretation_errors
           WHERE generation = (SELECT id FROM derived.generations WHERE state = 'active')"#,
    )
    .fetch_one(&state.pool)
    .await?;
    Ok(Json(SourcesHealth {
        errors,
        sources: sources
            .into_iter()
            .map(|s| SourceHealth {
                name: s.name,
                last_seen_at: s.last_seen_at,
                silence_threshold_seconds: s.threshold,
                silent: s.silent,
            })
            .collect(),
        metrics: metrics
            .into_iter()
            .map(|m| MetricHealth {
                first_event_at: m.first_at,
                last_event_at: m.last_at,
                freshness_threshold_seconds: m.threshold,
                stale: m.stale,
                coverage: coverage
                    .iter()
                    .filter(|c| c.metric == m.metric)
                    .map(|c| SourceCoverage {
                        source: c.source.clone(),
                        first_at: c.first_at,
                        last_at: c.last_at,
                    })
                    .collect(),
                metric: m.metric,
            })
            .collect(),
    }))
}

// ─── Lectures génériques (outils du MCP, voir `read`) ────────────────────────

/// Les paramètres des lectures génériques, tous facultatifs ici : chaque route dit ce qu'elle exige,
/// et une erreur ne nomme que le paramètre, jamais sa valeur.
#[derive(Deserialize)]
struct ReadParams {
    metric: Option<String>,
    level: Option<String>,
    key: Option<String>,
    from: Option<String>,
    to: Option<String>,
    form: Option<String>,
    group: Option<String>,
    limit: Option<String>,
    q: Option<String>,
}

impl ReadParams {
    fn period(&self) -> Result<Period, ApiError> {
        Ok(Period::parse(self.from.as_deref(), self.to.as_deref())?)
    }

    async fn metric(&self, pool: &PgPool) -> Result<read::Metric, ApiError> {
        let name = self
            .metric
            .as_deref()
            .ok_or(ApiError::InvalidParameter("metric"))?;
        Ok(read::metric(pool, name).await?)
    }

    fn required(value: &Option<String>, field: &'static str) -> Result<String, ApiError> {
        value
            .as_deref()
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
            .ok_or(ApiError::InvalidParameter(field))
    }
}

/// Une requête mal formée (paramètre répété, encodage) est un paramètre invalide, sans plus de détail.
type ReadQuery = Result<Query<ReadParams>, axum::extract::rejection::QueryRejection>;

fn params(query: ReadQuery) -> Result<ReadParams, ApiError> {
    query
        .map(|Query(p)| p)
        .map_err(|_| ApiError::InvalidParameter("query"))
}

async fn get_inventory(State(state): State<AppState>) -> Result<Json<read::Inventory>, ApiError> {
    Ok(Json(read::inventory(&state.pool).await?))
}

async fn get_search(
    State(state): State<AppState>,
    query: ReadQuery,
) -> Result<Json<Vec<read::Found>>, ApiError> {
    let p = params(query)?;
    let q = ReadParams::required(&p.q, "q")?;
    if let Some(metric) = p.metric.as_deref() {
        let metric = read::metric(&state.pool, metric).await?;
        if let Some(level) = p.level.as_deref() {
            metric.check_level(level)?;
        }
    }
    let limit = read::parse_limit(p.limit.as_deref(), 10, read::MAX_SEARCH, "limit")?;
    Ok(Json(
        read::search(
            &state.pool,
            &q,
            p.metric.as_deref(),
            p.level.as_deref(),
            limit,
        )
        .await?,
    ))
}

async fn get_occurrences(
    State(state): State<AppState>,
    query: ReadQuery,
) -> Result<Json<read::Occurrences>, ApiError> {
    let p = params(query)?;
    let metric = p.metric(&state.pool).await?;
    let period = p.period()?;
    let (level, key) = read::item_filter(&metric, p.level.as_deref(), p.key.as_deref())?;
    let form = p.form.as_deref().unwrap_or("list");
    let limit = read::parse_limit(p.limit.as_deref(), 100, read::MAX_LIST, "limit")?;
    Ok(Json(
        read::occurrences(&state.pool, &metric, period, level, key, form, limit).await?,
    ))
}

async fn get_durations(
    State(state): State<AppState>,
    query: ReadQuery,
) -> Result<Json<read::Durations>, ApiError> {
    let p = params(query)?;
    let metric = p.metric(&state.pool).await?;
    let period = p.period()?;
    let (level, key) = read::item_filter(&metric, p.level.as_deref(), p.key.as_deref())?;
    let group = p.group.as_deref().unwrap_or("day");
    Ok(Json(
        read::durations(&state.pool, &metric, period, level, key, group).await?,
    ))
}

async fn get_item(
    State(state): State<AppState>,
    query: ReadQuery,
) -> Result<Json<read::Item>, ApiError> {
    let p = params(query)?;
    let metric = p.metric(&state.pool).await?;
    let level = ReadParams::required(&p.level, "level")?;
    let key = ReadParams::required(&p.key, "key")?;
    Ok(Json(read::item(&state.pool, &metric, &level, &key).await?))
}

async fn get_top(
    State(state): State<AppState>,
    query: ReadQuery,
) -> Result<Json<read::Top>, ApiError> {
    let p = params(query)?;
    let metric = p.metric(&state.pool).await?;
    let level = ReadParams::required(&p.level, "level")?;
    let period = p.period()?;
    let limit = read::parse_limit(p.limit.as_deref(), 20, read::MAX_TOP, "limit")?;
    Ok(Json(
        read::top(&state.pool, &metric, &level, period, limit).await?,
    ))
}

async fn get_discoveries(
    State(state): State<AppState>,
    query: ReadQuery,
) -> Result<Json<read::Discoveries>, ApiError> {
    let p = params(query)?;
    let metric = p.metric(&state.pool).await?;
    let level = ReadParams::required(&p.level, "level")?;
    let period = p.period()?;
    let limit = read::parse_limit(p.limit.as_deref(), 100, read::MAX_LIST, "limit")?;
    Ok(Json(
        read::discoveries(&state.pool, &metric, &level, period, limit).await?,
    ))
}
