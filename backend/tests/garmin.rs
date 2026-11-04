//! Garmin Connect de bout en bout, sans aucun réseau extérieur : le connecteur contre un Garmin simulé
//! et la vraie API d'ingestion, puis le vrai worker et l'API de lecture. Les réponses suivent la
//! structure relevée sur le vrai compte le 4 nov. 2026 (voir docs/poc.md) ; toutes les valeurs, noms,
//! lieux et coordonnées sont fictifs, et le fichier FIT est fabriqué ici.

mod common;
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use base64::Engine;
use chrono::{Days, NaiveDate, Utc};
use common::*;
use life_tracker::api::{self, READ_ROUTES};
use life_tracker::collector::garmin::{
    self, Collector, Config, ServiceError, TickError, paris_today,
};
use life_tracker::interpreters::INTERPRETERS;
use life_tracker::privacy::Zones;
use life_tracker::{admin, engine, replay};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::watch;

const PASSWORD: &str = "un mot de passe assez long";
const KEY: &str = "cle-garmin-fictive";
/// Ce que la liste blanche doit écarter, et qui ne doit jamais sortir.
const OWNER: &str = "Personne Sentinelle";
const ACTIVITY_NAME: &str = "Sortie Sentinelle";
/// Les coordonnées fictives, reconnaissables : jamais lues par le MCP.
const LAT: f64 = 43.212_345;
const LON: f64 = 5.432_109;
const PLACE: &str = "Ville Fictive";

fn today() -> NaiveDate {
    paris_today(Utc::now())
}

fn day(ago: u64) -> NaiveDate {
    today() - Days::new(ago)
}

// ─── Les réponses de Garmin ──────────────────────────────────────────────────

fn summary(date: NaiveDate, steps: Option<i64>) -> Value {
    json!({
        "calendarDate": date.to_string(), "includesWellnessData": steps.is_some(),
        "includesActivityData": true, "userProfilePk": 424_242, "uuid": "uuid-sentinelle",
        "movement": {"steps": {"value": steps, "distanceInMeters": 6123.4, "goal": 7000}},
        "calories": {"burnedActive": 512, "burnedResting": 1838, "burnedTotal": 2350},
        "heartRate": {"restingValue": 52},
    })
}

