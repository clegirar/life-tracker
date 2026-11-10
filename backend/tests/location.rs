//! La position de l'iPhone de bout en bout, sur un vrai PostgreSQL : un lot tel qu'Overland l'envoie
//! (`fixtures/overland/batch.json`, coordonnées fictives en mer), reçu par la route d'adaptation avec le
//! token de la source, puis le vrai worker, la carte du jour de la web app et le jeton de service du
//! MCP, à travers une zone de confidentialité fictive.

mod common;
use common::*;
use life_tracker::interpreters::INTERPRETERS;
use life_tracker::privacy::Zones;
use life_tracker::{admin, api, engine, location};
use reqwest::StatusCode;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::sync::watch;

const PASSWORD: &str = "un mot de passe assez long";
const OVERLAND: &str = "/v1/locations/overland";
/// Autour de la visite du lot, en mer : fictive.
const ZONE: &str = "43.201,5.285,200";
const DAY: &str = "2026-10-20";

struct App {
    db: TestDb,
    base: String,
    http: reqwest::Client,
    cookie: String,
    service: String,
    iphone: String,
    _stop: watch::Sender<bool>,
}

impl App {
    async fn new() -> Self {
        Self::granted(&["overland.location", "overland.visit"]).await
    }

    async fn granted(kinds: &[&str]) -> Self {
        let db = TestDb::new().await;
        admin::set_password(&db.admin, "moi", PASSWORD)
            .await
            .unwrap();
        let service = admin::create_service_token(&db.admin, "mcp").await.unwrap();
        let iphone = admin::create_source(
            &db.admin,
            "iphone",
            Some("2 days"),
            Some("Position (iPhone)"),
        )
        .await
        .unwrap();
        for kind in kinds {
            admin::grant_kind(&db.admin, "iphone", kind).await.unwrap();
        }
        let (stop, stopped) = watch::channel(false);
        tokio::spawn(engine::run(
            db.app.clone(),
            INTERPRETERS,
            Duration::from_millis(20),
            stopped,
        ));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let router = api::router_with(
            db.app.clone(),
            api::Config {
                zones: Zones::parse(Some(ZONE)).0,
                ..api::Config::default()
            },
        );
        tokio::spawn(async move { axum::serve(listener, router).await });
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
            http,
            cookie,
            service,
            iphone,
            _stop: stop,
        }
    }

    /// Un lot comme Overland l'envoie : son token en `Bearer`.
    async fn post(&self, body: &Value) -> (StatusCode, Value) {
        let response = self
            .http
            .post(format!("{}{OVERLAND}", self.base))
            .bearer_auth(&self.iphone)
            .json(body)
            .send()
            .await
            .unwrap();
        let status = response.status();
        (status, response.json().await.unwrap_or(Value::Null))
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

    async fn locations(&self, date: &str) -> Value {
        self.get(&format!("/v1/days/{date}/locations")).await
    }

    /// La carte du jour à une heure fixée, pour le séjour en cours.
    async fn locations_at(&self, date: &str, now: &str) -> Value {
        self.settled().await;
        let day = location::day(&self.db.app, date.parse().unwrap(), now.parse().unwrap())
            .await
            .unwrap();
        serde_json::to_value(day).unwrap()
    }

    /// Le dernier contact de l'iPhone, que chaque envoi met à l'heure réelle.
    async fn seen_at(&self, at: &str) {
        sqlx::query(
            "UPDATE core.sources SET last_seen_at = $1::text::timestamptz WHERE name = 'iphone'",
        )
        .bind(at)
        .execute(&self.db.owner)
        .await
        .unwrap();
    }

    /// La source `garmin`, avec les séances et leurs traces ; renvoie son token.
    async fn garmin(&self) -> String {
        let token = admin::create_source(&self.db.admin, "garmin", Some("12 hours"), None)
            .await
            .unwrap();
        for kind in ["garmin.activity", "garmin.activity_track"] {
            admin::grant_kind(&self.db.admin, "garmin", kind)
                .await
                .unwrap();
        }
        token
    }

    /// Une séance comme le connecteur Garmin l'envoie, avec sa trace `[lat, lon, secondes]` s'il y en a
    /// une.
    #[allow(clippy::too_many_arguments)]
    async fn workout(
        &self,
        token: &str,
        id: u64,
        kind: &str,
        start: &str,
        duration_s: f64,
        distance_m: Option<f64>,
        track: Option<Value>,
    ) {
        let mut records = vec![json!({
            "key": format!("activity:{id}"), "kind": "garmin.activity", "schema_version": 1,
            "occurred_at": start,
            "payload": {"activity_id": id, "type": kind, "duration_s": duration_s,
                        "distance_m": distance_m},
        })];
        if let Some(points) = track {
            records.push(json!({
                "key": format!("track:{id}"), "kind": "garmin.activity_track", "schema_version": 1,
                "occurred_at": start, "payload": {"activity_id": id, "points": points},
            }));
        }
        let response = self
            .http
            .post(format!("{}/v1/records", self.base))
            .bearer_auth(token)
            .json(&json!({ "records": records }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
    }

    async fn records(&self, kind: &str) -> i64 {
        self.db
            .count(&format!(
                "SELECT count(*) FROM core.records WHERE kind = '{kind}'"
            ))
            .await
    }

    async fn rejected(&self) -> i64 {
        self.db
            .count("SELECT rejected_count FROM core.sources WHERE name = 'iphone'")
            .await
    }
}

fn batch() -> Value {
    read_json(&repo_path("backend/tests/fixtures/overland/batch.json"))
}

fn point(timestamp: &str, lat: f64, lon: f64) -> Value {
    json!({
        "type": "Feature",
        "geometry": {"type": "Point", "coordinates": [lon, lat]},
        "properties": {"timestamp": timestamp, "horizontal_accuracy": 10, "speed": 1.0,
                       "altitude": 2, "motion": ["walking"], "battery_level": 0.5},
    })
}

fn visit(lat: f64, lon: f64, arrival: &str, departure: &str) -> Value {
    json!({
        "type": "Feature",
        "geometry": {"type": "Point", "coordinates": [lon, lat]},
        "properties": {"timestamp": departure, "action": "visit", "arrival_date": arrival,
                       "departure_date": departure, "horizontal_accuracy": 20},
    })
}

#[tokio::test]
async fn a_batch_enters_with_its_whitelist_only_and_a_resend_is_still_ok() {
    let app = App::new().await;
    let (status, body) = app.post(&batch()).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({"result": "ok"}));
    // 16 points (une position invalide comprise, écartée à l'interprétation) et la visite complète ;
    // ni la visite ouverte, ni le trajet, ni la statistique, ni `current`.
    assert_eq!(app.records("overland.location").await, 16);
    assert_eq!(app.records("overland.visit").await, 1);
    assert_eq!(app.rejected().await, 0);
    let leaked = app
        .db
        .count(
            "SELECT count(*) FROM core.records
             WHERE payload::text ~ '(wifi|bssid|battery|device|unique_id|Réseau|course|trip)'",
        )
        .await;
    assert_eq!(
        leaked, 0,
        "un champ hors liste blanche est entré au journal"
    );
    let keys: Vec<String> = sqlx::query_scalar(
        "SELECT payload::text FROM core.records WHERE key = 'point:1792483200000'",
    )
    .fetch_all(&app.db.app)
    .await
    .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&keys[0]).unwrap(),
        json!({"lat": 43.2, "lon": 5.28, "horizontal_accuracy": 8, "altitude": 3,
               "speed": 1.4, "motion": ["walking"]})
    );
    let seen = app
        .db
        .count(
            "SELECT count(*) FROM core.sources WHERE name = 'iphone' AND last_seen_at IS NOT NULL",
        )
        .await;
    assert_eq!(seen, 1);

    // Overland renvoie un lot dont il n'a pas eu la réponse : des doublons, et `ok` pour le retirer.
    let (status, body) = app.post(&batch()).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"result": "ok"})));
    assert_eq!(app.records("overland.location").await, 16);
    assert_eq!(app.records("overland.visit").await, 1);

    app.settled().await;
    assert_eq!(
        app.db
            .count("SELECT count(*) FROM derived.live_events WHERE metric = 'location.point'")
            .await,
        15
    );
    assert_eq!(
        app.db
            .count("SELECT count(*) FROM derived.interpretation_errors")
            .await,
        0
    );
}

