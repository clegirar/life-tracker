//! Twitch de bout en bout, sur un vrai PostgreSQL : des segments de lecture envoyés à la vraie API
//! d'ingestion avec le token de la source, comme le fera l'extension Firefox, puis le vrai worker et les
//! lectures (carte du jour, durée cumulée, mois, classement, découvertes) et le jeton de service du MCP.
//! Tous les logins, noms et catégories sont fictifs.
//!
//! Le sens de la forme `duration` : chaque segment est l'intervalle `[début, début + durée]`, et le temps
//! d'un jour est l'union de ces intervalles, coupée aux minuits de Paris.

mod common;
use common::*;
use life_tracker::api;
use life_tracker::interpreters::INTERPRETERS;
use life_tracker::{admin, engine, replay};
use reqwest::StatusCode;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::sync::watch;

const PASSWORD: &str = "un mot de passe assez long";
const CSRF: (&str, &str) = (api::CSRF_HEADER, "1");
const MINUTE: i64 = 60_000;

struct App {
    db: TestDb,
    base: String,
    http: reqwest::Client,
    cookie: String,
    service: String,
    twitch: String,
    _stop: watch::Sender<bool>,
}

impl App {
    async fn new() -> Self {
        let db = TestDb::new().await;
        admin::set_password(&db.admin, "moi", PASSWORD)
            .await
            .unwrap();
        let service = admin::create_service_token(&db.admin, "mcp").await.unwrap();
        let twitch = admin::create_source(&db.admin, "twitch", Some("3 days"), Some("Twitch"))
            .await
            .unwrap();
        admin::grant_kind(&db.admin, "twitch", "twitch.watch")
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
            twitch,
            _stop: stop,
        }
    }

    /// Envoie un lot avec le token de la source, comme l'extension : tout doit être accepté.
    async fn send(&self, records: Vec<Value>) {
        let response = self
            .http
            .post(format!("{}/v1/records", self.base))
            .bearer_auth(&self.twitch)
            .header("x-client-version", "extension-test")
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

    /// Une lecture avec la session de la web app.
    async fn get(&self, path: &str) -> Value {
        self.settled().await;
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

    /// Une lecture avec le jeton de service, comme le MCP.
    async fn as_mcp(&self, path: &str) -> (StatusCode, Value) {
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

    async fn stream(&self, date: &str) -> Value {
        self.get(&format!("/v1/days/{date}/stream")).await
    }

    async fn durations(&self, query: &str) -> Value {
        self.get(&format!("/v1/durations?metric=stream.watch&{query}"))
            .await
    }

    async fn cell(&self, date: &str) -> Value {
        let month = self.get(&format!("/v1/months/{}", &date[..7])).await;
        month["days"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["date"] == date)
            .unwrap()["values"]["stream.watch"]
            .clone()
    }
}

/// Un segment de lecture tel que l'extension l'envoie : son début, avec le décalage de Paris.
fn segment(key: &str, at: &str, channel: Option<&str>, ms: i64, content: &str) -> Value {
    json!({
        "key": key,
        "kind": "twitch.watch",
        "schema_version": 1,
        "occurred_at": at,
        "payload": {
            "channel": channel,
            "channel_name": channel.map(|c| format!("Nom {c}")),
            "content": content,
            "duration_ms": ms,
            "category": "Catégorie fictive",
        },
    })
}

/// `count` segments consécutifs de `ms` sur une chaîne, à partir de `start` (UTC).
fn run(prefix: &str, start: &str, channel: Option<&str>, ms: i64, count: i64) -> Vec<Value> {
    let start: chrono::DateTime<chrono::Utc> = start.parse().unwrap();
    (0..count)
        .map(|i| {
            let at = start + chrono::Duration::milliseconds(i * ms);
            segment(
                &format!("{prefix}-{i}"),
                &at.with_timezone(&chrono::FixedOffset::east_opt(3600).unwrap())
                    .to_rfc3339(),
                channel,
                ms,
                "live",
            )
        })
        .collect()
}

fn minutes_of<'a>(channels: &'a Value, key: &str) -> &'a Value {
    &channels
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["key"] == key)
        .unwrap_or_else(|| panic!("{key} absente : {channels}"))["minutes"]
}