fn night(date: NaiveDate, seconds: i64) -> Value {
    let end = date
        .and_hms_opt(5, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp_millis();
    json!({
        "calendarDate": date.to_string(), "id": 1, "userProfilePK": 424_242,
        "sleepTimeSeconds": seconds, "deepSleepSeconds": 4_920, "lightSleepSeconds": 14_400,
        "remSleepSeconds": 6_660, "awakeSleepSeconds": 660,
        "sleepStartTimestampGMT": end - seconds * 1000 - 660_000, "sleepEndTimestampGMT": end,
    })
}

fn workout(id: u64, date: NaiveDate, gps: bool) -> Value {
    let mut raw = json!({
        "activityId": id, "activityName": ACTIVITY_NAME,
        "activityType": {"typeKey": "running", "typeId": 1},
        "startTimeGMT": format!("{date} 06:30:00"), "startTimeLocal": format!("{date} 07:30:00"),
        "distance": 5012.3, "duration": 1802.4, "movingDuration": 1780.0, "averageSpeed": 2.78,
        "averageHR": 151.0, "maxHR": 176.0, "calories": 389.0,
        "ownerDisplayName": OWNER, "ownerFullName": OWNER,
    });
    if gps {
        raw["startLatitude"] = json!(LAT);
        raw["startLongitude"] = json!(LON);
        raw["endLatitude"] = json!(LAT + 0.01);
        raw["endLongitude"] = json!(LON + 0.01);
        raw["locationName"] = json!(PLACE);
    }
    raw
}

/// Un FIT minimal : en-tête, définition du message `record` (heure, latitude, longitude), points, CRC.
fn fit(points: &[(f64, f64)]) -> Vec<u8> {
    fn crc(data: &[u8]) -> u16 {
        const TABLE: [u16; 16] = [
            0x0000, 0xCC01, 0xD801, 0x1400, 0xF001, 0x3C00, 0x2800, 0xE401, 0xA001, 0x6C00, 0x7800,
            0xB401, 0x5000, 0x9C01, 0x8801, 0x4400,
        ];
        let mut crc = 0u16;
        for byte in data {
            for nibble in [byte & 0xF, byte >> 4] {
                let tmp = TABLE[usize::from(crc & 0xF)];
                crc = ((crc >> 4) & 0x0FFF) ^ tmp ^ TABLE[usize::from(nibble)];
            }
        }
        crc
    }
    let semicircles = |degrees: f64| (degrees * 2_147_483_648.0 / 180.0).round() as i32;
    let mut data = vec![0x40, 0, 0, 20, 0, 3, 253, 4, 0x86, 0, 4, 0x85, 1, 4, 0x85];
    let start: u32 = 1_100_000_000;
    for (i, (lat, lon)) in points.iter().enumerate() {
        data.push(0x00);
        data.extend((start + i as u32 * 10).to_le_bytes());
        data.extend(semicircles(*lat).to_le_bytes());
        data.extend(semicircles(*lon).to_le_bytes());
    }
    let mut file = vec![14, 0x20];
    file.extend(2132u16.to_le_bytes());
    file.extend((data.len() as u32).to_le_bytes());
    file.extend(b".FIT");
    let header_crc = crc(&file);
    file.extend(header_crc.to_le_bytes());
    file.extend(&data);
    let file_crc = crc(&file);
    file.extend(file_crc.to_le_bytes());
    file
}

fn zipped(name: &str, content: &[u8]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file(name, options).unwrap();
    zip.write_all(content).unwrap();
    zip.finish().unwrap().into_inner()
}

/// Une trace fictive : un aller vers le nord-est, un virage, un retour vers le sud.
fn route() -> Vec<(f64, f64)> {
    let mut points = Vec::new();
    for i in 0..60 {
        points.push((LAT + f64::from(i) * 1e-4, LON + f64::from(i) * 1e-4));
    }
    for i in 1..60 {
        points.push((LAT + 0.0059 - f64::from(i) * 1e-4, LON + 0.0059));
    }
    points
}

// ─── Le Garmin simulé ────────────────────────────────────────────────────────

#[derive(Default)]
struct Data {
    summaries: Vec<Value>,
    sleeps: Vec<Value>,
    activities: Vec<Value>,
    files: HashMap<u64, Vec<u8>>,
    /// Statut de l'échange de la clé : 200, ou un refus.
    mint_status: u16,
    mints: u32,
    /// Le prochain jeton présenté est refusé une fois : Garmin l'a révoqué.
    revoke_next_bearer: bool,
    issued: Vec<String>,
    /// Chemin et paramètres de chaque appel de lecture.
    calls: Vec<(String, HashMap<String, String>)>,
    downloads: Vec<u64>,
}

#[derive(Clone, Default)]
struct Fake {
    url: String,
    data: Arc<Mutex<Data>>,
}

fn in_range(date: &str, query: &HashMap<String, String>) -> bool {
    query["startDate"].as_str() <= date && date <= query["endDate"].as_str()
}

fn jwt(n: u32) -> String {
    let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let exp = Utc::now().timestamp() + 24 * 3600;
    format!(
        "{}.{}.signature{n}",
        engine.encode(r#"{"alg":"RS256"}"#),
        engine.encode(format!(r#"{{"exp":{exp}}}"#))
    )
}

async fn mint(State(fake): State<Fake>, headers: HeaderMap) -> Response {
    let mut data = fake.data.lock().unwrap();
    assert_eq!(headers["authorization"], format!("ApiKey {KEY}"));
    if data.mint_status != 200 {
        return StatusCode::from_u16(data.mint_status)
            .unwrap()
            .into_response();
    }
    data.mints += 1;
    let token = jwt(data.mints);
    data.issued.push(token.clone());
    token.into_response()
}

/// Le jeton présenté doit être le dernier délivré, sauf révocation simulée.
fn authorized(data: &mut Data, headers: &HeaderMap) -> bool {
    let presented = headers["authorization"].to_str().unwrap().to_owned();
    if data.revoke_next_bearer {
        data.revoke_next_bearer = false;
        return false;
    }
    data.issued
        .last()
        .is_some_and(|t| presented == format!("Bearer {t}"))
}

async fn read(
    State(fake): State<Fake>,
    Path(path): Path<String>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let mut data = fake.data.lock().unwrap();
    if !authorized(&mut data, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    data.calls.push((path.clone(), query.clone()));
    let body = if path.starts_with("usersummary-service") {
        let days: Vec<Value> = data
            .summaries
            .iter()
            .filter(|s| in_range(s["calendarDate"].as_str().unwrap(), &query))
            .cloned()
            .collect();
        json!({ "data": days })
    } else if path.starts_with("sleep-service") {
        Value::Array(
            data.sleeps
                .iter()
                .filter(|s| in_range(s["calendarDate"].as_str().unwrap(), &query))
                .cloned()
                .collect(),
        )
    } else if path.starts_with("activitylist-service") {
        let start: usize = query["start"].parse().unwrap();
        let limit: usize = query["limit"].parse().unwrap();
        Value::Array(
            data.activities
                .iter()
                .filter(|a| in_range(&a["startTimeGMT"].as_str().unwrap()[..10], &query))
                .skip(start)
                .take(limit)
                .cloned()
                .collect(),
        )
    } else if let Some(id) = path.strip_prefix("download-service/files/activity/") {
        let id: u64 = id.parse().unwrap();
        data.downloads.push(id);
        return match data.files.get(&id) {
            Some(file) => (
                [("content-type", "application/x-zip-compressed")],
                file.clone(),
            )
                .into_response(),
            None => StatusCode::NOT_FOUND.into_response(),
        };
    } else {
        panic!("chemin inattendu {path}");
    };
    axum::Json(body).into_response()
}

impl Fake {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let fake = Self {
            url: format!("http://{}", listener.local_addr().unwrap()),
            data: Arc::new(Mutex::new(Data {
                mint_status: 200,
                ..Data::default()
            })),
        };
        let app = Router::new()
            .route(
                "/personalapitoken-service/personal-api/personal-api-key/token",
                post(mint),
            )
            .route("/{*path}", get(read))
            .with_state(fake.clone());
        tokio::spawn(async move { axum::serve(listener, app).await });
        fake
    }

    fn with<T>(&self, change: impl FnOnce(&mut Data) -> T) -> T {
        change(&mut self.data.lock().unwrap())
    }

    fn calls(&self, prefix: &str) -> Vec<HashMap<String, String>> {
        self.with(|d| {
            d.calls
                .iter()
                .filter(|(p, _)| p.starts_with(prefix))
                .map(|(_, q)| q.clone())
                .collect()
        })
    }

    fn clear_calls(&self) {
        self.with(|d| {
            d.calls.clear();
            d.downloads.clear();
        });
    }
}

// ─── Le banc : base, API, worker, connecteur ─────────────────────────────────

struct Bench {
    db: TestDb,
    base: String,
    token: String,
    service: String,
    cookie: String,
    http: reqwest::Client,
    state_dir: PathBuf,
    _stop: watch::Sender<bool>,
}

impl Drop for Bench {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.state_dir);
    }
}

impl Bench {
    async fn new() -> Self {
        Self::with_zones(None).await
    }

    /// Le vrai routeur, avec ses zones de confidentialité comme `serve` les lit.
    async fn with_zones(zones: Option<&str>) -> Self {
        let db = TestDb::new().await;
        admin::set_password(&db.admin, "moi", PASSWORD)
            .await
            .unwrap();
        let service = admin::create_service_token(&db.admin, "mcp").await.unwrap();
        let token = admin::create_source(&db.admin, "garmin", Some("12 hours"))
            .await
            .unwrap();
        for kind in [
            "garmin.daily_summary",
            "garmin.sleep",
            "garmin.activity",
            "garmin.activity_track",
            "record.cancel",
        ] {
            admin::grant_kind(&db.admin, "garmin", kind).await.unwrap();
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
                zones: Zones::parse(zones).0,
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
        let state_dir = std::env::temp_dir().join(unique_name());
        std::fs::create_dir_all(&state_dir).unwrap();
        Self {
            db,
            base,
            token,
            service,
            cookie,
            http,
            state_dir,
            _stop: stop,
        }
    }

    fn collector(&self, fake: &Fake) -> Collector {
        Collector::new(Config {
            api_url: fake.url.clone(),
            api_key: KEY.to_owned(),
            ingest_url: self.base.clone(),
            token: self.token.clone(),
            source: "garmin".to_owned(),
            state_dir: self.state_dir.clone(),
            interval: Duration::from_secs(900),
            pause: Duration::ZERO,
        })
        .unwrap()
    }

    async fn journal(&self, kind: &str) -> i64 {
        self.db
            .count(&format!(
                "SELECT count(*) FROM core.records WHERE kind = '{kind}'"
            ))
            .await
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
        panic!("le worker n'a pas tout lu");
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

    /// Une lecture avec le jeton de service du MCP : le statut et le corps en texte.
    async fn as_mcp(&self, path: &str, query: &[(&str, &str)]) -> (u16, String) {
        let response = self
            .http
            .get(format!("{}{path}", self.base))
            .bearer_auth(&self.service)
            .query(query)
            .send()
            .await
            .unwrap();
        (response.status().as_u16(), response.text().await.unwrap())
    }

    async fn value(&self, date: NaiveDate, metric: &str) -> Value {
        let month = self
            .get(&format!("/v1/months/{}", &date.to_string()[..7]))
            .await;
        month["days"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["date"] == date.to_string())
            .unwrap()["values"][metric]
            .clone()
    }

    async fn last_seen(&self) -> Option<chrono::DateTime<Utc>> {
        sqlx::query_scalar("SELECT last_seen_at FROM core.sources WHERE name = 'garmin'")
            .fetch_one(&self.db.app)
            .await
            .unwrap()
    }
}

// ─── Le connecteur ───────────────────────────────────────────────────────────

#[tokio::test]
async fn the_first_pass_backfills_in_chunks_then_rereads_a_week_with_the_same_token() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    fake.with(|d| {
        d.summaries = vec![
            summary(day(300), Some(4000)),
            summary(day(2), Some(8412)),
            summary(day(1), None),
        ];
        d.sleeps = vec![night(day(2), 25_980), night(day(1), 0)];
        // 150 séances sur l'année : deux pages de la liste.
        d.activities = (0..150)
            .map(|i| workout(1000 + i, day(10 + i * 2), false))
            .collect();
    });
    let mut collector = bench.collector(&fake);
    let report = collector.tick().await.unwrap();
    assert_eq!(report.accepted, 2 + 1 + 150);
    assert_eq!((report.rejected, report.cancelled), (0, 0));
    assert_eq!(
        report.skipped, 2,
        "un jour sans données, une nuit sans sommeil"
    );
    assert!(collector.backfilled());

    let summaries = fake.calls("usersummary-service");
    assert_eq!(
        summaries.len(),
        13,
        "des plages de 31 jours : Garmin refuse 400 jours"
    );
    assert_eq!(summaries[0]["startDate"], day(399).to_string());
    assert_eq!(summaries[12]["endDate"], today().to_string());
    let sleeps = fake.calls("sleep-service");
    assert_eq!(sleeps.len(), 13, "des plages de 31 jours");
    assert_eq!(sleeps[0]["startDate"], day(399).to_string());
    assert_eq!(sleeps[12]["endDate"], today().to_string());
    for pair in sleeps.windows(2) {
        let end: NaiveDate = pair[0]["endDate"].parse().unwrap();
        assert_eq!(pair[1]["startDate"], (end + Days::new(1)).to_string());
    }
    let pages: Vec<String> = fake
        .calls("activitylist-service")
        .iter()
        .map(|q| q["start"].clone())
        .collect();
    assert_eq!(pages, ["0", "100"]);
    assert!(bench.last_seen().await.is_some(), "signal de vie");

    // La liste blanche : ni propriétaire, ni nom de séance, ni identifiants du profil.
    let journal: String =
        sqlx::query_scalar("SELECT string_agg(payload::text, '') FROM core.records")
            .fetch_one(&bench.db.app)
            .await
            .unwrap();
    for secret in [OWNER, ACTIVITY_NAME, "uuid-sentinelle", "424242"] {
        assert!(!journal.contains(secret), "{secret} au journal");
    }

    // Le passage suivant relit une semaine, sans rien renvoyer, avec le même jeton.
    fake.clear_calls();
    let report = collector.tick().await.unwrap();
    assert_eq!(
        (report.accepted, report.duplicate, report.cancelled),
        (0, 0, 0)
    );
    let summaries = fake.calls("usersummary-service");
    assert_eq!(summaries[0]["startDate"], day(7).to_string());
    assert_eq!(fake.calls("sleep-service").len(), 1);
    assert_eq!(fake.with(|d| d.mints), 1, "le jeton de 24 h resservi");
    assert_eq!(bench.journal("garmin.activity").await, 150);
    assert_eq!(
        collector.subjects(),
        5,
        "l'état oublie les sujets de plus de 14 jours"
    );

    // Un nouveau collecteur (redémarrage) reprend l'état : il ne remonte pas 400 jours.
    fake.clear_calls();
    let mut restarted = bench.collector(&fake);
    restarted.tick().await.unwrap();
    assert_eq!(
        fake.calls("usersummary-service")[0]["startDate"],
        day(7).to_string()
    );
}

#[tokio::test]
async fn a_lost_state_only_gives_duplicates() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    fake.with(|d| {
        d.summaries = vec![summary(day(2), Some(8412))];
        d.sleeps = vec![night(day(2), 25_980)];
        d.activities = vec![workout(7, day(2), false)];
    });
    bench.collector(&fake).tick().await.unwrap();
    std::fs::remove_file(bench.state_dir.join("garmin-state.json")).unwrap();
    let report = bench.collector(&fake).tick().await.unwrap();
    assert_eq!(
        (report.accepted, report.duplicate, report.cancelled),
        (0, 3, 0)
    );
    assert_eq!(bench.journal("record.cancel").await, 0);
}

#[tokio::test]
async fn a_revision_sends_the_new_version_and_cancels_the_previous() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    let date = day(2);
    fake.with(|d| {
        d.summaries = vec![summary(date, Some(6000))];
        d.activities = vec![workout(7, date, false)];
    });
    let mut collector = bench.collector(&fake);
    collector.tick().await.unwrap();
    bench.settled().await;
    assert_eq!(bench.value(date, "steps").await, 6000.0);

    // Une synchro tardive : plus de pas, et une séance recalculée.
    fake.with(|d| {
        d.summaries = vec![summary(date, Some(8412))];
        d.activities[0]["calories"] = json!(401.0);
    });
    let report = collector.tick().await.unwrap();
    assert_eq!((report.accepted, report.cancelled), (2, 2));
    let cancels: Vec<Value> = sqlx::query_scalar(
        "SELECT payload FROM core.records WHERE kind = 'record.cancel' ORDER BY seq",
    )
    .fetch_all(&bench.db.app)
    .await
    .unwrap();
    assert_eq!(cancels.len(), 2);
    assert_eq!(cancels[0]["source"], "garmin");
    assert!(cancels.iter().any(|c| {
        c["key"]
            .as_str()
            .unwrap()
            .starts_with(&format!("summary:{date}:"))
    }));
    // La nouvelle version précède son annulation dans le même lot.
    let order: Vec<String> = sqlx::query_scalar(
        "SELECT kind FROM core.records WHERE kind <> 'garmin.sleep' ORDER BY seq OFFSET 2",
    )
    .fetch_all(&bench.db.app)
    .await
    .unwrap();
    assert_eq!(order[1], "record.cancel");
    assert_ne!(order[0], "record.cancel");

    bench.settled().await;
    assert_eq!(bench.value(date, "steps").await, 8412.0);
    assert_eq!(
        bench.value(date, "workout").await,
        1.0,
        "une seule séance vivante"
    );
    let (_, steps) = bench
        .as_mcp(
            "/v1/occurrences",
            &[
                ("metric", "steps"),
                ("from", &date.to_string()),
                ("to", &date.to_string()),
            ],
        )
        .await;
    let steps: Value = serde_json::from_str(&steps).unwrap();
    assert_eq!(steps["total"], 1, "une seule version vivante");
    assert_eq!(steps["occurrences"][0]["value"], 8412.0);

    // Retour au contenu d'origine : sa clé est annulée, il en prend une voisine.
    fake.with(|d| d.summaries = vec![summary(date, Some(6000))]);
    let report = collector.tick().await.unwrap();
    assert_eq!(
        (report.accepted, report.duplicate, report.cancelled),
        (1, 0, 1)
    );
    bench.settled().await;
    assert_eq!(bench.value(date, "steps").await, 6000.0);

    // Le rejeu complet donne le même résultat.
    let rebuilt = replay::rebuild(&bench.db.admin, INTERPRETERS, false)
        .await
        .unwrap();
    assert!(rebuilt.switched);
    assert!(rebuilt.comparison.differences.is_empty());
}

#[tokio::test]
async fn a_revoked_bearer_is_minted_again_once() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    fake.with(|d| d.summaries = vec![summary(day(1), Some(5000))]);
    let mut collector = bench.collector(&fake);
    collector.tick().await.unwrap();
    fake.with(|d| d.revoke_next_bearer = true);
    collector.tick().await.unwrap();
    assert_eq!(fake.with(|d| d.mints), 2);
}

