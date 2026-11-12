//! Riot (League of Legends et Teamfight Tactics) de bout en bout, sans aucun réseau extérieur : le
//! connecteur contre une API de Riot simulée et la vraie API d'ingestion, puis le vrai worker et les
//! lectures (carte du jour, durées, calendrier, MCP). Les réponses suivent la structure relevée sur le
//! vrai compte le 12 nov. 2026 (essai du chantier Riot) ; les PUUID, pseudos, identifiants de parties
//! et clés sont fictifs.

mod common;
use axum::Router;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use chrono::{DateTime, TimeDelta, Utc};
use common::*;
use life_tracker::api;
use life_tracker::collector::riot::{self, Collector, Config, Game, ServiceError, TickError};
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
const LOL_KEY: &str = "RGAPI-cle-lol-fictive-0000";
const TFT_KEY: &str = "RGAPI-cle-tft-fictive-1111";
/// Un Riot ID fictif, avec une espace : il doit être encodé dans l'URL, jamais dans un log.
const NAME: &str = "Joueur Fictif";
const TAG: &str = "EUW";
/// Le PUUID est chiffré par produit : un par clé.
const PUUID_LOL: &str = "puuid-lol-fictif-0123456789";
const PUUID_TFT: &str = "puuid-tft-fictif-9876543210";
/// Ce que la liste blanche écarte : jamais au journal ni dans les logs.
const OTHER: &str = "Autre Sentinelle";
const CHAMPION: &str = "ChampionSentinelle";
const MINUTE: i64 = 60_000;

fn at(text: &str) -> DateTime<Utc> {
    text.parse().unwrap()
}

/// Une partie de LoL telle que `match-v5` la rend, avec un autre joueur et ce qu'on écarte.
fn lol(start: DateTime<Utc>, duration_s: i64, queue: i64, win: bool, remake: bool) -> Value {
    json!({
        "metadata": {"participants": ["puuid-autre", PUUID_LOL]},
        "info": {
            "gameCreation": start.timestamp_millis() - 30_000,
            "gameStartTimestamp": start.timestamp_millis(),
            "gameEndTimestamp": start.timestamp_millis() + duration_s * 1000,
            "gameDuration": duration_s, "queueId": queue, "gameMode": "CLASSIC",
            "endOfGameResult": "GameComplete",
            "participants": [
                {"puuid": "puuid-autre", "riotIdGameName": OTHER, "championName": "Zed",
                 "win": !win},
                {"puuid": PUUID_LOL, "riotIdGameName": NAME, "riotIdTagline": TAG,
                 "championName": CHAMPION, "win": win, "teamPosition": "MIDDLE",
                 "gameEndedInEarlySurrender": remake, "kills": 7},
            ],
        },
    })
}

/// Une partie de TFT : `gameCreation` est le début, `game_datetime` la fin.
fn tft(created: DateTime<Utc>, length_s: f64, eliminated_s: f64, placement: i64) -> Value {
    json!({
        "metadata": {"participants": ["puuid-autre", PUUID_TFT]},
        "info": {
            "gameCreation": created.timestamp_millis(),
            "game_datetime": created.timestamp_millis() + (length_s * 1000.0) as i64 + 60_000,
            "game_length": length_s, "queue_id": 1100, "tft_game_type": "standard",
            "tft_set_number": 15,
            "participants": [
                {"puuid": "puuid-autre", "riotIdGameName": OTHER, "placement": 1,
                 "time_eliminated": length_s},
                {"puuid": PUUID_TFT, "riotIdGameName": NAME, "placement": placement,
                 "time_eliminated": eliminated_s, "units": [{"character_id": "TFT_Sentinelle"}]},
            ],
        },
    })
}

// ─── Riot simulé ─────────────────────────────────────────────────────────────