#[tokio::test]
async fn only_the_source_token_is_accepted_and_bad_bodies_are_refused() {
    let app = App::new().await;
    let url = format!("{}{OVERLAND}", app.base);
    let empty = json!({"locations": []});
    for (name, request) in [
        ("sans token", app.http.post(&url).json(&empty)),
        (
            "mauvais token",
            app.http.post(&url).bearer_auth("faux").json(&empty),
        ),
        (
            "jeton de service",
            app.http.post(&url).bearer_auth(&app.service).json(&empty),
        ),
        (
            "session",
            app.http
                .post(&url)
                .header("cookie", &app.cookie)
                .header(api::CSRF_HEADER, "1")
                .json(&empty),
        ),
    ] {
        let response = request.send().await.unwrap();
        assert_eq!(response.status(), 401, "{name}");
    }
    assert_eq!(app.post(&empty).await.0, 200);

    for body in [json!({"records": []}), json!([1]), json!({"locations": {}})] {
        let (status, answer) = app.post(&body).await;
        assert_eq!(status, 400, "{body}");
        assert_eq!(answer["error"], "invalid_json");
    }
    let raw = app
        .http
        .post(&url)
        .bearer_auth(&app.iphone)
        .body("pas du json")
        .send()
        .await
        .unwrap();
    assert_eq!(raw.status(), 400);

    let many: Vec<Value> = (0..501)
        .map(|i| {
            point(
                &format!("2026-10-20T09:{:02}:{:02}Z", i / 60, i % 60),
                43.2,
                5.28,
            )
        })
        .collect();
    let (status, answer) = app.post(&json!({ "locations": many })).await;
    assert_eq!(status, 413);
    assert_eq!(answer["error"], "batch_too_large");
    let huge = json!({"locations": [], "padding": "x".repeat(1_100_000)});
    assert_eq!(app.post(&huge).await.0, 413);
    assert_eq!(app.records("overland.location").await, 0);
}