#[tokio::test]
async fn a_rejected_key_sends_nothing_and_no_sign_of_life() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    fake.with(|d| {
        d.mint_status = 401;
        d.summaries = vec![summary(day(1), Some(5000))];
    });
    let mut collector = bench.collector(&fake);
    assert_eq!(
        collector.tick().await.unwrap_err(),
        TickError::Service(ServiceError::KeyRejected { status: 401 })
    );
    fake.with(|d| d.mint_status = 403);
    assert_eq!(
        collector.tick().await.unwrap_err(),
        TickError::Service(ServiceError::KeyRejected { status: 403 })
    );
    assert_eq!(
        bench.last_seen().await,
        None,
        "le seuil de silence alertera"
    );
    assert_eq!(bench.journal("garmin.daily_summary").await, 0);
    assert!(!collector.backfilled());
}

#[tokio::test]
async fn a_missing_cancel_grant_sends_everything_again_once_granted() {
    let bench = Bench::new().await;
    admin::revoke_kind(&bench.db.admin, "garmin", "record.cancel")
        .await
        .unwrap();
    let fake = Fake::start().await;
    fake.with(|d| d.summaries = vec![summary(day(1), Some(5000))]);
    let mut collector = bench.collector(&fake);
    collector.tick().await.unwrap();
    fake.with(|d| d.summaries = vec![summary(day(1), Some(6000))]);
    assert!(matches!(
        collector.tick().await.unwrap_err(),
        TickError::Ingest {
            reason: "kind_not_declared",
            ..
        }
    ));
    admin::grant_kind(&bench.db.admin, "garmin", "record.cancel")
        .await
        .unwrap();
    let report = collector.tick().await.unwrap();
    assert_eq!((report.duplicate, report.cancelled), (1, 1));
    bench.settled().await;
    assert_eq!(bench.value(day(1), "steps").await, 6000.0);
}