#[derive(Default)]
struct Data {
    /// Les parties de chaque jeu, des plus récentes aux plus anciennes, comme Riot les liste.
    lol: Vec<(String, Value)>,
    tft: Vec<(String, Value)>,
    /// Statut renvoyé pour une clé (401, 403…) ; absente, la clé est acceptée.
    refused: HashMap<String, u16>,
    /// Encore tant d'appels de détail, puis des 429.
    details_left: Option<u32>,
    detail_calls: u32,
    list_calls: u32,
}

#[derive(Clone)]
struct Fake {
    url: String,
    data: Arc<Mutex<Data>>,
}

async fn riot_api(
    State(fake): State<Fake>,
    uri: Uri,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let key = headers
        .get("x-riot-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let mut data = fake.data.lock().unwrap();
    if let Some(status) = data.refused.get(&key) {
        return (
            StatusCode::from_u16(*status).unwrap(),
            axum::Json(json!({"status": {"message": "Unknown apikey", "status_code": status}})),
        )
            .into_response();
    }
    let (game, puuid) = match key.as_str() {
        LOL_KEY => ("lol", PUUID_LOL),
        TFT_KEY => ("tft", PUUID_TFT),
        _ => return StatusCode::UNAUTHORIZED.into_response(),
    };
    let path = uri.path();
    let encoded = format!(
        "/riot/account/v1/accounts/by-riot-id/{}/{TAG}",
        NAME.replace(' ', "%20")
    );
    if path == encoded {
        return axum::Json(json!({"puuid": puuid, "gameName": NAME, "tagLine": TAG}))
            .into_response();
    }
    let prefix = if game == "lol" {
        "/lol/match/v5/matches/"
    } else {
        "/tft/match/v1/matches/"
    };
    let Some(rest) = path.strip_prefix(prefix) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let list = if game == "lol" { &data.lol } else { &data.tft };
    if let Some(asked) = rest.strip_prefix("by-puuid/") {
        assert_eq!(asked, format!("{puuid}/ids"), "le PUUID de la clé");
        let start: usize = query["start"].parse().unwrap();
        let count: usize = query["count"].parse().unwrap();
        assert_eq!(count, 100);
        let ids: Vec<String> = list
            .iter()
            .skip(start)
            .take(count)
            .map(|(id, _)| id.clone())
            .collect();
        data.list_calls += 1;
        return axum::Json(json!(ids)).into_response();
    }
    let detail = list
        .iter()
        .find(|(id, _)| id == rest)
        .map(|(_, d)| d.clone());
    if let Some(left) = data.details_left.as_mut() {
        if *left == 0 {
            return (StatusCode::TOO_MANY_REQUESTS, [("retry-after", "7")]).into_response();
        }
        *left -= 1;
    }
    data.detail_calls += 1;
    // Une partie listée à `null` : Riot ne la garde plus (404).
    match detail {
        Some(detail) if !detail.is_null() => axum::Json(detail).into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

impl Fake {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let fake = Self {
            url: format!("http://{}", listener.local_addr().unwrap()),
            data: Arc::new(Mutex::new(Data::default())),
        };
        let app = Router::new()
            .route("/{*path}", get(riot_api))
            .with_state(fake.clone());
        tokio::spawn(async move { axum::serve(listener, app).await });
        fake
    }

    fn with<T>(&self, change: impl FnOnce(&mut Data) -> T) -> T {
        change(&mut self.data.lock().unwrap())
    }

    /// Une partie de plus, la plus récente.
    fn play(&self, game: Game, id: &str, detail: Value) {
        self.with(|d| {
            let list = match game {
                Game::Lol => &mut d.lol,
                Game::Tft => &mut d.tft,
            };
            list.insert(0, (id.to_owned(), detail));
        });
    }

    /// Les appels depuis le dernier relevé : (listes, détails).
    fn calls(&self) -> (u32, u32) {
        self.with(|d| {
            (
                std::mem::take(&mut d.list_calls),
                std::mem::take(&mut d.detail_calls),
            )
        })
    }
}

/// 150 parties de LoL (deux pages) et 3 de TFT, à des heures passées.
fn history(fake: &Fake) {
    let base = at("2026-09-01T17:00:00Z");
    for i in 0..150 {
        let start = base + TimeDelta::hours(6 * i);
        fake.play(
            Game::Lol,
            &format!("EUW1_{}", 7_000_000 + i),
            lol(start, 1800, 420, i % 2 == 0, false),
        );
    }
    for i in 0..3 {
        let created = base + TimeDelta::hours(6 * i + 3);
        fake.play(
            Game::Tft,
            &format!("EUW1_{}", 8_000_000 + i),
            tft(created, 2000.0, 1500.0, 4),
        );
    }
}

// ─── Le banc ─────────────────────────────────────────────────────────────────

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
        let token = admin::create_source(
            &db.admin,
            "riot",
            Some("2 days"),
            Some("Riot Games (LoL, TFT)"),
        )
        .await
        .unwrap();
        for kind in ["riot.lol_match", "riot.tft_match"] {
            admin::grant_kind(&db.admin, "riot", kind).await.unwrap();
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
            lol_api_key: LOL_KEY.to_owned(),
            tft_api_key: TFT_KEY.to_owned(),
            riot_id: format!("{NAME}#{TAG}"),
            ingest_url: self.base.clone(),
            token: self.token.clone(),
            state_dir: self.state_dir.clone(),
            interval: Duration::from_secs(900),
            pause: Duration::ZERO,
        })
        .unwrap()
    }

    fn state_path(&self) -> PathBuf {
        self.state_dir.join("riot-state.json")
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

    async fn events(&self, metric: &str) -> i64 {
        self.settled().await;
        self.db
            .count(&format!(
                "SELECT count(*) FROM derived.live_events WHERE metric = '{metric}'"
            ))
            .await
    }

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

    async fn as_mcp(&self, path: &str) -> (u16, Value) {
        self.settled().await;
        let response = self
            .http
            .get(format!("{}{path}", self.base))
            .bearer_auth(&self.service)
            .send()
            .await
            .unwrap();
        let status = response.status().as_u16();
        (status, response.json().await.unwrap_or(Value::Null))
    }

    async fn games(&self, date: &str) -> Value {
        self.get(&format!("/v1/days/{date}/games")).await
    }

    async fn last_seen(&self) -> Option<DateTime<Utc>> {
        sqlx::query_scalar("SELECT last_seen_at FROM core.sources WHERE name = 'riot'")
            .fetch_one(&self.db.app)
            .await
            .unwrap()
    }

    async fn status(&self) -> Option<Value> {
        let text: Option<String> =
            sqlx::query_scalar("SELECT status::text FROM core.sources WHERE name = 'riot'")
                .fetch_one(&self.db.app)
                .await
                .unwrap();
        text.map(|t| serde_json::from_str(&t).unwrap())
    }

    /// Les tranches de jeu : début et durée, dans l'ordre.
    async fn plays(&self) -> Vec<(DateTime<Utc>, i64)> {
        self.settled().await;
        sqlx::query_as(
            "SELECT occurred_at, (details->>'played_ms')::bigint FROM derived.live_events
             WHERE metric = 'game.play' ORDER BY occurred_at",
        )
        .fetch_all(&self.db.app)
        .await
        .unwrap()
    }
}

