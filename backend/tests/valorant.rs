//! VALORANT de bout en bout, sans aucun réseau extérieur : le connecteur contre un HenrikDev simulé et la
//! vraie API d'ingestion, puis le vrai worker et la carte du jour. Les réponses suivent la structure de
//! `v4/matches` relevée le 12 nov. 2026 (essai du chantier Riot) ; les identifiants, pseudos et la clé
//! sont fictifs.

mod common;
use axum::Router;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use chrono::{DateTime, TimeDelta, Utc};
use common::*;
use life_tracker::api;
use life_tracker::collector::valorant::{self, Collector, Config, ServiceError, TickError};
use life_tracker::interpreters::INTERPRETERS;
use life_tracker::{admin, engine, replay};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::watch;

const PASSWORD: &str = "un mot de passe assez long";
const KEY: &str = "HDEV-cle-fictive-0123456789";
const NAME: &str = "Joueur Fictif";
const TAG: &str = "EUW";
const OTHER: &str = "Autre Sentinelle";
const AGENT: &str = "AgentSentinelle";
const MAP: &str = "CarteSentinelle";
/// La fenêtre de Riot : au-delà, HenrikDev rend une page vide.
const WINDOW: usize = 36;

fn at(text: &str) -> DateTime<Utc> {
    text.parse().unwrap()
}

/// Une partie telle que `v4/matches` la rend, avec un autre joueur et ce que la liste blanche écarte.
fn game(id: &str, started: DateTime<Utc>, minutes: i64, won: bool, rounds: (i64, i64)) -> Value {
    json!({
        "metadata": {
            "match_id": id, "map": {"id": "carte", "name": MAP},
            "game_version": "release-99", "game_length_in_ms": minutes * 60_000,
            "started_at": started.to_rfc3339(), "is_completed": true,
            "queue": {"id": "competitive", "name": "Competitive", "mode_type": "Standard"},
            "cluster": "Paris",
        },
        "players": [
            {"puuid": "puuid-autre", "name": OTHER, "tag": "0000", "team_id": "Blue",
             "agent": {"id": "x", "name": "Sova"}},
            {"puuid": "puuid-moi-fictif", "name": NAME, "tag": TAG, "team_id": "Red",
             "agent": {"id": "y", "name": AGENT}, "stats": {"score": 4200, "kills": 20}},
        ],
        "teams": [
            {"team_id": "Red", "won": won, "rounds": {"won": rounds.0, "lost": rounds.1}},
            {"team_id": "Blue", "won": !won && rounds.0 != rounds.1,
             "rounds": {"won": rounds.1, "lost": rounds.0}},
        ],
        "kills": [{"killer": {"name": OTHER}}],
    })
}

fn id(n: usize) -> String {
    format!("0d3c6a4e-0000-4000-8000-{n:012}")
}

// ─── HenrikDev simulé ────────────────────────────────────────────────────────

#[derive(Default)]
struct Data {
    /// Des plus récentes aux plus anciennes ; seules les `WINDOW` premières sont servies.
    matches: Vec<Value>,
    refused: Option<u16>,
    /// Encore tant de pages, puis des 429.
    pages_left: Option<u32>,
    /// Les `start` demandés.
    asked: Vec<usize>,
}

#[derive(Clone)]
struct Fake {
    url: String,
    data: Arc<Mutex<Data>>,
}

async fn henrik(
    State(fake): State<Fake>,
    uri: Uri,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let mut data = fake.data.lock().unwrap();
    if let Some(status) = data.refused {
        return (
            StatusCode::from_u16(status).unwrap(),
            axum::Json(json!({"errors": [{"message": "Invalid API key"}]})),
        )
            .into_response();
    }
    assert_eq!(headers["authorization"], KEY);
    assert_eq!(
        uri.path(),
        format!(
            "/valorant/v4/matches/eu/pc/{}/{TAG}",
            NAME.replace(' ', "%20")
        )
    );
    assert_eq!(query["size"], "10");
    if let Some(left) = data.pages_left.as_mut() {
        if *left == 0 {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                [("x-ratelimit-remaining", "0"), ("x-ratelimit-reset", "9")],
            )
                .into_response();
        }
        *left -= 1;
    }
    let start: usize = query["start"].parse().unwrap();
    data.asked.push(start);
    let page: Vec<Value> = data
        .matches
        .iter()
        .take(WINDOW)
        .skip(start)
        .take(10)
        .cloned()
        .collect();
    (
        [("x-ratelimit-remaining", "25"), ("x-ratelimit-reset", "30")],
        axum::Json(json!({"status": 200, "data": page})),
    )
        .into_response()
}