#[tokio::test]
async fn a_track_is_downloaded_once_for_each_workout_with_gps() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    let date = day(1);
    fake.with(|d| {
        d.activities = vec![
            workout(1, date, true),
            workout(2, date, false),
            workout(3, date, true),
        ];
        d.files.insert(1, zipped("1_ACTIVITY.fit", &fit(&route())));
        // La séance 3 n'a pas de fichier : 404, notée sans trace.
    });
    let mut collector = bench.collector(&fake);
    let report = collector.tick().await.unwrap();
    assert_eq!(report.accepted, 3 + 1);
    assert_eq!(
        fake.with(|d| d.downloads.clone()),
        [1, 3],
        "jamais sans GPS"
    );
    let points: Value = sqlx::query_scalar(
        "SELECT payload->'points' FROM core.records WHERE kind = 'garmin.activity_track'",
    )
    .fetch_one(&bench.db.app)
    .await
    .unwrap();
    let points = points.as_array().unwrap();
    assert!(
        (3..20).contains(&points.len()),
        "simplifiée : {}",
        points.len()
    );
    assert_eq!(points[0], json!([LAT, LON, 0]));
    fake.clear_calls();
    collector.tick().await.unwrap();
    assert!(fake.with(|d| d.downloads.is_empty()), "une fois par séance");

    bench.settled().await;
    let activity = bench.get(&format!("/v1/days/{date}/activity")).await;
    let workouts = activity["workouts"].as_array().unwrap();
    assert_eq!(workouts.len(), 3);
    assert_eq!(workouts[0]["has_track"], true);
    assert_eq!(workouts[1]["has_track"], false);
    assert_eq!(workouts[0]["details"]["location_name"], PLACE);
    assert_eq!(workouts[0]["details"]["start_lat"], json!(LAT));
    assert_eq!(workouts[0]["time"], "07:30");
    let track = bench.get("/v1/activities/1/track").await;
    assert_eq!(track["points"].as_array().unwrap().len(), points.len());
    let missing = bench
        .http
        .get(format!("{}/v1/activities/3/track", bench.base))
        .header("cookie", &bench.cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), 404);
}