// ─── Le connecteur ───────────────────────────────────────────────────────────

#[tokio::test]
async fn the_first_pass_reads_the_whole_history_then_only_the_new_matches() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    history(&fake);
    let mut collector = bench.collector(&fake);
    let report = collector.tick().await.unwrap();
    // LoL : une page pleine puis une page courte ; TFT : une page courte.
    assert_eq!(fake.calls(), (3, 153));
    assert_eq!(
        (
            report.matches,
            report.accepted,
            report.duplicate,
            report.skipped
        ),
        (153, 153, 0, 0)
    );
    assert_eq!(
        report.calls,
        2 + 3 + 153,
        "deux comptes, trois listes, les détails"
    );
    assert_eq!(bench.journal("riot.lol_match").await, 150);
    assert_eq!(bench.journal("riot.tft_match").await, 3);
    assert!(bench.last_seen().await.is_some(), "signal de vie");
    assert_eq!(bench.events("game.match").await, 153);
    assert_eq!(bench.events("game.play").await, 153);
    // Le journal garde ma ligne seulement : ni l'autre joueur, ni mon Riot ID, ni mon PUUID.
    let journal: String =
        sqlx::query_scalar("SELECT string_agg(payload::text, '') FROM core.records")
            .fetch_one(&bench.db.app)
            .await
            .unwrap();
    for absent in [
        OTHER,
        NAME,
        PUUID_LOL,
        PUUID_TFT,
        "Zed",
        "TFT_Sentinelle",
        "kills",
    ] {
        assert!(!journal.contains(absent), "{absent} au journal");
    }
    assert!(journal.contains(CHAMPION));
    // L'ordre du journal suit celui des parties : les plus anciennes d'abord.
    let first: String = sqlx::query_scalar(
        "SELECT key FROM core.records WHERE kind = 'riot.lol_match' ORDER BY seq LIMIT 1",
    )
    .fetch_one(&bench.db.app)
    .await
    .unwrap();
    assert_eq!(first, "lol:EUW1_7000000");

    // Rien de neuf : une page par jeu, aucun détail.
    let report = collector.tick().await.unwrap();
    assert_eq!(report.matches, 0);
    assert_eq!(fake.calls(), (2, 0));

    // Deux parties de LoL, une de TFT : seules elles sont lues.
    let now = Utc::now() - TimeDelta::hours(2);
    fake.play(Game::Lol, "EUW1_7100000", lol(now, 1500, 450, true, false));
    fake.play(
        Game::Lol,
        "EUW1_7100001",
        lol(now + TimeDelta::minutes(30), 1500, 450, false, false),
    );
    fake.play(Game::Tft, "EUW1_8100000", tft(now, 2000.0, 1200.0, 6));
    let report = collector.tick().await.unwrap();
    assert_eq!((report.matches, report.accepted), (3, 3));
    // La première page de LoL a des inconnues : la suivante est lue aussi, sans rien de neuf.
    assert_eq!(fake.calls(), (3, 3));
    assert_eq!(collector.done(Game::Lol), 152);
    assert_eq!(collector.done(Game::Tft), 4);
}