#[tokio::test]
async fn an_unreadable_feature_is_a_counted_rejection_and_the_rest_enters() {
    let app = App::new().await;
    let no_date = point("hier", 43.2, 5.28);
    let mut no_geometry = point("2026-10-20T09:00:00Z", 43.2, 5.28);
    no_geometry.as_object_mut().unwrap().remove("geometry");
    let body =
        json!({"locations": [no_date, no_geometry, point("2026-10-20T09:01:00Z", 43.2, 5.28)]});
    let (status, answer) = app.post(&body).await;
    assert_eq!((status, answer), (StatusCode::OK, json!({"result": "ok"})));
    assert_eq!(app.records("overland.location").await, 1);
    assert_eq!(app.rejected().await, 2);
}

#[tokio::test]
async fn an_ungranted_kind_keeps_the_batch_on_the_iphone_until_granted() {
    let app = App::granted(&["overland.location"]).await;
    let (status, answer) = app.post(&batch()).await;
    assert_eq!(status, 403);
    assert_eq!(answer["error"], "kind_not_granted");
    // Ce qui pouvait entrer est écrit ; le refus se voit au compteur de la source.
    assert_eq!(app.records("overland.location").await, 16);
    assert_eq!(app.records("overland.visit").await, 0);
    assert_eq!(app.rejected().await, 1);

    admin::grant_kind(&app.db.admin, "iphone", "overland.visit")
        .await
        .unwrap();
    let (status, answer) = app.post(&batch()).await;
    assert_eq!((status, answer), (StatusCode::OK, json!({"result": "ok"})));
    assert_eq!(app.records("overland.location").await, 16);
    assert_eq!(app.records("overland.visit").await, 1);
}

