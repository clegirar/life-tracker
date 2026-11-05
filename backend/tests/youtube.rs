//! YouTube de bout en bout, sans aucun réseau extérieur : le connecteur contre un Google simulé (jeton
//! OAuth, Data Portability, stockage des archives, Data v3) et la vraie API, avec le relais de l'accord
//! par la web app, puis le vrai worker et l'API de lecture. Les réponses suivent la structure relevée sur
//! le vrai compte le 5 nov. 2026 (voir docs/chantiers.md) ; tous les ids, titres et noms sont fictifs.

mod common;
use axum::extract::{Form, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, TimeDelta, Utc};
use common::*;
use life_tracker::api::CSRF_HEADER;
use life_tracker::collector::youtube::{
    self, Collector, Config, DEFAULT_AUTH_URL, RENEW_AHEAD, code_challenge,
};
use life_tracker::interpreters::INTERPRETERS;
use life_tracker::{admin, engine, replay, youtube as parser};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::watch;

const PASSWORD: &str = "un mot de passe assez long";
const CLIENT_ID: &str = "client-fictif.apps.googleusercontent.com";
const CLIENT_SECRET: &str = "SECRET-CLIENT-SENTINELLE";
const API_KEY: &str = "CLE-DATA-SENTINELLE";
const REDIRECT_URI: &str = "https://nas.tailnet-fictif.ts.net:8443/autorisation";
const CODE: &str = "4/CODE-SENTINELLE";
const SIGNATURE: &str = "SIGNATURE-SENTINELLE";
/// Ce que la liste blanche doit écarter, et qui ne doit jamais sortir.
const SENTINELS: [&str; 6] = [
    "SENTINELLE-CONTROLES",
    "SENTINELLE-DESCRIPTION",
    "SENTINELLE-PUBLICITE",
    "SENTINELLE-RECHERCHE",
    "SENTINELLE-POST",
    "SENTINELLE-AUTRE",
];

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(repo_path(&format!("backend/tests/fixtures/youtube/{name}"))).unwrap()
}

fn zipped(name: &str, content: &[u8]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    zip.start_file(name, options).unwrap();
    zip.write_all(content).unwrap();
    zip.finish().unwrap().into_inner()
}

/// Les deux archives d'un export : l'activité, et le navigateur d'archive sans intérêt.
fn archives() -> Vec<Vec<u8>> {
    vec![
        zipped(
            "Portability/My Activity/YouTube/MyActivity.json",
            &fixture("MyActivity.json"),
        ),
        zipped("Portability/archive_browser.json", b"{}"),
    ]
}

/// Une fiche de Data v3, avec la taille du lecteur que `maxWidth=1000` fait rendre : en texte, comme
/// Google rend ses entiers de 64 bits.
fn video(
    id: &str,
    duration: &str,
    size: (u32, u32),
    title: &str,
    channel: &str,
) -> (String, Value) {
    let name = format!("Chaîne Fictive {}", channel.chars().last().unwrap());
    let channel = format!("UCchaineFictive{channel}000000");
    (
        id.to_owned(),
        json!({"id": id, "contentDetails": {"duration": duration},
               "snippet": {"title": title, "channelId": channel, "channelTitle": name},
               "player": {"embedWidth": size.0.to_string(), "embedHeight": size.1.to_string()}}),
    )
}

const HORIZONTAL: (u32, u32) = (1000, 563);
const VERTICAL: (u32, u32) = (563, 1000);

/// Data v3 connaît les vidéos en ligne, pas la vidéo supprimée : deux classiques horizontales, une
/// verticale de 10 min (classique), un Short de 45 s, un de 2 min et un carré de 90 s.
fn known_videos() -> HashMap<String, Value> {
    HashMap::from([
        video(
            "vidFictif01",
            "PT9M9S",
            HORIZONTAL,
            "Première vidéo fictive",
            "A",
        ),
        video(
            "vidFictif02",
            "PT1H2M3S",
            HORIZONTAL,
            "Deuxième vidéo fictive",
            "B",
        ),
        video(
            "vertFict010",
            "PT10M",
            VERTICAL,
            "Une vidéo verticale fictive",
            "C",
        ),
        video("shortFict01", "PT45S", VERTICAL, "Un Short fictif", "A"),
        video(
            "shortFict02",
            "PT2M",
            VERTICAL,
            "Un Short de deux minutes",
            "B",
        ),
        video(
            "carreFict01",
            "PT1M30S",
            (1000, 1000),
            "Un Short carré fictif",
            "A",
        ),
    ])
}

/// Les vues classiques du fixture, entrées par le connecteur ; les autres sont des Shorts (4) ou une
/// vidéo supprimée (1).
const CLASSIC_WATCHES: i64 = 4;
/// Les annulations d'une correction sur le fixture : 2 réactions, 5 vues de Shorts ou de la vidéo
/// supprimée, 3 fiches de Shorts.
const CORRECTION_CANCELS: i64 = 10;

// ─── Le Google simulé ────────────────────────────────────────────────────────

struct Data {
    /// Le défi PKCE de la demande que j'ai ouverte, et le code que Google rendra pour elle.
    challenge: Option<String>,
    refresh_tokens: Vec<String>,
    access_tokens: Vec<String>,
    /// Le prochain rafraîchissement échoue avec ce code (`invalid_grant`).
    refresh_error: Option<&'static str>,
    refresh_expires_in: i64,
    exchanges: u32,
    refreshes: u32,
    initiates: Vec<Value>,
    /// Un export est refusé jusqu'à cette heure.
    refuse_until: Option<String>,
    job_state: &'static str,
    state_polls: u32,
    archives: Vec<Vec<u8>>,
    downloads: u32,
    download_with_authorization: bool,
    resets: u32,
    videos: HashMap<String, Value>,
    video_calls: Vec<Vec<String>>,
    /// Data v3 répond par ce statut, ou par ce corps illisible.
    videos_status: Option<StatusCode>,
    videos_body: Option<Value>,
}

impl Default for Data {
    fn default() -> Self {
        Self {
            challenge: None,
            refresh_tokens: Vec::new(),
            access_tokens: Vec::new(),
            refresh_error: None,
            refresh_expires_in: 604_799,
            exchanges: 0,
            refreshes: 0,
            initiates: Vec::new(),
            refuse_until: None,
            job_state: "IN_PROGRESS",
            state_polls: 0,
            archives: archives(),
            downloads: 0,
            download_with_authorization: false,
            resets: 0,
            videos: known_videos(),
            video_calls: Vec::new(),
            videos_status: None,
            videos_body: None,
        }
    }
}

#[derive(Clone)]
struct Fake {
    url: String,
    data: Arc<Mutex<Data>>,
}

fn error(status: StatusCode, body: Value) -> Response {
    (status, Json(body)).into_response()
}

async fn token(State(fake): State<Fake>, Form(form): Form<HashMap<String, String>>) -> Response {
    let mut data = fake.data.lock().unwrap();
    assert_eq!(form["client_id"], CLIENT_ID);
    assert_eq!(form["client_secret"], CLIENT_SECRET);
    let n = data.access_tokens.len() + 1;
    let access = format!("ACCES-SENTINELLE-{n}");
    match form["grant_type"].as_str() {
        "authorization_code" => {
            assert_eq!(form["redirect_uri"], REDIRECT_URI);
            let challenge = data.challenge.take();
            if form["code"] != CODE
                || challenge.as_deref() != Some(&code_challenge(&form["code_verifier"]))
            {
                return error(StatusCode::BAD_REQUEST, json!({"error": "invalid_grant"}));
            }
            data.exchanges += 1;
            let refresh = format!("RAFRAICHISSEMENT-SENTINELLE-{}", data.exchanges);
            data.refresh_tokens.push(refresh.clone());
            data.access_tokens.push(access.clone());
            Json(
                json!({"access_token": access, "expires_in": 3599, "refresh_token": refresh,
                        "refresh_token_expires_in": data.refresh_expires_in,
                        "scope": youtube::SCOPE, "token_type": "Bearer"}),
            )
            .into_response()
        }
        "refresh_token" => {
            if let Some(code) = data.refresh_error {
                return error(StatusCode::BAD_REQUEST, json!({"error": code}));
            }
            assert_eq!(Some(&form["refresh_token"]), data.refresh_tokens.last());
            data.refreshes += 1;
            data.access_tokens.push(access.clone());
            Json(json!({"access_token": access, "expires_in": 3599, "token_type": "Bearer"}))
                .into_response()
        }
        other => panic!("grant_type inattendu {other}"),
    }
}