#[tokio::test]
async fn a_lost_state_resends_everything_as_duplicates() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    history(&fake);
    bench.collector(&fake).tick().await.unwrap();
    std::fs::remove_file(bench.state_path()).unwrap();
    let report = bench.collector(&fake).tick().await.unwrap();
    assert_eq!(
        (report.matches, report.accepted, report.duplicate),
        (153, 0, 153)
    );
    assert_eq!(bench.journal("riot.lol_match").await, 150);
    assert_eq!(bench.events("game.match").await, 153, "aucun doublon");
}

#[tokio::test]
async fn a_rate_limit_stops_the_pass_and_the_next_one_resumes() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    history(&fake);
    // 45 détails, puis des 429 : deux lots de 20 et 5 parties lues partent quand même.
    fake.with(|d| d.details_left = Some(45));
    let mut collector = bench.collector(&fake);
    assert_eq!(
        collector.tick().await.unwrap_err(),
        TickError::Service(ServiceError::RateLimited {
            retry_after: Duration::from_secs(7)
        })
    );
    assert_eq!(bench.journal("riot.lol_match").await, 45);
    fake.with(|d| d.details_left = Some(0));
    assert_eq!(collector.pass().await, Duration::from_secs(7));
    let state = bench.status().await.unwrap();
    assert_eq!(
        (&state["state"], &state["reason"]),
        (&json!("error"), &json!("rate_limited"))
    );

    // La limite retombée : la suite, sans relire ce qui est parti.
    fake.with(|d| d.details_left = None);
    fake.calls();
    let report = collector.tick().await.unwrap();
    assert_eq!((report.accepted, report.duplicate), (108, 0));
    assert_eq!(fake.calls().1, 108);
    assert_eq!(bench.journal("riot.lol_match").await, 150);
    assert_eq!(bench.journal("riot.tft_match").await, 3);
}