#[tokio::test]
async fn the_day_map_has_its_cut_and_simplified_trace_and_its_places() {
    let app = App::new().await;
    assert_eq!(app.post(&batch()).await.0, 200);
    // Une nuit dehors, du 20 au 21, et un point le 23 : le 22 est suivi, sans déplacement.
    let night = visit(43.3, 5.35, "2026-10-20T20:00:00Z", "2026-10-21T06:00:00Z");
    let later = point("2026-10-23T09:00:00Z", 43.3, 5.35);
    assert_eq!(app.post(&json!({"locations": [night, later]})).await.0, 200);

    let day = app.locations(DAY).await;
    assert_eq!(day["date"], DAY);
    assert_eq!(day["covered"], true);
    assert_eq!(day["point_count"], 15);
    assert_eq!(
        day["segments"],
        json!([
            {"start_time": "10:00", "end_time": "10:10",
             "start": "2026-10-20T08:00:00Z", "end": "2026-10-20T08:10:00Z",
             "points": [[43.2, 5.28], [43.2, 5.2825], [43.2025, 5.2825]]},
            {"start_time": "10:50", "end_time": "10:52",
             "start": "2026-10-20T08:50:00Z", "end": "2026-10-20T08:52:00Z",
             "points": [[43.21, 5.29], [43.21, 5.291]]},
        ]),
        "coupée au silence de 40 min, simplifiée à 5 m, sans le point des antennes"
    );
    assert_eq!(
        day["places"],
        json!([
            {"origin": "visit", "lat": 43.201, "lon": 5.285, "arrival": "2026-10-20T08:12:00Z",
             "departure": "2026-10-20T08:48:00Z", "arrival_time": "10:12", "departure_time": "10:48",
             "arrived_before": false, "departed_after": false, "duration_s": 2160,
             "ongoing": false, "horizontal_accuracy": 35.0},
            {"origin": "visit", "lat": 43.3, "lon": 5.35, "arrival": "2026-10-20T20:00:00Z",
             "departure": "2026-10-21T06:00:00Z", "arrival_time": "22:00", "departure_time": "08:00",
             "arrived_before": false, "departed_after": true, "duration_s": 36000,
             "ongoing": false, "horizontal_accuracy": 20.0},
        ]),
        "les points du lot ne restent nulle part 15 min"
    );

    let next = app.locations("2026-10-21").await;
    assert_eq!(next["covered"], true);
    assert_eq!(next["segments"], json!([]));
    assert_eq!(next["places"][0]["arrived_before"], true);
    let quiet = app.locations("2026-10-22").await;
    assert_eq!(
        (&quiet["covered"], &quiet["point_count"], &quiet["places"]),
        (&json!(true), &json!(0), &json!([])),
        "un jour suivi sans point : sans déplacement"
    );
    for date in ["2026-10-19", "2026-10-24"] {
        let none = app.locations(date).await;
        assert_eq!(none["covered"], false, "{date} : sans données");
    }

    // Le calendrier compte les lieux, par jour d'arrivée ; les points n'y sont pas.
    let month = app.get("/v1/months/2026-10").await;
    let values = |date: &str| {
        month["days"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["date"] == date)
            .unwrap()["values"]
            .clone()
    };
    assert_eq!(values(DAY)["place.visit"], 2.0);
    assert!(values(DAY).get("location.point").is_none());
}

#[tokio::test]
async fn the_mcp_sees_places_through_the_zone_and_never_a_point() {
    let app = App::new().await;
    assert_eq!(app.post(&batch()).await.0, 200);
    let outside = visit(43.3, 5.35, "2026-10-20T12:00:00Z", "2026-10-20T13:00:00Z");
    assert_eq!(app.post(&json!({"locations": [outside]})).await.0, 200);

    let (status, places) = app
        .as_mcp(&format!(
            "/v1/occurrences?metric=place.visit&from={DAY}&to={DAY}"
        ))
        .await;
    assert_eq!(status, 200, "{places}");
    let details: Vec<&Value> = places["occurrences"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| &o["details"])
        .collect();
    assert_eq!(
        details[0],
        &json!({"visit_lat": null, "visit_lon": null, "visit_hidden": true,
                "arrival": "2026-10-20T08:12:00Z", "departure": "2026-10-20T08:48:00Z",
                "duration_s": 2160}),
        "dans la zone : ni latitude ni longitude"
    );
    assert_eq!(details[1]["visit_lat"], 43.3);
    assert_eq!(details[1]["duration_s"], 3600);

    let (status, points) = app
        .as_mcp(&format!(
            "/v1/occurrences?metric=location.point&from={DAY}&to={DAY}"
        ))
        .await;
    assert_eq!(status, 200);
    let text = points.to_string();
    for forbidden in ["43.2", "5.28", "point_lat", "details"] {
        assert!(!text.contains(forbidden), "{forbidden} visible par le MCP");
    }
    let (status, _) = app.as_mcp(&format!("/v1/days/{DAY}/locations")).await;
    assert_eq!(status, 401, "la carte du jour est réservée à la session");
}

/// Des points à l'heure dite, en `HH:MM` UTC du jour donné.
fn points(date: &str, at: &[(&str, f64, f64)]) -> Value {
    let features: Vec<Value> = at
        .iter()
        .map(|(time, lat, lon)| point(&format!("{date}T{time}:00Z"), *lat, *lon))
        .collect();
    json!({ "locations": features })
}

/// Un trajet : des points à plus de 150 m les uns des autres.
const MOVING: [(f64, f64); 3] = [(43.32, 5.38), (43.34, 5.40), (43.36, 5.42)];