impl Fake {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let fake = Self {
            url: format!("http://{}", listener.local_addr().unwrap()),
            data: Arc::new(Mutex::new(Data::default())),
        };
        let app = Router::new()
            .route("/{*path}", get(henrik))
            .with_state(fake.clone());
        tokio::spawn(async move { axum::serve(listener, app).await });
        fake
    }

    fn with<T>(&self, change: impl FnOnce(&mut Data) -> T) -> T {
        change(&mut self.data.lock().unwrap())
    }

    fn asked(&self) -> Vec<usize> {
        self.with(|d| std::mem::take(&mut d.asked))
    }
}

/// 40 parties dont 36 dans la fenêtre, et une partie non terminée parmi les plus récentes.
fn history(fake: &Fake) {
    let base = at("2026-10-01T18:00:00Z");
    fake.with(|d| {
        for n in (0..40).rev() {
            let started = base + TimeDelta::hours(5 * n as i64);
            d.matches
                .push(game(&id(n), started, 35, n % 2 == 0, (13, 8)));
        }
        d.matches[3]["metadata"]["is_completed"] = json!(false);
    });
}

// ─── Le banc ─────────────────────────────────────────────────────────────────

struct Bench {
    db: TestDb,
    base: String,
    token: String,
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
        let token = admin::create_source(
            &db.admin,
            "valorant",
            Some("2 days"),
            Some("Valorant (HenrikDev)"),
        )
        .await
        .unwrap();
        admin::grant_kind(&db.admin, "valorant", "valorant.match")
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
            riot_id: format!("{NAME}#{TAG}"),
            region: "eu".to_owned(),
            ingest_url: self.base.clone(),
            token: self.token.clone(),
            state_dir: self.state_dir.clone(),
            interval: Duration::from_secs(900),
        })
        .unwrap()
    }

    fn state_path(&self) -> PathBuf {
        self.state_dir.join("valorant-state.json")
    }

    async fn journal(&self) -> i64 {
        self.db
            .count("SELECT count(*) FROM core.records WHERE kind = 'valorant.match'")
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

    async fn games(&self, date: &str) -> Value {
        self.settled().await;
        let response = self
            .http
            .get(format!("{}/v1/days/{date}/games", self.base))
            .header("cookie", &self.cookie)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        response.json().await.unwrap()
    }

    async fn last_seen(&self) -> Option<DateTime<Utc>> {
        sqlx::query_scalar("SELECT last_seen_at FROM core.sources WHERE name = 'valorant'")
            .fetch_one(&self.db.app)
            .await
            .unwrap()
    }

    async fn status(&self) -> Option<Value> {
        let text: Option<String> =
            sqlx::query_scalar("SELECT status::text FROM core.sources WHERE name = 'valorant'")
                .fetch_one(&self.db.app)
                .await
                .unwrap();
        text.map(|t| serde_json::from_str(&t).unwrap())
    }
}

// ─── Le connecteur ───────────────────────────────────────────────────────────