// ─── La lecture ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_day_without_data_is_never_a_day_without_activity() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    fake.with(|d| {
        d.summaries = vec![
            summary(day(4), Some(8412)),
            summary(day(3), None),
            summary(day(2), Some(0)),
        ];
        d.sleeps = vec![night(day(4), 25_980)];
        d.activities = vec![workout(7, day(4), false)];
    });
    bench.collector(&fake).tick().await.unwrap();
    bench.settled().await;
    assert_eq!(bench.value(day(4), "steps").await, 8412.0);
    assert_eq!(
        bench.value(day(3), "steps").await,
        Value::Null,
        "montre pas portée"
    );
    assert_eq!(
        bench.value(day(2), "steps").await,
        0.0,
        "une vraie journée à zéro pas"
    );
    assert_eq!(bench.value(day(4), "sleep").await, 433.0);
    assert_eq!(bench.value(day(3), "sleep").await, Value::Null);
    assert_eq!(bench.value(day(4), "calories.burned").await, 2350.0);
    assert_eq!(bench.value(day(4), "workout").await, 1.0);
    assert_eq!(
        bench.value(day(3), "workout").await,
        0.0,
        "suivi, sans séance"
    );
    assert_eq!(
        bench.value(day(5), "workout").await,
        Value::Null,
        "avant la première"
    );

    let activity = bench.get(&format!("/v1/days/{}/activity", day(4))).await;
    assert_eq!(activity["steps"]["value"], 8412.0);
    assert_eq!(activity["calories"]["details"]["active"], 512);
    assert_eq!(activity["sleep"]["value"], 433.0);
    assert_eq!(activity["sleep"]["details"]["rem_seconds"], 6_660);
    let empty = bench.get(&format!("/v1/days/{}/activity", day(3))).await;
    assert_eq!(empty["steps"], Value::Null);
    assert_eq!(empty["sleep"], Value::Null);
    assert_eq!(empty["workouts"], json!([]));
}

