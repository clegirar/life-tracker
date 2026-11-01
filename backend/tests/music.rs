//! La musique, de l'API d'ingestion à l'API de lecture : interprètes, minutes en vue, santé des sources.

mod common;
use common::*;
use life_tracker::interpreters::INTERPRETERS;
use life_tracker::{admin, api, engine, replay};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::sync::watch;

const PASSWORD: &str = "un mot de passe assez long";

struct App {
    db: TestDb,
    base: String,
    cookie: String,
    token: String,
    http: reqwest::Client,
    _stop: watch::Sender<bool>,
}

impl App {
    async fn new() -> Self {
        let db = TestDb::new().await;
        admin::set_password(&db.admin, "moi", PASSWORD)
            .await
            .unwrap();
        let token = admin::create_source(&db.admin, "lastfm", Some("6 hours"))
            .await
            .unwrap();
        for kind in ["lastfm.scrobble", "lastfm.track_info", "record.cancel"] {
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
            .header(api::CSRF_HEADER, "1")
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
            cookie,
            token,
            http,
            _stop: stop,
        }
    }

    async fn send(&self, records: Vec<Value>) {
        let count = records.len();
        let response = self
            .http
            .post(format!("{}/v1/records", self.base))
            .bearer_auth(&self.token)
            .json(&json!({ "records": records }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["results"].as_array().unwrap().len(), count);
        assert!(
            body["results"]
                .as_array()
                .unwrap()
                .iter()
                .all(|r| r["status"] == "accepted"),
            "{body}"
        );
    }

    /// Le worker est asynchrone : attend qu'il ait lu tout le journal, sans dépendre de sa vitesse.
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

    async fn get(&self, path: &str) -> Value {
        let response = self
            .http
            .get(format!("{}{path}", self.base))
            .header("cookie", &self.cookie)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200, "{path}");
        response.json().await.unwrap()
    }

    async fn music(&self, date: &str) -> Value {
        self.settled().await;
        self.get(&format!("/v1/days/{date}/music")).await
    }

