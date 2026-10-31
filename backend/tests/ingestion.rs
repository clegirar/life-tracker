//! API d'ingestion contre un vrai PostgreSQL, en tracker_app.

mod common;
use chrono::{DateTime, Duration, Utc};
use common::*;
use serde_json::{Value, json};
use sqlx::Row;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const ALL: &[&str] = &["coffee.cup", "lastfm.scrobble"];
const COFFEE: &[&str] = &["coffee.cup"];

fn assert_rejected(result: &Value, reason: &str, field: &str) {
    assert_eq!(result["status"], "rejected", "{result}");
    assert_eq!(result["reason"], reason, "{result}");
    assert_eq!(result["field"], field, "{result}");
}

#[tokio::test]
async fn tests_run_as_tracker_app_not_superuser() {
    let ctx = Ctx::new(COFFEE).await;
    let row = sqlx::query(
        "SELECT current_user::text AS who, rolsuper FROM pg_roles WHERE rolname = current_user",
    )
    .fetch_one(&ctx.app)
    .await
    .unwrap();
    assert_eq!(row.get::<String, _>("who"), "tracker_app");
    assert!(!row.get::<bool, _>("rolsuper"));
}

#[tokio::test]
async fn accepted_writes_the_full_envelope() {
    let ctx = Ctx::new(COFFEE).await;
    let response = ctx
        .post("/v1/records")
        .header("X-Client-Version", "ios-1.2.3")
        .json(&json!({ "records": [{
            "key": "cup-1", "kind": "coffee.cup", "schema_version": 3,
            "occurred_at": "2026-10-13T09:42:10+02:00", "payload": {"note": "é"},
        }]}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let header_id = response.headers()["x-request-id"]
        .to_str()
        .unwrap()
        .to_owned();
    let body: Value = response.json().await.unwrap();
    assert!(
        validator(Some("records_response")).is_valid(&body),
        "{body}"
    );
    assert_eq!(body["request_id"], header_id.as_str());
    assert_eq!(
        body["results"],
        json!([{"key": "cup-1", "status": "accepted"}])
    );

    let row = sqlx::query(
        "SELECT kind, schema_version, occurred_at, utc_offset_minutes, client_version,
                request_id::text AS request_id, payload, received_at
         FROM core.records WHERE source_id = $1 AND key = 'cup-1'",
    )
    .bind(ctx.source_id)
    .fetch_one(&ctx.app)
    .await
    .unwrap();
    assert_eq!(row.get::<String, _>("kind"), "coffee.cup");
    assert_eq!(row.get::<i16, _>("schema_version"), 3);
    assert_eq!(
        row.get::<DateTime<Utc>, _>("occurred_at").to_rfc3339(),
        "2026-10-13T07:42:10+00:00"
    );
    assert_eq!(row.get::<Option<i16>, _>("utc_offset_minutes"), Some(120));
    assert_eq!(row.get::<String, _>("client_version"), "ios-1.2.3");
    assert_eq!(row.get::<String, _>("request_id"), header_id);
    assert!(row.get::<DateTime<Utc>, _>("received_at") > Utc::now() - Duration::minutes(5));
    let payload: String = sqlx::query_scalar(
        "SELECT payload::text FROM core.records WHERE source_id = $1 AND key = 'cup-1'",
    )
    .bind(ctx.source_id)
    .fetch_one(&ctx.app)
    .await
    .unwrap();
    assert_eq!(payload, r#"{"note": "é"}"#);
}

#[tokio::test]
async fn utc_offsets_are_kept_or_unknown() {
    let ctx = Ctx::new(COFFEE).await;
    let cases: &[(&str, &str, Option<i16>)] = &[
        ("z", "2026-10-13T09:42:10Z", None),
        ("minus-zero", "2026-10-13T09:42:10-00:00", None),
        ("plus-zero", "2026-10-13T09:42:10+00:00", Some(0)),
        ("paris", "2026-10-13T11:42:10+02:00", Some(120)),
        ("negative", "2026-10-13T00:12:10-09:30", Some(-570)),
    ];
    let records: Vec<Value> = cases
        .iter()
        .map(|(key, at, _)| {
            let mut r = record(key);
            r["occurred_at"] = json!(at);
            r
        })
        .collect();
    let results = ctx.results(json!(records)).await;
    assert!(statuses(&results).iter().all(|s| *s == "accepted"));
    for (key, _, expected) in cases {
        let row = sqlx::query(
            "SELECT occurred_at, utc_offset_minutes FROM core.records
             WHERE source_id = $1 AND key = $2",
        )
        .bind(ctx.source_id)
        .bind(key)
        .fetch_one(&ctx.app)
        .await
        .unwrap();
        assert_eq!(
            row.get::<Option<i16>, _>("utc_offset_minutes"),
            *expected,
            "{key}"
        );
        assert_eq!(
            row.get::<DateTime<Utc>, _>("occurred_at").to_rfc3339(),
            "2026-10-13T09:42:10+00:00",
            "{key}"
        );
    }
}

#[tokio::test]
async fn duplicate_is_a_success_and_writes_nothing() {
    let ctx = Ctx::new(COFFEE).await;
    assert_eq!(
        statuses(&ctx.results(json!([record("a")])).await),
        ["accepted"]
    );
    assert_eq!(
        statuses(&ctx.results(json!([record("a")])).await),
        ["duplicate"]
    );
    assert_eq!(ctx.journal_count().await, 1);
}

#[tokio::test]
async fn duplicate_inside_one_batch_keeps_the_first() {
    let ctx = Ctx::new(COFFEE).await;
    let mut first = record("same");
    first["payload"] = json!({"n": 1});
    let mut second = record("same");
    second["payload"] = json!({"n": 2});
    let results = ctx.results(json!([first, record("other"), second])).await;
    assert_eq!(statuses(&results), ["accepted", "accepted", "duplicate"]);
    let n: i64 = sqlx::query_scalar(
        "SELECT (payload->>'n')::bigint FROM core.records WHERE source_id = $1 AND key = 'same'",
    )
    .bind(ctx.source_id)
    .fetch_one(&ctx.app)
    .await
    .unwrap();
    assert_eq!(n, 1);
}

#[tokio::test]
async fn resent_batch_returns_only_duplicates() {
    let ctx = Ctx::new(COFFEE).await;
    let batch: Vec<Value> = (0..50).map(|i| record(&format!("k{i}"))).collect();
    let first = ctx.results(json!(batch)).await;
    assert!(statuses(&first).iter().all(|s| *s == "accepted"));
    let again = ctx.results(json!(batch)).await;
    assert_eq!(again.len(), 50);
    assert!(statuses(&again).iter().all(|s| *s == "duplicate"));
    assert_eq!(ctx.journal_count().await, 50);
}

/// Lots concurrents sur les mêmes clés, avec des rejets : ni erreur, ni interblocage, ni doublon.
#[tokio::test]
async fn concurrent_batches_write_each_key_once() {
    let ctx = std::sync::Arc::new(Ctx::new(COFFEE).await);
    let mut tasks = Vec::new();
    for _ in 0..16 {
        let ctx = ctx.clone();
        tasks.push(tokio::spawn(async move {
            let mut batch: Vec<Value> = (0..20).map(|i| record(&format!("k{i}"))).collect();
            batch.push(json!(42));
            ctx.results(json!(batch)).await
        }));
    }
    let mut accepted = 0;
    for task in tasks {
        let results = task.await.unwrap();
        accepted += statuses(&results)
            .iter()
            .filter(|s| **s == "accepted")
            .count();
    }
    assert_eq!(accepted, 20);
    assert_eq!(ctx.journal_count().await, 20);
    assert_rejected_count(&ctx, 16).await;
}

#[tokio::test]
async fn results_and_sequence_follow_batch_order() {
    let ctx = Ctx::new(COFFEE).await;
    let batch: Vec<Value> = ["c", "a", "d", "b"].iter().map(|k| record(k)).collect();
    let results = ctx.results(json!(batch)).await;
    let keys: Vec<_> = results.iter().map(|r| r["key"].as_str().unwrap()).collect();
    assert_eq!(keys, ["c", "a", "d", "b"]);
    let stored: Vec<String> =
        sqlx::query_scalar("SELECT key FROM core.records WHERE source_id = $1 ORDER BY seq")
            .bind(ctx.source_id)
            .fetch_all(&ctx.app)
            .await
            .unwrap();
    assert_eq!(stored, ["c", "a", "d", "b"]);
}

#[tokio::test]
async fn kind_never_declared_is_rejected() {
    let ctx = Ctx::new(COFFEE).await;
    let mut other = record("x");
    other["kind"] = json!("lastfm.scrobble");
    let mut unknown = record("y");
    unknown["kind"] = json!("does.not.exist");
    let results = ctx.results(json!([other, unknown])).await;
    assert_rejected(&results[0], "kind_not_declared", "kind");
    assert_rejected(&results[1], "kind_not_declared", "kind");
    assert_eq!(ctx.journal_count().await, 0);
}

#[tokio::test]
async fn revoked_grant_is_rejected_and_can_be_restored() {
    let ctx = Ctx::new(COFFEE).await;
    assert_eq!(
        statuses(&ctx.results(json!([record("before")])).await),
        ["accepted"]
    );
    life_tracker::admin::revoke_kind(&ctx.admin, &ctx.source, "coffee.cup")
        .await
        .unwrap();
    let results = ctx.results(json!([record("during")])).await;
    assert_rejected(&results[0], "kind_not_declared", "kind");
    life_tracker::admin::grant_kind(&ctx.admin, &ctx.source, "coffee.cup")
        .await
        .unwrap();
    assert_eq!(
        statuses(&ctx.results(json!([record("after")])).await),
        ["accepted"]
    );
    assert_eq!(ctx.journal_count().await, 2);
}

#[tokio::test]
async fn missing_fields_are_rejected_one_by_one() {
    let ctx = Ctx::new(COFFEE).await;
    let fields = ["key", "kind", "schema_version", "occurred_at", "payload"];
    let batch: Vec<Value> = fields
        .iter()
        .map(|field| {
            let mut r = record(&format!("missing-{field}"));
            r.as_object_mut().unwrap().remove(*field);
            r
        })
        .collect();
    let results = ctx.results(json!(batch)).await;
    for (result, field) in results.iter().zip(fields) {
        assert_rejected(result, "missing_field", field);
    }
    assert_eq!(ctx.journal_count().await, 0);
}

#[tokio::test]
async fn invalid_fields_are_rejected_without_failing_the_batch() {
    let ctx = Ctx::new(COFFEE).await;
    let cases: Vec<(&str, Value)> = vec![
        ("key", json!(7)),
        ("key", json!("")),
        ("key", json!("x".repeat(513))),
        ("key", json!("nul\u{0}")),
        ("kind", json!(7)),
        ("schema_version", json!("1")),
        ("schema_version", json!(0)),
        ("schema_version", json!(-1)),
        ("schema_version", json!(1.5)),
        ("schema_version", json!(32768)),
        ("occurred_at", json!(1760000000)),
        ("occurred_at", json!("2026-10-13T09:42:10")),
        ("occurred_at", json!("2026-10-13")),
        ("occurred_at", json!("2026-10-13 09:42:10Z")),
        ("occurred_at", json!("2026-13-45T09:42:10Z")),
        // Hors des bornes de core.records.utc_offset_minutes.
        ("occurred_at", json!("2026-10-13T09:42:10+16:00")),
    ];
    let mut batch: Vec<Value> = cases
        .iter()
        .enumerate()
        .map(|(i, (field, value))| {
            let mut r = record(&format!("bad-{i}"));
            r[*field] = value.clone();
            r
        })
        .collect();
    batch.push(json!(42));
    batch.push(record("good"));
    let results = ctx.results(json!(batch)).await;
    for (i, (field, _)) in cases.iter().enumerate() {
        assert_rejected(&results[i], "invalid_field", field);
    }
    assert_rejected(&results[cases.len()], "invalid_field", "record");
    assert_eq!(results.last().unwrap()["status"], "accepted");
    assert_eq!(ctx.journal_count().await, 1);
    let body = json!({"request_id": "018f0000-0000-7000-8000-000000000000", "results": results});
    assert!(
        validator(Some("records_response")).is_valid(&body),
        "{body}"
    );
}

#[tokio::test]
async fn key_of_512_characters_is_accepted() {
    let ctx = Ctx::new(COFFEE).await;
    // 512 caractères de 4 octets : la limite compte des caractères, et l'index doit suivre.
    let results = ctx.results(json!([record(&"🎵".repeat(512))])).await;
    assert_eq!(statuses(&results), ["accepted"]);
}

#[tokio::test]
async fn occurred_at_more_than_a_day_ahead_is_rejected() {
    let ctx = Ctx::new(COFFEE).await;
    let at = |d: Duration| (Utc::now() + d).to_rfc3339();
    let mut near = record("near");
    near["occurred_at"] = json!(at(Duration::hours(23)));
    let mut far = record("far");
    far["occurred_at"] = json!(at(Duration::hours(25)));
    let results = ctx.results(json!([near, far])).await;
    assert_eq!(results[0]["status"], "accepted");
    assert_rejected(&results[1], "occurred_at_too_far_in_future", "occurred_at");
}

#[tokio::test]
async fn payload_is_not_validated_and_stays_exact() {
    let ctx = Ctx::new(COFFEE).await;
    let cases = [
        ("null", "null", "null"),
        ("string", r#""text""#, r#""text""#),
        ("array", "[1, 2]", "[1, 2]"),
        (
            "big-int",
            "12345678901234567890123",
            "12345678901234567890123",
        ),
        (
            "exact-decimal",
            "0.1000000000000000000001",
            "0.1000000000000000000001",
        ),
    ];
    let items: Vec<String> = cases
        .iter()
        .map(|(key, raw, _)| {
            format!(
                r#"{{"key":"{key}","kind":"coffee.cup","schema_version":1,"occurred_at":"2026-10-13T09:42:10Z","payload":{raw}}}"#
            )
        })
        .collect();
    let response = ctx
        .post("/v1/records")
        .body(format!(r#"{{"records":[{}]}}"#, items.join(",")))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.unwrap();
    assert!(
        body["results"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["status"] == "accepted"),
        "{body}"
    );
    for (key, _, stored) in cases {
        let text: String = sqlx::query_scalar(
            "SELECT payload::text FROM core.records WHERE source_id = $1 AND key = $2",
        )
        .bind(ctx.source_id)
        .bind(key)
        .fetch_one(&ctx.app)
        .await
        .unwrap();
        assert_eq!(text, stored, "{key}");
    }
}

/// jsonb ne peut pas les stocker : sans rejet individuel, le lot échouerait à chaque nouvel essai.
#[tokio::test]
async fn unstorable_payload_fails_alone() {
    let ctx = Ctx::new(COFFEE).await;
    let body = r#"{"records":[
      {"key":"nul","kind":"coffee.cup","schema_version":1,"occurred_at":"2026-10-13T09:42:10Z","payload":{"a":"x\u0000y"}},
      {"key":"huge","kind":"coffee.cup","schema_version":1,"occurred_at":"2026-10-13T09:42:10Z","payload":1e999999},
      {"key":"good","kind":"coffee.cup","schema_version":1,"occurred_at":"2026-10-13T09:42:10Z","payload":{}}
    ]}"#;
    let response = ctx.post("/v1/records").body(body).send().await.unwrap();
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.unwrap();
    let results = body["results"].as_array().unwrap();
    assert_rejected(&results[0], "invalid_field", "payload");
    assert_rejected(&results[1], "invalid_field", "payload");
    assert_eq!(results[2]["status"], "accepted");
    assert_eq!(ctx.journal_count().await, 1);
}

#[tokio::test]
async fn source_in_body_is_ignored() {
    let ctx = Ctx::new(COFFEE).await;
    let mut r = record("spoof");
    r["source"] = json!("webapp");
    r["source_id"] = json!(1);
    assert_eq!(statuses(&ctx.results(json!([r])).await), ["accepted"]);
    assert_eq!(ctx.journal_count().await, 1);
}

// ─── 401 ─────────────────────────────────────────────────────────────────────

async fn status_with(ctx: &Ctx, auth: Option<String>, path: &str, body: Value) -> u16 {
    let mut request = ctx.http.post(format!("{}{path}", ctx.base)).json(&body);
    if let Some(auth) = auth {
        request = request.header("Authorization", auth);
    }
    request.send().await.unwrap().status().as_u16()
}

#[tokio::test]
async fn unauthorized_writes_nothing() {
    let ctx = Ctx::new(COFFEE).await;
    let batch = json!({ "records": [record("never")] });
    let bad = [
        None,
        Some(String::new()),
        Some("Bearer".to_owned()),
        Some("Bearer ".to_owned()),
        Some("Bearer unknown-token".to_owned()),
        Some(format!("Basic {}", ctx.token)),
        Some(ctx.token.clone()),
    ];
    for auth in bad {
        for path in ["/v1/records", "/v1/heartbeat"] {
            assert_eq!(
                status_with(&ctx, auth.clone(), path, batch.clone()).await,
                401,
                "{auth:?}"
            );
        }
    }
    assert_eq!(ctx.journal_count().await, 0);
    let seen: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT last_seen_at FROM core.sources WHERE id = $1")
            .bind(ctx.source_id)
            .fetch_one(&ctx.app)
            .await
            .unwrap();
    assert_eq!(seen, None);
}

#[tokio::test]
async fn unauthorized_response_matches_the_contract() {
    let ctx = Ctx::new(COFFEE).await;
    let response = ctx
        .http
        .post(format!("{}/v1/records", ctx.base))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
    assert_eq!(response.headers()["www-authenticate"], "Bearer");
    let body: Value = response.json().await.unwrap();
    assert!(validator(Some("error_response")).is_valid(&body));
}

#[tokio::test]
async fn revoked_source_is_refused() {
    let ctx = Ctx::new(COFFEE).await;
    assert_eq!(
        ctx.post("/v1/heartbeat").send().await.unwrap().status(),
        200
    );
    life_tracker::admin::revoke_source(&ctx.admin, &ctx.source)
        .await
        .unwrap();
    let (status, _) = ctx.send(json!([record("late")])).await;
    assert_eq!(status, 401);
    assert_eq!(
        ctx.post("/v1/heartbeat").send().await.unwrap().status(),
        401
    );
    assert_eq!(ctx.journal_count().await, 0);
}

/// La source webapp a un token_hash NULL : aucune valeur ne peut la désigner.
#[tokio::test]
async fn webapp_source_cannot_authenticate_by_token() {
    let ctx = Ctx::new(COFFEE).await;
    let row = sqlx::query(
        "SELECT auth::text AS auth, token_hash FROM core.sources WHERE name = 'webapp'",
    )
    .fetch_one(&ctx.app)
    .await
    .unwrap();
    assert_eq!(row.get::<String, _>("auth"), "session");
    assert_eq!(row.get::<Option<Vec<u8>>, _>("token_hash"), None);
    for token in ["webapp", "null", "NULL", "00"] {
        let auth = Some(format!("Bearer {token}"));
        assert_eq!(
            status_with(&ctx, auth, "/v1/heartbeat", json!({})).await,
            401
        );
    }
}

// ─── 400 et 413 ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn unreadable_json_is_400_and_writes_nothing() {
    let ctx = Ctx::new(COFFEE).await;
    let bodies: [&[u8]; 6] = [
        b"",
        b"{not json",
        b"[]",
        b"{}",
        br#"{"records": {}}"#,
        b"\xff\xfe",
    ];
    for body in bodies {
        let response = ctx
            .post("/v1/records")
            .body(body.to_vec())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 400, "{body:?}");
        let error: Value = response.json().await.unwrap();
        assert_eq!(error["error"], "invalid_json");
    }
    assert_eq!(ctx.journal_count().await, 0);
    assert_rejected_count(&ctx, 0).await;
}

#[tokio::test]
async fn more_than_500_records_is_413_and_writes_nothing() {
    let ctx = Ctx::new(COFFEE).await;
    let batch: Vec<Value> = (0..501).map(|i| record(&format!("k{i}"))).collect();
    let (status, body) = ctx.send(json!(batch)).await;
    assert_eq!(status, 413);
    assert_eq!(body["error"], "batch_too_large");
    assert_eq!(ctx.journal_count().await, 0);

    let results = ctx.results(json!(batch[..500])).await;
    assert_eq!(results.len(), 500);
    assert_eq!(ctx.journal_count().await, 500);
}

/// Un corps de `size` octets exactement, JSON valide contenant un enregistrement.
fn body_of_size(size: usize) -> String {
    let empty = |pad: &str| {
        format!(
            r#"{{"records":[{{"key":"big","kind":"coffee.cup","schema_version":1,"occurred_at":"2026-10-13T09:42:10Z","payload":"{pad}"}}]}}"#
        )
    };
    empty(&"x".repeat(size - empty("").len()))
}

#[tokio::test]
async fn body_limit_is_one_megabyte() {
    let ctx = Ctx::new(COFFEE).await;
    let response = ctx
        .post("/v1/records")
        .body(body_of_size(1_000_001))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 413);
    assert_eq!(ctx.journal_count().await, 0);

    let response = ctx
        .post("/v1/records")
        .body(body_of_size(1_000_000))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(ctx.journal_count().await, 1);
}

/// Sans Content-Length, la limite doit s'appliquer pendant la lecture.
#[tokio::test]
async fn chunked_body_over_the_limit_is_413() {
    let ctx = Ctx::new(COFFEE).await;
    let addr = ctx.base.trim_start_matches("http://");
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let head = format!(
        "POST /v1/records HTTP/1.1\r\nHost: t\r\nAuthorization: Bearer {}\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
        ctx.token
    );
    stream.write_all(head.as_bytes()).await.unwrap();
    let chunk = "x".repeat(100_000);
    let framed = format!("{:x}\r\n{chunk}\r\n", chunk.len());
    for _ in 0..11 {
        // Le serveur peut couper dès la limite atteinte : seule sa réponse compte.
        if stream.write_all(framed.as_bytes()).await.is_err() {
            break;
        }
    }
    let _ = stream.write_all(b"0\r\n\r\n").await;
    let mut response = Vec::new();
    let _ = stream.read_to_end(&mut response).await;
    let response = String::from_utf8_lossy(&response);
    assert!(response.starts_with("HTTP/1.1 413"), "{response}");
    assert_eq!(ctx.journal_count().await, 0);
}

/// Un token invalide n'obtient jamais la lecture du corps, même trop gros.
#[tokio::test]
async fn authentication_comes_before_body_limits() {
    let ctx = Ctx::new(COFFEE).await;
    let response = ctx
        .http
        .post(format!("{}/v1/records", ctx.base))
        .body(body_of_size(1_000_001))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
}

// ─── Dernier contact et rejets ───────────────────────────────────────────────

async fn assert_rejected_count(ctx: &Ctx, expected: i64) {
    let count: i64 = sqlx::query_scalar("SELECT rejected_count FROM core.sources WHERE id = $1")
        .bind(ctx.source_id)
        .fetch_one(&ctx.app)
        .await
        .unwrap();
    assert_eq!(count, expected);
}

async fn last_seen(ctx: &Ctx) -> (Option<DateTime<Utc>>, Option<String>) {
    let row = sqlx::query("SELECT last_seen_at, client_version FROM core.sources WHERE id = $1")
        .bind(ctx.source_id)
        .fetch_one(&ctx.app)
        .await
        .unwrap();
    (row.get("last_seen_at"), row.get("client_version"))
}

#[tokio::test]
async fn heartbeat_updates_contact_and_writes_no_record() {
    let ctx = Ctx::new(COFFEE).await;
    assert_eq!(last_seen(&ctx).await, (None, None));
    let response = ctx
        .post("/v1/heartbeat")
        .header("X-Client-Version", "lastfm-0.1")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.unwrap();
    assert!(
        validator(Some("heartbeat_response")).is_valid(&body),
        "{body}"
    );
    let (seen, version) = last_seen(&ctx).await;
    assert!(seen.unwrap() > Utc::now() - Duration::minutes(5));
    assert_eq!(version.as_deref(), Some("lastfm-0.1"));
    assert_eq!(ctx.journal_count().await, 0);
}

#[tokio::test]
async fn every_authenticated_call_updates_the_contact() {
    let ctx = Ctx::new(COFFEE).await;
    let mut previous = None;
    // Lot normal, JSON illisible, trop gros : le token était bon, le dernier contact avance.
    let bodies = [
        json!({"records": [record("a")]}).to_string(),
        "{not json".to_owned(),
        json!({"records": vec![record("x"); 501]}).to_string(),
    ];
    for (body, status) in bodies.iter().zip([200, 400, 413]) {
        let response = ctx
            .post("/v1/records")
            .header("X-Client-Version", format!("v{status}"))
            .body(body.clone())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), status);
        let (seen, version) = last_seen(&ctx).await;
        assert!(seen > previous, "{status}");
        assert_eq!(version, Some(format!("v{status}")));
        previous = seen;
    }
}