#[tokio::test]
async fn the_fixture_is_ingested_and_interpreted() {
    let app = App::new().await;
    let fixture = read_json(&repo_path("contract/fixtures/valid/twitch.json"));
    app.send(fixture["records"].as_array().unwrap().clone())
        .await;
    app.settled().await;
    assert_eq!(
        app.db
            .count("SELECT count(*) FROM derived.interpretation_errors")
            .await,
        0
    );
    assert_eq!(
        app.db
            .count("SELECT count(*) FROM derived.live_events WHERE metric = 'stream.watch'")
            .await,
        2
    );
    // Une chaîne connue a son élément ; la chaîne nulle n'en a pas, son temps compte à part.
    assert_eq!(
        app.db
            .count(
                "SELECT count(*) FROM derived.live_event_items
                 WHERE metric = 'stream.watch' AND level = 'channel' AND item_key = 'chaine_fictive'
                   AND display = 'Chaîne_Fictive'"
            )
            .await,
        1
    );
    let day = app.stream("2026-11-06").await;
    assert_eq!(day["minutes"], 2, "{day}");
    assert_eq!(day["segments"], 2);
    assert_eq!(day["unknown_minutes"], 1);
    assert_eq!(day["channels"][0]["key"], "chaine_fictive");
    assert_eq!(day["channels"][0]["name"], "Chaîne_Fictive");
    assert_eq!(day["channels"][0]["minutes"], 1);
    assert_eq!(day["channels"][0]["contents"], json!(["live"]));
    assert_eq!(day["channels"][0]["first_time"], "21:14");
    assert_eq!(day["channels"][0]["last_time"], "21:15");
}

#[tokio::test]
async fn two_streams_at_once_count_once_in_the_total_and_fully_for_each_channel() {
    let app = App::new().await;
    // Le 3 nov. (UTC+1) : alpha de 20:00 à 21:00, beta de 20:30 à 21:30 (rediffusion), et 10 min sans
    // chaîne de 21:20 à 21:30, pendant beta.
    let mut records = run("a", "2026-11-03T19:00:00Z", Some("alpha"), 20 * MINUTE, 3);
    let mut beta = run("b", "2026-11-03T19:30:00Z", Some("beta"), 30 * MINUTE, 2);
    for record in &mut beta {
        record["payload"]["content"] = json!("vod");
    }
    records.extend(beta);
    records.extend(run("u", "2026-11-03T20:20:00Z", None, 10 * MINUTE, 1));
    // Un clip d'alpha le lendemain.
    records.push(segment(
        "clip",
        "2026-11-04T12:00:00+01:00",
        Some("alpha"),
        MINUTE,
        "clip",
    ));
    app.send(records).await;

    let day = app.stream("2026-11-03").await;
    assert_eq!(day["minutes"], 90, "l'union : {day}");
    assert_eq!(day["summed_minutes"], 130, "60 + 60 + 10 : {day}");
    assert_eq!(day["unknown_minutes"], 10);
    assert_eq!(day["segments"], 6);
    let channels = &day["channels"];
    assert_eq!(channels.as_array().unwrap().len(), 2);
    assert_eq!(minutes_of(channels, "alpha"), 60);
    assert_eq!(minutes_of(channels, "beta"), 60);
    assert_eq!(channels[0]["key"], "alpha", "à temps égal, par nom");
    assert_eq!(channels[1]["contents"], json!(["vod"]));
    assert_eq!(channels[0]["first_time"], "20:00");
    assert_eq!(channels[0]["last_time"], "21:00");
    assert_eq!(channels[1]["first_time"], "20:30");
    assert_eq!(channels[1]["last_time"], "21:30");

    // Les mêmes minutes par les lectures génériques, en tout et par chaîne.
    let total = app.durations("from=2026-11-03&to=2026-11-03").await;
    assert_eq!(total["total_minutes"], 90, "{total}");
    assert_eq!(total["events"], 6);
    assert_eq!(total["events_without_duration"], 0);
    let beta = app
        .durations("from=2026-11-01&to=2026-11-30&level=channel&key=beta")
        .await;
    assert_eq!(beta["total_minutes"], 60, "{beta}");
    assert_eq!(beta["events"], 2);
    let alpha = app
        .durations("from=2026-11-01&to=2026-11-30&level=channel&key=alpha&group=day")
        .await;
    assert_eq!(
        alpha["periods"],
        json!([
            {"period": "2026-11-03", "minutes": 60, "events": 3, "events_without_duration": 0},
            {"period": "2026-11-04", "minutes": 1, "events": 1, "events_without_duration": 0},
        ])
    );
    // La cellule du mois : l'union, en minutes.
    assert_eq!(app.cell("2026-11-03").await, 90.0);
    assert_eq!(app.cell("2026-11-04").await, 1.0);
    // Avant la première donnée : pas de données, jamais zéro.
    assert_eq!(app.cell("2026-11-02").await, Value::Null);
}