fn authorized(data: &Data, headers: &HeaderMap) -> bool {
    data.access_tokens
        .last()
        .is_some_and(|t| headers["authorization"] == format!("Bearer {t}").as_str())
}

async fn initiate(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let mut data = fake.data.lock().unwrap();
    if !authorized(&data, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    data.initiates.push(body);
    if let Some(until) = &data.refuse_until {
        return error(
            StatusCode::TOO_MANY_REQUESTS,
            json!({"error": {"code": 429, "message": "Requested resources have been exported recently.",
                   "status": "RESOURCE_EXHAUSTED", "details": [
                     {"@type": "type.googleapis.com/google.rpc.ErrorInfo",
                      "reason": "RESOURCE_EXHAUSTED_TIME_BASED",
                      "domain": "dataportability.googleapis.com",
                      "metadata": {"timestamp_after_24hrs": until}}]}}),
        );
    }
    let id = format!("job-{}", data.initiates.len());
    Json(json!({"archiveJobId": id, "accessType": "ACCESS_TYPE_TIME_BASED"})).into_response()
}

async fn archive_state(
    State(fake): State<Fake>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let mut data = fake.data.lock().unwrap();
    if !authorized(&data, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    assert!(id.starts_with("job-"));
    data.state_polls += 1;
    let urls: Vec<String> = (0..data.archives.len())
        .map(|i| {
            format!(
                "{}/storage/archive-{i}.zip?X-Goog-Signature={SIGNATURE}",
                fake.url
            )
        })
        .collect();
    let mut body = json!({"state": data.job_state});
    if data.job_state == "COMPLETE" {
        body["urls"] = json!(urls);
        body["exportTime"] = json!(Utc::now().to_rfc3339());
    }
    Json(body).into_response()
}

async fn reset(State(fake): State<Fake>, headers: HeaderMap) -> Response {
    let mut data = fake.data.lock().unwrap();
    if !authorized(&data, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    data.resets += 1;
    Json(json!({})).into_response()
}

async fn storage(
    State(fake): State<Fake>,
    Path(name): Path<String>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let mut data = fake.data.lock().unwrap();
    assert_eq!(query["X-Goog-Signature"], SIGNATURE);
    data.download_with_authorization |= headers.contains_key("authorization");
    data.downloads += 1;
    let index: usize = name
        .trim_start_matches("archive-")
        .trim_end_matches(".zip")
        .parse()
        .unwrap();
    data.archives[index].clone().into_response()
}

async fn videos(
    State(fake): State<Fake>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let mut data = fake.data.lock().unwrap();
    if query["key"] != API_KEY {
        return error(
            StatusCode::BAD_REQUEST,
            json!({"error": {"code": 400, "errors": [{"reason": "keyInvalid"}]}}),
        );
    }
    assert_eq!(query["part"], "contentDetails,snippet,player");
    assert_eq!(query["maxWidth"], "1000");
    assert!(query["fields"].contains("player(embedWidth,embedHeight)"));
    if let Some(status) = data.videos_status {
        return error(
            status,
            json!({"error": {"code": status.as_u16(), "errors": [{"reason": "backendError"}]}}),
        );
    }
    if let Some(body) = &data.videos_body {
        return Json(body.clone()).into_response();
    }
    let ids: Vec<String> = query["id"].split(',').map(str::to_owned).collect();
    assert!(ids.len() <= youtube::VIDEOS_PER_CALL);
    let items: Vec<Value> = ids
        .iter()
        .filter_map(|id| data.videos.get(id).cloned())
        .collect();
    data.video_calls.push(ids);
    // Comme Google avec `fields` : sans vidéo trouvée, pas de liste `items`.
    let page = json!({"totalResults": items.len(), "resultsPerPage": items.len()});
    if items.is_empty() {
        return Json(json!({"pageInfo": page})).into_response();
    }
    Json(json!({"items": items, "pageInfo": page})).into_response()
}

impl Fake {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let fake = Self {
            url: format!("http://{}", listener.local_addr().unwrap()),
            data: Arc::default(),
        };
        let app = Router::new()
            .route("/token", post(token))
            .route("/v1/portabilityArchive:initiate", post(initiate))
            .route(
                "/v1/archiveJobs/{id}/portabilityArchiveState",
                get(archive_state),
            )
            .route("/v1/authorization:reset", post(reset))
            .route("/storage/{name}", get(storage))
            .route("/youtube/v3/videos", get(videos))
            .with_state(fake.clone());
        tokio::spawn(async move { axum::serve(listener, app).await });
        fake
    }

    fn with<T>(&self, change: impl FnOnce(&mut Data) -> T) -> T {
        change(&mut self.data.lock().unwrap())
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
        let db = TestDb::new().await;
        admin::set_password(&db.admin, "moi", PASSWORD)
            .await
            .unwrap();
        let service = admin::create_service_token(&db.admin, "mcp").await.unwrap();
        let token = admin::create_source(&db.admin, "youtube", Some("3 days"), Some("YouTube"))
            .await
            .unwrap();
        for kind in ["youtube.watch", "youtube.video_info", "record.cancel"] {
            admin::grant_kind(&db.admin, "youtube", kind).await.unwrap();
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
            .header(CSRF_HEADER, "1")
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

    fn config(&self, fake: &Fake) -> Config {
        Config {
            client_id: CLIENT_ID.to_owned(),
            client_secret: CLIENT_SECRET.to_owned(),
            redirect_uri: REDIRECT_URI.to_owned(),
            api_key: API_KEY.to_owned(),
            ingest_url: self.base.clone(),
            token: self.token.clone(),
            source: "youtube".to_owned(),
            state_dir: self.state_dir.clone(),
            interval: Duration::from_secs(300),
            consent_poll: Duration::from_secs(60),
            auth_url: DEFAULT_AUTH_URL.to_owned(),
            oauth_url: fake.url.clone(),
            portability_url: fake.url.clone(),
            data_url: format!("{}/youtube/v3", fake.url),
            storage_url: format!("{}/storage", fake.url),
            videos_per_call: youtube::VIDEOS_PER_CALL,
            classify_calls_per_pass: youtube::CLASSIFY_CALLS_PER_PASS,
        }
    }

    fn collector(&self, fake: &Fake) -> Collector {
        Collector::new(self.config(fake)).unwrap()
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

    async fn as_mcp(&self, path: &str) -> Value {
        let response = self
            .http
            .get(format!("{}{path}", self.base))
            .bearer_auth(&self.service)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200, "{path}");
        response.json().await.unwrap()
    }

    async fn last_seen(&self) -> Option<DateTime<Utc>> {
        sqlx::query_scalar("SELECT last_seen_at FROM core.sources WHERE name = 'youtube'")
            .fetch_one(&self.db.app)
            .await
            .unwrap()
    }

    /// La carte YouTube de la page d'état.
    async fn card(&self) -> Value {
        let page = self.get("/v1/sources/status").await;
        page["sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["name"] == "youtube")
            .unwrap()
            .clone()
    }

    async fn callback(&self, body: &Value) -> u16 {
        self.http
            .post(format!("{}/v1/authorizations/callback", self.base))
            .header("cookie", &self.cookie)
            .header(CSRF_HEADER, "1")
            .json(body)
            .send()
            .await
            .unwrap()
            .status()
            .as_u16()
    }

    /// Je donne mon accord : la page d'état montre l'URL, Google renvoie le code à la web app, qui le
    /// transmet. Le Google simulé n'acceptera le code qu'avec le vérificateur de cette demande.
    async fn consent(&self, fake: &Fake) -> String {
        let card = self.card().await;
        let url = card["authorization_url"]
            .as_str()
            .expect("une demande en cours");
        let parsed = reqwest::Url::parse(url).unwrap();
        assert_eq!(parsed.host_str(), Some("accounts.google.com"));
        let query: HashMap<String, String> = parsed.query_pairs().into_owned().collect();
        assert_eq!(query["client_id"], CLIENT_ID);
        assert_eq!(query["redirect_uri"], REDIRECT_URI);
        fake.with(|d| d.challenge = Some(query["code_challenge"].clone()));
        let state = query["state"].clone();
        assert_eq!(
            self.callback(&json!({"state": state, "code": CODE})).await,
            204
        );
        assert_eq!(self.card().await["authorization_url"], Value::Null);
        state
    }

    fn state_file(&self) -> PathBuf {
        self.state_dir.join("youtube-state.json")
    }
}

/// Un accord donné, puis un export lancé, terminé et lu, et ses fiches demandées.
async fn exported(bench: &Bench, fake: &Fake) -> Collector {
    let mut collector = bench.collector(fake);
    collector.pass().await;
    bench.consent(fake).await;
    collector.tick().await.unwrap();
    fake.with(|d| d.job_state = "COMPLETE");
    let report = collector.tick().await.unwrap();
    assert!(report.exported);
    collector
}

// ─── Le connecteur ───────────────────────────────────────────────────────────

#[tokio::test]
async fn the_first_consent_goes_through_the_web_app_then_the_history_is_exported_and_enriched() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    let mut collector = bench.collector(&fake);

    // Aucun jeton : une demande d'accord, sans signal de vie.
    let delay = collector.pass().await;
    assert_eq!(
        delay,
        Duration::from_secs(60),
        "les passages se rapprochent"
    );
    let card = bench.card().await;
    assert_eq!(card["status"]["state"], "action_required");
    assert_eq!(card["status"]["reason"], "consent_required");
    assert!(card["authorization_url"].is_string());
    assert_eq!(bench.last_seen().await, None);
    assert_eq!(fake.with(|d| d.exchanges), 0);

    // La même demande attend tant que je ne l'ai pas ouverte.
    collector.pass().await;
    let again = bench.card().await;
    assert_eq!(again["authorization_url"], card["authorization_url"]);

    bench.consent(&fake).await;
    let report = collector.tick().await.unwrap();
    assert_eq!(report.awaiting, None);
    assert!(!report.exported, "l'export est en cours");
    assert_eq!(fake.with(|d| (d.exchanges, d.initiates.len())), (1, 1));
    assert_eq!(
        fake.with(|d| d.initiates[0].clone()),
        json!({"resources": ["myactivity.youtube"]}),
        "tout l'historique, la première fois"
    );
    assert!(bench.last_seen().await.is_some(), "Google a répondu");
    let expires = collector.refresh_expires_at().unwrap();
    assert!(
        (expires - (Utc::now() + TimeDelta::days(7)))
            .num_seconds()
            .abs()
            < 60
    );
    let next_export = collector.next_export_at().unwrap();
    assert!(
        (next_export - (Utc::now() + TimeDelta::hours(24)))
            .num_seconds()
            .abs()
            < 60
    );

    // Un passage sans export terminé ne télécharge rien.
    collector.pass().await;
    assert_eq!(fake.with(|d| d.downloads), 0);
    let card = bench.card().await;
    assert_eq!(card["status"]["state"], "ok");
    let status_expires: DateTime<Utc> = card["status"]["expires_at"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!((status_expires - expires).num_seconds().abs() < 2);
    let next_run: DateTime<Utc> = card["status"]["next_run_at"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!(
        (next_run - next_export).num_seconds().abs() < 2,
        "le prochain export permis"
    );

    fake.with(|d| d.job_state = "COMPLETE");
    let report = collector.tick().await.unwrap();
    assert!(report.exported);
    assert_eq!(
        (report.watches, report.accepted, report.duplicate),
        (9, 4, 0),
        "seules les vues de vidéos classiques entrent"
    );
    assert_eq!(
        (report.shorts, report.unavailable),
        (4, 1),
        "trois Shorts (dont un carré) et la vidéo supprimée"
    );
    let skipped = report.skipped;
    assert_eq!(
        (
            skipped.ad,
            skipped.search,
            skipped.post,
            skipped.subscription,
            skipped.music
        ),
        (1, 1, 1, 1, 0)
    );
    assert_eq!(skipped.reaction, 2, "« Liked » et « Disliked »");
    assert_eq!(
        (report.cancelled, report.cancels_refused, report.rejected),
        (CORRECTION_CANCELS as u32, 0, 0),
        "ce que la version d'avant a pu entrer, annulé une fois"
    );
    assert_eq!(bench.journal("record.cancel").await, CORRECTION_CANCELS);
    assert_eq!(
        (report.classified, report.enriched),
        (7, 3),
        "une fiche par vidéo classique"
    );
    assert_eq!(collector.awaiting_classification(), 0);
    assert_eq!(fake.with(|d| d.video_calls.len()), 1, "7 vidéos, un appel");
    assert_eq!(fake.with(|d| d.downloads), 2);
    assert!(
        !fake.with(|d| d.download_with_authorization),
        "le jeton ne part jamais vers le stockage"
    );
    assert_eq!(
        collector.last_watched_at(),
        Some("2026-10-15T08:20:00Z".parse().unwrap()),
        "la dernière vue traitée, même écartée"
    );
    assert!(collector.last_exported_at().is_some());
    assert_eq!(bench.journal("youtube.watch").await, CLASSIC_WATCHES);
    assert_eq!(bench.journal("youtube.video_info").await, 3);
    assert!(
        !bench.state_dir.join("youtube-archive.zip.tmp").exists(),
        "l'archive ne reste pas sur le disque"
    );

    // L'état : lisible par le seul collecteur, sans jeton d'accès.
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(bench.state_file())
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
    let state = std::fs::read_to_string(bench.state_file()).unwrap();
    assert!(
        !state.contains("ACCES-SENTINELLE"),
        "le jeton d'accès reste en mémoire"
    );

    bench.settled().await;
    let day = bench.get("/v1/days/2026-10-14/video").await;
    assert_eq!(
        day,
        json!({
            "date": "2026-10-14",
            "count": 2,
            "seconds": 549 + 3723,
            "without_duration": 0,
            "watches": [
                {"time": "20:30", "occurred_at": "2026-10-14T18:30:00.123Z",
                 "video_id": "vidFictif01", "title": "Première vidéo fictive",
                 "channel_id": "UCchaineFictiveA000000", "channel_name": "Chaîne Fictive A",
                 "duration_s": 549},
                {"time": "21:00", "occurred_at": "2026-10-14T19:00:00Z",
                 "video_id": "vidFictif02", "title": "Deuxième vidéo fictive",
                 "channel_id": "UCchaineFictiveB000000", "channel_name": "Chaîne Fictive B",
                 "duration_s": 3723},
            ],
            "discoveries": [
                {"level": "channel", "key": "UCchaineFictiveB000000", "name": "Chaîne Fictive B"},
                {"level": "video", "key": "vidFictif02", "name": "Deuxième vidéo fictive"},
            ],
        })
    );
    let shorts_day = bench.get("/v1/days/2026-10-15/video").await;
    assert_eq!(
        (&shorts_day["count"], &shorts_day["seconds"]),
        (&json!(1), &json!(600)),
        "la vidéo verticale de 10 min, pas les Shorts : {shorts_day}"
    );
    assert_eq!(shorts_day["watches"][0]["video_id"], "vertFict010");
    let empty = bench.get("/v1/days/2026-10-16/video").await;
    assert_eq!(
        (
            &empty["count"],
            &empty["seconds"],
            &empty["without_duration"]
        ),
        (&json!(0), &Value::Null, &json!(0))
    );

    // Le calendrier, générique : une vidéo par vue.
    let month = bench.get("/v1/months/2026-10").await;
    let value = |date: &str| {
        month["days"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["date"] == date)
            .unwrap()["values"]["video.watch"]
            .clone()
    };
    assert_eq!(
        (
            value("2026-10-13"),
            value("2026-10-14"),
            value("2026-10-15")
        ),
        (json!(1.0), json!(2.0), json!(1.0))
    );
    assert!(
        month["metrics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["metric"] == "video.watch" && m["label"] == "Vidéos YouTube")
    );

    // Les lectures génériques du MCP : classement, durée, recherche par nom, occurrences.
    let top = bench
        .as_mcp("/v1/top?metric=video.watch&level=channel&from=2026-10-01&to=2026-10-31")
        .await;
    assert_eq!(top["items"][0]["key"], "UCchaineFictiveA000000");
    assert_eq!(top["items"][0]["name"], "Chaîne Fictive A");
    assert_eq!(top["items"][0]["occurrences"], 2);
    let durations = bench
        .as_mcp("/v1/durations?metric=video.watch&from=2026-10-14&to=2026-10-14")
        .await;
    assert_eq!(durations["total_minutes"], 71);
    assert_eq!(durations["events_without_duration"], 0);
    let found = bench
        .as_mcp("/v1/search?q=fictive%20b&metric=video.watch")
        .await;
    assert_eq!(found[0]["key"], "UCchaineFictiveB000000", "{found}");
    let videos = bench
        .as_mcp("/v1/search?q=deuxi%C3%A8me%20vid%C3%A9o&metric=video.watch&level=video")
        .await;
    assert_eq!(videos[0]["key"], "vidFictif02", "{videos}");
    assert_eq!(videos[0]["context"]["channel"], "Chaîne Fictive B");
    let occurrences = bench
        .as_mcp("/v1/occurrences?metric=video.watch&from=2026-10-14&to=2026-10-14&form=list")
        .await;
    assert_eq!(
        occurrences["occurrences"][0]["details"],
        json!({"title": "Première vidéo fictive", "channel_name": "Chaîne Fictive A"}),
        "seuls les détails partagés"
    );
    let discoveries = bench
        .as_mcp("/v1/discoveries?metric=video.watch&level=channel&from=2026-10-14&to=2026-10-14")
        .await;
    assert!(
        discoveries.to_string().contains("UCchaineFictiveB000000"),
        "{discoveries}"
    );

    // Le rejeu complet donne le même résultat.
    let rebuilt = replay::rebuild(&bench.db.admin, INTERPRETERS, false)
        .await
        .unwrap();
    assert!(rebuilt.switched);
    assert!(rebuilt.comparison.differences.is_empty());
}

#[tokio::test]
async fn a_second_export_waits_for_the_time_google_gives() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    let until =
        (Utc::now() + TimeDelta::hours(20)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    fake.with(|d| d.refuse_until = Some(until.clone()));
    let mut collector = bench.collector(&fake);
    collector.pass().await;
    bench.consent(&fake).await;
    collector.pass().await;
    let until: DateTime<Utc> = until.parse().unwrap();
    assert_eq!(collector.next_export_at(), Some(until));
    let card = bench.card().await;
    assert_eq!(
        card["status"]["state"], "ok",
        "un refus prévu n'est pas une panne"
    );
    assert_eq!(
        card["status"]["next_run_at"],
        json!(until.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
    );
    collector.pass().await;
    assert_eq!(
        fake.with(|d| d.initiates.len()),
        1,
        "rien avant l'heure permise"
    );
}

/// Réécrit des champs de l'état du connecteur, pour faire passer le temps.
fn rewrite_state(bench: &Bench, change: impl FnOnce(&mut Value)) {
    let mut state: Value = read_json(&bench.state_file());
    change(&mut state);
    std::fs::write(bench.state_file(), state.to_string()).unwrap();
}

fn instant(value: &Value) -> DateTime<Utc> {
    value.as_str().unwrap().parse().unwrap()
}

/// Google répond à chaque appel, mais aucun export n'aboutit depuis deux jours : la carte le dit, avec
/// la date du dernier export reçu ; un export reçu la remet à `ok`.
#[tokio::test]
async fn exports_that_keep_failing_turn_the_card_to_an_error_until_one_succeeds() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    let mut collector = exported(&bench, &fake).await;
    collector.pass().await;
    let card = bench.card().await;
    assert_eq!(card["status"]["state"], "ok");
    let collected = instant(&card["status"]["last_collected_at"]);
    assert_eq!(Some(collected), collector.last_exported_at());
    assert!((Utc::now() - collected).num_seconds().abs() < 60);

    // Deux jours plus tard, l'export suivant échoue.
    let old = Utc::now() - TimeDelta::hours(49);
    rewrite_state(&bench, |state| {
        state["last_exported_at"] = json!(old);
        state["next_export_at"] = json!("2026-01-01T00:00:00Z");
    });
    fake.with(|d| d.job_state = "FAILED");
    let mut collector = bench.collector(&fake);
    collector.pass().await;
    assert_eq!(fake.with(|d| d.initiates.len()), 2);
    collector.pass().await;
    let card = bench.card().await;
    assert_eq!(card["status"]["state"], "error");
    assert_eq!(card["status"]["reason"], "service_unavailable");
    assert_eq!(instant(&card["status"]["last_collected_at"]), old);
    assert!(card["status"]["expires_at"].is_string(), "l'échéance reste");
    assert!(card["status"]["next_run_at"].is_string());
    assert!(
        bench.last_seen().await.is_some(),
        "Google répond : le signal de vie continue"
    );

    // Le suivant aboutit : retour à `ok`, avec la date de cet export.
    fake.with(|d| d.job_state = "COMPLETE");
    collector.pass().await;
    assert_eq!(fake.with(|d| d.initiates.len()), 3);
    collector.pass().await;
    let card = bench.card().await;
    assert_eq!(card["status"]["state"], "ok");
    assert_eq!(card["status"]["reason"], Value::Null);
    let collected = instant(&card["status"]["last_collected_at"]);
    assert!((Utc::now() - collected).num_seconds().abs() < 60);
}

/// Jamais d'export reçu : l'erreur vient 48 h après le premier demandé, sans date de collecte.
#[tokio::test]
async fn no_export_at_all_48_hours_after_the_first_asked_is_an_error() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    let mut collector = bench.collector(&fake);
    collector.pass().await;
    bench.consent(&fake).await;
    collector.pass().await;
    let card = bench.card().await;
    assert_eq!(
        card["status"]["state"], "ok",
        "l'export vient d'être demandé"
    );
    assert_eq!(card["status"]["last_collected_at"], Value::Null);

    rewrite_state(&bench, |state| {
        assert!(state["exports_since"].is_string());
        state["exports_since"] = json!(Utc::now() - TimeDelta::hours(49));
    });
    fake.with(|d| d.job_state = "FAILED");
    let mut collector = bench.collector(&fake);
    collector.pass().await;
    let card = bench.card().await;
    assert_eq!(card["status"]["state"], "error");
    assert_eq!(card["status"]["reason"], "service_unavailable");
    assert_eq!(card["status"]["last_collected_at"], Value::Null);
}

#[tokio::test]
async fn an_export_read_again_gives_duplicates_only_and_no_new_enrichment() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    exported(&bench, &fake).await;
    let calls = fake.with(|d| d.video_calls.len());

    // L'export suivant : l'état dit qu'il est permis, et Google rend le même historique.
    let mut state: Value = read_json(&bench.state_file());
    state["next_export_at"] = json!("2026-01-01T00:00:00Z");
    std::fs::write(bench.state_file(), state.to_string()).unwrap();
    let mut collector = bench.collector(&fake);
    collector.tick().await.unwrap();
    assert_eq!(
        fake.with(|d| d.initiates[1].clone()),
        json!({"resources": ["myactivity.youtube"], "startTime": "2026-10-13T08:20:00Z"}),
        "depuis la dernière vue, moins 48 h : les corrections sont faites"
    );
    let report = collector.tick().await.unwrap();
    assert!(report.exported);
    assert_eq!((report.accepted, report.duplicate), (0, 4));
    assert_eq!((report.shorts, report.unavailable), (4, 1), "écartés");
    assert_eq!(
        report.cancelled, 0,
        "les annulations ne partent qu'une fois"
    );
    assert_eq!(bench.journal("record.cancel").await, CORRECTION_CANCELS);
    assert_eq!(
        fake.with(|d| d.video_calls.len()),
        calls,
        "aucune vidéo reclassée"
    );
    assert_eq!(bench.journal("youtube.watch").await, CLASSIC_WATCHES);
}

/// Le bug du 5 nov. 2026 : la version d'avant faisait une vue de chaque réaction (« Disliked https://… »).
/// Le correctif ne l'importe plus, et annule la fausse vue déjà entrée, sous la clé d'alors.
#[tokio::test]
async fn a_reaction_is_no_watch_and_the_false_watch_of_the_version_before_is_cancelled() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    let false_watch = parser::Watch {
        video_id: "vidReactif1".to_owned(),
        title: Some("Disliked https://www.youtube.com/watch?v=vidReactif1".to_owned()),
        channel_id: None,
        channel_name: None,
        watched_at: "2026-10-13T09:00:00.789Z".parse().unwrap(),
    };
    let response = bench
        .http
        .post(format!("{}/v1/records", bench.base))
        .bearer_auth(&bench.token)
        .json(&json!({"records": [false_watch.record()]}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    bench.settled().await;
    let day = bench.get("/v1/days/2026-10-13/video").await;
    assert_eq!(day["count"], 1);
    assert_eq!(day["watches"][0]["video_id"], "vidReactif1");
    let occurrences = "/v1/occurrences?metric=video.watch&from=2026-10-13&to=2026-10-13&form=list";
    assert_eq!(
        bench.as_mcp(occurrences).await["occurrences"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    // L'export : la vue du 13 entre, la réaction non, et la fausse vue est annulée.
    let mut collector = bench.collector(&fake);
    collector.pass().await;
    bench.consent(&fake).await;
    collector.tick().await.unwrap();
    fake.with(|d| d.job_state = "COMPLETE");
    let report = collector.tick().await.unwrap();
    assert!(report.exported);
    assert_eq!((report.watches, report.accepted), (9, 4));
    assert_eq!(
        (report.skipped.reaction, report.cancelled),
        (2, CORRECTION_CANCELS as u32)
    );
    let cancelled: Vec<(String, Value)> = sqlx::query_as(
        "SELECT key, payload FROM core.records WHERE kind = 'record.cancel' ORDER BY key",
    )
    .fetch_all(&bench.db.app)
    .await
    .unwrap();
    for key in ["vidFictif02:1792007100000".to_owned(), false_watch.key()] {
        assert!(
            cancelled.contains(&(
                format!("cancel:{key}"),
                json!({"source": "youtube", "key": key})
            )),
            "{cancelled:?}"
        );
    }

    bench.settled().await;
    let day = bench.get("/v1/days/2026-10-13/video").await;
    assert_eq!(day["count"], 1);
    assert_eq!(day["watches"][0]["video_id"], "vidFictif01");
    assert!(!day.to_string().contains("Disliked"), "{day}");
    let occurrences = bench.as_mcp(occurrences).await;
    assert_eq!(
        occurrences["occurrences"].as_array().unwrap().len(),
        1,
        "{occurrences}"
    );
    assert!(
        !occurrences.to_string().contains("Disliked"),
        "{occurrences}"
    );
    let month = bench.get("/v1/months/2026-10").await;
    let day13 = month["days"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["date"] == "2026-10-13")
        .unwrap()
        .clone();
    assert_eq!(day13["values"]["video.watch"], json!(1.0));
    let top = bench
        .as_mcp("/v1/top?metric=video.watch&level=video&from=2026-10-13&to=2026-10-13")
        .await;
    assert!(!top.to_string().contains("vidReactif1"), "{top}");

    let rebuilt = replay::rebuild(&bench.db.admin, INTERPRETERS, false)
        .await
        .unwrap();
    assert!(rebuilt.switched);
    assert!(rebuilt.comparison.differences.is_empty());
}

/// Ce que la version d'avant entrait : toutes les vues de l'export, Shorts et vidéos supprimées compris,
/// et la fiche de chaque vidéo que Data v3 connaissait.
async fn ingest_the_version_before(bench: &Bench) {
    let activity = parser::parse_activity(&fixture("MyActivity.json")[..]).unwrap();
    let mut records: Vec<Value> = activity.watches.iter().map(parser::Watch::record).collect();
    for (id, video) in known_videos() {
        records.push(parser::video_info_record(
            &id,
            parser::iso8601_seconds(video["contentDetails"]["duration"].as_str().unwrap()),
            video["snippet"]["title"].as_str(),
            video["snippet"]["channelId"].as_str(),
            video["snippet"]["channelTitle"].as_str(),
            Utc::now(),
        ));
    }
    let response = bench
        .http
        .post(format!("{}/v1/records", bench.base))
        .bearer_auth(&bench.token)
        .json(&json!({ "records": records }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    bench.settled().await;
}

/// La règle du 5 nov. 2026 : rien que des vidéos classiques. Les Shorts et la vidéo supprimée que la
/// version d'avant avait entrés sont annulés une fois, sous leurs clés d'alors, et disparaissent de la
/// carte du jour, du calendrier et des lectures du MCP.
#[tokio::test]
async fn shorts_and_unavailable_videos_entered_before_are_cancelled_once() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    ingest_the_version_before(&bench).await;
    let day = bench.get("/v1/days/2026-10-15/video").await;
    assert_eq!(day["count"], 5, "{day}");
    let occurrences = "/v1/occurrences?metric=video.watch&from=2026-10-15&to=2026-10-15&form=list";
    assert_eq!(
        bench.as_mcp(occurrences).await["occurrences"]
            .as_array()
            .unwrap()
            .len(),
        5
    );

    let collector = exported(&bench, &fake).await;
    assert_eq!(collector.awaiting_classification(), 0);
    assert_eq!(bench.journal("record.cancel").await, CORRECTION_CANCELS);
    bench.settled().await;

    let day = bench.get("/v1/days/2026-10-15/video").await;
    assert_eq!(day["count"], 1, "{day}");
    assert_eq!(day["watches"][0]["video_id"], "vertFict010");
    assert_eq!(day["seconds"], 600);
    let day14 = bench.get("/v1/days/2026-10-14/video").await;
    assert_eq!(day14["count"], 2, "{day14}");
    assert!(!day14.to_string().contains("vidSupprime"), "{day14}");
    let occurrences = bench.as_mcp(occurrences).await;
    assert_eq!(
        occurrences["occurrences"].as_array().unwrap().len(),
        1,
        "{occurrences}"
    );
    let top = bench
        .as_mcp("/v1/top?metric=video.watch&level=video&from=2026-10-01&to=2026-10-31")
        .await;
    let text = top.to_string();
    for id in ["shortFict01", "shortFict02", "carreFict01", "vidSupprime"] {
        assert!(!text.contains(id), "{id} : {top}");
    }
    let month = bench.get("/v1/months/2026-10").await;
    let value = |date: &str| {
        month["days"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["date"] == date)
            .unwrap()["values"]["video.watch"]
            .clone()
    };
    assert_eq!(
        (value("2026-10-14"), value("2026-10-15")),
        (json!(2.0), json!(1.0))
    );
    let rebuilt = replay::rebuild(&bench.db.admin, INTERPRETERS, false)
        .await
        .unwrap();
    assert!(rebuilt.switched);
    assert!(rebuilt.comparison.differences.is_empty());

    // L'export suivant, depuis la dernière vue : rien de plus n'est annulé.
    rewrite_state(&bench, |state| {
        assert_eq!(state["corrections_version"], 2);
        state["next_export_at"] = json!("2026-01-01T00:00:00Z");
    });
    let mut collector = bench.collector(&fake);
    collector.tick().await.unwrap();
    assert!(fake.with(|d| d.initiates[1]["startTime"].is_string()));
    let report = collector.tick().await.unwrap();
    assert!(report.exported);
    assert_eq!(
        (report.shorts, report.unavailable, report.cancelled),
        (4, 1, 0)
    );
    assert_eq!(bench.journal("record.cancel").await, CORRECTION_CANCELS);
}

/// Une panne de Data v3 (HTTP, ou une réponse sans `items` ni `pageInfo`) ne classe rien : aucune vue
/// n'entre, aucune n'est annulée (seules les réactions, qui n'en dépendent pas), et l'export ne compte
/// pas comme reçu. Le passage suivant reprend.
#[tokio::test]
async fn a_data_api_failure_classifies_nothing_and_cancels_no_watch() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    fake.with(|d| d.videos_status = Some(StatusCode::INTERNAL_SERVER_ERROR));
    let mut collector = bench.collector(&fake);
    collector.pass().await;
    bench.consent(&fake).await;
    collector.tick().await.unwrap();
    fake.with(|d| d.job_state = "COMPLETE");
    assert_eq!(
        collector.tick().await,
        Err(youtube::TickError::Service(youtube::ServiceError::Failed {
            status: 500,
            call: "videos"
        }))
    );
    let nothing = |collector: &Collector| {
        assert_eq!(collector.awaiting_classification(), 9);
        assert_eq!(collector.last_exported_at(), None);
    };
    nothing(&collector);
    assert_eq!(bench.journal("youtube.watch").await, 0);
    assert_eq!(bench.journal("youtube.video_info").await, 0);
    assert_eq!(
        bench.journal("record.cancel").await,
        2,
        "les réactions seules"
    );

    fake.with(|d| {
        d.videos_status = None;
        d.videos_body = Some(json!({}));
    });
    assert_eq!(
        collector.tick().await,
        Err(youtube::TickError::Service(
            youtube::ServiceError::Unavailable { reason: "shape" }
        ))
    );
    nothing(&collector);
    collector.pass().await;
    let card = bench.card().await;
    assert_eq!(card["status"]["state"], "error");
    assert_eq!(card["status"]["reason"], "service_unavailable");
    assert_eq!(bench.journal("youtube.watch").await, 0);
    assert_eq!(bench.journal("record.cancel").await, 2);

    fake.with(|d| d.videos_body = None);
    let report = collector.tick().await.unwrap();
    assert_eq!((report.accepted, report.classified), (4, 7));
    assert_eq!(collector.awaiting_classification(), 0);
    assert!(collector.last_exported_at().is_some());
    assert_eq!(bench.journal("record.cancel").await, CORRECTION_CANCELS);
}

/// Un classement plafonné se fait en plusieurs passages, et reprend après un redémarrage : les vues
/// attendent dans l'état, chaque vidéo n'est demandée qu'une fois, et l'export ne compte comme reçu
/// qu'à la fin.
#[tokio::test]
async fn a_capped_classification_spreads_over_passes_and_survives_a_restart() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    let capped = |bench: &Bench| {
        let mut config = bench.config(&fake);
        config.videos_per_call = 2;
        config.classify_calls_per_pass = 1;
        Collector::new(config).unwrap()
    };
    let mut collector = capped(&bench);
    collector.pass().await;
    bench.consent(&fake).await;
    collector.tick().await.unwrap();
    fake.with(|d| d.job_state = "COMPLETE");
    let report = collector.tick().await.unwrap();
    assert!(report.exported);
    // Les deux premiers ids : le Short carré et le Short de 45 s, vu deux fois.
    assert_eq!((report.classified, report.shorts), (2, 3));
    assert_eq!(collector.awaiting_classification(), 6);
    assert_eq!(collector.last_exported_at(), None, "pas encore reçu");
    collector.pass().await;
    assert_eq!(
        bench.card().await["status"]["state"],
        "ok",
        "un classement en cours n'est pas une panne"
    );
    assert_eq!(collector.awaiting_classification(), 4);

    // Redémarrage : l'état garde les vues en attente et le classement fait.
    drop(collector);
    let state: Value = read_json(&bench.state_file());
    assert_eq!(state["pending"].as_array().unwrap().len(), 4);
    assert!(
        !state.to_string().contains("ACCES-SENTINELLE"),
        "le jeton d'accès reste en mémoire"
    );
    let mut collector = capped(&bench);
    let mut passes = 0;
    while collector.awaiting_classification() > 0 {
        collector.tick().await.unwrap();
        passes += 1;
        assert!(passes <= 2, "deux appels restent");
    }
    assert!(collector.last_exported_at().is_some());
    let calls = fake.with(|d| d.video_calls.clone());
    assert_eq!(calls.len(), 4);
    assert!(calls.iter().all(|ids| ids.len() <= 2));
    let mut ids: Vec<String> = calls.concat();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 7, "chaque vidéo une fois : {calls:?}");
    assert_eq!(bench.journal("youtube.watch").await, CLASSIC_WATCHES);
    assert_eq!(bench.journal("youtube.video_info").await, 3);
    assert_eq!(bench.journal("record.cancel").await, CORRECTION_CANCELS);
    let state: Value = read_json(&bench.state_file());
    assert_eq!(state["corrections_version"], 2);
    assert_eq!(state["pending"], json!([]));
}

/// Sans le droit `record.cancel`, les vues entrent quand même ; les annulations attendent l'export
/// suivant, qui reprend tout l'historique, et la carte le signale.
#[tokio::test]
async fn without_the_cancel_right_watches_enter_and_cancellations_wait_for_the_next_export() {
    let bench = Bench::new().await;
    admin::revoke_kind(&bench.db.admin, "youtube", "record.cancel")
        .await
        .unwrap();
    let fake = Fake::start().await;
    let mut collector = bench.collector(&fake);
    collector.pass().await;
    bench.consent(&fake).await;
    collector.pass().await;
    fake.with(|d| d.job_state = "COMPLETE");
    collector.pass().await;
    assert_eq!(bench.journal("youtube.watch").await, CLASSIC_WATCHES);
    assert_eq!(bench.journal("record.cancel").await, 0);
    let card = bench.card().await;
    assert_eq!(card["status"]["state"], "error");
    assert_eq!(card["status"]["reason"], "ingest_failed");
    assert!(
        card["status"]["last_collected_at"].is_string(),
        "l'export est reçu"
    );

    admin::grant_kind(&bench.db.admin, "youtube", "record.cancel")
        .await
        .unwrap();
    rewrite_state(&bench, |state| {
        state["next_export_at"] = json!("2026-01-01T00:00:00Z");
    });
    let mut collector = bench.collector(&fake);
    collector.tick().await.unwrap();
    assert_eq!(
        fake.with(|d| d.initiates[1].clone()),
        json!({"resources": ["myactivity.youtube"]}),
        "tout l'historique, tant que les annulations ne sont pas passées"
    );
    let report = collector.tick().await.unwrap();
    assert_eq!(
        (report.duplicate, report.cancelled),
        (4, CORRECTION_CANCELS as u32)
    );
    assert_eq!(report.classified, 0, "le classement est gardé");
    assert_eq!(bench.journal("record.cancel").await, CORRECTION_CANCELS);
    collector.pass().await;
    assert_eq!(bench.card().await["status"]["state"], "ok");
}

/// Un export dont aucune entrée `/watch` n'a de préfixe connu (une langue de plus) : rien n'entre ni
/// n'est annulé, la carte est en erreur, et la dernière vue ni le dernier export n'avancent.
#[tokio::test]
async fn an_export_without_any_recognized_watch_is_an_error_and_changes_nothing() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    let german = json!([
        {"header": "YouTube", "title": "Angesehen: Erstes fiktives Video",
         "titleUrl": "https://www.youtube.com/watch?v=vidFictif01", "time": "2026-10-14T18:30:00Z",
         "products": ["YouTube"]},
        {"header": "YouTube", "title": "Gesucht nach x",
         "titleUrl": "https://www.youtube.com/results?search_query=x", "time": "2026-10-14T18:00:00Z",
         "products": ["YouTube"]},
    ]);
    fake.with(|d| {
        d.archives = vec![zipped(
            "Portability/My Activity/YouTube/MyActivity.json",
            german.to_string().as_bytes(),
        )];
    });
    let mut collector = bench.collector(&fake);
    collector.pass().await;
    bench.consent(&fake).await;
    collector.pass().await;
    fake.with(|d| d.job_state = "COMPLETE");
    let report = collector.tick().await.unwrap();
    assert!(report.unrecognized);
    assert!(!report.exported);
    assert_eq!((report.watches, report.skipped.reaction), (0, 1));
    assert_eq!(bench.journal("youtube.watch").await, 0);
    assert_eq!(bench.journal("record.cancel").await, 0);
    assert_eq!(collector.last_watched_at(), None);
    assert_eq!(collector.last_exported_at(), None);
    assert!(
        collector.next_export_at().unwrap() > Utc::now(),
        "pas d'autre export avant 24 h"
    );

    // Le même, par un passage : la carte est en erreur.
    rewrite_state(&bench, |state| {
        state["next_export_at"] = json!("2026-01-01T00:00:00Z");
    });
    let mut collector = bench.collector(&fake);
    collector.pass().await;
    collector.pass().await;
    let card = bench.card().await;
    assert_eq!(card["status"]["state"], "error");
    assert_eq!(card["status"]["reason"], "service_unavailable");
    assert_eq!(card["status"]["last_collected_at"], Value::Null);
}

#[tokio::test]
async fn an_expired_consent_is_asked_again_from_the_status_page() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    exported(&bench, &fake).await;
    let seen = bench.last_seen().await;

    // Le jeton d'accès vit en mémoire : un nouveau collecteur rafraîchit, et Google refuse.
    fake.with(|d| d.refresh_error = Some("invalid_grant"));
    let mut collector = bench.collector(&fake);
    let delay = collector.pass().await;
    assert_eq!(delay, Duration::from_secs(60));
    let card = bench.card().await;
    assert_eq!(card["status"]["state"], "action_required");
    assert_eq!(card["status"]["reason"], "consent_expired");
    assert!(card["authorization_url"].is_string());
    assert_eq!(bench.last_seen().await, seen, "aucun signal de vie");
    assert_eq!(fake.with(|d| d.resets), 0, "un jeton mort ne se retire pas");

    fake.with(|d| d.refresh_error = None);
    bench.consent(&fake).await;
    collector.pass().await;
    assert_eq!(fake.with(|d| d.exchanges), 2);
    assert_eq!(bench.card().await["status"]["state"], "ok");
}

#[tokio::test]
async fn a_refused_consent_is_asked_again_with_a_new_state() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    let mut collector = bench.collector(&fake);
    collector.pass().await;
    let first = bench.card().await["authorization_url"].clone();
    let state = reqwest::Url::parse(first.as_str().unwrap())
        .unwrap()
        .query_pairs()
        .find(|(k, _)| k == "state")
        .unwrap()
        .1
        .into_owned();
    assert_eq!(
        bench
            .callback(&json!({"state": state, "error": "access_denied"}))
            .await,
        204
    );
    collector.pass().await;
    let second = bench.card().await;
    assert!(second["authorization_url"].is_string());
    assert_ne!(second["authorization_url"], first);
    assert_eq!(second["status"]["reason"], "consent_required");
    assert_eq!(fake.with(|d| d.exchanges), 0);
}

#[tokio::test]
async fn a_consent_close_to_its_end_is_reset_then_renewed() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    // Moins d'un jour : le renouvellement commence dès le passage suivant l'accord.
    let short = RENEW_AHEAD.num_seconds() - 3600;
    fake.with(|d| d.refresh_expires_in = short);
    let mut collector = bench.collector(&fake);
    collector.pass().await;
    bench.consent(&fake).await;
    let report = collector.tick().await.unwrap();
    assert_eq!(
        report.awaiting,
        Some(life_tracker::source_status::Reason::ConsentExpired)
    );
    assert_eq!(
        fake.with(|d| d.resets),
        1,
        "l'accord en cours est retiré avant d'en demander un"
    );
    assert!(fake.with(|d| d.initiates.is_empty()));
    assert!(bench.last_seen().await.is_some(), "Google a répondu");
    assert!(bench.card().await["authorization_url"].is_string());
}

#[tokio::test]
async fn a_rejected_api_key_asks_for_an_action() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    let mut config = bench.config(&fake);
    config.api_key = "mauvaise-cle".to_owned();
    let mut collector = Collector::new(config).unwrap();
    collector.pass().await;
    bench.consent(&fake).await;
    collector.pass().await;
    fake.with(|d| d.job_state = "COMPLETE");
    collector.pass().await;
    let card = bench.card().await;
    assert_eq!(card["status"]["state"], "action_required");
    assert_eq!(card["status"]["reason"], "key_rejected");
    assert_eq!(
        bench.journal("youtube.watch").await,
        0,
        "les vues attendent leur classement"
    );
    assert_eq!(collector.awaiting_classification(), 9);
    assert_eq!(collector.last_exported_at(), None);
}

// ─── Le relais de l'accord ───────────────────────────────────────────────────

const STATE: &str = "etat-fictif-abcdefghijklmnopqrstuvwxyz0123456789";

fn consent_url(state: &str) -> String {
    format!("https://accounts.google.com/o/oauth2/v2/auth?client_id=x&state={state}")
}

#[tokio::test]
async fn the_hand_off_routes_keep_a_code_for_its_source_only_and_once() {
    let bench = Bench::new().await;
    let other = admin::create_source(&bench.db.admin, "autre", Some("1 day"), None)
        .await
        .unwrap();
    let put = |token: &str, body: Value| {
        bench
            .http
            .put(format!("{}/v1/sources/me/authorization", bench.base))
            .bearer_auth(token)
            .json(&body)
    };
    let code = |token: &str| {
        bench
            .http
            .get(format!("{}/v1/sources/me/authorization/code", bench.base))
            .bearer_auth(token)
    };
    let valid = json!({"authorization_url": consent_url(STATE), "state": STATE});

    // Le token d'une source seulement : ni la session, ni le jeton de service, ni rien.
    for request in [
        bench
            .http
            .put(format!("{}/v1/sources/me/authorization", bench.base))
            .json(&valid),
        bench
            .http
            .put(format!("{}/v1/sources/me/authorization", bench.base))
            .header("cookie", &bench.cookie)
            .header(CSRF_HEADER, "1")
            .json(&valid),
        put(&bench.service, valid.clone()),
        put("inconnu", valid.clone()),
        code(&bench.service),
    ] {
        assert_eq!(request.send().await.unwrap().status(), 401);
    }
    // Rien que le vocabulaire attendu, et une URL de Google qui porte l'état.
    for body in [
        json!({"authorization_url": consent_url(STATE)}),
        json!({"authorization_url": consent_url(STATE), "state": STATE, "x": 1}),
        json!({"authorization_url": consent_url(STATE), "state": "court"}),
        json!({"authorization_url": consent_url("autre-etat-abcdefghijklmnopqrstuvwxyz0123456789"),
               "state": STATE}),
        json!({"authorization_url": consent_url(STATE).replace("https", "http"), "state": STATE}),
        json!({"authorization_url": consent_url(STATE).replace("accounts.google.com", "evil.example"),
               "state": STATE}),
    ] {
        let response = put(&bench.token, body.clone()).send().await.unwrap();
        assert_eq!(response.status(), 400, "{body}");
        assert_eq!(
            response.json::<Value>().await.unwrap(),
            json!({"error": "invalid_json"})
        );
    }
    assert_eq!(bench.card().await["authorization_url"], Value::Null);

    assert_eq!(
        put(&bench.token, valid.clone())
            .send()
            .await
            .unwrap()
            .status(),
        204
    );
    assert_eq!(
        bench.card().await["authorization_url"],
        json!(consent_url(STATE))
    );
    assert_eq!(
        bench.last_seen().await,
        None,
        "une demande n'est pas un contact"
    );
    let none: Value = code(&bench.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(none, json!({"code": null}));

    // Le retour de Google, par la web app : la session et l'en-tête anti-CSRF.
    let callback = format!("{}/v1/authorizations/callback", bench.base);
    let body = json!({"state": STATE, "code": CODE});
    assert_eq!(
        bench
            .http
            .post(&callback)
            .json(&body)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        bench
            .http
            .post(&callback)
            .bearer_auth(&bench.token)
            .json(&body)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        bench
            .http
            .post(&callback)
            .header("cookie", &bench.cookie)
            .json(&body)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    for bad in [
        json!({"state": STATE}),
        json!({"state": STATE, "code": CODE, "error": "access_denied"}),
        json!({"state": STATE, "code": "a b"}),
        json!({"state": STATE, "error": "Refusé par moi"}),
        json!({"state": STATE, "code": CODE, "scope": "x"}),
    ] {
        assert_eq!(bench.callback(&bad).await, 400, "{bad}");
    }
    assert_eq!(
        bench
            .callback(
                &json!({"state": "inconnu-abcdefghijklmnopqrstuvwxyz0123456789", "code": CODE})
            )
            .await,
        404
    );
    assert_eq!(bench.callback(&body).await, 204);
    assert_eq!(
        bench.callback(&body).await,
        404,
        "un état ne sert qu'une fois"
    );
    assert_eq!(bench.card().await["authorization_url"], Value::Null);

    let none: Value = code(&other).send().await.unwrap().json().await.unwrap();
    assert_eq!(none, json!({"code": null}), "le code d'une autre source");
    let taken: Value = code(&bench.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(taken, json!({"code": CODE}));
    let again: Value = code(&bench.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(again, json!({"code": null}), "une seule fois");
    assert_eq!(
        bench.last_seen().await,
        None,
        "chercher le code n'est pas un contact"
    );

    // Un refus passe tel quel.
    assert_eq!(put(&bench.token, valid).send().await.unwrap().status(), 204);
    assert_eq!(
        bench
            .callback(&json!({"state": STATE, "error": "access_denied"}))
            .await,
        204
    );
    let refused: Value = code(&bench.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(refused, json!({"error": "access_denied"}));
}

// ─── Le parseur et le contrat ────────────────────────────────────────────────

#[test]
fn only_watched_videos_reach_a_payload_and_never_a_dropped_field() {
    let english = parser::parse_activity(&fixture("MyActivity.json")[..]).unwrap();
    let french = parser::parse_activity(&fixture("watch-history.json")[..]).unwrap();
    let s = english.skipped;
    assert_eq!(
        (
            english.watches.len(),
            s.ad,
            s.search,
            s.post,
            s.subscription,
            s.music,
            s.reaction,
            s.other
        ),
        (9, 1, 1, 1, 1, 0, 2, 0)
    );
    let s = french.skipped;
    assert_eq!(
        (
            french.watches.len(),
            s.ad,
            s.search,
            s.post,
            s.subscription,
            s.music,
            s.reaction,
            s.other
        ),
        (1, 1, 1, 0, 0, 1, 1, 1)
    );
    assert_eq!(
        english
            .reactions
            .iter()
            .map(parser::Reaction::watch_key)
            .collect::<Vec<_>>(),
        ["vidFictif02:1792007100000", "vidReactif1:1791882000789"]
    );
    assert_eq!(
        french.watches[0].title.as_deref(),
        Some("Troisième vidéo fictive")
    );
    let records: Vec<Value> = english
        .watches
        .iter()
        .chain(&french.watches)
        .map(parser::Watch::record)
        .collect();
    for record in &records {
        let keys: Vec<&String> = record["payload"].as_object().unwrap().keys().collect();
        assert_eq!(keys, ["channel_id", "channel_name", "title", "video_id"]);
    }
    let text = serde_json::to_string(&records).unwrap();
    for sentinel in SENTINELS.iter().chain(&[
        "From Google Ads",
        "Provenant",
        "http",
        "Watched",
        "regardé",
        "Liked",
        "aimé",
        "vidReactif1",
        "PUBsentinel",
    ]) {
        assert!(!text.contains(sentinel), "{sentinel} dans {text}");
    }
}

#[test]
fn what_the_connector_sends_respects_the_contract() {
    let schema = validator(None);
    let activity = parser::parse_activity(&fixture("MyActivity.json")[..]).unwrap();
    let mut records: Vec<Value> = activity.watches.iter().map(parser::Watch::record).collect();
    records.push(parser::video_info_record(
        "vidFictif01",
        Some(549),
        Some("Première vidéo fictive"),
        Some("UCchaineFictiveA000000"),
        Some("Chaîne Fictive A"),
        Utc::now(),
    ));
    records.push(parser::video_info_record(
        "vidSupprime",
        None,
        None,
        None,
        None,
        Utc::now(),
    ));
    let reaction = &activity.reactions[0];
    records.push(life_tracker::collector::cancel_record(
        "youtube",
        &reaction.watch_key(),
        Utc::now(),
    ));
    let batch = json!({ "records": records });
    let errors: Vec<_> = schema.iter_errors(&batch).map(|e| e.to_string()).collect();
    assert!(errors.is_empty(), "{errors:?}");

    let request = validator(Some("source_authorization_request"));
    assert!(request.is_valid(&json!({"authorization_url": consent_url(STATE), "state": STATE})));
    assert!(!request.is_valid(&json!({"authorization_url": consent_url(STATE)})));
    let callback = validator(Some("authorization_callback_request"));
    assert!(callback.is_valid(&json!({"state": STATE, "code": CODE})));
    assert!(callback.is_valid(&json!({"state": STATE, "error": "access_denied"})));
    assert!(!callback.is_valid(&json!({"state": STATE})));
    let handover = validator(Some("source_authorization_code_response"));
    for valid in [
        json!({"code": CODE}),
        json!({"code": null}),
        json!({"error": "access_denied"}),
    ] {
        assert!(handover.is_valid(&valid), "{valid}");
    }
}

// ─── Le vrai binaire : ce qui sort dans les logs ──────────────────────────────

#[tokio::test]
async fn logs_never_cite_a_secret_a_title_nor_an_id() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    fake.with(|d| {
        d.job_state = "COMPLETE";
        d.refresh_tokens
            .push("RAFRAICHISSEMENT-SENTINELLE-0".to_owned());
    });
    // Un accord déjà donné : le processus rafraîchit, lance l'export et le lit au passage suivant.
    let expires = (Utc::now() + TimeDelta::days(6)).to_rfc3339();
    std::fs::write(
        bench.state_file(),
        json!({"refresh_token": "RAFRAICHISSEMENT-SENTINELLE-0", "refresh_expires_at": expires,
               "consented": true})
        .to_string(),
    )
    .unwrap();
    let log = bench.state_dir.join("collector.log");
    let file = std::fs::File::create(&log).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_life-tracker"))
        .arg("collect")
        .env_clear()
        .env("RUST_LOG", "trace")
        .env("YOUTUBE_CLIENT_ID", CLIENT_ID)
        .env("YOUTUBE_CLIENT_SECRET", CLIENT_SECRET)
        .env("YOUTUBE_REDIRECT_URI", REDIRECT_URI)
        .env("YOUTUBE_API_KEY", API_KEY)
        .env("YOUTUBE_TOKEN", &bench.token)
        .env("YOUTUBE_INTERVAL_SECONDS", "1")
        .env("YOUTUBE_OAUTH_URL", &fake.url)
        .env("YOUTUBE_PORTABILITY_URL", &fake.url)
        .env("YOUTUBE_DATA_URL", format!("{}/youtube/v3", fake.url))
        .env("YOUTUBE_STORAGE_URL", format!("{}/storage", fake.url))
        .env("INGEST_URL", &bench.base)
        .env("COLLECTOR_STATE_DIR", &bench.state_dir)
        .stdout(Stdio::from(file))
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(120);
    let logs = loop {
        let logs = std::fs::read_to_string(&log).unwrap_or_default();
        if logs.contains("\"exported\":true") && logs.contains("\"enriched\":3") {
            break logs;
        }
        assert!(Instant::now() < deadline, "export absent : {logs}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    Command::new("kill")
        .arg(child.id().to_string())
        .status()
        .unwrap();
    assert!(child.wait().unwrap().success());
    assert!(logs.contains("\"skipped_ads\":1"), "{logs}");
    assert!(logs.contains("\"skipped_reactions\":2"), "{logs}");
    assert!(logs.contains("\"skipped_shorts\":4"), "{logs}");
    assert!(logs.contains("\"skipped_unavailable\":1"), "{logs}");
    assert!(logs.contains("\"cancelled\":10"), "{logs}");
    assert!(logs.contains("\"awaiting_classification\":0"), "{logs}");
    let access = fake.with(|d| d.access_tokens.clone());
    let mut secrets: Vec<&str> = vec![
        CLIENT_SECRET,
        API_KEY,
        &bench.token,
        SIGNATURE,
        "RAFRAICHISSEMENT-SENTINELLE",
        "vidFictif",
        "vidReactif",
        "vidSupprime",
        "shortFict",
        "vertFict",
        "carreFict",
        "Short",
        "UCchaine",
        "Chaîne Fictive",
        "vidéo fictive",
        "job-1",
    ];
    secrets.extend(access.iter().map(String::as_str));
    secrets.extend(SENTINELS);
    for secret in secrets {
        assert!(!logs.contains(secret), "fuite de {secret} : {logs}");
    }
    assert!(
        !bench.state_dir.join("state.json").exists(),
        "Last.fm n'est pas configuré"
    );
}

#[test]
fn an_incomplete_youtube_configuration_stops_the_collector_at_startup() {
    let output = Command::new(env!("CARGO_BIN_EXE_life-tracker"))
        .arg("collect")
        .env_clear()
        .env("YOUTUBE_CLIENT_ID", CLIENT_ID)
        .env("INGEST_URL", "http://127.0.0.1:9")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("YOUTUBE_CLIENT_SECRET"), "{stderr}");
    assert!(!stderr.contains(CLIENT_ID), "{stderr}");
}