#[tokio::test]
async fn client_version_is_kept_when_the_header_is_absent() {
    let ctx = Ctx::new(COFFEE).await;
    let with = ctx.post("/v1/heartbeat").header("X-Client-Version", "1.0");
    assert_eq!(with.send().await.unwrap().status(), 200);
    assert_eq!(
        ctx.post("/v1/heartbeat").send().await.unwrap().status(),
        200
    );
    assert_eq!(last_seen(&ctx).await.1.as_deref(), Some("1.0"));
}

#[tokio::test]
async fn rejections_update_the_counter() {
    let ctx = Ctx::new(COFFEE).await;
    let mut no_kind = record("a");
    no_kind.as_object_mut().unwrap().remove("kind");
    let at_before: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT last_rejected_at FROM core.sources WHERE id = $1")
            .bind(ctx.source_id)
            .fetch_one(&ctx.app)
            .await
            .unwrap();
    assert_eq!(at_before, None);

    ctx.results(json!([record("ok")])).await;
    assert_rejected_count(&ctx, 0).await;

    let results = ctx
        .results(json!([no_kind.clone(), record("ok2"), 42]))
        .await;
    assert_eq!(statuses(&results), ["rejected", "accepted", "rejected"]);
    assert_rejected_count(&ctx, 2).await;
    ctx.results(json!([no_kind])).await;
    assert_rejected_count(&ctx, 3).await;

    let at: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT last_rejected_at FROM core.sources WHERE id = $1")
            .bind(ctx.source_id)
            .fetch_one(&ctx.app)
            .await
            .unwrap();
    assert!(at.unwrap() > Utc::now() - Duration::minutes(5));
}