#[tokio::test]
async fn hourly_points_make_stays_the_night_before_included() {
    let app = App::new().await;
    // La nuit à (43.4, 5.4) depuis 22:00 à Paris la veille, un trajet, la matinée à (43.3, 5.35)
    // avec un peu de bruit, puis un départ.
    let mut at = vec![
        ("21:00", 43.4, 5.4),
        ("22:00", 43.4003, 5.4),
        ("23:00", 43.4, 5.4004),
    ];
    let night_before = points("2026-11-01", &at);
    at = vec![
        ("01:00", 43.4, 5.4),
        ("04:00", 43.4002, 5.4002),
        ("06:00", 43.4, 5.4),
        ("07:00", MOVING[0].0, MOVING[0].1),
        ("07:10", MOVING[1].0, MOVING[1].1),
        ("07:20", MOVING[2].0, MOVING[2].1),
        ("08:00", 43.3, 5.35),
        ("09:00", 43.3005, 5.35),
        ("10:00", 43.3, 5.3505),
        ("11:00", 43.2995, 5.35),
        ("11:30", 43.25, 5.30),
        ("11:40", 43.22, 5.28),
    ];
    let day = points("2026-11-02", &at);
    assert_eq!(app.post(&night_before).await.0, 200);
    assert_eq!(app.post(&day).await.0, 200);

    let places = app.locations("2026-11-02").await["places"].clone();
    assert_eq!(
        places,
        json!([
            {"origin": "points", "lat": 43.400083, "lon": 5.4001,
             "arrival": "2026-11-01T21:00:00Z", "departure": "2026-11-02T07:00:00Z",
             "arrival_time": "22:00", "departure_time": "08:00",
             "arrived_before": true, "departed_after": false, "duration_s": 36000,
             "ongoing": false, "horizontal_accuracy": null},
            {"origin": "points", "lat": 43.3, "lon": 5.350125,
             "arrival": "2026-11-02T08:00:00Z", "departure": "2026-11-02T11:30:00Z",
             "arrival_time": "09:00", "departure_time": "12:30",
             "arrived_before": false, "departed_after": false, "duration_s": 12600,
             "ongoing": false, "horizontal_accuracy": null},
        ]),
        "le départ est le premier point du trajet qui suit"
    );
    // La veille voit la nuit partir le lendemain.
    let before = app.locations("2026-11-01").await["places"].clone();
    assert_eq!(before.as_array().unwrap().len(), 1);
    assert_eq!(before[0]["departed_after"], true);
    assert_eq!(before[0]["arrived_before"], false);
    assert_eq!(before[0]["departure"], "2026-11-02T07:00:00Z");

    // Une visite iOS sur la matinée : le séjour détecté s'efface devant elle.
    let morning = visit(43.3, 5.35, "2026-11-02T07:55:00Z", "2026-11-02T11:05:00Z");
    assert_eq!(app.post(&json!({"locations": [morning]})).await.0, 200);
    let places = app.locations("2026-11-02").await["places"].clone();
    let origins: Vec<(&str, &str)> = places
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                p["origin"].as_str().unwrap(),
                p["arrival"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        origins,
        [
            ("points", "2026-11-01T21:00:00Z"),
            ("visit", "2026-11-02T07:55:00Z")
        ]
    );
}

#[tokio::test]
async fn an_isolated_point_then_a_move_is_no_stay() {
    let app = App::new().await;
    let at = [
        ("09:00", 43.3, 5.35),
        ("09:20", MOVING[0].0, MOVING[0].1),
        ("09:21", MOVING[1].0, MOVING[1].1),
        ("09:22", MOVING[2].0, MOVING[2].1),
    ];
    assert_eq!(app.post(&points("2026-11-02", &at)).await.0, 200);
    assert_eq!(app.locations("2026-11-02").await["places"], json!([]));
    // Même aujourd'hui : le dernier point n'a que 3 min, je repars peut-être.
    app.seen_at("2026-11-02T09:24:00Z").await;
    let today = app.locations_at("2026-11-02", "2026-11-02T09:25:00Z").await;
    assert_eq!(today["places"], json!([]));
}

