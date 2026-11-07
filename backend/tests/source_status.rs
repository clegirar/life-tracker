//! La page d'état des trackers : l'état rapporté par une source (`PUT /v1/sources/me/status`) et sa
//! lecture par la web app (`GET /v1/sources/status`), sur un vrai PostgreSQL, une base neuve par test.

mod common;
use common::*;
use life_tracker::admin;
use life_tracker::api::{self, CSRF_HEADER};
use life_tracker::engine;
use life_tracker::interpreters::INTERPRETERS;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::sync::watch;

const STATUS: &str = "/v1/sources/me/status";
const PAGE: &str = "/v1/sources/status";
const USER: &str = "moi";
const PASSWORD: &str = "un mot de passe assez long";

struct Bench {
    db: TestDb,
    base: String,
    http: reqwest::Client,
    _stop: watch::Sender<bool>,
}

impl Bench {
    async fn new() -> Self {
        let db = TestDb::new().await;
        admin::set_password(&db.admin, USER, PASSWORD)
            .await
            .unwrap();
        let (stop, stopped) = watch::channel(false);
        tokio::spawn(engine::run(
            db.app.clone(),
            INTERPRETERS,
            Duration::from_millis(20),
            stopped,
        ));
        let base = serve(db.app.clone()).await;
        Self {
            db,
            base,
            http: client(),
            _stop: stop,
        }
    }

    async fn create(&self, name: &str, threshold: Option<&str>, label: Option<&str>) -> String {
        admin::create_source(&self.db.admin, name, threshold, label)
            .await
            .unwrap()
    }

    fn put(&self, token: &str, body: &Value) -> reqwest::RequestBuilder {
        self.http
            .put(format!("{}{STATUS}", self.base))
            .bearer_auth(token)
            .header("X-Client-Version", "test-1")
            .json(body)
    }

    async fn report(&self, token: &str, body: &Value) -> u16 {
        self.put(token, body)
            .send()
            .await
            .unwrap()
            .status()
            .as_u16()
    }