// ─── Ce que voit le MCP : tout, à travers les zones de confidentialité ────────

/// Une zone fictive de 600 m centrée sur le départ des séances fictives.
const ZONE: &str = "43.212345,5.432109,600";
const ZONE_RADIUS_M: f64 = 600.0;

/// La distance en mètres au centre de la zone, sur un plan local.
fn from_zone(lat: f64, lon: f64) -> f64 {
    let m = 6_371_008.8 * std::f64::consts::PI / 180.0;
    ((lon - LON) * m * LAT.to_radians().cos()).hypot((lat - LAT) * m)
}

/// La distance au centre de la zone du segment le plus proche : jamais une ligne à travers.
fn segment_from_zone(a: [f64; 2], b: [f64; 2]) -> f64 {
    let (n, step) = (50, 1.0 / 50.0);
    (0..=n)
        .map(|i| {
            let t = f64::from(i) * step;
            from_zone(a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1]))
        })
        .fold(f64::INFINITY, f64::min)
}

/// Un tracé en zigzag de 30 m, un point tous les 50 m environ : la simplification les garde tous.
fn path(stops: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut points = Vec::new();
    for pair in stops.windows(2) {
        let ((lat0, lon0), (lat1, lon1)) = (pair[0], pair[1]);
        // Environ 50 m par point : un degré vaut au plus 111 km.
        let n = (((lat1 - lat0).abs() + (lon1 - lon0).abs()) * 111_000.0 / 50.0).ceil() as i32;
        for i in 0..n {
            let t = f64::from(i) / f64::from(n);
            let wiggle = if i % 2 == 0 { 0.0 } else { 0.0003 };
            points.push((lat0 + t * (lat1 - lat0) + wiggle, lon0 + t * (lon1 - lon0)));
        }
    }
    points.push(*stops.last().unwrap());
    points
}

fn run_with_track(id: u64, date: NaiveDate, stops: &[(f64, f64)]) -> (Value, Vec<u8>) {
    let mut raw = workout(id, date, true);
    let (first, last) = (stops[0], *stops.last().unwrap());
    raw["startLatitude"] = json!(first.0);
    raw["startLongitude"] = json!(first.1);
    raw["endLatitude"] = json!(last.0);
    raw["endLongitude"] = json!(last.1);
    (raw, zipped(&format!("{id}.fit"), &fit(&path(stops))))
}

/// Trois séances : départ dans la zone et arrivée dehors, boucle qui part et revient dans la zone,
/// et une séance loin de tout.
fn three_runs(fake: &Fake) {
    let date = day(1);
    let runs = [
        run_with_track(11, date, &[(LAT, LON), (LAT, LON + 0.037)]),
        run_with_track(
            12,
            date,
            &[
                (LAT, LON),
                (LAT, LON + 0.025),
                (LAT + 0.01, LON + 0.025),
                (LAT, LON),
            ],
        ),
        run_with_track(13, date, &[(LAT + 0.05, LON), (LAT + 0.05, LON + 0.03)]),
    ];
    fake.with(|d| {
        for (raw, file) in runs {
            let id = raw["activityId"].as_u64().unwrap();
            d.activities.push(raw);
            d.files.insert(id, file);
        }
        d.summaries = vec![summary(date, Some(8412))];
        d.sleeps = vec![night(date, 25_980)];
    });
}

impl Bench {
    async fn mcp_json(&self, path: &str, query: &[(&str, &str)]) -> Value {
        let (status, body) = self.as_mcp(path, query).await;
        assert_eq!(status, 200, "{path} : {body}");
        serde_json::from_str(&body).unwrap()
    }

    /// Les séances du jour, telles que le jeton de service les lit.
    async fn mcp_workouts(&self) -> Vec<Value> {
        let date = day(1).to_string();
        let list = self
            .mcp_json(
                "/v1/occurrences",
                &[("metric", "workout"), ("from", &date), ("to", &date)],
            )
            .await;
        list["occurrences"].as_array().unwrap().clone()
    }
}