#[tokio::test]
async fn the_first_pass_reads_every_page_then_only_the_new_matches() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    history(&fake);
    let mut collector = bench.collector(&fake);
    let report = collector.tick().await.unwrap();
    assert_eq!(fake.asked(), [0, 10, 20, 30, 36], "jusqu'à la page vide");
    assert_eq!(
        (
            report.pages,
            report.accepted,
            report.incomplete,
            report.skipped
        ),
        (5, 35, 1, 0)
    );
    assert_eq!(bench.journal().await, 35);
    assert!(bench.last_seen().await.is_some());
    let journal: String =
        sqlx::query_scalar("SELECT string_agg(payload::text, '') FROM core.records")
            .fetch_one(&bench.db.app)
            .await
            .unwrap();
    for absent in [OTHER, NAME, "Sova", "puuid", "Paris", "kills"] {
        assert!(!journal.contains(absent), "{absent} au journal");
    }

    // Rien de neuf : la première page seulement.
    let report = collector.tick().await.unwrap();
    assert_eq!((report.pages, report.matches), (1, 0));
    assert_eq!(fake.asked(), [0]);

    // Deux parties de plus : la première page en a, la suivante non.
    let now = Utc::now() - TimeDelta::hours(1);
    fake.with(|d| {
        d.matches.insert(0, game(&id(100), now, 30, true, (13, 5)));
        d.matches.insert(
            0,
            game(&id(101), now + TimeDelta::minutes(40), 30, false, (5, 13)),
        );
    });
    let report = collector.tick().await.unwrap();
    assert_eq!((report.matches, report.accepted), (2, 2));
    assert_eq!(fake.asked(), [0, 10]);
    assert_eq!(collector.done(), 37);
}

#[tokio::test]
async fn a_lost_state_resends_the_window_as_duplicates() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    history(&fake);
    bench.collector(&fake).tick().await.unwrap();
    std::fs::remove_file(bench.state_path()).unwrap();
    let report = bench.collector(&fake).tick().await.unwrap();
    assert_eq!((report.accepted, report.duplicate), (0, 35));
    assert_eq!(bench.journal().await, 35);
}

#[tokio::test]
async fn a_rate_limit_keeps_what_was_read_and_the_next_pass_resumes() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    history(&fake);
    fake.with(|d| d.pages_left = Some(2));
    let mut collector = bench.collector(&fake);
    assert_eq!(
        collector.tick().await.unwrap_err(),
        TickError::Service(ServiceError::RateLimited {
            retry_after: Duration::from_secs(9)
        })
    );
    assert_eq!(
        bench.journal().await,
        19,
        "deux pages, moins la partie non terminée"
    );
    fake.with(|d| d.pages_left = Some(0));
    assert_eq!(collector.pass().await, Duration::from_secs(9));
    let state = bench.status().await.unwrap();
    assert_eq!(
        (&state["state"], &state["reason"]),
        (&json!("error"), &json!("rate_limited"))
    );
    fake.with(|d| d.pages_left = None);
    fake.asked();
    let report = collector.tick().await.unwrap();
    assert_eq!((report.accepted, report.duplicate), (16, 0));
    assert_eq!(bench.journal().await, 35);
    collector.pass().await;
    assert_eq!(bench.status().await.unwrap()["state"], "ok");
}

#[tokio::test]
async fn a_rejected_key_asks_for_an_action_and_sends_no_sign_of_life() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    history(&fake);
    for status in [401, 403] {
        fake.with(|d| d.refused = Some(status));
        let mut collector = bench.collector(&fake);
        assert_eq!(
            collector.tick().await.unwrap_err(),
            TickError::Service(ServiceError::KeyRejected { status })
        );
        collector.pass().await;
        let state = bench.status().await.unwrap();
        assert_eq!(
            (&state["state"], &state["reason"]),
            (&json!("action_required"), &json!("key_rejected"))
        );
    }
    assert_eq!(bench.last_seen().await, None);
    fake.with(|d| d.refused = None);
    bench.collector(&fake).pass().await;
    let state = bench.status().await.unwrap();
    assert_eq!(state["state"], "ok");
    assert!(state["last_collected_at"].is_string());
    assert_eq!(bench.journal().await, 35);
}

// ─── Ce qui se lit ───────────────────────────────────────────────────────────