#[tokio::test]
async fn the_last_stay_is_ongoing_today_while_the_iphone_answers() {
    let app = App::new().await;
    let at = [
        ("11:00", MOVING[0].0, MOVING[0].1),
        ("12:00", 43.3, 5.35),
        ("13:00", 43.3, 5.35),
        ("14:00", 43.3, 5.35),
    ];
    assert_eq!(app.post(&points("2026-11-04", &at)).await.0, 200);
    app.seen_at("2026-11-04T14:30:00Z").await;

    let today = app.locations_at("2026-11-04", "2026-11-04T15:00:00Z").await;
    assert_eq!(
        today["places"],
        json!([
            {"origin": "points", "lat": 43.3, "lon": 5.35,
             "arrival": "2026-11-04T12:00:00Z", "departure": null,
             "arrival_time": "13:00", "departure_time": null,
             "arrived_before": false, "departed_after": false, "duration_s": 10800,
             "ongoing": true, "horizontal_accuracy": null},
        ])
    );

    // L'iPhone se tait depuis plus de 70 min : le séjour s'arrête à son dernier point.
    app.seen_at("2026-11-04T13:49:00Z").await;
    let silent = app.locations_at("2026-11-04", "2026-11-04T15:00:00Z").await;
    assert_eq!(silent["places"][0]["ongoing"], false);
    assert_eq!(silent["places"][0]["departure"], "2026-11-04T14:00:00Z");
    assert_eq!(silent["places"][0]["duration_s"], 7200);

    // Un autre jour que celui de maintenant : jamais en cours.
    app.seen_at("2026-11-05T09:30:00Z").await;
    let later = app.locations_at("2026-11-04", "2026-11-05T10:00:00Z").await;
    assert_eq!(later["places"][0]["ongoing"], false);
    assert_eq!(later["places"][0]["departure_time"], "15:00");

    // Un point ailleurs 2 h 30 après le dernier : trop tard pour dater le départ, le séjour finit
    // toujours à son dernier point.
    let far = points("2026-11-04", &[("16:30", MOVING[2].0, MOVING[2].1)]);
    assert_eq!(app.post(&far).await.0, 200);
    let after = app.locations_at("2026-11-04", "2026-11-05T10:00:00Z").await;
    assert_eq!(after["places"][0]["departure"], "2026-11-04T14:00:00Z");
    assert_eq!(after["places"][0]["duration_s"], 7200);
}

#[tokio::test]
async fn a_single_point_becomes_an_ongoing_stay_after_ten_minutes() {
    let app = App::new().await;
    let at = [
        ("13:40", MOVING[0].0, MOVING[0].1),
        ("13:50", MOVING[1].0, MOVING[1].1),
        ("14:00", 43.3, 5.35),
    ];
    assert_eq!(app.post(&points("2026-11-04", &at)).await.0, 200);
    app.seen_at("2026-11-04T14:20:00Z").await;
    let early = app.locations_at("2026-11-04", "2026-11-04T14:05:00Z").await;
    assert_eq!(early["places"], json!([]), "5 min : peut-être un feu rouge");

    let settled = app.locations_at("2026-11-04", "2026-11-04T14:30:00Z").await;
    assert_eq!(
        settled["places"],
        json!([
            {"origin": "points", "lat": 43.3, "lon": 5.35,
             "arrival": "2026-11-04T14:00:00Z", "departure": null,
             "arrival_time": "15:00", "departure_time": null,
             "arrived_before": false, "departed_after": false, "duration_s": 1800,
             "ongoing": true, "horizontal_accuracy": null},
        ])
    );
    // Sans nouvelle de l'iPhone depuis ce point, rien n'est en cours.
    app.seen_at("2026-11-04T13:59:00Z").await;
    let unheard = app.locations_at("2026-11-04", "2026-11-04T14:30:00Z").await;
    assert_eq!(unheard["places"], json!([]));
}

