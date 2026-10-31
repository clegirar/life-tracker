//! API d'ingestion v1 : contrat dans contract/ingestion-v1.schema.json.

use std::collections::HashSet;
use std::time::Instant;

use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::{DefaultBodyLimit, FromRequestParts, MatchedPath, Request, State};
use axum::http::header::{AUTHORIZATION, WWW_AUTHENTICATE};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{Value, json};
use sqlx::PgPool;
use tracing::{Instrument, error, info, info_span, warn};
use uuid::Uuid;

use crate::admin::token_hash;
use crate::ingest::{Envelope, RejectReason, Rejection, parse_envelope};

pub const MAX_RECORDS: usize = 500;
pub const MAX_BODY_BYTES: usize = 1_000_000;

#[derive(Clone)]
struct AppState {
    pool: PgPool,
}

#[derive(Clone, Copy)]
struct RequestId(Uuid);

pub fn router(pool: PgPool) -> Router {
    Router::new()
        .route(
            "/v1/records",
            post(post_records).layer(DefaultBodyLimit::max(MAX_BODY_BYTES)),
        )
        .route("/v1/heartbeat", post(post_heartbeat))
        .route("/health", get(health))
        .layer(middleware::from_fn(request_context))
        .with_state(AppState { pool })
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
    InvalidJson,
    BatchTooLarge,
    Internal,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code) = match self {
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            Self::InvalidJson => (StatusCode::BAD_REQUEST, "invalid_json"),
            Self::BatchTooLarge => (StatusCode::PAYLOAD_TOO_LARGE, "batch_too_large"),
            Self::Internal => (StatusCode::INTERNAL_SERVER_ERROR, "internal"),
        };
        let mut response = (status, Json(json!({ "error": code }))).into_response();
        if status == StatusCode::UNAUTHORIZED {
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

/// Source authentifiée par son token. Tout appel authentifié met à jour son dernier contact.
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
        let token = bearer_token(&parts.headers).ok_or(ApiError::Unauthorized)?;
        let hash = token_hash(token);
        let client_version = parts
            .headers
            .get("x-client-version")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        // auth = 'token' : la source webapp passe par la session et n'a pas de token.
        let id = sqlx::query_scalar!(
            "UPDATE core.sources
             SET last_seen_at = now(), client_version = COALESCE($2, client_version)
             WHERE token_hash = $1 AND auth = 'token' AND revoked_at IS NULL
             RETURNING id",
            hash,
            client_version,
        )
        .fetch_optional(&state.pool)
        .await?
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