    async fn health(&self) -> Value {
        self.settled().await;
        self.get("/v1/sources/health").await
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

fn info(key: &str, at: &str, artist: &str, track: &str, duration: Value) -> Value {
    json!({
        "key": key,
        "kind": "lastfm.track_info",
        "schema_version": 1,
        "occurred_at": at,
        "payload": {"artist": artist, "track": track, "info": {"track": {"duration": duration}}},
    })
}

#[tokio::test]
async fn a_day_of_music() {
    let app = App::new().await;
    app.send(vec![
        // Deux orthographes d'un même titre, et l'autre titre du même artiste.
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
        // Sans album ni durée connue.
        scrobble("2026-10-16T12:00:00Z", "Air", "Sans fiche", ""),
        // Hors de la journée de Paris : la veille à 21:59 UTC, le lendemain à 22:00 UTC.
        scrobble("2026-10-15T21:59:59Z", "Avant", "Avant", "Avant"),
        scrobble("2026-10-16T22:00:00Z", "Après", "Après", "Après"),
        info(
            "a",
            "2026-10-16T08:01:00Z",
            "Daft Punk",
            "Veridis Quo",
            json!("345000"),
        ),
        info(
            "b",
            "2026-10-16T10:01:00Z",
            "Daft Punk",
            "Voyager",
            json!("227000"),
        ),
        info(
            "c",
            "2026-10-16T11:01:00Z",
            "Justice",
            "Genesis",
            json!("0"),
        ),
    ])
    .await;

    let day = app.music("2026-10-16").await;
    assert_eq!(day["date"], "2026-10-16");
    assert_eq!(day["listens"], 5);
    // 345 × 2 + 227 = 917 s ; Genesis (durée 0) et « Sans fiche » n'en ont pas.
    assert_eq!(day["minutes"], 15);
    assert_eq!(day["listens_without_duration"], 2);
    assert_eq!(
        day["artist_of_day"],
        json!({"name": "Daft Punk", "listens": 3})
    );
    assert_eq!(
        day["tracks"],
        json!([
            // La dernière orthographe vue. Égalités : par nom.
            {"name": "Veridis Quo (Remastered)", "artist": "DAFT PUNK", "listens": 2},
            {"name": "Genesis", "artist": "Justice", "listens": 1},
            {"name": "Sans fiche", "artist": "Air", "listens": 1},
            {"name": "Voyager", "artist": "Daft Punk", "listens": 1},
        ])
    );
    assert_eq!(
        day["artists"],
        json!([
            {"name": "Daft Punk", "tracks": 2, "listens": 3},
            {"name": "Air", "tracks": 1, "listens": 1},
            {"name": "Justice", "tracks": 1, "listens": 1},
        ])
    );
    assert_eq!(
        day["albums"],
        json!([
            {"name": "Discovery", "artist": "Daft Punk", "listens": 3},
            {"name": "Cross", "artist": "Justice", "listens": 1},
        ])
    );
}

#[tokio::test]
async fn an_empty_day_and_a_day_without_any_duration() {
    let app = App::new().await;
    let empty = app.music("2026-10-10").await;
    assert_eq!(
        empty,
        json!({"date": "2026-10-10", "listens": 0, "minutes": null, "listens_without_duration": 0,
               "artist_of_day": null, "tracks": [], "artists": [], "albums": []})
    );

    app.send(vec![scrobble("2026-10-16T08:00:00Z", "A", "T", "")])
        .await;
    let day = app.music("2026-10-16").await;
    assert_eq!(day["listens"], 1);
    assert_eq!(day["minutes"], Value::Null, "inconnu n'est pas zéro");
    assert_eq!(day["listens_without_duration"], 1);
}

#[tokio::test]
async fn the_duration_is_joined_at_read_time_whatever_the_order_of_arrival() {
    let app = App::new().await;
    app.send(vec![scrobble("2026-10-16T08:00:00Z", "A", "T", "")])
        .await;
    assert_eq!(app.music("2026-10-16").await["minutes"], Value::Null);

    // La fiche arrive après l'écoute : la journée se corrige sans toucher à l'événement.
    app.send(vec![info(
        "old",
        "2026-10-16T09:00:00Z",
        "A",
        "T",
        json!("60000"),
    )])
    .await;
    assert_eq!(app.music("2026-10-16").await["minutes"], 1);

    // La dernière durée connue l'emporte, d'après sa date et même arrivée avant.
    app.send(vec![
        info(
            "newer",
            "2026-10-18T09:00:00Z",
            "A",
            "T (Live)",
            json!("120000"),
        ),
        info("older", "2026-10-01T09:00:00Z", "A", "T", json!("600000")),
    ])
    .await;
    assert_eq!(app.music("2026-10-16").await["minutes"], 2);

    // Annuler la fiche la plus récente rend la précédente.
    app.send(vec![
        json!({"key": "x", "kind": "record.cancel", "schema_version": 1,
        "occurred_at": "2026-10-20T00:00:00Z", "payload": {"source": "lastfm", "key": "newer"}}),
    ])
    .await;
    assert_eq!(app.music("2026-10-16").await["minutes"], 1);
}

#[tokio::test]
async fn a_cancelled_listen_disappears_from_every_list() {
    let app = App::new().await;
    let listen = scrobble("2026-10-16T08:00:00Z", "A", "T", "L");
    let key = listen["key"].as_str().unwrap().to_owned();
    app.send(vec![listen]).await;
    assert_eq!(app.music("2026-10-16").await["listens"], 1);
    app.send(vec![
        json!({"key": "x", "kind": "record.cancel", "schema_version": 1,
        "occurred_at": "2026-10-20T00:00:00Z", "payload": {"source": "lastfm", "key": key}}),
    ])
    .await;
    let day = app.music("2026-10-16").await;
    assert_eq!(day["listens"], 0);
    assert_eq!(day["albums"], json!([]));
}

#[tokio::test]
async fn the_longest_and_the_shortest_days_keep_their_listens() {
    let app = App::new().await;
    app.send(vec![
        // Le 25 oct. 2026 dure 25 heures, de 22:00 UTC la veille à 23:00 UTC.
        scrobble("2026-10-24T21:59:59Z", "A", "Veille", ""),
        scrobble("2026-10-24T22:00:00Z", "A", "Premier", ""),
        scrobble("2026-10-25T22:59:59Z", "A", "Dernier", ""),
        scrobble("2026-10-25T23:00:00Z", "A", "Lendemain", ""),
    ])
    .await;
    assert_eq!(app.music("2026-10-25").await["listens"], 2);
    assert_eq!(app.music("2026-10-24").await["listens"], 1);
    assert_eq!(app.music("2026-10-26").await["listens"], 1);
}

#[tokio::test]
async fn invalid_dates_are_refused() {
    let app = App::new().await;
    let response = app
        .http
        .get(format!("{}/v1/days/pas-une-date/music", app.base))
        .header("cookie", &app.cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
}

#[tokio::test]
async fn both_read_routes_require_a_session() {
    let app = App::new().await;
    for path in ["/v1/days/2026-10-16/music", "/v1/sources/health"] {
        let url = format!("{}{path}", app.base);
        assert_eq!(
            app.http.get(&url).send().await.unwrap().status(),
            401,
            "{path}"
        );
        // Le token d'une source n'ouvre pas la lecture.
        let with_token = app
            .http
            .get(&url)
            .bearer_auth(&app.token)
            .send()
            .await
            .unwrap();
        assert_eq!(with_token.status(), 401, "{path}");
    }
}

async fn set_last_seen(app: &App, ago: &str) {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "UPDATE core.sources SET last_seen_at = now() - interval '{ago}' WHERE name = 'lastfm'"
    )))
    .execute(&app.db.app)
    .await
    .unwrap();
}