// ─── Fixtures et 5xx ─────────────────────────────────────────────────────────

#[tokio::test]
async fn server_agrees_with_the_fixtures() {
    for (name, fixture) in fixtures("valid") {
        let ctx = Ctx::new(ALL).await;
        let results = ctx.results(fixture["records"].clone()).await;
        assert!(
            statuses(&results).iter().all(|s| *s == "accepted"),
            "{name} : {results:?}"
        );
    }
    for (name, fixture) in fixtures("invalid") {
        let ctx = Ctx::new(ALL).await;
        let response = ctx.post("/v1/records").json(&fixture).send().await.unwrap();
        let status = response.status().as_u16();
        let body: Value = response.json().await.unwrap();
        match status {
            400 => {}
            200 => assert!(
                statuses(body["results"].as_array().unwrap())
                    .iter()
                    .all(|s| *s == "rejected"),
                "{name} : {body}"
            ),
            other => panic!("{name} : statut {other}"),
        }
        assert_eq!(ctx.journal_count().await, 0, "{name}");
    }
}

/// Base injoignable : 500 pour que le client réessaie, et le contrôle de santé le dit.
#[tokio::test]
async fn database_down_gives_5xx() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .acquire_timeout(std::time::Duration::from_millis(200))
        .connect_lazy("postgres://tracker_app:x@127.0.0.1:1/tracker")
        .unwrap();
    let base = serve(pool).await;
    let http = reqwest::Client::new();
    let response = http
        .post(format!("{base}/v1/records"))
        .bearer_auth("any")
        .json(&json!({"records": [record("k")]}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 500);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"], "internal");
    let health = http.get(format!("{base}/health")).send().await.unwrap();
    assert_eq!(health.status(), 503);
}

