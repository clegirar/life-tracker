//! Le jeton de service du MCP et les lectures génériques (étape 9), sur un vrai PostgreSQL : une base
//! neuve par test, l'API en tracker_app, le jeton créé en tracker_admin, le vrai worker.
//! Le jeton lit, au niveau « pseudonymes », et n'écrit jamais : aucune route d'écriture ne l'accepte.

mod common;
use common::*;
use life_tracker::api::{self, READ_ROUTES};
use life_tracker::interpreters::INTERPRETERS;
use life_tracker::{admin, engine};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::sync::watch;

const PASSWORD: &str = "un mot de passe assez long";
const CSRF: (&str, &str) = (api::CSRF_HEADER, "1");

struct App {
    db: TestDb,
    base: String,
    http: reqwest::Client,
    cookie: String,
    service: String,
    lastfm: String,
    _stop: watch::Sender<bool>,
}

impl App {
    async fn new() -> Self {
        let db = TestDb::new().await;
        admin::set_password(&db.admin, "moi", PASSWORD)
            .await
            .unwrap();
        let service = admin::create_service_token(&db.admin, "mcp").await.unwrap();
        let lastfm = admin::create_source(&db.admin, "lastfm", Some("6 hours"))
            .await
            .unwrap();
        for kind in ["lastfm.scrobble", "lastfm.track_info"] {
            admin::grant_kind(&db.admin, "lastfm", kind).await.unwrap();
        }
        let (stop, stopped) = watch::channel(false);
        tokio::spawn(engine::run(
            db.app.clone(),
            INTERPRETERS,
            Duration::from_millis(20),
            stopped,
        ));
        let base = serve(db.app.clone()).await;
        let http = client();
        let login = http
            .post(format!("{base}/v1/auth/login"))
            .header(CSRF.0, CSRF.1)
            .json(&json!({"username": "moi", "password": PASSWORD}))
            .send()
            .await
            .unwrap();
        assert_eq!(login.status(), 204);
        let cookie = login.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned();
        Self {
            db,
            base,
            http,
            cookie,
            service,
            lastfm,
            _stop: stop,
        }
    }