#[tokio::test]
async fn a_rejected_key_asks_for_an_action_and_the_other_game_still_comes_in() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    history(&fake);
    for status in [401, 403] {
        fake.with(|d| {
            d.refused.insert(LOL_KEY.to_owned(), status);
            d.refused.insert(TFT_KEY.to_owned(), status);
        });
        let mut collector = bench.collector(&fake);
        assert_eq!(
            collector.tick().await.unwrap_err(),
            TickError::Service(ServiceError::KeyRejected {
                status,
                game: "lol"
            })
        );
        assert_eq!(collector.pass().await, Duration::from_secs(60));
        let state = bench.status().await.unwrap();
        assert_eq!(
            (
                &state["state"],
                &state["reason"],
                &state["last_collected_at"]
            ),
            (
                &json!("action_required"),
                &json!("key_rejected"),
                &Value::Null
            )
        );
    }
    assert_eq!(
        bench.last_seen().await,
        None,
        "le seuil de silence alertera"
    );

    // Seule la clé TFT refusée : LoL entre, l'état demande toujours d'agir.
    fake.with(|d| {
        d.refused.remove(LOL_KEY);
    });
    let mut collector = bench.collector(&fake);
    assert_eq!(
        collector.tick().await.unwrap_err(),
        TickError::Service(ServiceError::KeyRejected {
            status: 403,
            game: "tft"
        })
    );
    assert_eq!(bench.journal("riot.lol_match").await, 150);
    assert_eq!(bench.journal("riot.tft_match").await, 0);
    assert!(bench.last_seen().await.is_some());
    collector.pass().await;
    assert_eq!(bench.status().await.unwrap()["reason"], "key_rejected");

    // La clé remplacée : l'état revient à `ok`.
    fake.with(|d| d.refused.clear());
    collector.pass().await;
    let state = bench.status().await.unwrap();
    assert_eq!(state["state"], "ok");
    assert!(state["last_collected_at"].is_string());
    assert_eq!(bench.journal("riot.tft_match").await, 3);
}

#[tokio::test]
async fn a_match_riot_no_longer_has_is_skipped_once() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    fake.with(|d| {
        d.lol.push(("EUW1_404".to_owned(), Value::Null));
        // Une partie où je ne figure pas : sans enregistrement.
        let mut stranger = lol(at("2026-09-01T17:00:00Z"), 1800, 420, true, false);
        stranger["info"]["participants"][1]["puuid"] = json!("quelqu-un");
        d.lol.push(("EUW1_1".to_owned(), stranger));
    });
    let mut collector = bench.collector(&fake);
    let report = collector.tick().await.unwrap();
    assert_eq!((report.skipped, report.matches), (2, 0));
    let report = collector.tick().await.unwrap();
    assert_eq!(report.skipped, 0, "plus redemandées");
}

// ─── Ce qui se lit ───────────────────────────────────────────────────────────