/// Écriture refusée par la base : 500, et ni journal ni compteur ne bougent. Un serveur connecté par
/// tracker_admin s'authentifie bien mais n'a pas INSERT sur le journal.
#[tokio::test]
async fn failed_write_gives_500_and_rolls_back_the_counter() {
    let ctx = Ctx::new(COFFEE).await;
    let base = serve(life_tracker::db::pool(&admin_url()).unwrap()).await;
    let response = ctx
        .http
        .post(format!("{base}/v1/records"))
        .bearer_auth(&ctx.token)
        .json(&json!({"records": [record("k"), 42]}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 500);
    assert_eq!(ctx.journal_count().await, 0);
    assert_rejected_count(&ctx, 0).await;

    let results = ctx.results(json!([record("k")])).await;
    assert_eq!(statuses(&results), ["accepted"], "le nouvel essai réussit");
}

#[tokio::test]
async fn health_is_ok_with_a_database() {
    let ctx = Ctx::new(COFFEE).await;
    let health = ctx
        .http
        .get(format!("{}/health", ctx.base))
        .send()
        .await
        .unwrap();
    assert_eq!(health.status(), 200);
}

#[tokio::test]
async fn app_role_cannot_rewrite_sources() {
    let ctx = Ctx::new(COFFEE).await;
    let err = sqlx::query("UPDATE core.sources SET revoked_at = NULL WHERE id = $1")
        .bind(ctx.source_id)
        .execute(&ctx.app)
        .await
        .unwrap_err();
    assert_eq!(
        err.as_database_error().unwrap().code().as_deref(),
        Some("42501")
    );
}