    async fn send(&self, records: Vec<Value>) {
        let response = self
            .http
            .post(format!("{}/v1/records", self.base))
            .bearer_auth(&self.lastfm)
            .json(&json!({ "records": records }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body: Value = response.json().await.unwrap();
        for result in body["results"].as_array().unwrap() {
            assert_eq!(result["status"], "accepted", "{body}");
        }
    }

    async fn write_as_webapp(&self, records: Vec<Value>) {
        let response = self
            .http
            .post(format!("{}/v1/records", self.base))
            .header("cookie", &self.cookie)
            .header(CSRF.0, CSRF.1)
            .json(&json!({ "records": records }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
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
        panic!("le worker n'a pas rattrapé le journal");
    }

    /// Une lecture avec le jeton de service, comme le MCP.
    async fn read(&self, path: &str) -> (StatusCode, Value) {
        self.settled().await;
        let response = self
            .http
            .get(format!("{}{path}", self.base))
            .bearer_auth(&self.service)
            .send()
            .await
            .unwrap();
        let status = response.status();
        (status, response.json().await.unwrap_or(Value::Null))
    }

    async fn ok(&self, path: &str) -> Value {
        let (status, body) = self.read(path).await;
        assert_eq!(status, 200, "{path} : {body}");
        body
    }

    async fn journal(&self) -> i64 {
        self.db.count("SELECT count(*) FROM core.records").await
    }
}

fn scrobble(at: &str, artist: &str, track: &str, album: &str) -> Value {
    json!({
        "key": format!("{at}|{artist}|{track}"),
        "kind": "lastfm.scrobble",
        "schema_version": 1,
        "occurred_at": at,
        "payload": {
            "artist": {"mbid": "", "#text": artist},
            "album": {"mbid": "", "#text": album},
            "name": track,
            "date": {"uts": "0"},
        },
    })
}

fn info(key: &str, artist: &str, track: &str, ms: &str) -> Value {
    json!({
        "key": key,
        "kind": "lastfm.track_info",
        "schema_version": 1,
        "occurred_at": "2026-10-16T00:00:00Z",
        "payload": {"artist": artist, "track": track, "info": {"track": {"duration": ms}}},
    })
}

fn cup(key: &str, at: &str) -> Value {
    json!({"key": key, "kind": "coffee.cup", "schema_version": 1, "occurred_at": at, "payload": {}})
}

fn mark(key: &str, level: &str, artist: &str) -> Value {
    json!({"key": key, "kind": "music.mark_known", "schema_version": 1,
           "occurred_at": "2026-10-20T10:00:00+02:00",
           "payload": {"level": level, "artist": artist}})
}

/// Une URL concrète par route de lecture, avec des paramètres valides.
fn read_paths() -> Vec<String> {
    let paths = vec![
        "/v1/days/2026-10-16".to_owned(),
        "/v1/days/2026-10-16/music".to_owned(),
        "/v1/months/2026-10".to_owned(),
        "/v1/sources/health".to_owned(),
        "/v1/inventory".to_owned(),
        "/v1/search?q=daft".to_owned(),
        "/v1/occurrences?metric=coffee&from=2026-10-01&to=2026-10-31".to_owned(),
        "/v1/durations?metric=music.listen&from=2026-10-01&to=2026-10-31".to_owned(),
        "/v1/item?metric=music.listen&level=artist&key=daft%20punk".to_owned(),
        "/v1/top?metric=music.listen&level=artist&from=2026-10-01&to=2026-10-31".to_owned(),
        "/v1/discoveries?metric=music.listen&level=artist&from=2026-10-01&to=2026-10-31".to_owned(),
        "/v1/route?id=1:0".to_owned(),
    ];
    assert_eq!(
        paths.len(),
        READ_ROUTES.len(),
        "une URL par route de lecture"
    );
    paths
}

async fn seed(app: &App) {
    app.send(vec![
        scrobble("2026-10-10T18:00:00Z", "Justice", "Genesis", "Cross"),
        scrobble(
            "2026-10-16T08:00:00Z",
            "Daft Punk",
            "Veridis Quo",
            "Discovery",
        ),
        scrobble(
            "2026-10-16T09:00:00Z",
            "DAFT PUNK",
            "Veridis Quo (Remastered)",
            "Discovery",
        ),
        scrobble("2026-10-16T10:00:00Z", "Daft Punk", "Voyager", "Discovery"),
        scrobble("2026-10-16T11:00:00Z", "Justice", "Genesis", "Cross"),
        scrobble("2026-10-17T21:30:00Z", "Air", "La femme d'argent", ""),
        info("a", "Daft Punk", "Veridis Quo", "345000"),
        info("b", "Daft Punk", "Voyager", "227000"),
    ])
    .await;
    // 06:30 et 10:00 UTC le 16 : 08:30 et 12:00 à Paris ; 22:15 UTC le 16 : 00:15 le 17.
    app.write_as_webapp(vec![
        cup("c1", "2026-10-16T06:30:00Z"),
        cup("c2", "2026-10-16T10:00:00Z"),
        cup("c3", "2026-10-16T22:15:00Z"),
        cup("c4", "2026-10-17T07:05:00+02:00"),
    ])
    .await;
}

// ─── Le jeton de service ─────────────────────────────────────────────────────

#[tokio::test]
async fn the_service_token_reads_every_read_route() {
    let app = App::new().await;
    seed(&app).await;
    for path in read_paths() {
        if path.starts_with("/v1/route") {
            // Sans zone de confidentialité, aucun parcours pour le jeton (voir tests/garmin.rs).
            assert_eq!(app.read(&path).await.0, 403, "{path}");
            continue;
        }
        app.ok(&path).await;
    }
}

#[tokio::test]
async fn the_service_token_never_writes() {
    let app = App::new().await;
    seed(&app).await;
    app.settled().await;
    let before = app.journal().await;
    let c = &app.http;
    let base = &app.base;
    let t = &app.service;
    let records = |records: Value| json!({ "records": records });
    let attempts = [
        (
            "café",
            c.post(format!("{base}/v1/records"))
                .bearer_auth(t)
                .json(&records(json!([cup("par-le-mcp", "2026-10-16T12:00:00Z")]))),
        ),
        (
            "annulation",
            c.post(format!("{base}/v1/records"))
                .bearer_auth(t)
                .json(&records(json!([{
                    "key": "annule", "kind": "record.cancel", "schema_version": 1,
                    "occurred_at": "2026-10-16T12:00:00Z",
                    "payload": {"source": "webapp", "key": "c1"}}]))),
        ),
        (
            "déjà connu",
            c.post(format!("{base}/v1/records"))
                .bearer_auth(t)
                .json(&records(json!([mark("connu", "artist", "Daft Punk")]))),
        ),
        (
            "jeton et cookie : pas de repli sur la session",
            c.post(format!("{base}/v1/records"))
                .bearer_auth(t)
                .header("cookie", &app.cookie)
                .header(CSRF.0, CSRF.1)
                .json(&records(json!([cup("repli", "2026-10-16T12:00:00Z")]))),
        ),
        (
            "signal de vie",
            c.post(format!("{base}/v1/heartbeat")).bearer_auth(t),
        ),
        (
            "déconnexion",
            c.post(format!("{base}/v1/auth/logout"))
                .bearer_auth(t)
                .header(CSRF.0, CSRF.1),
        ),
        (
            "déconnexion, jeton et cookie",
            c.post(format!("{base}/v1/auth/logout"))
                .bearer_auth(t)
                .header("cookie", &app.cookie)
                .header(CSRF.0, CSRF.1),
        ),
        (
            "route inconnue",
            c.get(format!("{base}/v1/inconnu")).bearer_auth(t),
        ),
        (
            "année : une lecture de la web app, hors des routes du MCP",
            c.get(format!("{base}/v1/years/2026")).bearer_auth(t),
        ),
    ];
    for (name, attempt) in attempts {
        let response = attempt.send().await.unwrap();
        assert_eq!(response.status(), 401, "{name}");
    }
    // Toute autre méthode sur une route de lecture.
    for path in read_paths() {
        for method in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
            let response = c
                .request(method.clone(), format!("{base}{path}"))
                .bearer_auth(t)
                .header(CSRF.0, CSRF.1)
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 401, "{method} {path}");
        }
    }
    assert_eq!(app.journal().await, before, "rien n'est entré au journal");
    // La session n'a pas été fermée par la tentative de déconnexion.
    let still = c
        .get(format!("{base}/v1/days/2026-10-16"))
        .header("cookie", &app.cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(still.status(), 200);
}

#[tokio::test]
async fn a_revoked_or_unknown_token_reads_nothing() {
    let app = App::new().await;
    let unknown = "0".repeat(64);
    let source = admin::create_source(&app.db.admin, "ios", None)
        .await
        .unwrap();
    for token in [&unknown, &source, &app.lastfm] {
        let response = app
            .http
            .get(format!("{}/v1/inventory", app.base))
            .bearer_auth(token)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 401);
    }
    assert_eq!(app.read("/v1/inventory").await.0, 200);
    admin::revoke_service_token(&app.db.admin, "mcp")
        .await
        .unwrap();
    for path in read_paths() {
        assert_eq!(app.read(&path).await.0, 401, "{path}");
    }
    // Un jeton ne se rattrape pas par la session, même sur une lecture.
    let both = app
        .http
        .get(format!("{}/v1/inventory", app.base))
        .bearer_auth(&app.service)
        .header("cookie", &app.cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(both.status(), 401);
}

#[tokio::test]
async fn the_new_routes_require_the_session_or_the_token() {
    let app = App::new().await;
    for path in read_paths() {
        let anonymous = app
            .http
            .get(format!("{}{path}", app.base))
            .send()
            .await
            .unwrap();
        assert_eq!(anonymous.status(), 401, "{path}");
        let session = app
            .http
            .get(format!("{}{path}", app.base))
            .header("cookie", &app.cookie)
            .send()
            .await
            .unwrap();
        assert_ne!(session.status(), 401, "{path}");
    }
}

#[tokio::test]
async fn admin_creates_and_revokes_service_tokens() {
    let app = App::new().await;
    assert!(
        admin::create_service_token(&app.db.admin, "mcp")
            .await
            .is_err(),
        "un nom ne se réutilise pas"
    );
    let hashed = app
        .db
        .count("SELECT count(*) FROM core.service_tokens WHERE octet_length(token_hash) = 32")
        .await;
    assert_eq!(hashed, 1);
    assert_eq!(app.service.len(), 64);
    admin::revoke_service_token(&app.db.admin, "mcp")
        .await
        .unwrap();
    assert!(
        admin::revoke_service_token(&app.db.admin, "mcp")
            .await
            .is_err()
    );
}

// ─── Les lectures génériques ─────────────────────────────────────────────────

#[tokio::test]
async fn inventory_lists_the_registry_and_its_sources() {
    let app = App::new().await;
    seed(&app).await;
    let inventory = app.ok("/v1/inventory").await;
    assert_eq!(inventory["timezone"], "Europe/Paris");
    let metric = |name: &str| {
        inventory["metrics"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["metric"] == name)
            .unwrap()
            .clone()
    };
    let coffee = metric("coffee");
    assert_eq!(coffee["events"], 4);
    assert_eq!(coffee["sources"][0]["source"], "webapp");
    let listen = metric("music.listen");
    assert_eq!(listen["events"], 6);
    assert_eq!(listen["item_levels"], json!(["artist", "album", "track"]));
    assert_eq!(listen["discovery"], true);
    assert_eq!(listen["duration_from"], "music.track_duration");
    assert_eq!(listen["first_at"], "2026-10-10T18:00:00Z");
}

#[tokio::test]
async fn first_coffee_of_each_day_in_paris_time() {
    let app = App::new().await;
    seed(&app).await;
    let body = app
        .ok("/v1/occurrences?metric=coffee&from=2026-10-16&to=2026-10-17&form=first_last")
        .await;
    assert_eq!(body["form"], "first_last");
    assert_eq!(body["total"], 4);
    assert_eq!(
        body["days"],
        json!([
            {"date": "2026-10-16", "first": "08:30", "last": "12:00", "count": 2},
            {"date": "2026-10-17", "first": "00:15", "last": "07:05", "count": 2},
        ])
    );
    let daily = app
        .ok("/v1/occurrences?metric=coffee&from=2026-10-01&to=2026-10-31&form=daily")
        .await;
    assert_eq!(
        daily["days"],
        json!([{"date": "2026-10-16", "count": 2}, {"date": "2026-10-17", "count": 2}])
    );
}

#[tokio::test]
async fn occurrences_of_one_track_with_their_context() {
    let app = App::new().await;
    seed(&app).await;
    let found = app.ok("/v1/search?q=veridis&metric=music.listen").await;
    let first = &found[0];
    assert_eq!(first["level"], "track");
    assert_eq!(first["key"], "daft punk\tveridis quo");
    assert_eq!(first["occurrences"], 2);
    assert_eq!(first["name"], "Veridis Quo (Remastered)");
    assert_eq!(first["context"]["artist"], "DAFT PUNK");
    // Un nom approximatif.
    let fuzzy = app.ok("/v1/search?q=veridis%20qou&level=track").await;
    assert_eq!(fuzzy[0]["key"], "daft punk\tveridis quo", "{fuzzy}");

    let list = app
        .ok("/v1/occurrences?metric=music.listen&level=track&key=daft%20punk%09veridis%20quo&from=2026-10-01&to=2026-10-31")
        .await;
    assert_eq!(list["total"], 2);
    assert_eq!(list["truncated"], false);
    assert_eq!(list["occurrences"][0]["local"], "2026-10-16 10:00");
    assert_eq!(list["occurrences"][0]["items"]["album"], "Discovery");
    let limited = app
        .ok("/v1/occurrences?metric=music.listen&from=2026-10-01&to=2026-10-31&limit=2")
        .await;
    assert_eq!(limited["total"], 6);
    assert_eq!(limited["truncated"], true);
    assert_eq!(limited["occurrences"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn minutes_per_day_and_per_week() {
    let app = App::new().await;
    seed(&app).await;
    let days = app
        .ok("/v1/durations?metric=music.listen&from=2026-10-12&to=2026-10-18&group=day")
        .await;
    // 2 × 345 s + 227 s = 917 s, soit 15 min ; Genesis et Air n'ont pas de durée connue.
    assert_eq!(days["total_minutes"], 15);
    assert_eq!(days["events"], 5);
    assert_eq!(days["events_without_duration"], 2);
    assert_eq!(days["periods"][0]["period"], "2026-10-16");
    assert_eq!(days["periods"][0]["minutes"], 15);
    assert_eq!(days["periods"][1]["period"], "2026-10-17");
    assert_eq!(days["periods"][1]["minutes"], Value::Null);
    let weeks = app
        .ok("/v1/durations?metric=music.listen&from=2026-10-01&to=2026-10-31&group=week")
        .await;
    let periods: Vec<&str> = weeks["periods"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["period"].as_str().unwrap())
        .collect();
    assert_eq!(periods, ["2026-W41", "2026-W42"]);
    let one = app
        .ok("/v1/durations?metric=music.listen&level=artist&key=daft%20punk&from=2026-10-01&to=2026-10-31&group=month")
        .await;
    assert_eq!(
        one["periods"],
        json!([{"period": "2026-10", "minutes": 15, "events": 3, "events_without_duration": 0}])
    );
}

#[tokio::test]
async fn first_and_last_time_of_an_artist_and_its_known_marks() {
    let app = App::new().await;
    seed(&app).await;
    let justice = app
        .ok("/v1/item?metric=music.listen&level=artist&key=justice")
        .await;
    assert_eq!(justice["occurrences"], 2);
    assert_eq!(justice["first_local"], "2026-10-10 20:00");
    assert_eq!(justice["last_local"], "2026-10-16 13:00");
    assert_eq!(justice["discovery"], true);
    assert_eq!(justice["known_marks"], json!([]));

    app.write_as_webapp(vec![mark("m1", "artist", "Justice")])
        .await;
    let marked = app
        .ok("/v1/item?metric=music.listen&level=artist&key=justice")
        .await;
    assert_eq!(marked["known_marks"][0]["source"], "webapp");

    let (status, _) = app
        .read("/v1/item?metric=music.listen&level=artist&key=inconnu")
        .await;
    assert_eq!(status, 404);
}

/// L'exemple de référence : une écoute à trois artistes compte trois artistes uniques, mais une seule
/// écoute avec un artiste. Le contexte d'un titre joint les artistes crédités dans leur ordre.
#[tokio::test]
async fn top_counts_each_credited_artist_and_each_listen_once() {
    let app = App::new().await;
    app.send(vec![scrobble(
        "2026-10-20T08:00:00Z",
        "Zek, Jungle Jack & Caballero",
        "T",
        "GRAND LUXXXE Vol.1",
    )])
    .await;
    let path = |level: &str| {
        format!("/v1/top?metric=music.listen&level={level}&from=2026-10-20&to=2026-10-20")
    };
    let artists = app.ok(&path("artist")).await;
    assert_eq!(artists["distinct_items"], 3);
    assert_eq!(artists["occurrences_with_item"], 1);
    let names: Vec<&str> = artists["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Caballero", "Jungle Jack", "Zek"]);
    let tracks = app.ok(&path("track")).await;
    assert_eq!(tracks["distinct_items"], 1);
    assert_eq!(tracks["occurrences_with_item"], 1);
    assert_eq!(
        tracks["items"][0]["context"],
        json!({"artist": "Zek, Jungle Jack & Caballero", "album": "GRAND LUXXXE Vol.1"})
    );
    let albums = app.ok(&path("album")).await;
    assert_eq!(albums["items"][0]["key"], "zek\tgrand luxxxe vol.1");
    let occurrences = app
        .ok("/v1/occurrences?metric=music.listen&from=2026-10-20&to=2026-10-20&level=artist&key=jungle%20jack")
        .await;
    assert_eq!(occurrences["total"], 1);
    let durations = app
        .ok("/v1/durations?metric=music.listen&from=2026-10-20&to=2026-10-20")
        .await;
    assert_eq!(durations["events"], 1);
}

#[tokio::test]
async fn top_and_discoveries_over_a_period() {
    let app = App::new().await;
    seed(&app).await;
    let top = app
        .ok("/v1/top?metric=music.listen&level=artist&from=2026-10-16&to=2026-10-17&limit=1")
        .await;
    assert_eq!(top["distinct_items"], 3);
    assert_eq!(top["occurrences_with_item"], 5);
    assert_eq!(
        top["items"],
        json!([{"key": "daft punk", "name": "Daft Punk", "occurrences": 3}])
    );
    let albums = app
        .ok("/v1/top?metric=music.listen&level=album&from=2026-10-16&to=2026-10-16")
        .await;
    assert_eq!(albums["items"][0]["context"]["artist"], "Daft Punk");

    // Justice est écouté dès le 10 : ce n'est pas une découverte du 16.
    let discoveries = app
        .ok("/v1/discoveries?metric=music.listen&level=artist&from=2026-10-16&to=2026-10-17")
        .await;
    let names: Vec<&str> = discoveries["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Daft Punk", "Air"]);
    assert_eq!(discoveries["items"][0]["first_local"], "2026-10-16 10:00");
    assert_eq!(discoveries["marked_known"], 0);

    app.write_as_webapp(vec![mark("m2", "artist", "Daft Punk")])
        .await;
    let marked = app
        .ok("/v1/discoveries?metric=music.listen&level=artist&from=2026-10-16&to=2026-10-17")
        .await;
    assert_eq!(marked["total"], 1);
    assert_eq!(marked["marked_known"], 1);
    // Un marquage n'agit qu'à son niveau.
    let tracks = app
        .ok("/v1/discoveries?metric=music.listen&level=track&from=2026-10-16&to=2026-10-16")
        .await;
    assert_eq!(tracks["total"], 2);
}

#[tokio::test]
async fn invalid_parameters_name_the_parameter() {
    let app = App::new().await;
    for (path, field) in [
        (
            "/v1/occurrences?metric=inconnue&from=2026-10-01&to=2026-10-02",
            "metric",
        ),
        ("/v1/occurrences?from=2026-10-01&to=2026-10-02", "metric"),
        (
            "/v1/occurrences?metric=coffee&from=2026-10-1&to=2026-10-02",
            "from",
        ),
        (
            "/v1/occurrences?metric=coffee&from=2026-10-03&to=2026-10-02",
            "to",
        ),
        (
            "/v1/occurrences?metric=coffee&from=2026-10-01&to=2026-10-02&form=moyenne",
            "form",
        ),
        (
            "/v1/occurrences?metric=coffee&from=2026-10-01&to=2026-10-02&limit=0",
            "limit",
        ),
        (
            "/v1/occurrences?metric=coffee&from=2026-10-01&to=2026-10-02&level=artist&key=x",
            "level",
        ),
        (
            "/v1/occurrences?metric=music.listen&from=2026-10-01&to=2026-10-02&key=x",
            "level",
        ),
        (
            "/v1/durations?metric=coffee&from=2026-10-01&to=2026-10-02",
            "metric",
        ),
        (
            "/v1/durations?metric=music.listen&from=2026-10-01&to=2026-10-02&group=an",
            "group",
        ),
        (
            "/v1/discoveries?metric=coffee&level=artist&from=2026-10-01&to=2026-10-02",
            "metric",
        ),
        (
            "/v1/top?metric=music.listen&level=contact&from=2026-10-01&to=2026-10-02",
            "level",
        ),
        ("/v1/item?metric=music.listen&level=artist", "key"),
        ("/v1/search?q=a", "q"),
        ("/v1/search?q=daft&limit=51", "limit"),
    ] {
        let (status, body) = app.read(path).await;
        assert_eq!(status, 400, "{path}");
        assert_eq!(
            body,
            json!({"error": "invalid_parameter", "field": field}),
            "{path}"
        );
    }
}