/// Décision du 4 nov. 2026 : le MCP voit les détails des métriques Garmin, et leurs coordonnées à
/// travers les zones. Sans zone, aucune coordonnée ni aucun parcours ; le reste, si.
#[tokio::test]
async fn without_a_zone_the_mcp_gets_every_detail_but_no_coordinate_nor_route() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    three_runs(&fake);
    bench.collector(&fake).tick().await.unwrap();
    bench.settled().await;
    let date = day(1).to_string();

    let workouts = bench.mcp_workouts().await;
    assert_eq!(workouts.len(), 3);
    for workout in &workouts {
        let details = &workout["details"];
        assert_eq!(details["avg_hr"], 151);
        assert_eq!(details["max_hr"], 176);
        assert_eq!(details["moving_s"], 1780);
        assert_eq!(details["location_name"], PLACE);
        for point in ["start", "end"] {
            assert_eq!(details[format!("{point}_lat")], Value::Null);
            assert_eq!(details[format!("{point}_hidden")], true);
        }
        let id = workout["id"].as_str().unwrap();
        let (status, body) = bench.as_mcp("/v1/route", &[("id", id)]).await;
        assert_eq!(status, 403, "{body}");
        assert!(body.contains("privacy_zone_missing"));
    }
    let sleep = bench
        .mcp_json(
            "/v1/occurrences",
            &[("metric", "sleep"), ("from", &date), ("to", &date)],
        )
        .await;
    assert_eq!(sleep["occurrences"][0]["details"]["rem_seconds"], 6_660);
    let calories = bench
        .mcp_json(
            "/v1/occurrences",
            &[
                ("metric", "calories.burned"),
                ("from", &date),
                ("to", &date),
            ],
        )
        .await;
    assert_eq!(calories["occurrences"][0]["details"]["active"], 512);

    // Toutes les routes du jeton : ni coordonnée, ni point de trace.
    let month = &date[..7];
    let mut reads: Vec<(String, Vec<(&str, &str)>)> = Vec::new();
    for metric in ["workout", "workout.track", "steps", "sleep"] {
        for form in ["list", "daily", "first_last"] {
            reads.push((
                "/v1/occurrences".into(),
                vec![
                    ("metric", metric),
                    ("from", &date),
                    ("to", &date),
                    ("form", form),
                ],
            ));
        }
    }
    for level in ["activity_type", "place"] {
        reads.push((
            "/v1/top".into(),
            vec![
                ("metric", "workout"),
                ("level", level),
                ("from", &date),
                ("to", &date),
            ],
        ));
    }
    reads.push(("/v1/search".into(), vec![("q", "ville")]));
    reads.push((
        "/v1/item".into(),
        vec![
            ("metric", "workout"),
            ("level", "place"),
            ("key", "ville fictive"),
        ],
    ));
    reads.push((format!("/v1/days/{date}"), vec![]));
    reads.push((format!("/v1/days/{date}/music"), vec![]));
    reads.push((format!("/v1/months/{month}"), vec![]));
    reads.push(("/v1/inventory".into(), vec![]));
    reads.push(("/v1/sources/health".into(), vec![]));
    reads.push(("/v1/route".into(), vec![("id", "1:0")]));
    for route in READ_ROUTES {
        let prefix = route.split('{').next().unwrap();
        assert!(
            reads.iter().any(|(p, _)| p.starts_with(prefix))
                || ["/v1/durations", "/v1/discoveries"].contains(&route),
            "{route} non essayée"
        );
    }
    for (path, query) in &reads {
        let (status, body) = bench.as_mcp(path, query).await;
        assert!([200, 400, 403].contains(&status), "{path} : {status}");
        for forbidden in [
            "43.2",
            "43.26",
            "5.43",
            "5.46",
            "\"points\"",
            "\"segments\"",
        ] {
            assert!(
                !body.contains(forbidden),
                "{forbidden} dans {path} : {body}"
            );
        }
    }

    // La session voit tout, sans zone : un seul morceau, départ compris.
    let id = workouts[0]["id"].as_str().unwrap().to_owned();
    let full = bench.get(&format!("/v1/route?id={id}")).await;
    assert_eq!(full["segments"].as_array().unwrap().len(), 1);
    assert_eq!(full["trimmed"], false);
    let start: [f64; 2] = serde_json::from_value(full["segments"][0][0].clone()).unwrap();
    assert!(from_zone(start[0], start[1]) < 2.0, "au mètre près");
}

#[tokio::test]
async fn a_zone_hides_every_coordinate_and_point_inside_it() {
    let bench = Bench::with_zones(Some(ZONE)).await;
    let fake = Fake::start().await;
    three_runs(&fake);
    bench.collector(&fake).tick().await.unwrap();
    bench.settled().await;

    let workouts = bench.mcp_workouts().await;
    let by_id = |id: u64| {
        workouts
            .iter()
            .find(|w| w["details"]["activity_id"] == id)
            .unwrap()
            .clone()
    };
    // Départ dans la zone, arrivée dehors ; boucle de la zone à la zone ; loin de tout.
    for (id, start_hidden, end_hidden) in [(11, true, false), (12, true, true), (13, false, false)]
    {
        let details = &by_id(id)["details"];
        for (point, hidden) in [("start", start_hidden), ("end", end_hidden)] {
            if hidden {
                assert_eq!(details[format!("{point}_lat")], Value::Null, "{id} {point}");
                assert_eq!(details[format!("{point}_hidden")], true);
            } else {
                let (lat, lon) = (
                    details[format!("{point}_lat")].as_f64().unwrap(),
                    details[format!("{point}_lon")].as_f64().unwrap(),
                );
                assert!(from_zone(lat, lon) > ZONE_RADIUS_M, "{id} {point}");
                assert_eq!(details.get(format!("{point}_hidden")), None);
            }
        }
    }

    for (id, trimmed) in [(11, true), (12, true), (13, false)] {
        let occurrence = by_id(id)["id"].as_str().unwrap().to_owned();
        let route = bench.mcp_json("/v1/route", &[("id", &occurrence)]).await;
        assert_eq!(route["trimmed"], trimmed, "{id}");
        let segments = route["segments"].as_array().unwrap();
        assert!(
            !segments.is_empty(),
            "{id} : il reste un parcours hors de la zone"
        );
        let mut points = 0;
        for segment in segments {
            let segment: Vec<[f64; 2]> = serde_json::from_value(segment.clone()).unwrap();
            assert!(segment.len() >= 2);
            points += segment.len();
            for point in &segment {
                assert!(
                    from_zone(point[0], point[1]) > ZONE_RADIUS_M,
                    "{id} : {point:?}"
                );
            }
            for pair in segment.windows(2) {
                assert!(
                    segment_from_zone(pair[0], pair[1]) > ZONE_RADIUS_M,
                    "{id} : ligne à travers"
                );
            }
        }
        // La boucle sort et rentre : deux morceaux au moins, jamais reliés à travers la zone.
        if id == 12 {
            assert!(points > 10);
        }

        // La session voit tout le parcours, un seul morceau, zone comprise.
        let full = bench.get(&format!("/v1/route?id={occurrence}")).await;
        let full: Vec<Vec<[f64; 2]>> = serde_json::from_value(full["segments"].clone()).unwrap();
        assert_eq!(full.len(), 1);
        assert_eq!(full[0].len() > points, trimmed);
        if trimmed {
            assert!(
                full[0]
                    .iter()
                    .any(|p| from_zone(p[0], p[1]) < ZONE_RADIUS_M)
            );
        }
        let track = bench.get(&format!("/v1/activities/{id}/track")).await;
        assert_eq!(track["points"].as_array().unwrap().len(), full[0].len());
    }
    // Une occurrence sans parcours, ou inconnue.
    let (status, _) = bench.as_mcp("/v1/route", &[("id", "999999:0")]).await;
    assert_eq!(status, 404);
    let (status, _) = bench.as_mcp("/v1/route", &[("id", "x")]).await;
    assert_eq!(status, 400);
}