/// Envoie des parties telles que le connecteur les forme, avec le token de la source.
async fn send(bench: &Bench, records: Vec<Value>) {
    let response = bench
        .http
        .post(format!("{}/v1/records", bench.base))
        .bearer_auth(&bench.token)
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

/// Une soirée : une partie de LoL de 23:40 à 00:20 (heure de Paris), à cheval sur minuit, un remake,
/// et une partie de TFT où je suis éliminé au bout de 25 min d'une partie de 38.
fn evening() -> Vec<Value> {
    vec![
        riot::lol_record(
            "EUW1_1",
            &lol(at("2026-11-07T22:40:00Z"), 2400, 420, true, false),
            PUUID_LOL,
        )
        .unwrap(),
        riot::lol_record(
            "EUW1_2",
            &lol(at("2026-11-07T19:00:00Z"), 210, 440, false, true),
            PUUID_LOL,
        )
        .unwrap(),
        riot::tft_record(
            "EUW1_3",
            &tft(at("2026-11-07T20:00:00Z"), 2280.0, 1500.4, 5),
            PUUID_TFT,
        )
        .unwrap(),
    ]
}

#[tokio::test]
async fn an_evening_of_play_lands_on_the_right_days_with_its_matches() {
    let bench = Bench::new().await;
    send(&bench, evening()).await;
    assert_eq!(
        bench
            .db
            .count("SELECT count(*) FROM derived.interpretation_errors")
            .await,
        0
    );
    // TFT : à `gameCreation`, pour mon temps de jeu seulement.
    let plays = bench.plays().await;
    assert!(
        plays.contains(&(at("2026-11-07T20:00:00Z"), 1_500_400)),
        "{plays:?}"
    );
    assert!(
        plays.contains(&(at("2026-11-07T22:40:00Z"), 40 * MINUTE)),
        "{plays:?}"
    );

    let card = bench.games("2026-11-07").await;
    // 3 min 30 de remake + 25 min de TFT + 20 min avant minuit.
    assert_eq!(card["minutes"], 49, "{card}");
    assert_eq!(card["discoveries"], json!([]), "jamais des découvertes");
    let names: Vec<&str> = card["games"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| g["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Teamfight Tactics", "League of Legends"], "{card}");
    assert_eq!(card["games"][1]["key"], "riot:lol");
    assert_eq!(card["games"][1]["total_minutes"], Value::Null);
    assert_eq!(
        card["matches"],
        json!([
            {"key": "lol:EUW1_2", "time": "20:00", "game": "riot:lol",
             "game_name": "League of Legends", "queue": "Classée flexible", "result": "remake",
             "character": CHAMPION, "position": "MIDDLE", "map": null, "placement": null,
             "rounds_won": null, "rounds_lost": null, "minutes": 4},
            {"key": "tft:EUW1_3", "time": "21:00", "game": "riot:tft",
             "game_name": "Teamfight Tactics", "queue": "Classée", "result": null,
             "character": null, "position": null, "map": null, "placement": 5,
             "rounds_won": null, "rounds_lost": null, "minutes": 25},
            {"key": "lol:EUW1_1", "time": "23:40", "game": "riot:lol",
             "game_name": "League of Legends", "queue": "Classée solo/duo", "result": "win",
             "character": CHAMPION, "position": "MIDDLE", "map": null, "placement": null,
             "rounds_won": null, "rounds_lost": null, "minutes": 40},
        ])
    );
    // Le lendemain : les 20 min d'après minuit, sans la partie, rangée au jour de son début.
    let next = bench.games("2026-11-08").await;
    assert_eq!(next["minutes"], 20);
    assert_eq!(next["matches"], json!([]));

    // Le calendrier : le temps et le nombre de parties du jour.
    let month = bench.get("/v1/months/2026-11").await;
    let values = |date: &str| {
        month["days"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["date"] == date)
            .unwrap()["values"]
            .clone()
    };
    // Des minutes non arrondies : 3,5 + 25,0067 + 20.
    let minutes = values("2026-11-07")["game.play"].as_f64().unwrap();
    assert!((minutes - 48.507).abs() < 0.01, "{minutes}");
    // Les parties n'ont pas de case : sans valeur du jour (`none`), comme les succès Steam.
    assert!(values("2026-11-07").get("game.match").is_none());
    assert_eq!(values("2026-11-08")["game.play"], 20.0);

    // Le MCP : durée, classement par personnage, et les seuls détails partagés.
    let (status, minutes) = bench
        .as_mcp("/v1/durations?metric=game.play&from=2026-11-07&to=2026-11-08")
        .await;
    assert_eq!(status, 200);
    assert_eq!(minutes["total_minutes"], 69);
    let (status, top) = bench
        .as_mcp("/v1/top?metric=game.match&level=character&from=2026-11-01&to=2026-11-30")
        .await;
    assert_eq!(status, 200, "{top}");
    assert_eq!(top["items"][0]["key"], format!("lol:{CHAMPION}"));
    let (status, occurrences) = bench
        .as_mcp("/v1/occurrences?metric=game.match&from=2026-11-07&to=2026-11-07&form=list")
        .await;
    assert_eq!(status, 200, "{occurrences}");
    assert_eq!(
        occurrences["occurrences"][0]["details"],
        json!({"game": "lol", "queue": "Classée flexible", "mode": "CLASSIC",
               "result": "remake", "character": CHAMPION})
    );
    let (_, found) = bench
        .as_mcp("/v1/discoveries?metric=game.play&level=game&from=2026-11-01&to=2026-11-30")
        .await;
    assert_eq!(found["items"], json!([]), "{found}");
}

#[tokio::test]
async fn a_rebuild_gives_no_difference() {
    let bench = Bench::new().await;
    send(&bench, evening()).await;
    let days = ["2026-11-07", "2026-11-08"];
    let mut before = Vec::new();
    for day in days {
        before.push(bench.games(day).await);
    }
    let rebuilt = replay::rebuild(&bench.db.admin, INTERPRETERS, false)
        .await
        .unwrap();
    assert!(rebuilt.switched);
    assert!(rebuilt.comparison.differences.is_empty());
    for (day, before) in days.iter().zip(before) {
        assert_eq!(bench.games(day).await, before);
    }
}

// ─── Le vrai binaire ─────────────────────────────────────────────────────────

fn collect_process(fake: &Fake, bench: &Bench) -> (std::process::Child, PathBuf) {
    let log = bench.state_dir.join("collector.log");
    let file = std::fs::File::create(&log).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_life-tracker"))
        .arg("collect")
        .env_clear()
        .env("RUST_LOG", "trace")
        .env("RIOT_API_URL", &fake.url)
        .env("RIOT_LOL_API_KEY", LOL_KEY)
        .env("RIOT_TFT_API_KEY", TFT_KEY)
        .env("RIOT_ID", format!("{NAME}#{TAG}"))
        .env("RIOT_TOKEN", &bench.token)
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
async fn logs_never_cite_a_key_the_account_or_a_champion() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    fake.play(
        Game::Lol,
        "EUW1_1",
        lol(at("2026-11-07T19:00:00Z"), 1800, 420, true, false),
    );
    fake.play(
        Game::Tft,
        "EUW1_2",
        tft(at("2026-11-07T20:00:00Z"), 2000.0, 1500.0, 2),
    );
    let (child, log) = collect_process(&fake, &bench);
    let logs = logs_until(child, &log, "riot pass completed").await;
    assert!(logs.contains("\"accepted\":2"), "{logs}");
    let leaks = [
        LOL_KEY,
        TFT_KEY,
        NAME,
        "Joueur",
        PUUID_LOL,
        PUUID_TFT,
        CHAMPION,
        OTHER,
        &bench.token,
    ];
    for secret in leaks {
        assert!(!logs.contains(secret), "fuite de {secret} : {logs}");
    }
    let state = std::fs::read_to_string(bench.state_path()).unwrap();
    for secret in [LOL_KEY, TFT_KEY, NAME, PUUID_LOL, PUUID_TFT, CHAMPION] {
        assert!(!state.contains(secret), "{secret} dans l'état");
    }

    fake.with(|d| {
        d.refused.insert(LOL_KEY.to_owned(), 401);
    });
    let (child, log) = collect_process(&fake, &bench);
    let logs = logs_until(child, &log, "clé Riot refusée").await;
    for secret in leaks {
        assert!(!logs.contains(secret), "fuite de {secret} : {logs}");
    }
}

#[test]
fn what_the_connector_sends_respects_the_contract() {
    let schema = validator(None);
    let batch = json!({ "records": evening() });
    let errors: Vec<_> = schema.iter_errors(&batch).map(|e| e.to_string()).collect();
    assert!(errors.is_empty(), "{errors:?}");
}