#[tokio::test]
async fn the_jitter_of_a_stay_is_no_trace() {
    let app = App::new().await;
    // Un trajet, une heure sur place avec un point toutes les 2 min qui saute de 33 m, puis un départ.
    let mut at: Vec<(String, f64, f64)> = vec![
        ("08:00".into(), MOVING[0].0, MOVING[0].1),
        ("08:01".into(), MOVING[1].0, MOVING[1].1),
        ("08:02".into(), 43.3, 5.35),
    ];
    for i in 1..=30 {
        let minutes = 2 + 2 * i;
        let lat = 43.3 + f64::from(i % 2) * 3e-4;
        at.push((
            format!("{:02}:{:02}", 8 + minutes / 60, minutes % 60),
            lat,
            5.35,
        ));
    }
    at.push(("09:04".into(), 43.31, 5.35));
    at.push(("09:05".into(), 43.32, 5.35));
    let at: Vec<(&str, f64, f64)> = at
        .iter()
        .map(|(t, la, lo)| (t.as_str(), *la, *lo))
        .collect();
    assert_eq!(app.post(&points("2026-11-05", &at)).await.0, 200);

    let day = app.locations("2026-11-05").await;
    assert_eq!(day["places"].as_array().unwrap().len(), 1);
    assert_eq!(day["places"][0]["arrival"], "2026-11-05T08:02:00Z");
    assert_eq!(day["places"][0]["departure"], "2026-11-05T09:04:00Z");
    let segments = day["segments"].as_array().unwrap();
    let bounds: Vec<(&Value, &Value)> = segments.iter().map(|s| (&s["start"], &s["end"])).collect();
    assert_eq!(
        bounds,
        [
            (
                &json!("2026-11-05T08:00:00Z"),
                &json!("2026-11-05T08:02:00Z")
            ),
            (
                &json!("2026-11-05T09:04:00Z"),
                &json!("2026-11-05T09:05:00Z")
            ),
        ],
        "le trajet jusqu'à l'arrivée, puis depuis le départ, sans les sauts sur place"
    );
    assert!(
        segments
            .iter()
            .flat_map(|s| s["points"].as_array().unwrap())
            .all(|p| p[0] != json!(43.3003)),
        "aucun point pris sur place"
    );
}

#[tokio::test]
async fn a_workout_with_a_track_cuts_the_place_and_silences_the_iphone() {
    let app = App::new().await;
    // La matinée à la maison, un point toutes les 10 min de 07:00 à 12:00 UTC, à quelques mètres près.
    let home: Vec<(String, f64, f64)> = (0..=30)
        .map(|i| {
            let minutes = 7 * 60 + 10 * i;
            let jitter = f64::from(i % 3) * 1e-4;
            (
                format!("{:02}:{:02}", minutes / 60, minutes % 60),
                43.3 + jitter,
                5.35,
            )
        })
        .collect();
    let home: Vec<(&str, f64, f64)> = home.iter().map(|(t, a, o)| (t.as_str(), *a, *o)).collect();
    assert_eq!(app.post(&points("2026-11-06", &home)).await.0, 200);

    // Une course de 09:00 à 09:45 UTC avec sa trace, et une séance sans trace l'après-midi.
    let garmin = app.garmin().await;
    let track = json!([
        [43.25, 5.3, 0],
        [43.26, 5.31, 1200],
        [43.2700004, 5.3, 2700]
    ]);
    app.workout(
        &garmin,
        4242,
        "running",
        "2026-11-06T10:00:00+01:00",
        2700.4,
        Some(8200.0),
        Some(track),
    )
    .await;
    app.workout(
        &garmin,
        4343,
        "strength_training",
        "2026-11-06T15:00:00+01:00",
        1800.0,
        None,
        None,
    )
    .await;

    let day = app.locations("2026-11-06").await;
    assert_eq!(
        day["workouts"],
        json!([{"activity_id": 4242, "activity_type": "running",
                "start": "2026-11-06T09:00:00Z", "end": "2026-11-06T09:45:00Z",
                "start_time": "10:00", "end_time": "10:45", "distance_m": 8200.0,
                "points": [[43.25, 5.3], [43.26, 5.31], [43.27, 5.3]]}]),
        "la séance sans trace n'y est pas"
    );
    let places: Vec<(&Value, &Value, &Value, &Value)> = day["places"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                &p["arrival"],
                &p["departure"],
                &p["departure_time"],
                &p["duration_s"],
            )
        })
        .collect();
    assert_eq!(
        places,
        [
            (
                &json!("2026-11-06T07:00:00Z"),
                &json!("2026-11-06T09:00:00Z"),
                &json!("10:00"),
                &json!(7200)
            ),
            (
                &json!("2026-11-06T09:45:00Z"),
                &json!("2026-11-06T12:00:00Z"),
                &json!("13:00"),
                &json!(8100)
            ),
        ],
        "le séjour coupé autour de la course"
    );
    assert_eq!(day["places"][1]["arrival_time"], "10:45");
    let during = |at: &Value| {
        let at = at.as_str().unwrap();
        ("2026-11-06T09:00:00Z" < at) && (at < "2026-11-06T09:45:00Z")
    };
    for segment in day["segments"].as_array().unwrap() {
        assert!(
            !during(&segment["start"]) && !during(&segment["end"]),
            "aucun point de l'iPhone pendant la course : {segment}"
        );
        assert!(
            !(segment["start"].as_str() < Some("2026-11-06T09:45:00Z")
                && segment["end"].as_str() > Some("2026-11-06T09:00:00Z")),
            "la trace de l'iPhone ne passe pas par-dessus la course : {segment}"
        );
    }
    // La veille, sans séance.
    assert_eq!(app.locations("2026-11-05").await["workouts"], json!([]));
}

