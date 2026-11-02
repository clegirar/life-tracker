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
        json!({"date": "2026-10-10", "listens": 0, "minutes": null, "listens_without_duration": 0, "listens_without_artist": 0,
               "artist_of_day": null, "tracks": [], "artists": [], "albums": [],
               "discoveries": 0, "first_listens": []})
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
    for path in [
        "/v1/days/2026-10-16/music",
        "/v1/sources/health",
        "/v1/months/2026-10",
    ] {
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

// ─── Découvertes et « déjà connu » ───────────────────────────────────────────

impl App {
    /// Écrit comme la web app : par la session, avec l'en-tête anti-CSRF. La source vient de la session.
    async fn write_as_webapp(&self, records: Vec<Value>) {
        let response = self
            .http
            .post(format!("{}/v1/records", self.base))
            .header("cookie", &self.cookie)
            .header(api::CSRF_HEADER, "1")
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

    async fn mark(&self, key: &str, payload: Value) {
        self.write_as_webapp(vec![mark_known(key, payload)]).await;
    }

    async fn cancel_mark(&self, cancel_key: &str, key: &str) {
        self.write_as_webapp(vec![json!({
            "key": cancel_key, "kind": "record.cancel", "schema_version": 1,
            "occurred_at": "2026-10-20T00:00:00Z",
            "payload": {"source": "webapp", "key": key},
        })])
        .await;
    }

    /// (niveau, nom) des découvertes du jour : sans les éléments marqués.
    async fn discoveries(&self, date: &str) -> Vec<(String, String)> {
        let day = self.music(date).await;
        let found: Vec<_> = day["first_listens"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|f| f["known_marks"].as_array().unwrap().is_empty())
            .map(|f| {
                (
                    f["level"].as_str().unwrap().to_owned(),
                    f["name"].as_str().unwrap().to_owned(),
                )
            })
            .collect();
        assert_eq!(day["discoveries"], found.len());
        found
    }
}

fn mark_known(key: &str, payload: Value) -> Value {
    json!({"key": key, "kind": "music.mark_known", "schema_version": 1,
           "occurred_at": "2026-10-20T00:00:00Z", "payload": payload})
}

fn found(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(level, name)| ((*level).to_owned(), (*name).to_owned()))
        .collect()
}

#[tokio::test]
async fn the_daft_punk_example_of_the_architecture() {
    let app = App::new().await;
    app.mark("m", json!({"level": "artist", "artist": "Daft Punk"}))
        .await;
    app.send(vec![scrobble(
        "2026-10-16T08:00:00Z",
        "Daft Punk",
        "Veridis Quo",
        "Discovery",
    )])
    .await;
    assert_eq!(
        app.discoveries("2026-10-16").await,
        found(&[("album", "Discovery"), ("track", "Veridis Quo")]),
        "Daft Punk n'est pas un artiste découvert, mais le titre est bien un titre découvert"
    );
}

#[tokio::test]
async fn a_discovery_is_the_first_listen_of_a_life_at_each_level() {
    let app = App::new().await;
    app.send(vec![
        scrobble("2026-10-14T08:00:00Z", "A", "Un", "L1"),
        // Le 16 : un titre connu, un nouveau titre du même album, puis un nouvel album du même artiste.
        scrobble("2026-10-16T08:00:00Z", "A", "Un", "L1"),
        scrobble("2026-10-16T09:00:00Z", "A", "Deux", "L1"),
        scrobble("2026-10-16T10:00:00Z", "A", "Trois", "L2"),
        scrobble("2026-10-16T11:00:00Z", "B", "Quatre", ""),
        // Réécouté le même jour : une seule découverte.
        scrobble("2026-10-16T12:00:00Z", "B", "Quatre", ""),
    ])
    .await;
    assert_eq!(
        app.discoveries("2026-10-14").await,
        found(&[("artist", "A"), ("album", "L1"), ("track", "Un")])
    );
    assert_eq!(
        app.discoveries("2026-10-16").await,
        found(&[
            ("artist", "B"),
            ("album", "L2"),
            ("track", "Deux"),
            ("track", "Trois"),
            ("track", "Quatre"),
        ])
    );
    assert_eq!(app.discoveries("2026-10-17").await, found(&[]));
}

#[tokio::test]
async fn the_day_of_a_discovery_is_the_day_in_paris() {
    let app = App::new().await;
    // 22:30 UTC le 15 : 00:30 le 16 à Paris. Le 25 oct. 2026 dure 25 heures.
    app.send(vec![
        scrobble("2026-10-15T22:30:00Z", "A", "T", ""),
        scrobble("2026-10-25T22:30:00Z", "B", "T", ""),
    ])
    .await;
    assert_eq!(app.discoveries("2026-10-15").await, found(&[]));
    assert_eq!(
        app.discoveries("2026-10-16").await,
        found(&[("artist", "A"), ("track", "T")])
    );
    assert_eq!(
        app.discoveries("2026-10-25").await,
        found(&[("artist", "B"), ("track", "T")])
    );
}

#[tokio::test]
async fn the_past_can_change_a_discovery() {
    let app = App::new().await;
    let first = scrobble("2026-10-10T08:00:00Z", "A", "T", "L");
    let first_key = first["key"].as_str().unwrap().to_owned();
    app.send(vec![scrobble("2026-10-16T08:00:00Z", "A", "T", "L")])
        .await;
    let all = found(&[("artist", "A"), ("album", "L"), ("track", "T")]);
    assert_eq!(app.discoveries("2026-10-16").await, all);

    // Une écoute plus ancienne arrive en retard : la découverte recule.
    app.send(vec![first]).await;
    assert_eq!(app.discoveries("2026-10-16").await, found(&[]));
    assert_eq!(app.discoveries("2026-10-10").await, all);

    // Annuler cette écoute rétablit l'ancienne découverte.
    app.send(vec![
        json!({"key": "x", "kind": "record.cancel", "schema_version": 1,
        "occurred_at": "2026-10-20T00:00:00Z", "payload": {"source": "lastfm", "key": first_key}}),
    ])
    .await;
    assert_eq!(app.discoveries("2026-10-16").await, all);
    assert_eq!(app.discoveries("2026-10-10").await, found(&[]));
}

#[tokio::test]
async fn a_mark_acts_only_at_its_own_level() {
    for (payload, expected) in [
        (
            json!({"level": "artist", "artist": "A"}),
            [("album", "L"), ("track", "T")],
        ),
        (
            json!({"level": "album", "artist": "A", "album": "L"}),
            [("artist", "A"), ("track", "T")],
        ),
        (
            json!({"level": "track", "artist": "A", "track": "T"}),
            [("artist", "A"), ("album", "L")],
        ),
    ] {
        let app = App::new().await;
        app.send(vec![scrobble("2026-10-16T08:00:00Z", "A", "T", "L")])
            .await;
        app.mark("m", payload).await;
        assert_eq!(app.discoveries("2026-10-16").await, found(&expected));
        let day = app.music("2026-10-16").await;
        assert_eq!(day["listens"], 1, "l'écoute elle-même ne change pas");
        assert_eq!(day["first_listens"].as_array().unwrap().len(), 3);
    }

    // Un homonyme n'est pas touché : un album se lit avec son artiste, et un nom d'artiste n'est pas un titre.
    let app = App::new().await;
    app.send(vec![
        scrobble("2026-10-16T08:00:00Z", "A", "Même nom", "Même nom"),
        scrobble("2026-10-16T09:00:00Z", "B", "Même nom", "Même nom"),
    ])
    .await;
    app.mark(
        "m",
        json!({"level": "album", "artist": "A", "album": "Même nom"}),
    )
    .await;
    assert_eq!(
        app.discoveries("2026-10-16").await,
        found(&[
            ("artist", "A"),
            ("artist", "B"),
            ("album", "Même nom"),
            ("track", "Même nom"),
            ("track", "Même nom"),
        ])
    );
}

/// Ce que renvoie l'API suffit à marquer, quelle que soit l'orthographe : c'est ce que fait la web app.
#[tokio::test]
async fn a_mark_from_the_displayed_names_covers_every_spelling() {
    let app = App::new().await;
    app.send(vec![
        scrobble("2026-10-16T08:00:00Z", "Daft Punk", "Voyager", "Discovery"),
        scrobble(
            "2026-10-16T09:00:00Z",
            "DAFT PUNK feat. X",
            "Voyager (Live) - Remastered",
            "DISCOVERY ",
        ),
    ])
    .await;
    let day = app.music("2026-10-16").await;
    let rows = day["first_listens"].as_array().unwrap();
    assert_eq!(rows.len(), 3, "{rows:?}");
    assert_eq!(rows[0]["name"], "DAFT PUNK feat. X", "le dernier nom vu");
    for (index, row) in rows.iter().enumerate() {
        let (name, artist) = (row["name"].clone(), row["artist"].clone());
        let payload = match row["level"].as_str().unwrap() {
            "artist" => json!({"level": "artist", "artist": name}),
            "album" => json!({"level": "album", "artist": artist, "album": name}),
            _ => json!({"level": "track", "artist": artist, "track": name}),
        };
        app.mark(&format!("m{index}"), payload).await;
    }
    assert_eq!(app.discoveries("2026-10-16").await, found(&[]));
}

#[tokio::test]
async fn cancelling_a_mark_restores_the_discovery_and_gives_what_it_needs() {
    let app = App::new().await;
    app.send(vec![scrobble("2026-10-16T08:00:00Z", "A", "T", "")])
        .await;
    let all = found(&[("artist", "A"), ("track", "T")]);
    let artist = json!({"level": "artist", "artist": "A"});
    app.mark("m1", artist.clone()).await;
    assert_eq!(
        app.discoveries("2026-10-16").await,
        found(&[("track", "T")])
    );
    assert_eq!(
        app.music("2026-10-16").await["first_listens"][0]["known_marks"],
        json!([{"source": "webapp", "key": "m1"}])
    );

    // Marqué deux fois (deux appareils) : annuler l'un ne suffit pas.
    app.mark("m2", artist.clone()).await;
    app.cancel_mark("c1", "m1").await;
    assert_eq!(
        app.discoveries("2026-10-16").await,
        found(&[("track", "T")])
    );
    app.cancel_mark("c2", "m2").await;
    assert_eq!(app.discoveries("2026-10-16").await, all);

    // Se marque de nouveau, et une annulation peut arriver avant sa cible.
    app.mark("m3", artist.clone()).await;
    assert_eq!(
        app.discoveries("2026-10-16").await,
        found(&[("track", "T")])
    );
    app.cancel_mark("c3", "m3").await;
    app.cancel_mark("c4", "m4").await;
    app.mark("m4", artist).await;
    assert_eq!(app.discoveries("2026-10-16").await, all);
}

#[tokio::test]
async fn marks_and_discoveries_are_identical_after_a_rebuild() {
    let app = App::new().await;
    app.send(vec![
        scrobble("2026-10-14T08:00:00Z", "A", "Un", "L1"),
        scrobble("2026-10-16T08:00:00Z", "A", "Deux", "L2"),
        scrobble("2026-10-16T09:00:00Z", "B", "Trois", "L3"),
    ])
    .await;
    app.mark(
        "kept",
        json!({"level": "album", "artist": "A", "album": "L2"}),
    )
    .await;
    app.mark("undone", json!({"level": "artist", "artist": "B"}))
        .await;
    app.cancel_mark("c", "undone").await;
    // Une déclaration inutilisable reste une erreur, à l'identique dans les deux générations.
    app.mark("bad", json!({"level": "genre", "artist": "A"}))
        .await;

    let before = (app.music("2026-10-14").await, app.music("2026-10-16").await);
    assert_eq!(
        app.discoveries("2026-10-16").await,
        found(&[
            ("artist", "B"),
            ("album", "L3"),
            ("track", "Deux"),
            ("track", "Trois")
        ])
    );
    let report = replay::rebuild(&app.db.admin, INTERPRETERS, false)
        .await
        .unwrap();
    assert!(report.switched);
    assert_eq!(report.comparison.differences, vec![]);
    let after = (app.music("2026-10-14").await, app.music("2026-10-16").await);
    assert_eq!(after, before);
}

/// Le déploiement sur un NAS en service : le journal et les curseurs existent déjà, le binaire apporte un
/// interprète de plus. Le worker le rattrape par son curseur, sans rebuild ni perte ni différence.
#[tokio::test]
async fn a_new_interpreter_catches_up_by_its_cursor_alone() {
    let db = TestDb::new().await;
    let without_mark: Vec<_> = INTERPRETERS
        .iter()
        .filter(|i| i.kind != "music.mark_known")
        .copied()
        .collect();
    let source = admin::create_source(&db.admin, "lastfm", None)
        .await
        .unwrap();
    admin::grant_kind(&db.admin, "lastfm", "lastfm.scrobble")
        .await
        .unwrap();
    let webapp_mark = |key: &str, payload: Value| {
        let (key, payload) = (key.to_owned(), payload.to_string());
        let pool = db.app.clone();
        async move {
            sqlx::query(
                "INSERT INTO core.records (source_id, kind, schema_version, key, occurred_at, payload)
                 SELECT id, 'music.mark_known', 1, $1, now(), $2::jsonb FROM core.sources
                 WHERE name = 'webapp'",
            )
            .bind(key)
            .bind(payload)
            .execute(&pool)
            .await
            .unwrap();
        }
    };
    let base = serve(db.app.clone()).await;
    let sent = client()
        .post(format!("{base}/v1/records"))
        .bearer_auth(&source)
        .json(&json!({"records": [scrobble("2026-10-16T08:00:00Z", "A", "T", "")]}))
        .send()
        .await
        .unwrap();
    assert_eq!(sent.status(), 200);
    webapp_mark("avant", json!({"level": "artist", "artist": "A"})).await;

    // L'ancien binaire : l'écoute est traitée, le marquage attend sans erreur ni curseur.
    while engine::pass(&db.app, &without_mark, engine::Target::Active)
        .await
        .unwrap()
        > 0
    {}
    let known = "SELECT count(*) FROM derived.events WHERE metric = 'music.known'";
    assert_eq!(db.count(known).await, 0);
    assert_eq!(
        db.count("SELECT count(*) FROM derived.events WHERE metric = 'music.listen'")
            .await,
        1
    );

    // Le nouveau binaire : rien d'autre à faire.
    while engine::pass(&db.app, INTERPRETERS, engine::Target::Active)
        .await
        .unwrap()
        > 0
    {}
    assert_eq!(db.count(known).await, 1);
    assert_eq!(
        db.count("SELECT count(*) FROM derived.interpretation_errors")
            .await,
        0
    );
    assert_eq!(
        db.count("SELECT count(*) FROM derived.events WHERE metric = 'music.listen'")
            .await,
        1,
        "l'écoute n'est pas rejouée"
    );
    let report = replay::rebuild(&db.admin, INTERPRETERS, false)
        .await
        .unwrap();
    assert_eq!(report.comparison.differences, vec![]);
}

#[tokio::test]
async fn a_mark_needs_the_session_the_csrf_header_and_the_right() {
    let app = App::new().await;
    let url = format!("{}/v1/records", app.base);
    let body = json!({"records": [mark_known("m", json!({"level": "artist", "artist": "A"}))]});

    let anonymous = app.http.post(&url).json(&body).send().await.unwrap();
    assert_eq!(anonymous.status(), 401);
    let no_csrf = app
        .http
        .post(&url)
        .header("cookie", &app.cookie)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(no_csrf.status(), 403);

    // Une source à token n'a ce droit que si on le lui donne.
    let by_token: Value = app
        .http
        .post(&url)
        .bearer_auth(&app.token)
        .json(&body)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(by_token["results"][0]["status"], "rejected");
    assert_eq!(
        app.db
            .count("SELECT count(*) FROM core.records WHERE kind = 'music.mark_known'")
            .await,
        0
    );
}

/// Le droit vient de la migration, pas de `grant-kind` : rien à accorder en production.
#[tokio::test]
async fn the_webapp_already_has_the_right_to_mark_and_to_cancel() {
    let db = TestDb::new().await;
    assert_eq!(
        db.count(
            "SELECT count(*) FROM core.source_kinds k JOIN core.sources s ON s.id = k.source_id
             WHERE s.name = 'webapp' AND k.revoked_at IS NULL
               AND k.kind IN ('music.mark_known', 'record.cancel')"
        )
        .await,
        2
    );
}

#[tokio::test]
async fn an_unusable_mark_is_set_aside_without_citing_its_payload() {
    let app = App::new().await;
    app.send(vec![scrobble("2026-10-16T08:00:00Z", "A", "T", "")])
        .await;
    app.mark(
        "bad",
        json!({"level": "artist", "artist": "x".repeat(2000)}),
    )
    .await;
    app.mark(
        "good",
        json!({"level": "track", "artist": "A", "track": "T"}),
    )
    .await;
    assert_eq!(
        app.discoveries("2026-10-16").await,
        found(&[("artist", "A")])
    );
    let stored: String = sqlx::query_scalar("SELECT error FROM derived.interpretation_errors")
        .fetch_one(&app.db.app)
        .await
        .unwrap();
    assert_eq!(stored, "music.mark_known: invalid artist");
}