    async fn login(&self) -> String {
        let response = self
            .http
            .post(format!("{}/v1/auth/login", self.base))
            .header(CSRF_HEADER, "1")
            .json(&json!({ "username": USER, "password": PASSWORD }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 204);
        let cookie = response.headers()["set-cookie"].to_str().unwrap();
        cookie.split(';').next().unwrap().to_owned()
    }

    async fn page(&self, cookie: &str) -> Value {
        let response = self
            .http
            .get(format!("{}{PAGE}", self.base))
            .header("cookie", cookie)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        response.json().await.unwrap()
    }

    /// L'état stocké, sa date, le dernier contact et la version du client.
    async fn stored(&self, name: &str) -> (Option<Value>, bool, bool, Option<String>) {
        let (status, reported, seen, version): (Option<String>, bool, bool, Option<String>) =
            sqlx::query_as(
                "SELECT status::text, status_reported_at IS NOT NULL, last_seen_at IS NOT NULL,
                        client_version
                 FROM core.sources WHERE name = $1",
            )
            .bind(name)
            .fetch_one(&self.db.app)
            .await
            .unwrap();
        (
            status.map(|s| serde_json::from_str(&s).unwrap()),
            reported,
            seen,
            version,
        )
    }

    async fn settled(&self) {
        for _ in 0..3000 {
            let behind = self
                .db
                .count(
                    "SELECT count(*) FROM core.records r WHERE NOT EXISTS (
                       SELECT 1 FROM derived.cursors c JOIN derived.generations g
                         ON g.id = c.generation AND g.state = 'active'
                       WHERE c.kind = r.kind AND c.last_seq >= r.seq)",
                )
                .await;
            if behind == 0 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("les interprètes n'ont pas rattrapé le journal");
    }
}

#[tokio::test]
async fn a_reported_status_is_stored_normalized_without_touching_the_last_contact() {
    let bench = Bench::new().await;
    let token = bench.create("garmin", Some("12 hours"), None).await;
    assert_eq!(bench.stored("garmin").await, (None, false, false, None));

    let full = json!({
        "state": "action_required",
        "reason": "key_rejected",
        "expires_at": "2027-11-05T00:00:00+01:00",
        "next_run_at": "2026-11-05T12:15:00Z",
        "last_collected_at": "2026-11-05T13:00:00+01:00",
    });
    assert_eq!(bench.report(&token, &full).await, 204);
    let (status, reported, seen, version) = bench.stored("garmin").await;
    assert_eq!(
        status.unwrap(),
        json!({
            "state": "action_required",
            "reason": "key_rejected",
            "expires_at": "2027-11-04T23:00:00Z",
            "next_run_at": "2026-11-05T12:15:00Z",
            "last_collected_at": "2026-11-05T12:00:00Z",
        })
    );
    assert!(reported);
    // Rapporter un état n'est pas un contact réussi : ni dernier contact, ni version du client.
    assert!(!seen, "le dernier contact ne bouge pas");
    assert_eq!(version, None);

    // Le dernier état remplace le précédent ; les champs omis valent null.
    assert_eq!(bench.report(&token, &json!({ "state": "ok" })).await, 204);
    assert_eq!(
        bench.stored("garmin").await.0.unwrap(),
        json!({ "state": "ok", "reason": null, "expires_at": null, "next_run_at": null,
                "last_collected_at": null })
    );
    assert_eq!(bench.db.count("SELECT count(*) FROM core.records").await, 0);
}

/// Le contrat et le serveur disent la même chose de chaque corps.
#[tokio::test]
async fn only_the_closed_vocabulary_is_accepted() {
    let bench = Bench::new().await;
    let token = bench.create("lastfm", Some("6 hours"), None).await;
    let schema = validator(Some("source_status_request"));
    let accepted = [
        json!({ "state": "ok" }),
        json!({ "state": "ok", "reason": null, "expires_at": null, "next_run_at": null,
                "last_collected_at": null }),
        json!({ "state": "ok", "last_collected_at": "2026-11-05T10:00:00Z" }),
        json!({ "state": "error", "reason": "rate_limited", "next_run_at": "2026-11-05T12:00:00Z" }),
        json!({ "state": "error", "reason": "service_unavailable",
                "last_collected_at": "2026-11-03T10:00:00+01:00" }),
        json!({ "state": "error", "reason": "service_unavailable" }),
        json!({ "state": "error", "reason": "ingest_failed" }),
        json!({ "state": "action_required", "reason": "consent_expired" }),
        json!({ "state": "action_required", "reason": "consent_required" }),
        json!({ "state": "action_required", "reason": "profile_private",
                "last_collected_at": "2026-11-06T10:00:00Z" }),
        json!({ "state": "action_required", "reason": "key_rejected",
                "expires_at": "2027-01-01T00:00:00+02:00" }),
    ];
    for body in &accepted {
        assert!(schema.is_valid(body), "contrat : {body}");
        assert_eq!(bench.report(&token, body).await, 204, "{body}");
    }
    let before = bench.stored("lastfm").await.0;
    let refused = [
        json!({}),
        json!([]),
        json!("ok"),
        json!({ "state": "fine" }),
        json!({ "state": "ok", "reason": "key_rejected" }),
        json!({ "state": "error" }),
        json!({ "state": "error", "reason": null }),
        json!({ "state": "action_required", "reason": "the key 1234 was refused" }),
        json!({ "state": "ok", "message": "texte libre" }),
        json!({ "state": "ok", "expires_at": "demain" }),
        json!({ "state": "ok", "next_run_at": 1_762_340_000 }),
        json!({ "state": "ok", "expires_at": "2026-11-05" }),
        json!({ "state": "ok", "last_collected_at": "hier" }),
        json!({ "state": "ok", "last_collected_at": 1_762_340_000 }),
    ];
    for body in &refused {
        assert!(!schema.is_valid(body), "contrat : {body}");
        let response = bench.put(&token, body).send().await.unwrap();
        assert_eq!(response.status(), 400, "{body}");
        let error: Value = response.json().await.unwrap();
        assert_eq!(error, json!({ "error": "invalid_json" }));
        assert!(validator(Some("error_response")).is_valid(&error));
    }
    let response = bench
        .http
        .put(format!("{}{STATUS}", bench.base))
        .bearer_auth(&token)
        .body("{not json")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    let huge = json!({ "state": "ok", "padding": "x".repeat(4096) });
    assert_eq!(bench.report(&token, &huge).await, 400);
    assert_eq!(bench.stored("lastfm").await.0, before, "rien n'a changé");
}

#[tokio::test]
async fn only_a_source_token_reports_a_status() {
    let bench = Bench::new().await;
    let token = bench.create("garmin", Some("12 hours"), None).await;
    let revoked = bench.create("old", Some("1 hour"), None).await;
    admin::revoke_source(&bench.db.admin, "old").await.unwrap();
    let service = admin::create_service_token(&bench.db.admin, "mcp")
        .await
        .unwrap();
    let cookie = bench.login().await;
    let body = json!({ "state": "ok" });
    let url = format!("{}{STATUS}", bench.base);
    let c = &bench.http;
    let attempts = [
        ("sans rien", c.put(&url).json(&body)),
        (
            "mauvais token",
            c.put(&url).bearer_auth("mauvais").json(&body),
        ),
        (
            "source révoquée",
            c.put(&url).bearer_auth(&revoked).json(&body),
        ),
        (
            "jeton de service",
            c.put(&url).bearer_auth(&service).json(&body),
        ),
        (
            "session de la web app",
            c.put(&url)
                .header("cookie", &cookie)
                .header(CSRF_HEADER, "1")
                .json(&body),
        ),
        (
            "session et mauvais token",
            c.put(&url)
                .header("cookie", &cookie)
                .header(CSRF_HEADER, "1")
                .bearer_auth("mauvais")
                .json(&body),
        ),
    ];
    for (name, attempt) in attempts {
        assert_eq!(attempt.send().await.unwrap().status(), 401, "{name}");
    }
    assert_eq!(bench.stored("garmin").await.0, None);
    assert_eq!(bench.stored("webapp").await.0, None);
    assert_eq!(bench.report(&token, &body).await, 204);
}

#[tokio::test]
async fn the_status_page_shows_the_trackers_and_requires_the_session() {
    let bench = Bench::new().await;
    let garmin = bench
        .create("garmin", Some("12 hours"), Some("Garmin Connect"))
        .await;
    bench.create("lastfm", Some("6 hours"), None).await;
    let ios = bench.create("ios", None, Some("Raccourci café")).await;
    let probe = bench.create("probe", None, None).await;
    let apple = bench.create("apple", None, None).await;
    bench.create("old", Some("1 hour"), None).await;
    admin::revoke_source(&bench.db.admin, "old").await.unwrap();
    for (source, token) in [("ios", &ios), ("apple", &apple)] {
        admin::grant_kind(&bench.db.admin, source, "coffee.cup")
            .await
            .unwrap();
        let response = bench
            .http
            .post(format!("{}/v1/records", bench.base))
            .bearer_auth(token)
            .json(&json!({ "records": [
                record("a"),
                { "key": "b", "kind": "coffee.cup", "schema_version": 1,
                  "occurred_at": "2026-10-14T08:00:00Z", "payload": {} },
            ]}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
    }
    let rate_limited = json!({ "state": "error", "reason": "rate_limited",
                               "next_run_at": "2026-11-05T12:00:00Z" });
    assert_eq!(bench.report(&garmin, &rate_limited).await, 204);
    bench.settled().await;

    let cookie = bench.login().await;
    let page = bench.page(&cookie).await;
    let sources = page["sources"].as_array().unwrap();
    let names: Vec<&str> = sources
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    // Par libellé, sinon par nom, sans casse ; ni `webapp`, ni l'import sans seuil, état ni libellé,
    // ni la source révoquée. `ios` n'a qu'un libellé : il suffit, une journée sans café n'est pas un
    // silence. `probe` n'a rien : absente tant qu'elle n'a rien rapporté.
    assert_eq!(names, ["garmin", "lastfm", "ios"]);
    let garmin_entry = &sources[0];
    assert_eq!(garmin_entry["label"], "Garmin Connect");
    assert_eq!(garmin_entry["silence_threshold_seconds"], 43200.0);
    assert_eq!(garmin_entry["silent"], false);
    assert_eq!(garmin_entry["last_seen_at"], Value::Null);
    assert_eq!(
        garmin_entry["status"],
        json!({ "state": "error", "reason": "rate_limited", "expires_at": null,
                "next_run_at": "2026-11-05T12:00:00Z", "last_collected_at": null })
    );
    assert!(garmin_entry["status_reported_at"].is_string());
    assert_eq!(garmin_entry["metrics"], json!([]));
    assert_eq!(
        garmin_entry["authorization_url"],
        Value::Null,
        "aucun accord OAuth en cours"
    );
    let keys: Vec<&String> = garmin_entry.as_object().unwrap().keys().collect();
    assert_eq!(
        keys.len(),
        9,
        "name, label, last_seen_at, silence_threshold_seconds, silent, status, status_reported_at, authorization_url, metrics : {keys:?}"
    );
    assert_eq!(sources[1]["label"], Value::Null);
    assert_eq!(sources[1]["status"], Value::Null);
    assert_eq!(sources[1]["status_reported_at"], Value::Null);

    // La dernière donnée de chaque métrique : celle de la source, pas celle d'une autre.
    let ios_entry = &sources[2];
    assert_eq!(ios_entry["label"], "Raccourci café");
    assert_eq!(ios_entry["silence_threshold_seconds"], Value::Null);
    assert_eq!(ios_entry["silent"], false);
    assert!(ios_entry["last_seen_at"].is_string());
    assert_eq!(
        ios_entry["metrics"],
        json!([{ "metric": "coffee", "label": "Cafés", "last_at": "2026-10-14T08:00:00Z" }])
    );

    // Un état rapporté suffit à faire paraître une source.
    assert_eq!(bench.report(&probe, &json!({ "state": "ok" })).await, 204);
    let page = bench.page(&cookie).await;
    let names: Vec<&str> = page["sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["garmin", "lastfm", "probe", "ios"]);

    // Une seule entrée : la web app, sous session. Ni le MCP ni un token de source.
    let service = admin::create_service_token(&bench.db.admin, "mcp")
        .await
        .unwrap();
    let url = format!("{}{PAGE}", bench.base);
    for (name, attempt) in [
        ("sans session", bench.http.get(&url)),
        (
            "jeton de service",
            bench.http.get(&url).bearer_auth(&service),
        ),
        ("token de source", bench.http.get(&url).bearer_auth(&garmin)),
    ] {
        assert_eq!(attempt.send().await.unwrap().status(), 401, "{name}");
    }
    assert!(!api::READ_ROUTES.contains(&PAGE));
}

#[tokio::test]
async fn a_label_is_set_by_the_admin_tool_only() {
    let bench = Bench::new().await;
    bench.create("lastfm", Some("6 hours"), None).await;
    admin::set_label(&bench.db.admin, "lastfm", "  Last.fm  ")
        .await
        .unwrap();
    let label: Option<String> =
        sqlx::query_scalar("SELECT label FROM core.sources WHERE name = 'lastfm'")
            .fetch_one(&bench.db.app)
            .await
            .unwrap();
    assert_eq!(label.as_deref(), Some("Last.fm"));
    assert!(
        admin::set_label(&bench.db.admin, "inconnue", "X")
            .await
            .is_err()
    );
    assert!(
        admin::set_label(&bench.db.admin, "lastfm", " ")
            .await
            .is_err()
    );
    assert!(
        admin::set_label(&bench.db.admin, "lastfm", &"x".repeat(81))
            .await
            .is_err()
    );
    assert!(
        admin::create_source(&bench.db.admin, "other", None, Some(""))
            .await
            .is_err()
    );
    let exists = bench
        .db
        .count("SELECT count(*) FROM core.sources WHERE name = 'other'")
        .await;
    assert_eq!(exists, 0, "un libellé refusé ne crée pas la source");
}