#[tokio::test]
async fn the_day_card_lists_the_matches_with_their_map_and_score_and_a_rebuild_keeps_them() {
    let bench = Bench::new().await;
    let records: Vec<Value> = [
        game(&id(1), at("2026-11-07T19:00:00Z"), 38, true, (13, 9)),
        game(&id(2), at("2026-11-07T20:00:00Z"), 45, false, (12, 12)),
    ]
    .iter()
    .map(|raw| valorant::match_record(raw, NAME, TAG).unwrap())
    .collect();
    let response = bench
        .http
        .post(format!("{}/v1/records", bench.base))
        .bearer_auth(&bench.token)
        .json(&json!({ "records": records }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let card = bench.games("2026-11-07").await;
    assert_eq!(card["minutes"], 83, "{card}");
    assert_eq!(card["games"][0]["key"], "riot:valorant");
    assert_eq!(card["games"][0]["name"], "VALORANT");
    assert_eq!(
        card["matches"],
        json!([
            {"key": format!("valorant:{}", id(1)), "time": "20:00", "game": "riot:valorant",
             "game_name": "VALORANT", "queue": "Competitive", "result": "win",
             "character": AGENT, "position": null, "map": MAP, "placement": null,
             "rounds_won": 13, "rounds_lost": 9, "minutes": 38},
            {"key": format!("valorant:{}", id(2)), "time": "21:00", "game": "riot:valorant",
             "game_name": "VALORANT", "queue": "Competitive", "result": "draw",
             "character": AGENT, "position": null, "map": MAP, "placement": null,
             "rounds_won": 12, "rounds_lost": 12, "minutes": 45},
        ])
    );
    let rebuilt = replay::rebuild(&bench.db.admin, INTERPRETERS, false)
        .await
        .unwrap();
    assert!(rebuilt.switched);
    assert!(rebuilt.comparison.differences.is_empty());
    assert_eq!(bench.games("2026-11-07").await, card);
}

#[test]
fn what_the_connector_sends_respects_the_contract() {
    let schema = validator(None);
    let records = vec![
        valorant::match_record(
            &game(&id(1), at("2026-11-07T19:00:00Z"), 38, true, (13, 9)),
            NAME,
            TAG,
        )
        .unwrap(),
    ];
    let batch = json!({ "records": records });
    let errors: Vec<_> = schema.iter_errors(&batch).map(|e| e.to_string()).collect();
    assert!(errors.is_empty(), "{errors:?}");
}

// ─── Le vrai binaire ─────────────────────────────────────────────────────────

fn collect_process(fake: &Fake, bench: &Bench) -> (std::process::Child, PathBuf) {
    let log = bench.state_dir.join("collector.log");
    let file = std::fs::File::create(&log).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_life-tracker"))
        .arg("collect")
        .env_clear()
        .env("RUST_LOG", "trace")
        .env("HENRIK_API_URL", &fake.url)
        .env("HENRIK_API_KEY", KEY)
        .env("VALORANT_RIOT_ID", format!("{NAME}#{TAG}"))
        .env("VALORANT_TOKEN", &bench.token)
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
async fn logs_never_cite_the_key_the_account_an_agent_or_a_map() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    history(&fake);
    let (child, log) = collect_process(&fake, &bench);
    let logs = logs_until(child, &log, "valorant pass completed").await;
    assert!(logs.contains("\"accepted\":35"), "{logs}");
    let leaks = [KEY, NAME, "Joueur", AGENT, MAP, OTHER, &bench.token];
    for secret in leaks {
        assert!(!logs.contains(secret), "fuite de {secret} : {logs}");
    }
    let state = std::fs::read_to_string(bench.state_path()).unwrap();
    for secret in [KEY, NAME, AGENT, MAP] {
        assert!(!state.contains(secret), "{secret} dans l'état");
    }
    fake.with(|d| d.refused = Some(401));
    let (child, log) = collect_process(&fake, &bench);
    let logs = logs_until(child, &log, "clé HenrikDev refusée").await;
    for secret in leaks {
        assert!(!logs.contains(secret), "fuite de {secret} : {logs}");
    }
}