// ─── Le vrai binaire : ce qui sort dans les logs ──────────────────────────────

fn collect_process(fake: &Fake, bench: &Bench) -> (std::process::Child, PathBuf) {
    let log = bench.state_dir.join("collector.log");
    let file = std::fs::File::create(&log).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_life-tracker"))
        .arg("collect")
        .env_clear()
        .env("RUST_LOG", "trace")
        .env("GARMIN_API_URL", &fake.url)
        .env("GARMIN_API_KEY", KEY)
        .env("GARMIN_TOKEN", &bench.token)
        .env("INGEST_URL", &bench.base)
        .env("COLLECTOR_STATE_DIR", &bench.state_dir)
        .stdout(Stdio::from(file))
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    (child, log)
}

async fn logs_until(child: std::process::Child, log: &std::path::Path, line: &str) -> String {
    let mut child = child;
    let deadline = Instant::now() + Duration::from_secs(120);
    let logs = loop {
        let logs = std::fs::read_to_string(log).unwrap_or_default();
        if logs.contains(line) {
            break logs;
        }
        assert!(Instant::now() < deadline, "« {line} » absent : {logs}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    Command::new("kill")
        .arg(child.id().to_string())
        .status()
        .unwrap();
    assert!(child.wait().unwrap().success());
    logs
}

#[tokio::test]
async fn logs_never_cite_the_key_the_token_a_place_or_a_coordinate() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    fake.with(|d| {
        d.activities = vec![workout(1, day(1), true)];
        d.files.insert(1, zipped("1.fit", &fit(&route())));
        d.summaries = vec![summary(day(1), Some(8412))];
    });
    let (child, log) = collect_process(&fake, &bench);
    let logs = logs_until(child, &log, "garmin pass completed").await;
    assert!(logs.contains("\"accepted\":3"), "{logs}");
    let issued = fake.with(|d| d.issued.clone());
    for secret in [
        KEY,
        &bench.token,
        PLACE,
        "43.21",
        OWNER,
        ACTIVITY_NAME,
        &issued[0],
    ] {
        assert!(!logs.contains(secret), "fuite de {secret} : {logs}");
    }
    assert!(bench.state_dir.join("garmin-state.json").exists());
    assert!(
        !bench.state_dir.join("state.json").exists(),
        "Last.fm n'est pas configuré"
    );
    let state = std::fs::read_to_string(bench.state_dir.join("garmin-state.json")).unwrap();
    assert!(
        !state.contains(&issued[0]),
        "le jeton ne touche jamais le disque"
    );

    fake.with(|d| d.mint_status = 401);
    std::fs::remove_file(bench.state_dir.join("garmin-state.json")).unwrap();
    let (child, log) = collect_process(&fake, &bench);
    let logs = logs_until(child, &log, "clé Garmin refusée : expirée ou révoquée").await;
    assert!(!logs.contains(KEY), "{logs}");
}

#[test]
fn what_the_connector_sends_respects_the_contract() {
    let schema = validator(None);
    let mut records = Vec::new();
    for observation in [
        garmin::daily_summary(&summary(day(1), Some(1))).unwrap(),
        garmin::sleep(&night(day(1), 60)).unwrap(),
        garmin::activity(&workout(1, day(1), true)).unwrap(),
    ] {
        records.push(observation.record(&observation.key()));
    }
    let workout = garmin::activity(&workout(1, day(1), true)).unwrap();
    let fit = garmin::fit_from_zip(&zipped("a.FIT", &fit(&route()))).unwrap();
    let track = garmin::activity_track(&workout, &fit).unwrap();
    records.push(track.record(&track.key()));
    records.push(garmin::cancel_record("garmin", &workout.key(), Utc::now()));
    let batch = json!({ "records": records });
    let errors: Vec<_> = schema.iter_errors(&batch).map(|e| e.to_string()).collect();
    assert!(errors.is_empty(), "{errors:?}");
}