#[tokio::test]
async fn a_segment_across_midnight_is_split_and_the_dst_day_lasts_25_hours() {
    let app = App::new().await;
    // Le 25 oct. 2026 dure 25 heures à Paris : de 2026-10-24T22:00Z à 2026-10-25T23:00Z. 25 segments
    // d'une heure le couvrent tout entier.
    let mut records = run(
        "dst",
        "2026-10-24T22:00:00Z",
        Some("marathon"),
        60 * MINUTE,
        25,
    );
    // Le 4 nov., de 23:30 à 00:30 : 30 min chaque jour.
    records.push(segment(
        "minuit",
        "2026-11-04T23:30:00+01:00",
        Some("nocturne"),
        60 * MINUTE,
        "live",
    ));
    app.send(records).await;

    assert_eq!(app.stream("2026-10-25").await["minutes"], 1500);
    assert_eq!(app.cell("2026-10-25").await, 1500.0);
    assert_eq!(app.cell("2026-10-24").await, Value::Null);

    let before = app.stream("2026-11-04").await;
    let after = app.stream("2026-11-05").await;
    assert_eq!(before["minutes"], 30, "{before}");
    assert_eq!(after["minutes"], 30, "{after}");
    assert_eq!(
        after["segments"], 1,
        "le segment de la veille touche ce jour"
    );
    assert_eq!(after["channels"][0]["first_time"], "23:30");
    assert_eq!(after["channels"][0]["last_time"], "00:30");
    assert_eq!(before["discoveries"][0]["key"], "nocturne");
    assert_eq!(
        after["discoveries"],
        json!([]),
        "commencée la veille : pas une découverte de ce jour"
    );
    let days = app
        .durations("from=2026-11-04&to=2026-11-05&group=day")
        .await;
    assert_eq!(
        days["periods"],
        json!([
            {"period": "2026-11-04", "minutes": 30, "events": 1, "events_without_duration": 0},
            {"period": "2026-11-05", "minutes": 30, "events": 0, "events_without_duration": 0},
        ])
    );
    // Une période qui commence après minuit ne compte que sa part.
    let only_after = app.durations("from=2026-11-05&to=2026-11-05").await;
    assert_eq!(only_after["total_minutes"], 30);
    assert_eq!(only_after["events"], 0);

    // Semaine ISO et mois : la somme des unions de chaque jour.
    let weeks = app
        .durations("from=2026-10-01&to=2026-11-30&group=week")
        .await;
    assert_eq!(
        weeks["periods"],
        json!([
            {"period": "2026-W43", "minutes": 1500, "events": 25, "events_without_duration": 0},
            {"period": "2026-W45", "minutes": 60, "events": 1, "events_without_duration": 0},
        ]),
        "{weeks}"
    );
    let months = app
        .durations("from=2026-10-01&to=2026-11-30&group=month&level=channel&key=nocturne")
        .await;
    assert_eq!(
        months["periods"],
        json!([{"period": "2026-11", "minutes": 60, "events": 1, "events_without_duration": 0}])
    );
    assert_eq!(months["total_minutes"], 60);
    // Sans rien sur la période : aucune minute, pas zéro.
    let empty = app.durations("from=2026-09-01&to=2026-09-30").await;
    assert_eq!(empty["total_minutes"], Value::Null);
    assert_eq!(empty["periods"], json!([]));
}

