//! Le compte web, l'écriture par session et la lecture d'un jour, sur un vrai PostgreSQL : une base
//! neuve par test, le serveur en tracker_app, le mot de passe posé en tracker_admin.

mod common;
use axum::Router;
use axum::routing::get;
use common::*;
use life_tracker::api::{self, Config, NO_SESSION_ROUTES};
use life_tracker::interpreters::INTERPRETERS;
use life_tracker::{admin, engine};
use reqwest::Response;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::sync::watch;

const USER: &str = "moi";
const PASSWORD: &str = "un mot de passe assez long";
const CSRF: (&str, &str) = (api::CSRF_HEADER, "1");

/// Une base, son API et son worker.
struct Web {
    db: TestDb,
    base: String,
    http: reqwest::Client,
    _stop: watch::Sender<bool>,
}

impl Web {
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
            http: reqwest::Client::new(),
            _stop: stop,
        }
    }

    async fn login_response(&self, username: &str, password: &str) -> Response {
        self.http
            .post(format!("{}/v1/auth/login", self.base))
            .header(CSRF.0, CSRF.1)
            .json(&json!({ "username": username, "password": password }))
            .send()
            .await
            .unwrap()
    }

    /// Renvoie l'en-tête `Cookie` d'une nouvelle session.
    async fn login(&self) -> String {
        let response = self.login_response(USER, PASSWORD).await;
        assert_eq!(response.status(), 204);
        cookie_pair(&response)
    }

    fn get(&self, path: &str, cookie: &str) -> reqwest::RequestBuilder {
        self.http
            .get(format!("{}{path}", self.base))
            .header("cookie", cookie)
    }

    fn post(&self, path: &str, cookie: &str) -> reqwest::RequestBuilder {
        self.http
            .post(format!("{}{path}", self.base))
            .header("cookie", cookie)
            .header(CSRF.0, CSRF.1)
    }

    async fn write(&self, cookie: &str, records: Value) -> Vec<Value> {
        let response = self
            .post("/v1/records", cookie)
            .json(&json!({ "records": records }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body: Value = response.json().await.unwrap();
        body["results"].as_array().unwrap().clone()
    }

    async fn write_with_token(&self, token: &str, records: Value) -> Vec<Value> {
        let response = self
            .http
            .post(format!("{}/v1/records", self.base))
            .bearer_auth(token)
            .json(&json!({ "records": records }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body: Value = response.json().await.unwrap();
        body["results"].as_array().unwrap().clone()
    }

    async fn day(&self, cookie: &str, date: &str) -> Value {
        let response = self
            .get(&format!("/v1/days/{date}"), cookie)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        response.json().await.unwrap()
    }

    /// Le worker est asynchrone : attend que le jour montre ce nombre de cafés.
    async fn day_with_count(&self, cookie: &str, date: &str, count: u64) -> Value {
        for _ in 0..500 {
            let day = self.day(cookie, date).await;
            if day["count"] == count {
                return day;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!(
            "{date} : jamais {count} café(s) : {}",
            self.day(cookie, date).await
        );
    }

    /// Plus rien en attente pour le worker : le nombre lu ensuite est définitif.
    async fn wait_for_worker(&self) {
        for _ in 0..500 {
            let waiting = self
                .db
                .count(
                    "SELECT count(*) FROM core.records r WHERE r.seq > COALESCE((
                       SELECT c.last_seq FROM derived.cursors c
                       JOIN derived.generations g ON g.id = c.generation AND g.state = 'active'
                       WHERE c.kind = r.kind), 0)",
                )
                .await;
            if waiting == 0 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("le worker n'a pas fini");
    }
}

fn set_cookie(response: &Response) -> String {
    response.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .to_owned()
}

fn cookie_pair(response: &Response) -> String {
    set_cookie(response).split(';').next().unwrap().to_owned()
}

fn cup(key: &str, occurred_at: &str) -> Value {
    json!({ "key": key, "kind": "coffee.cup", "schema_version": 1,
            "occurred_at": occurred_at, "payload": {} })
}

fn cancel(key: &str, source: &str, target: &str) -> Value {
    json!({ "key": key, "kind": "record.cancel", "schema_version": 1,
            "occurred_at": "2026-10-13T10:00:00+02:00",
            "payload": { "source": source, "key": target } })
}

fn times(day: &Value) -> Vec<&str> {
    day["cups"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["time"].as_str().unwrap())
        .collect()
}

// ─── Compte ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn login_cookie_has_every_attribute() {
    let web = Web::new().await;
    let response = web.login_response(USER, PASSWORD).await;
    assert_eq!(response.status(), 204);
    let cookie = set_cookie(&response);
    let attributes: Vec<&str> = cookie.split("; ").skip(1).collect();
    for attribute in ["Path=/", "HttpOnly", "SameSite=Strict", "Secure"] {
        assert!(attributes.contains(&attribute), "{cookie}");
    }
    assert!(
        attributes.iter().any(|a| a.starts_with("Max-Age=")),
        "{cookie}"
    );
}

#[tokio::test]
async fn only_the_explicit_dev_setting_removes_secure() {
    let web = Web::new().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = api::router_with(
        web.db.app.clone(),
        Config {
            insecure_cookie: true,
        },
    );
    tokio::spawn(async move { axum::serve(listener, router).await });
    let response = web
        .http
        .post(format!("http://{addr}/v1/auth/login"))
        .header(CSRF.0, CSRF.1)
        .json(&json!({ "username": USER, "password": PASSWORD }))
        .send()
        .await
        .unwrap();
    let cookie = set_cookie(&response);
    assert!(!cookie.contains("Secure"), "{cookie}");
    assert!(
        cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"),
        "{cookie}"
    );
    assert!(!Config::default().insecure_cookie);
}

#[tokio::test]
async fn session_is_stored_hashed_and_password_in_argon2id() {
    let web = Web::new().await;
    let response = web.login_response(USER, PASSWORD).await;
    let pair = cookie_pair(&response);
    let token = pair.split_once('=').unwrap().1;
    assert_eq!(token.len(), 64);
    let stored: Vec<u8> = sqlx::query_scalar("SELECT token_hash FROM core.sessions")
        .fetch_one(&web.db.app)
        .await
        .unwrap();
    assert_eq!(stored, admin::token_hash(token));
    let hash: String = sqlx::query_scalar("SELECT password_hash FROM core.account")
        .fetch_one(&web.db.admin)
        .await
        .unwrap();
    assert!(hash.starts_with("$argon2id$"));
    assert!(!hash.contains(PASSWORD));
}

#[tokio::test]
async fn login_failure_is_one_generic_answer() {
    let web = Web::new().await;
    let mut answers = Vec::new();
    for (username, password) in [
        (USER, "mauvais mot de passe"),
        ("inconnu", PASSWORD),
        ("inconnu", "mauvais mot de passe"),
        (USER, ""),
    ] {
        let response = web.login_response(username, password).await;
        assert_eq!(response.status(), 401);
        assert!(response.headers().get("set-cookie").is_none());
        answers.push(response.text().await.unwrap());
    }
    answers.dedup();
    assert_eq!(answers, [r#"{"error":"invalid_credentials"}"#]);
    assert_eq!(web.db.count("SELECT count(*) FROM core.sessions").await, 0);
}

#[tokio::test]
async fn login_without_an_account_gives_the_same_answer() {
    let db = TestDb::new().await;
    let base = serve(db.app.clone()).await;
    let response = reqwest::Client::new()
        .post(format!("{base}/v1/auth/login"))
        .header(CSRF.0, CSRF.1)
        .json(&json!({ "username": USER, "password": PASSWORD }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
    assert_eq!(
        response.text().await.unwrap(),
        r#"{"error":"invalid_credentials"}"#
    );
}

#[tokio::test]
async fn logged_out_session_is_refused_and_cookie_cleared() {
    let web = Web::new().await;
    let cookie = web.login().await;
    assert_eq!(
        web.get("/v1/days/2026-10-25", &cookie)
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    let response = web.post("/v1/auth/logout", &cookie).send().await.unwrap();
    assert_eq!(response.status(), 204);
    let cleared = set_cookie(&response);
    assert!(
        cleared.contains("Max-Age=0") && cleared.contains("Secure"),
        "{cleared}"
    );
    assert_eq!(
        web.get("/v1/days/2026-10-25", &cookie)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        web.post("/v1/records", &cookie)
            .json(&json!({ "records": [] }))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(web.db.count("SELECT count(*) FROM core.sessions").await, 0);
}

#[tokio::test]
async fn expired_session_is_refused() {
    let web = Web::new().await;
    let cookie = web.login().await;
    TestDb::exec(
        &web.db.app,
        "UPDATE core.sessions SET expires_at = now() - interval '1 second'",
    )
    .await;
    assert_eq!(
        web.get("/v1/days/2026-10-25", &cookie)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        web.post("/v1/records", &cookie)
            .json(&json!({ "records": [cup("late", "2026-10-13T09:00:00+02:00")] }))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    // La connexion suivante fait le ménage.
    web.login().await;
    assert_eq!(web.db.count("SELECT count(*) FROM core.sessions").await, 1);
}

#[tokio::test]
async fn sessions_expire_by_schema() {
    let web = Web::new().await;
    web.login().await;
    let days = web
        .db
        .count("SELECT extract(day FROM expires_at - created_at)::bigint FROM core.sessions")
        .await;
    assert_eq!(days, 30);
}

#[tokio::test]
async fn changing_the_password_closes_sessions() {
    let web = Web::new().await;
    let cookie = web.login().await;
    admin::set_password(&web.db.admin, USER, "un autre mot de passe long")
        .await
        .unwrap();
    assert_eq!(
        web.get("/v1/days/2026-10-25", &cookie)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(web.login_response(USER, PASSWORD).await.status(), 401);
    assert_eq!(
        web.login_response(USER, "un autre mot de passe long")
            .await
            .status(),
        204
    );
}

#[tokio::test]
async fn a_short_password_is_refused() {
    let db = TestDb::new().await;
    assert!(admin::set_password(&db.admin, USER, "court").await.is_err());
}

// ─── Contrôle d'accès ────────────────────────────────────────────────────────

/// Toutes les routes de `routes()`. Une route oubliée ici reste protégée par le refus par défaut,
/// éprouvé par `a_future_route_requires_the_session_by_default`.
#[tokio::test]
async fn every_route_refuses_a_request_without_session() {
    let web = Web::new().await;
    let c = &web.http;
    let base = &web.base;
    let attempts = [
        (
            "records",
            c.post(format!("{base}/v1/records"))
                .json(&json!({ "records": [] })),
        ),
        ("heartbeat", c.post(format!("{base}/v1/heartbeat"))),
        ("logout", c.post(format!("{base}/v1/auth/logout"))),
        (
            "logout+csrf",
            c.post(format!("{base}/v1/auth/logout"))
                .header(CSRF.0, CSRF.1),
        ),
        ("day", c.get(format!("{base}/v1/days/2026-10-25"))),
        ("invalid day", c.get(format!("{base}/v1/days/pas-une-date"))),
        ("head day", c.head(format!("{base}/v1/days/2026-10-25"))),
        ("unknown", c.get(format!("{base}/v1/inconnu"))),
        (
            "forged cookie",
            c.get(format!("{base}/v1/days/2026-10-25"))
                .header("cookie", "lt_session=abcd"),
        ),
        (
            "empty cookie",
            c.get(format!("{base}/v1/days/2026-10-25"))
                .header("cookie", "lt_session="),
        ),
    ];
    for (name, attempt) in attempts {
        let response = attempt.send().await.unwrap();
        assert_eq!(response.status(), 401, "{name}");
    }
}

#[tokio::test]
async fn the_routes_without_session_are_exactly_these() {
    assert_eq!(
        NO_SESSION_ROUTES,
        ["/health", "/v1/auth/login", "/v1/heartbeat"]
    );
    let web = Web::new().await;
    let health = web
        .http
        .get(format!("{}/health", web.base))
        .send()
        .await
        .unwrap();
    assert_eq!(health.status(), 200);
}

#[tokio::test]
async fn a_future_route_requires_the_session_by_default() {
    let web = Web::new().await;
    let cookie = web.login().await;
    let future = Router::new().route(
        "/v1/future",
        get(|| async { "secret" }).post(|| async { "secret" }),
    );
    let router = api::harden(future, web.db.app.clone(), Config::default());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await });
    let url = format!("http://{addr}/v1/future");

    assert_eq!(web.http.get(&url).send().await.unwrap().status(), 401);
    assert_eq!(web.http.post(&url).send().await.unwrap().status(), 401);
    let with_session = |request: reqwest::RequestBuilder| request.header("cookie", &cookie);
    assert_eq!(
        with_session(web.http.get(&url))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        with_session(web.http.post(&url))
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        with_session(web.http.post(&url))
            .header(CSRF.0, CSRF.1)
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
}

#[tokio::test]
async fn a_token_gives_no_read_access() {
    let web = Web::new().await;
    let (_, token) = web.db.source("ios", &["coffee.cup"]).await;
    let response = web
        .http
        .get(format!("{}/v1/days/2026-10-25", web.base))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
}

// ─── CSRF ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn writes_by_cookie_without_the_csrf_header_are_refused() {
    let web = Web::new().await;
    let cookie = web.login().await;
    let plain = |path: &str| {
        web.http
            .post(format!("{}{path}", web.base))
            .header("cookie", cookie.as_str())
    };
    // Ce qu'un formulaire d'un autre site peut envoyer : du JSON déguisé en texte.
    let forged = plain("/v1/records")
        .header("content-type", "text/plain")
        .header("origin", "https://evil.example")
        .body(json!({ "records": [cup("forged", "2026-10-13T09:00:00+02:00")] }).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(forged.status(), 403);
    assert_eq!(forged.text().await.unwrap(), r#"{"error":"csrf_rejected"}"#);
    assert_eq!(web.db.count("SELECT count(*) FROM core.records").await, 0);

    assert_eq!(plain("/v1/auth/logout").send().await.unwrap().status(), 403);
    assert_eq!(
        web.get("/v1/days/2026-10-25", &cookie)
            .send()
            .await
            .unwrap()
            .status(),
        200
    );

    let login = web
        .http
        .post(format!("{}/v1/auth/login", web.base))
        .json(&json!({ "username": USER, "password": PASSWORD }))
        .send()
        .await
        .unwrap();
    assert_eq!(login.status(), 403);
}

#[tokio::test]
async fn no_cross_origin_authorization_is_ever_given() {
    let web = Web::new().await;
    let cookie = web.login().await;
    let response = web
        .http
        .request(reqwest::Method::OPTIONS, format!("{}/v1/records", web.base))
        .header("cookie", &cookie)
        .header("origin", "https://evil.example")
        .header("access-control-request-method", "POST")
        .header("access-control-request-headers", api::CSRF_HEADER)
        .send()
        .await
        .unwrap();
    assert!(!response.status().is_success());
    assert!(
        !response
            .headers()
            .keys()
            .any(|name| name.as_str().starts_with("access-control-")),
        "{:?}",
        response.headers()
    );
}

// ─── Écriture par session ────────────────────────────────────────────────────

#[tokio::test]
async fn a_session_writes_as_webapp_and_only_as_webapp() {
    let web = Web::new().await;
    let cookie = web.login().await;
    let (ios, _) = web.db.source("ios", &["coffee.cup"]).await;
    let response = web
        .post("/v1/records", &cookie)
        .header("x-client-version", "web-test")
        .json(&json!({
            "source": "ios",
            "records": [{
                "key": "k", "kind": "coffee.cup", "schema_version": 1, "source": "ios",
                "source_id": ios, "occurred_at": "2026-10-13T09:00:00+02:00", "payload": {},
            }],
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let source: String = sqlx::query_scalar(
        "SELECT s.name FROM core.records r JOIN core.sources s ON s.id = r.source_id",
    )
    .fetch_one(&web.db.app)
    .await
    .unwrap();
    assert_eq!(source, "webapp");
    let version: Option<String> = sqlx::query_scalar(
        "SELECT client_version FROM core.sources WHERE name = 'webapp' AND last_seen_at IS NOT NULL",
    )
    .fetch_one(&web.db.app)
    .await
    .unwrap();
    assert_eq!(version.as_deref(), Some("web-test"));
}

#[tokio::test]
async fn a_session_goes_through_the_same_validation_as_a_token() {
    let web = Web::new().await;
    let cookie = web.login().await;
    let results = web
        .write(
            &cookie,
            json!([
                { "key": "a", "kind": "lastfm.scrobble", "schema_version": 1,
                  "occurred_at": "2026-10-13T09:00:00+02:00", "payload": {} },
                { "key": "b", "kind": "coffee.cup", "schema_version": 1,
                  "occurred_at": "2999-01-01T00:00:00+01:00", "payload": {} },
                { "key": "c", "kind": "coffee.cup", "schema_version": 1, "payload": {} },
            ]),
        )
        .await;
    let reasons: Vec<&str> = results
        .iter()
        .map(|r| r["reason"].as_str().unwrap())
        .collect();
    assert_eq!(
        reasons,
        [
            "kind_not_declared",
            "occurred_at_too_far_in_future",
            "missing_field"
        ]
    );
    let rejected = web
        .db
        .count("SELECT rejected_count FROM core.sources WHERE name = 'webapp'")
        .await;
    assert_eq!(rejected, 3);
}

#[tokio::test]
async fn a_bad_token_is_not_rescued_by_a_session() {
    let web = Web::new().await;
    let cookie = web.login().await;
    let response = web
        .post("/v1/records", &cookie)
        .bearer_auth("mauvais")
        .json(&json!({ "records": [] }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn a_token_wins_over_a_session_and_keeps_its_source() {
    let web = Web::new().await;
    let cookie = web.login().await;
    let (ios, token) = web.db.source("ios", &["coffee.cup"]).await;
    let response = web
        .post("/v1/records", &cookie)
        .bearer_auth(&token)
        .json(&json!({ "records": [cup("k", "2026-10-13T09:00:00+02:00")] }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let source: i16 = sqlx::query_scalar("SELECT source_id FROM core.records")
        .fetch_one(&web.db.app)
        .await
        .unwrap();
    assert_eq!(source, ios);
}

#[tokio::test]
async fn a_token_needs_no_session_nor_csrf_header() {
    let web = Web::new().await;
    let (_, token) = web.db.source("ios", &["coffee.cup"]).await;
    let results = web
        .write_with_token(&token, json!([cup("k", "2026-10-13T09:00:00+02:00")]))
        .await;
    assert_eq!(statuses(&results), ["accepted"]);
    let heartbeat = web
        .http
        .post(format!("{}/v1/heartbeat", web.base))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(heartbeat.status(), 200);
}

#[tokio::test]
async fn a_session_cannot_send_a_heartbeat() {
    let web = Web::new().await;
    let cookie = web.login().await;
    assert_eq!(
        web.post("/v1/heartbeat", &cookie)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
}

#[tokio::test]
async fn a_double_send_of_one_click_counts_one_coffee() {
    let web = Web::new().await;
    let cookie = web.login().await;
    let click = json!([cup("un-clic", "2026-10-13T09:00:00+02:00")]);
    // Le double envoi simultané, puis le renvoi après une réponse perdue.
    let (first, second) = tokio::join!(
        web.write(&cookie, click.clone()),
        web.write(&cookie, click.clone())
    );
    let mut both = vec![
        first[0]["status"].as_str().unwrap().to_owned(),
        second[0]["status"].as_str().unwrap().to_owned(),
    ];
    both.sort();
    assert_eq!(both, ["accepted", "duplicate"]);
    assert_eq!(statuses(&web.write(&cookie, click).await), ["duplicate"]);
    web.wait_for_worker().await;
    assert_eq!(web.day(&cookie, "2026-10-13").await["count"], 1);
}

#[tokio::test]
async fn a_session_cancels_a_cup_from_another_source() {
    let web = Web::new().await;
    let cookie = web.login().await;
    let (_, token) = web.db.source("ios", &["coffee.cup"]).await;
    web.write_with_token(
        &token,
        json!([cup("du-raccourci", "2026-10-13T09:00:00+02:00")]),
    )
    .await;
    web.write(&cookie, json!([cup("du-web", "2026-10-13T10:00:00+02:00")]))
        .await;
    let day = web.day_with_count(&cookie, "2026-10-13", 2).await;
    assert_eq!(day["cups"][0]["source"], "ios");
    assert_eq!(day["cups"][0]["key"], "du-raccourci");
    assert_eq!(day["cups"][1]["source"], "webapp");

    let target = &day["cups"][0];
    web.write(
        &cookie,
        json!([cancel(
            "annulation",
            target["source"].as_str().unwrap(),
            target["key"].as_str().unwrap()
        )]),
    )
    .await;
    let day = web.day_with_count(&cookie, "2026-10-13", 1).await;
    assert_eq!(day["cups"][0]["key"], "du-web");
}

// ─── Lecture d'un jour ───────────────────────────────────────────────────────

#[tokio::test]
async fn the_day_of_25_hours_keeps_its_edges() {
    let web = Web::new().await;
    let cookie = web.login().await;
    let (_, token) = web.db.source("ios", &["coffee.cup"]).await;
    web.write_with_token(
        &token,
        json!([
            cup("avant", "2026-10-24T21:59:59Z"), // 23:59:59 le 24, heure d'été
            cup("minuit", "2026-10-24T22:00:00Z"), // 00:00 le 25
            cup("ete-0030", "2026-10-25T00:30:00+02:00"), // 22:30Z le 24
            cup("z-2230", "2026-10-24T22:30:00Z"), // le même instant, en Z
            cup("0230-ete", "2026-10-25T02:30:00+02:00"), // 02:30, premier passage
            cup("0230-hiver", "2026-10-25T02:30:00+01:00"), // 02:30, second passage
            cup("hiver-2330", "2026-10-25T23:30:00+01:00"), // 22:30Z le 25
            cup("fin", "2026-10-25T23:59:59+01:00"), // dernière seconde
            cup("apres", "2026-10-25T23:00:00Z"), // 00:00 le 26
        ]),
    )
    .await;
    let day = web.day_with_count(&cookie, "2026-10-25", 7).await;
    assert_eq!(day["date"], "2026-10-25");
    assert_eq!(
        times(&day),
        [
            "00:00", "00:30", "00:30", "02:30", "02:30", "23:30", "23:59"
        ]
    );
    let keys: Vec<&str> = day["cups"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["key"].as_str().unwrap())
        .collect();
    assert_eq!(keys[3..5], ["0230-ete", "0230-hiver"]);

    let before = web.day_with_count(&cookie, "2026-10-24", 1).await;
    assert_eq!(before["cups"][0]["key"], "avant");
    assert_eq!(before["cups"][0]["time"], "23:59");
    let after = web.day_with_count(&cookie, "2026-10-26", 1).await;
    assert_eq!(after["cups"][0]["key"], "apres");
    assert_eq!(after["cups"][0]["time"], "00:00");
}

#[tokio::test]
async fn the_day_of_23_hours_keeps_its_edges() {
    let web = Web::new().await;
    let cookie = web.login().await;
    let (_, token) = web.db.source("ios", &["coffee.cup"]).await;
    web.write_with_token(
        &token,
        json!([
            cup("avant", "2026-03-28T22:59:59Z"),
            cup("minuit", "2026-03-28T23:00:00Z"),
            cup("0330", "2026-03-29T01:30:00Z"),
            cup("fin", "2026-03-29T21:59:59Z"),
            cup("apres", "2026-03-29T22:00:00Z"),
        ]),
    )
    .await;
    let day = web.day_with_count(&cookie, "2026-03-29", 3).await;
    assert_eq!(times(&day), ["00:00", "03:30", "23:59"]);
}

#[tokio::test]
async fn an_empty_day_and_no_cache() {
    let web = Web::new().await;
    let cookie = web.login().await;
    let response = web
        .get("/v1/days/2026-10-25", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let day: Value = response.json().await.unwrap();
    assert_eq!(day, json!({ "date": "2026-10-25", "count": 0, "cups": [] }));
}

#[tokio::test]
async fn an_invalid_date_is_a_400() {
    let web = Web::new().await;
    let cookie = web.login().await;
    for date in [
        "pas-une-date",
        "2026-02-30",
        "2026-1-5",
        "2026-13-01",
        "20261025",
        "2026-10-25T00:00:00Z",
        "25-10-2026",
        "%20",
    ] {
        let response = web
            .get(&format!("/v1/days/{date}"), &cookie)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 400, "{date}");
        assert_eq!(
            response.text().await.unwrap(),
            r#"{"error":"invalid_date"}"#
        );
    }
}

/// Une valeur de bord ne doit jamais devenir une erreur 500.
#[tokio::test]
async fn extreme_dates_are_empty_days() {
    let web = Web::new().await;
    let cookie = web.login().await;
    for date in ["0000-01-01", "0001-01-01", "9999-12-31"] {
        assert_eq!(web.day(&cookie, date).await["count"], 0, "{date}");
    }
}

#[tokio::test]
async fn reads_only_see_live_events() {
    let web = Web::new().await;
    let cookie = web.login().await;
    // Un café annulé avant même d'arriver : la vue l'écarte.
    web.write(&cookie, json!([cancel("c", "webapp", "tardif")]))
        .await;
    web.write(
        &cookie,
        json!([
            cup("tardif", "2026-10-13T09:00:00+02:00"),
            cup("garde", "2026-10-13T10:00:00+02:00")
        ]),
    )
    .await;
    let day = web.day_with_count(&cookie, "2026-10-13", 1).await;
    assert_eq!(day["cups"][0]["key"], "garde");
    web.wait_for_worker().await;
    assert_eq!(web.day(&cookie, "2026-10-13").await["count"], 1);
}