#[tokio::test]
async fn a_run_to_a_friend_then_a_drive_home_is_drawn_once() {
    let app = App::new().await;
    // Une course de la maison (43.3, 5.35) jusqu'à X (43.36, 5.42), avec la montre et le téléphone, de
    // 10:00 à 10:45 UTC ; trois heures chez X, le retour en voiture sans la montre, puis la maison.
    let (home, x) = ((43.3, 5.35), (43.36, 5.42));
    let along = |k: u32| {
        let f = f64::from(k) / 14.0;
        (home.0 + (x.0 - home.0) * f, home.1 + (x.1 - home.1) * f)
    };
    let mut at: Vec<(String, f64, f64)> = Vec::new();
    let mut push = |minutes: u32, (lat, lon): (f64, f64)| {
        at.push((format!("{:02}:{:02}", minutes / 60, minutes % 60), lat, lon));
    };
    for i in 0..12 {
        // La matinée à la maison, à quelques mètres près.
        push(8 * 60 + 10 * i, (home.0 + f64::from(i % 3) * 1e-4, home.1));
    }
    for k in 1..=13 {
        // Le téléphone court aussi : un point toutes les 3 min, le premier une minute après le départ.
        push(10 * 60 - 2 + 3 * k, along(k));
    }
    for minutes in [
        10 * 60 + 40,
        10 * 60 + 45,
        11 * 60,
        12 * 60,
        13 * 60,
        13 * 60 + 45,
    ] {
        // L'arrivée chez X notée pendant le retour au calme, 5 min avant la fin de la séance.
        push(minutes, (x.0 + 2e-4, x.1));
    }
    for (j, k) in [12, 10, 8, 6, 4, 2].into_iter().enumerate() {
        // Le retour en voiture.
        push(13 * 60 + 50 + 5 * j as u32, along(k));
    }
    for minutes in [14 * 60 + 20, 15 * 60, 16 * 60, 17 * 60] {
        push(minutes, home);
    }
    let at: Vec<(&str, f64, f64)> = at.iter().map(|(t, a, o)| (t.as_str(), *a, *o)).collect();
    assert_eq!(app.post(&points("2026-11-07", &at)).await.0, 200);
    let garmin = app.garmin().await;
    let track = json!([[43.3, 5.35, 0], [43.33, 5.39, 1300], [43.36, 5.42, 2700]]);
    let start = "2026-11-07T11:00:00+01:00";
    app.workout(
        &garmin,
        77,
        "running",
        start,
        2700.0,
        Some(9100.0),
        Some(track),
    )
    .await;

    let day = app.locations("2026-11-07").await;
    assert_eq!(day["workouts"].as_array().unwrap().len(), 1);
    assert_eq!(
        (
            &day["workouts"][0]["activity_type"],
            &day["workouts"][0]["distance_m"]
        ),
        (&json!("running"), &json!(9100.0))
    );
    assert_eq!(
        day["workouts"][0]["points"],
        json!([[43.3, 5.35], [43.33, 5.39], [43.36, 5.42]])
    );
    let places: Vec<(&str, &str)> = day["places"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                p["arrival"].as_str().unwrap(),
                p["departure"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        places,
        [
            ("2026-11-07T08:00:00Z", "2026-11-07T10:00:00Z"),
            ("2026-11-07T10:45:00Z", "2026-11-07T13:50:00Z"),
            ("2026-11-07T14:20:00Z", "2026-11-07T17:00:00Z"),
        ],
        "la maison jusqu'au départ de la course, X depuis sa fin, sans bord de quelques minutes, la maison au retour"
    );
    let segments: Vec<(&str, &str)> = day["segments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| (s["start"].as_str().unwrap(), s["end"].as_str().unwrap()))
        .collect();
    assert!(
        segments
            .iter()
            .all(|(from, to)| *to <= "2026-11-07T10:00:00Z" || *from >= "2026-11-07T10:45:00Z"),
        "aucun point de l'iPhone pendant la course : {segments:?}"
    );
    assert!(
        segments.contains(&("2026-11-07T13:50:00Z", "2026-11-07T14:20:00Z")),
        "le retour en voiture, du départ de X à l'arrivée à la maison : {segments:?}"
    );
}