#[tokio::test]
async fn top_ranks_channels_by_minutes_and_discoveries_are_first_times() {
    let app = App::new().await;
    // Le 1er nov. : cinq segments d'une minute sur « bavard », un de 30 min sur « long ».
    let mut records = run("bavard", "2026-11-01T18:00:00Z", Some("bavard"), MINUTE, 5);
    records.extend(run(
        "long",
        "2026-11-01T19:00:00Z",
        Some("long"),
        30 * MINUTE,
        1,
    ));
    // Le 2 nov. : « long » revient, « nouveau » apparaît.
    records.extend(run(
        "long2",
        "2026-11-02T19:00:00Z",
        Some("long"),
        10 * MINUTE,
        1,
    ));
    records.extend(run(
        "nouveau",
        "2026-11-02T20:00:00Z",
        Some("nouveau"),
        2 * MINUTE,
        1,
    ));
    app.send(records).await;

    let top = app
        .get("/v1/top?metric=stream.watch&level=channel&from=2026-11-01&to=2026-11-02")
        .await;
    let ranked: Vec<(&str, i64, i64)> = top["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| {
            (
                i["key"].as_str().unwrap(),
                i["minutes"].as_i64().unwrap(),
                i["occurrences"].as_i64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        ranked,
        [("long", 40, 2), ("bavard", 5, 5), ("nouveau", 2, 1)],
        "par minutes, pas par occurrences"
    );
    assert_eq!(top["distinct_items"], 3);
    assert_eq!(top["occurrences_with_item"], 8);
    assert_eq!(top["items"][0]["name"], "Nom long");
    let limited = app
        .get("/v1/top?metric=stream.watch&level=channel&from=2026-11-01&to=2026-11-02&limit=1")
        .await;
    assert_eq!(limited["items"].as_array().unwrap().len(), 1);
    assert_eq!(limited["distinct_items"], 3);

    let discoveries = app
        .get("/v1/discoveries?metric=stream.watch&level=channel&from=2026-11-02&to=2026-11-02")
        .await;
    assert_eq!(discoveries["total"], 1, "{discoveries}");
    assert_eq!(discoveries["items"][0]["key"], "nouveau");
    let day = app.stream("2026-11-02").await;
    assert_eq!(
        day["discoveries"],
        json!([{"level": "channel", "key": "nouveau", "name": "Nom nouveau"}])
    );
    let first = app.stream("2026-11-01").await;
    assert_eq!(first["discoveries"].as_array().unwrap().len(), 2);
    // Un jour sans Twitch : rien, sans erreur.
    let none = app.stream("2026-11-10").await;
    assert_eq!(none["minutes"], 0);
    assert_eq!(none["channels"], json!([]));
}

#[tokio::test]
async fn a_duration_over_one_hour_is_an_interpretation_error() {
    let app = App::new().await;
    app.send(vec![
        segment(
            "trop",
            "2026-11-03T20:00:00+01:00",
            Some("alpha"),
            60 * MINUTE + 1,
            "live",
        ),
        segment(
            "juste",
            "2026-11-03T22:00:00+01:00",
            Some("alpha"),
            60 * MINUTE,
            "live",
        ),
    ])
    .await;
    app.settled().await;
    let message: String = sqlx::query_scalar(
        "SELECT x.error FROM derived.interpretation_errors x JOIN core.records r USING (seq)
         WHERE r.key = 'trop'",
    )
    .fetch_one(&app.db.app)
    .await
    .unwrap();
    assert_eq!(message, "twitch.watch: invalid duration_ms");
    let month = app.get("/v1/months/2026-11").await;
    let day = month["days"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["date"] == "2026-11-03")
        .unwrap();
    assert_eq!(day["errors"], 1, "la journée est incomplète");
    assert_eq!(day["values"]["stream.watch"], 60.0);
}

#[tokio::test]
async fn a_rebuild_gives_no_difference() {
    let app = App::new().await;
    let mut records = run("a", "2026-11-03T19:00:00Z", Some("alpha"), 20 * MINUTE, 3);
    records.extend(run("u", "2026-11-03T20:20:00Z", None, 10 * MINUTE, 1));
    records.push(segment(
        "trop",
        "2026-11-03T22:00:00+01:00",
        Some("alpha"),
        2 * 60 * MINUTE,
        "live",
    ));
    app.send(records).await;
    app.settled().await;
    let before = app.stream("2026-11-03").await;
    let rebuilt = replay::rebuild(&app.db.admin, INTERPRETERS, false)
        .await
        .unwrap();
    assert!(rebuilt.switched);
    assert!(rebuilt.comparison.differences.is_empty());
    assert_eq!(app.stream("2026-11-03").await, before);
}

#[tokio::test]
async fn the_service_token_sees_only_the_shared_details() {
    let app = App::new().await;
    app.send(run(
        "a",
        "2026-11-03T19:00:00Z",
        Some("alpha"),
        20 * MINUTE,
        1,
    ))
    .await;
    let (status, list) = app
        .as_mcp("/v1/occurrences?metric=stream.watch&from=2026-11-03&to=2026-11-03&form=list")
        .await;
    assert_eq!(status, 200, "{list}");
    let occurrence = &list["occurrences"][0];
    assert_eq!(
        occurrence["details"],
        json!({"channel_name": "Nom alpha", "content": "live",
               "category": "Catégorie fictive", "played_ms": 20 * MINUTE})
    );
    assert_eq!(occurrence["items"]["channel"], "Nom alpha", "{occurrence}");
    let inventory = app.as_mcp("/v1/inventory").await.1;
    let metric = inventory["metrics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["metric"] == "stream.watch")
        .unwrap();
    assert_eq!(metric["shape"], "duration");
    assert_eq!(metric["unit"], "min");
    assert_eq!(metric["item_levels"], json!(["channel"]));
    assert_eq!(metric["events"], 1);
    let (status, minutes) = app
        .as_mcp("/v1/durations?metric=stream.watch&from=2026-11-03&to=2026-11-03")
        .await;
    assert_eq!(status, 200);
    assert_eq!(minutes["total_minutes"], 20);
    // La carte du jour n'est pas une route de lecture du MCP.
    assert_eq!(app.as_mcp("/v1/days/2026-11-03/stream").await.0, 401);
}