#[tokio::test]
async fn a_connector_silent_for_more_than_6_hours_is_reported() {
    let app = App::new().await;
    let lastfm = async || -> Value { app.health().await["sources"][0].clone() };

    // Tout juste créée, jamais vue : pas encore muette.
    let source = lastfm().await;
    assert_eq!(source["name"], "lastfm");
    assert_eq!(source["silent"], false);
    assert_eq!(source["last_seen_at"], Value::Null);
    assert_eq!(source["silence_threshold_seconds"], 21600.0);

    set_last_seen(&app, "5 hours 59 minutes").await;
    assert_eq!(lastfm().await["silent"], false);
    set_last_seen(&app, "6 hours 1 minute").await;
    assert_eq!(lastfm().await["silent"], true);

    // Un battement met fin au silence.
    let beat = app
        .http
        .post(format!("{}/v1/heartbeat", app.base))
        .bearer_auth(&app.token)
        .send()
        .await
        .unwrap();
    assert_eq!(beat.status(), 200);
    assert_eq!(lastfm().await["silent"], false);

    // Jamais vue depuis sa création, il y a longtemps : muette aussi. Le registre, en administrateur.
    sqlx::query("UPDATE core.sources SET last_seen_at = NULL WHERE name = 'lastfm'")
        .execute(&app.db.app)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE core.sources SET silence_threshold = interval '0 seconds' WHERE name = 'lastfm'",
    )
    .execute(&app.db.admin)
    .await
    .unwrap();
    assert_eq!(lastfm().await["silent"], true);
}

#[tokio::test]
async fn only_sources_with_a_threshold_can_be_silent_and_a_revoked_one_is_gone() {
    let app = App::new().await;
    let names = async || -> Vec<String> {
        app.health().await["sources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["name"].as_str().unwrap().to_owned())
            .collect()
    };
    admin::create_source(&app.db.admin, "ios", None)
        .await
        .unwrap();
    assert_eq!(
        names().await,
        ["lastfm"],
        "le café et l'iOS n'ont pas de seuil"
    );
    admin::revoke_source(&app.db.admin, "lastfm").await.unwrap();
    assert!(names().await.is_empty());
}

#[tokio::test]
async fn no_new_listen_for_3_days_is_to_be_checked() {
    let app = App::new().await;
    let music = async || -> Value {
        let health = app.health().await;
        let metrics = health["metrics"].as_array().unwrap();
        assert_eq!(
            metrics.len(),
            1,
            "seule music.listen a un seuil de fraîcheur"
        );
        metrics[0].clone()
    };
    let at =
        |ago_hours: i64| (chrono::Utc::now() - chrono::Duration::hours(ago_hours)).to_rfc3339();

    // Rien n'a jamais été reçu : ce n'est pas la même chose qu'un arrêt.
    let empty = music().await;
    assert_eq!(empty["metric"], "music.listen");
    assert_eq!(empty["stale"], false);
    assert_eq!(empty["last_event_at"], Value::Null);
    assert_eq!(empty["freshness_threshold_seconds"], 259200.0);

    app.send(vec![scrobble(&at(73), "A", "Ancienne", "")]).await;
    let old = music().await;
    assert_eq!(old["stale"], true);
    assert!(old["first_event_at"].is_string());

    // La fraîcheur est indépendante du signal de vie : le connecteur peut aller bien.
    assert_eq!(app.health().await["sources"][0]["silent"], false);

    app.send(vec![scrobble(&at(71), "A", "Récente", "")]).await;
    assert_eq!(music().await["stale"], false);
}

#[tokio::test]
async fn a_bad_payload_is_set_aside_and_the_rest_goes_on() {
    let app = App::new().await;
    let mut bad = scrobble("2026-10-16T08:00:00Z", "A", "T", "");
    bad["key"] = json!("bad");
    bad["payload"] = json!({"unexpected": true});
    app.send(vec![bad, scrobble("2026-10-16T09:00:00Z", "A", "T", "")])
        .await;
    assert_eq!(app.music("2026-10-16").await["listens"], 1);
    assert_eq!(
        app.db
            .count("SELECT count(*) FROM derived.interpretation_errors")
            .await,
        1
    );
    let stored: String = sqlx::query_scalar("SELECT error FROM derived.interpretation_errors")
        .fetch_one(&app.db.app)
        .await
        .unwrap();
    assert_eq!(stored, "lastfm.scrobble: missing artist");
}

#[tokio::test]
async fn a_rebuild_gives_the_same_music() {
    let app = App::new().await;
    app.send(vec![
        scrobble(
            "2026-10-16T08:00:00Z",
            "Daft Punk feat. X",
            "Veridis Quo (Live)",
            "Discovery",
        ),
        scrobble("2026-10-16T09:00:00Z", "A", "T", ""),
        info(
            "a",
            "2026-10-16T08:01:00Z",
            "Daft Punk",
            "Veridis Quo",
            json!("345000"),
        ),
        info("c", "2026-10-16T08:02:00Z", "A", "T", json!("0")),
    ])
    .await;
    let before = app.music("2026-10-16").await;
    let report = replay::rebuild(&app.db.admin, INTERPRETERS, false)
        .await
        .unwrap();
    assert!(report.switched);
    assert_eq!(report.comparison.differences, vec![]);
    assert_eq!(app.music("2026-10-16").await, before);
}
