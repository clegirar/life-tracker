//! Steam de bout en bout, sans aucun réseau extérieur : le connecteur contre un Steam simulé et la vraie
//! API d'ingestion, puis le vrai worker et les lectures (durée cumulée, carte du jour). Les réponses
//! suivent la structure relevée sur le vrai compte le 6 nov. 2026 (étape 0 du chantier) ; tous les
//! appids, noms, identifiants et la clé sont fictifs.
//!
//! Le connecteur relève l'heure lui-même : pour placer une partie hier ou il y a trois heures, les tests
//! vieillissent le relevé précédent dans l'état du connecteur, comme si le passage d'avant avait eu
//! lieu alors.

mod common;
use axum::Router;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use chrono::{DateTime, Days, NaiveDate, TimeDelta, Utc};
use common::*;
use life_tracker::api;
use life_tracker::collector::garmin::{paris_offset, paris_today};
use life_tracker::collector::steam::{
    self, Collector, Config, OwnedGame, Previous, ServiceError, TickError,
};
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
const KEY: &str = "CLESTEAMFICTIVE0123456789ABCDEF";
const STEAM_ID: &str = "76500000000000042";
/// Des jeux fictifs, aux noms reconnaissables : jamais dans les logs.
const ALPHA: u64 = 4_000_010;
const BETA: u64 = 4_000_020;
const GAMMA: u64 = 4_000_030;
const DELTA: u64 = 4_000_040;
const MINUTE: i64 = 60_000;

fn name(appid: u64) -> String {
    format!("Jeu Sentinelle {appid}")
}

/// Un jeu tel que `GetOwnedGames` le rend, avec ce que la liste blanche écarte.
fn game(appid: u64, minutes: i64, last_played: i64) -> Value {
    json!({
        "appid": appid, "name": name(appid), "playtime_forever": minutes,
        "img_icon_url": "icone-fictive", "has_community_visible_stats": true,
        "playtime_windows_forever": minutes, "playtime_mac_forever": 0,
        "playtime_linux_forever": 0, "playtime_deck_forever": 0,
        "rtime_last_played": last_played, "content_descriptorids": [5],
        "playtime_disconnected": 0,
    })
}

/// Un succès tel que `GetPlayerAchievements` le rend ; verrouillé, `unlocktime` vaut 0.
fn achievement(apiname: &str, unlocked_at: Option<DateTime<Utc>>) -> Value {
    json!({
        "apiname": apiname, "achieved": i64::from(unlocked_at.is_some()),
        "unlocktime": unlocked_at.map_or(0, |at| at.timestamp()),
        "name": format!("Succès {apiname}"), "description": "Description sentinelle",
    })
}

fn with_achievements(appid: u64, list: Vec<Value>) -> (u16, Value) {
    (
        200,
        json!({"playerstats": {"steamID": STEAM_ID, "gameName": name(appid),
                               "achievements": list, "success": true}}),
    )
}

fn without_achievements(appid: u64) -> (u16, Value) {
    (
        200,
        json!({"playerstats": {"steamID": STEAM_ID, "gameName": name(appid), "success": true}}),
    )
}

fn profile_not_public(status: u16) -> (u16, Value) {
    (
        status,
        json!({"playerstats": {"error": "Profile is not public", "success": false}}),
    )
}

fn at(text: &str) -> DateTime<Utc> {
    text.parse().unwrap()
}

/// Minuit à Paris au début d'un jour.
fn paris_midnight(date: NaiveDate) -> DateTime<Utc> {
    let around = date.and_hms_opt(0, 0, 0).unwrap().and_utc();
    date.and_hms_opt(0, 0, 0)
        .unwrap()
        .and_local_timezone(paris_offset(around))
        .single()
        .unwrap()
        .with_timezone(&Utc)
}

fn paris_day(at: DateTime<Utc>) -> NaiveDate {
    at.with_timezone(&paris_offset(at)).date_naive()
}

// ─── Le Steam simulé ─────────────────────────────────────────────────────────

#[derive(Default)]
struct Data {
    games: Vec<Value>,
    /// Statut de `GetOwnedGames` : 200, ou une erreur.
    owned_status: u16,
    /// `{"response": {}}` : la bibliothèque n'est pas visible.
    hide_library: bool,
    /// Statut et corps de `GetPlayerAchievements` par jeu ; un jeu absent n'a pas de statistiques.
    achievements: HashMap<u64, (u16, Value)>,
    /// Les appids dont les succès ont été demandés, dans l'ordre.
    asked: Vec<u64>,
    owned_calls: u32,
}

#[derive(Clone, Default)]
struct Fake {
    url: String,
    data: Arc<Mutex<Data>>,
}

fn check_identity(query: &HashMap<String, String>) {
    assert_eq!(query["key"], KEY);
    assert_eq!(query["steamid"], STEAM_ID);
}

async fn owned_games(
    State(fake): State<Fake>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    check_identity(&query);
    assert_eq!(query["include_appinfo"], "1");
    assert_eq!(query["include_played_free_games"], "1");
    let mut data = fake.data.lock().unwrap();
    data.owned_calls += 1;
    match data.owned_status {
        200 => {}
        429 => return (StatusCode::TOO_MANY_REQUESTS, [("retry-after", "120")]).into_response(),
        other => {
            return (
                StatusCode::from_u16(other).unwrap(),
                "<html>Forbidden</html>",
            )
                .into_response();
        }
    }
    if data.hide_library {
        return axum::Json(json!({"response": {}})).into_response();
    }
    axum::Json(json!({"response": {"game_count": data.games.len(), "games": data.games}}))
        .into_response()
}

async fn player_achievements(
    State(fake): State<Fake>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    check_identity(&query);
    assert_eq!(query["l"], "french");
    let appid: u64 = query["appid"].parse().unwrap();
    let mut data = fake.data.lock().unwrap();
    data.asked.push(appid);
    let (status, body) = data.achievements.get(&appid).cloned().unwrap_or((
        400,
        json!({"playerstats": {"error": "Requested app has no stats", "success": false}}),
    ));
    if status == 429 {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    (StatusCode::from_u16(status).unwrap(), axum::Json(body)).into_response()
}

impl Fake {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let fake = Self {
            url: format!("http://{}", listener.local_addr().unwrap()),
            data: Arc::new(Mutex::new(Data {
                owned_status: 200,
                ..Data::default()
            })),
        };
        let app = Router::new()
            .route("/IPlayerService/GetOwnedGames/v1/", get(owned_games))
            .route(
                "/ISteamUserStats/GetPlayerAchievements/v1/",
                get(player_achievements),
            )
            .with_state(fake.clone());
        tokio::spawn(async move { axum::serve(listener, app).await });
        fake
    }

    fn with<T>(&self, change: impl FnOnce(&mut Data) -> T) -> T {
        change(&mut self.data.lock().unwrap())
    }

    /// Les succès demandés depuis le dernier appel.
    fn asked(&self) -> Vec<u64> {
        self.with(|d| std::mem::take(&mut d.asked))
    }

    fn set_playtime(&self, appid: u64, minutes: i64, last_played: i64) {
        self.with(|d| {
            let game = d
                .games
                .iter_mut()
                .find(|g| g["appid"] == appid)
                .expect("jeu simulé");
            game["playtime_forever"] = json!(minutes);
            game["rtime_last_played"] = json!(last_played);
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
        let db = TestDb::new().await;
        admin::set_password(&db.admin, "moi", PASSWORD)
            .await
            .unwrap();
        let service = admin::create_service_token(&db.admin, "mcp").await.unwrap();
        let token = admin::create_source(&db.admin, "steam", Some("6 hours"), Some("Steam"))
            .await
            .unwrap();
        for kind in ["steam.playtime", "steam.achievement"] {
            admin::grant_kind(&db.admin, "steam", kind).await.unwrap();
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

    fn config(&self, fake: &Fake) -> Config {
        Config {
            api_url: fake.url.clone(),
            api_key: KEY.to_owned(),
            steam_id: STEAM_ID.to_owned(),
            ingest_url: self.base.clone(),
            token: self.token.clone(),
            state_dir: self.state_dir.clone(),
            interval: Duration::from_secs(900),
            pause: Duration::ZERO,
        }
    }

    fn collector(&self, fake: &Fake) -> Collector {
        Collector::new(self.config(fake)).unwrap()
    }

    fn state_path(&self) -> PathBuf {
        self.state_dir.join("steam-state.json")
    }

    /// Recule le dernier passage et le relevé gardé de chaque jeu à `when` : le passage suivant
    /// mesure ce qui s'est joué depuis.
    fn age_state(&self, when: DateTime<Utc>) {
        let mut state: Value =
            serde_json::from_str(&std::fs::read_to_string(self.state_path()).unwrap()).unwrap();
        let when = json!(when.to_rfc3339());
        state["last_pass_at"] = when.clone();
        for game in state["games"].as_object_mut().unwrap().values_mut() {
            game["observed_at"] = when.clone();
        }
        std::fs::write(self.state_path(), state.to_string()).unwrap();
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

    async fn games(&self, date: NaiveDate) -> Value {
        self.get(&format!("/v1/days/{date}/games")).await
    }

    /// Les minutes de `game.play` par jour de Paris, sur la période.
    async fn minutes_by_day(&self, from: NaiveDate, to: NaiveDate) -> Vec<(String, i64)> {
        let body = self
            .get(&format!(
                "/v1/durations?metric=game.play&from={from}&to={to}&group=day"
            ))
            .await;
        body["periods"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                (
                    p["period"].as_str().unwrap().to_owned(),
                    p["minutes"].as_i64().unwrap(),
                )
            })
            .collect()
    }

    async fn total_minutes(&self, from: NaiveDate, to: NaiveDate) -> Value {
        self.get(&format!(
            "/v1/durations?metric=game.play&from={from}&to={to}"
        ))
        .await["total_minutes"]
            .clone()
    }

    async fn last_seen(&self) -> Option<DateTime<Utc>> {
        sqlx::query_scalar("SELECT last_seen_at FROM core.sources WHERE name = 'steam'")
            .fetch_one(&self.db.app)
            .await
            .unwrap()
    }

    async fn status(&self) -> Option<Value> {
        let text: Option<String> =
            sqlx::query_scalar("SELECT status::text FROM core.sources WHERE name = 'steam'")
                .fetch_one(&self.db.app)
                .await
                .unwrap();
        text.map(|t| serde_json::from_str(&t).unwrap())
    }

    /// La charge utile du relevé d'un jeu à un compteur.
    async fn reading(&self, appid: u64, minutes: i64) -> Value {
        sqlx::query_scalar("SELECT payload FROM core.records WHERE key = $1")
            .bind(steam::playtime_key(appid, minutes))
            .fetch_one(&self.db.app)
            .await
            .unwrap()
    }
}

/// Une bibliothèque de trois jeux : ALPHA joué avec des succès, BETA jamais joué, GAMMA joué sans succès.
fn library(fake: &Fake) {
    fake.with(|d| {
        d.games = vec![
            game(ALPHA, 120, at("2026-10-01T20:00:00Z").timestamp()),
            game(BETA, 0, 0),
            game(GAMMA, 55, at("2020-01-26T17:50:27Z").timestamp()),
        ];
        d.achievements.insert(
            ALPHA,
            with_achievements(
                ALPHA,
                vec![
                    achievement("ACH_ONE", Some(at("2025-03-30T12:01:24Z"))),
                    achievement("ACH_TWO", Some(at("2026-10-01T19:40:00Z"))),
                    achievement("ACH_LOCKED", None),
                ],
            ),
        );
        d.achievements.insert(GAMMA, without_achievements(GAMMA));
    });
}

// ─── Le connecteur ───────────────────────────────────────────────────────────

#[tokio::test]
async fn the_first_pass_sets_starting_points_and_the_history_of_achievements() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    library(&fake);
    let mut collector = bench.collector(&fake);
    let report = collector.tick().await.unwrap();
    assert_eq!(report.games, 3);
    assert_eq!(report.readings, 2, "les jeux déjà joués, pas BETA");
    assert_eq!(report.achievements, 2, "les succès débloqués seulement");
    assert_eq!((report.accepted, report.rejected), (4, 0));
    assert!(!report.profile_private);
    let mut asked = fake.asked();
    asked.sort_unstable();
    assert_eq!(asked, [ALPHA, GAMMA], "jamais pour un jeu jamais joué");
    assert!(bench.last_seen().await.is_some(), "signal de vie");
    assert_eq!(collector.games(), 3, "BETA est noté, à zéro");

    // Des points de départ : le total de chaque jeu, aucun temps compté.
    assert_eq!(bench.events("game.total_playtime").await, 2);
    assert_eq!(bench.events("game.play").await, 0);
    // ALPHA et GAMMA avaient déjà du temps : déjà connus, jamais des découvertes. Pas BETA, à zéro.
    assert_eq!(bench.events("game.known").await, 2);
    let first = bench.reading(ALPHA, 120).await;
    assert_eq!(first["previous"], Value::Null);
    assert_eq!(first["name"], name(ALPHA));
    assert_eq!(
        first["rtime_last_played"],
        at("2026-10-01T20:00:00Z").timestamp()
    );
    // Les succès, à leur vraie date, sans description.
    assert_eq!(bench.events("game.achievement").await, 2);
    let dates: Vec<DateTime<Utc>> = sqlx::query_scalar(
        "SELECT occurred_at FROM core.records WHERE kind = 'steam.achievement' ORDER BY occurred_at",
    )
    .fetch_all(&bench.db.app)
    .await
    .unwrap();
    assert_eq!(
        dates,
        [at("2025-03-30T12:01:24Z"), at("2026-10-01T19:40:00Z")]
    );
    let journal: String =
        sqlx::query_scalar("SELECT string_agg(payload::text, '') FROM core.records")
            .fetch_one(&bench.db.app)
            .await
            .unwrap();
    for absent in [
        "Description sentinelle",
        "icone-fictive",
        STEAM_ID,
        "ACH_LOCKED",
    ] {
        assert!(!journal.contains(absent), "{absent} au journal");
    }

    // Rien n'a changé : rien ne part, aucun succès n'est redemandé.
    let report = collector.tick().await.unwrap();
    assert_eq!(
        (report.readings, report.achievements, report.accepted),
        (0, 0, 0)
    );
    assert_eq!(fake.asked(), Vec::<u64>::new());
}

#[tokio::test]
async fn a_40_minute_game_ends_at_its_last_played_time() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    library(&fake);
    let mut collector = bench.collector(&fake);
    collector.tick().await.unwrap();
    fake.asked();
    let before = Utc::now() - TimeDelta::hours(1);
    bench.age_state(before);
    let ended = Utc::now() - TimeDelta::minutes(10);
    fake.set_playtime(ALPHA, 160, ended.timestamp());
    // GAMMA joué aussi : il n'a pas de succès, on ne les redemande pas.
    fake.set_playtime(GAMMA, 60, 0);
    fake.with(|d| {
        d.achievements.insert(
            ALPHA,
            with_achievements(
                ALPHA,
                vec![
                    achievement("ACH_ONE", Some(at("2025-03-30T12:01:24Z"))),
                    achievement("ACH_TWO", Some(at("2026-10-01T19:40:00Z"))),
                    achievement("ACH_LOCKED", Some(ended - TimeDelta::minutes(5))),
                ],
            ),
        );
    });
    let mut collector = bench.collector(&fake);
    let report = collector.tick().await.unwrap();
    assert_eq!(report.readings, 2);
    assert_eq!(report.achievements, 1, "seul le nouveau succès part");
    assert_eq!(fake.asked(), [ALPHA]);

    let reading = bench.reading(ALPHA, 160).await;
    assert_eq!(reading["previous"]["playtime_forever"], 120);
    assert_eq!(
        reading["previous"]["observed_at"],
        before.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    );
    let ended = DateTime::from_timestamp(ended.timestamp(), 0).unwrap();
    let plays = bench.plays().await;
    assert_eq!(plays.len(), 2, "{plays:?}");
    assert!(
        plays.contains(&(ended - TimeDelta::minutes(40), 40 * MINUTE)),
        "ALPHA finit à sa dernière partie : {plays:?}"
    );
    let today = paris_today(Utc::now());
    let yesterday = today - Days::new(1);
    assert_eq!(bench.total_minutes(yesterday, today).await, 45, "40 + 5");
    // Le total d'heures de chaque jeu suit le dernier relevé (le même jour, sauf juste après minuit).
    if paris_day(ended) == today {
        let card = bench.games(today).await;
        let alpha = card["games"]
            .as_array()
            .unwrap()
            .iter()
            .find(|g| g["key"].as_str() == Some(&*ALPHA.to_string()))
            .unwrap()
            .clone();
        assert_eq!(alpha["total_minutes"], 160, "{card}");
        assert_eq!(alpha["name"], name(ALPHA));
        assert_eq!(alpha["achievements"].as_array().unwrap().len(), 1, "{card}");
    }
}

#[tokio::test]
async fn a_game_across_midnight_is_split_between_the_two_days() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    library(&fake);
    bench.collector(&fake).tick().await.unwrap();
    let today = paris_today(Utc::now());
    let midnight = paris_midnight(today - Days::new(1));
    bench.age_state(midnight - TimeDelta::hours(3));
    // Une heure, de 23:20 à 00:20, finie à minuit vingt.
    fake.set_playtime(ALPHA, 180, (midnight + TimeDelta::minutes(20)).timestamp());
    bench.collector(&fake).tick().await.unwrap();
    assert_eq!(
        bench.plays().await,
        [(midnight - TimeDelta::minutes(40), 60 * MINUTE)]
    );
    let before = today - Days::new(2);
    let after = today - Days::new(1);
    assert_eq!(
        bench.minutes_by_day(before, after).await,
        [(before.to_string(), 40), (after.to_string(), 20)]
    );
    let card = bench.games(before).await;
    assert_eq!(card["minutes"], 40, "{card}");
    assert_eq!(card["games"][0]["first_time"], "23:20");
    assert_eq!(card["games"][0]["last_time"], "00:20");
    let card = bench.games(after).await;
    assert_eq!(card["minutes"], 20, "{card}");
    assert_eq!(
        card["segments"], 1,
        "la tranche de la veille touche ce jour"
    );
}

#[tokio::test]
async fn a_three_hour_gap_is_split_into_hours_back_to_back() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    library(&fake);
    bench.collector(&fake).tick().await.unwrap();
    bench.age_state(Utc::now() - TimeDelta::hours(4));
    // Sans dernière partie connue : la fin est l'heure du relevé.
    fake.set_playtime(GAMMA, 55 + 180, 0);
    let mut collector = bench.collector(&fake);
    collector.tick().await.unwrap();
    let observed = collector.last_pass_at().unwrap();
    let observed = DateTime::from_timestamp_millis(observed.timestamp_millis()).unwrap();
    let plays = bench.plays().await;
    assert_eq!(
        plays,
        [
            (observed - TimeDelta::hours(3), 60 * MINUTE),
            (observed - TimeDelta::hours(2), 60 * MINUTE),
            (observed - TimeDelta::hours(1), 60 * MINUTE),
        ]
    );
}

#[tokio::test]
async fn a_game_bought_between_two_passes_starts_from_zero_at_the_last_pass() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    library(&fake);
    bench.collector(&fake).tick().await.unwrap();
    bench.age_state(Utc::now() - TimeDelta::minutes(15));
    let mut collector = bench.collector(&fake);
    let last_pass = collector.last_pass_at().unwrap();
    fake.asked();
    fake.with(|d| d.games.push(game(DELTA, 25, Utc::now().timestamp())));
    let report = collector.tick().await.unwrap();
    assert_eq!(report.readings, 1);
    let reading = bench.reading(DELTA, 25).await;
    assert_eq!(
        reading["previous"],
        json!({"playtime_forever": 0,
               "observed_at": last_pass.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)})
    );
    assert_eq!(fake.asked(), [DELTA], "ses succès sont demandés");
    let plays = bench.plays().await;
    assert_eq!(plays.len(), 1);
    assert_eq!(plays[0].1, 25 * MINUTE);
}

#[tokio::test]
async fn a_lost_state_only_gives_duplicates_and_a_new_starting_point() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    library(&fake);
    bench.collector(&fake).tick().await.unwrap();
    std::fs::remove_file(bench.state_path()).unwrap();
    // Le renvoi des mêmes relevés et des mêmes succès : rien de neuf.
    let report = bench.collector(&fake).tick().await.unwrap();
    assert_eq!(
        (
            report.readings,
            report.achievements,
            report.accepted,
            report.duplicate
        ),
        (2, 2, 0, 4)
    );
    // Un compteur qui a bougé pendant la perte : un nouveau point de départ, sans temps compté.
    std::fs::remove_file(bench.state_path()).unwrap();
    fake.set_playtime(GAMMA, 70, 0);
    let report = bench.collector(&fake).tick().await.unwrap();
    assert_eq!((report.accepted, report.duplicate), (1, 3));
    assert_eq!(bench.reading(GAMMA, 70).await["previous"], Value::Null);
    assert_eq!(bench.events("game.play").await, 0);
    assert_eq!(bench.journal("steam.playtime").await, 3);
}

#[tokio::test]
async fn a_closed_profile_sends_the_readings_and_asks_to_open_it() {
    for status in [200, 403] {
        let bench = Bench::new().await;
        let fake = Fake::start().await;
        library(&fake);
        fake.with(|d| {
            d.achievements.insert(ALPHA, profile_not_public(status));
            d.achievements.insert(GAMMA, profile_not_public(status));
        });
        let mut collector = bench.collector(&fake);
        assert_eq!(collector.pass().await, Duration::from_secs(900));
        assert_eq!(
            fake.asked().len(),
            1,
            "la tournée s'arrête au premier refus"
        );
        assert_eq!(
            bench.journal("steam.playtime").await,
            2,
            "les relevés partent"
        );
        assert_eq!(bench.journal("steam.achievement").await, 0);
        let state = bench.status().await.unwrap();
        assert_eq!(
            (&state["state"], &state["reason"]),
            (&json!("action_required"), &json!("profile_private")),
            "{status}"
        );
        assert!(state["last_collected_at"].is_string());
        assert!(bench.last_seen().await.is_some());

        // Le profil ouvert : les succès en attente partent, sans changement de compteur.
        library(&fake);
        collector.pass().await;
        let mut asked = fake.asked();
        asked.sort_unstable();
        assert_eq!(asked, [ALPHA, GAMMA]);
        assert_eq!(bench.journal("steam.achievement").await, 2);
        assert_eq!(bench.status().await.unwrap()["state"], "ok");
        collector.pass().await;
        assert_eq!(fake.asked(), Vec::<u64>::new(), "plus rien en attente");
    }
}

#[tokio::test]
async fn a_hidden_library_asks_to_open_the_profile_and_sends_nothing() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    library(&fake);
    fake.with(|d| d.hide_library = true);
    let mut collector = bench.collector(&fake);
    assert_eq!(
        collector.tick().await.unwrap_err(),
        TickError::Service(ServiceError::ProfilePrivate)
    );
    collector.pass().await;
    let state = bench.status().await.unwrap();
    assert_eq!(state["reason"], "profile_private");
    assert_eq!(bench.journal("steam.playtime").await, 0);
    assert_eq!(collector.last_pass_at(), None);
}

#[tokio::test]
async fn a_rejected_key_sends_nothing_and_no_sign_of_life() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    library(&fake);
    for status in [401, 403] {
        fake.with(|d| d.owned_status = status);
        let mut collector = bench.collector(&fake);
        assert_eq!(
            collector.tick().await.unwrap_err(),
            TickError::Service(ServiceError::KeyRejected { status })
        );
        assert_eq!(collector.pass().await, Duration::from_secs(60));
        let state = bench.status().await.unwrap();
        assert_eq!(
            (&state["state"], &state["reason"], &state["expires_at"]),
            (
                &json!("action_required"),
                &json!("key_rejected"),
                &Value::Null
            )
        );
        assert_eq!(state["last_collected_at"], Value::Null);
    }
    assert_eq!(
        bench.last_seen().await,
        None,
        "le seuil de silence alertera"
    );
    assert_eq!(bench.journal("steam.playtime").await, 0);

    // La clé remplacée : tout repart, et l'état revient à `ok`.
    fake.with(|d| d.owned_status = 200);
    bench.collector(&fake).pass().await;
    let state = bench.status().await.unwrap();
    assert_eq!(state["state"], "ok");
    assert!(state["last_collected_at"].is_string());
    assert_eq!(bench.journal("steam.playtime").await, 2);
}

#[tokio::test]
async fn a_rate_limit_waits_and_the_achievements_resume_at_the_next_pass() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    library(&fake);
    fake.with(|d| d.owned_status = 429);
    let mut collector = bench.collector(&fake);
    assert_eq!(
        collector.tick().await.unwrap_err(),
        TickError::Service(ServiceError::RateLimited {
            retry_after: Duration::from_secs(120)
        })
    );
    assert_eq!(collector.pass().await, Duration::from_secs(120));
    let state = bench.status().await.unwrap();
    assert_eq!(
        (&state["state"], &state["reason"]),
        (&json!("error"), &json!("rate_limited"))
    );

    // Une limite au milieu des succès : les relevés sont partis, les succès attendent.
    fake.with(|d| {
        d.owned_status = 200;
        d.achievements.insert(ALPHA, (429, Value::Null));
    });
    assert!(matches!(
        collector.tick().await.unwrap_err(),
        TickError::Service(ServiceError::RateLimited { .. })
    ));
    assert_eq!(bench.journal("steam.playtime").await, 2);
    library(&fake);
    fake.asked();
    collector.tick().await.unwrap();
    let mut asked = fake.asked();
    asked.sort_unstable();
    assert_eq!(asked, [ALPHA, GAMMA], "les jeux encore en attente");
    assert_eq!(bench.journal("steam.achievement").await, 2);
}

#[tokio::test]
async fn a_game_steam_refuses_does_not_block_the_pass_and_a_game_without_stats_is_never_asked() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    library(&fake);
    fake.with(|d| {
        // Un jeu sans statistiques : pas de demande de succès.
        let mut silent = game(DELTA, 30, at("2026-10-02T20:00:00Z").timestamp());
        silent["has_community_visible_stats"] = json!(false);
        d.games.push(silent);
        // Steam répond une erreur 500 pour les succès d'un jeu (vu en production le 7 nov. 2026).
        d.achievements.insert(
            GAMMA,
            (
                500,
                json!({"playerstats": {"error": "Unknown error", "success": false}}),
            ),
        );
    });
    let mut collector = bench.collector(&fake);
    for _ in 0..3 {
        let report = collector.tick().await.unwrap();
        assert_eq!(
            report.achievement_failures, 1,
            "le passage aboutit malgré le refus"
        );
    }
    let asked = fake.asked();
    assert!(!asked.contains(&DELTA), "jamais demandé : {asked:?}");
    assert_eq!(asked.iter().filter(|a| **a == GAMMA).count(), 3);
    assert_eq!(bench.journal("steam.playtime").await, 3);
    assert_eq!(bench.journal("steam.achievement").await, 2);
    // Après trois refus d'affilée, le jeu n'est plus demandé.
    let report = collector.tick().await.unwrap();
    assert_eq!(report.achievement_failures, 0);
    assert!(!fake.asked().contains(&GAMMA));
    collector.pass().await;
    let state = bench.status().await.unwrap();
    assert_eq!(state["state"], json!("ok"));
}

#[tokio::test]
async fn a_rebuild_gives_no_difference() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    library(&fake);
    bench.collector(&fake).tick().await.unwrap();
    bench.age_state(Utc::now() - TimeDelta::hours(3));
    fake.set_playtime(ALPHA, 270, 0);
    fake.set_playtime(GAMMA, 80, (Utc::now() - TimeDelta::hours(2)).timestamp());
    bench.collector(&fake).tick().await.unwrap();
    let today = paris_today(Utc::now());
    let days = [today - Days::new(1), today];
    let mut before = Vec::new();
    for day in days {
        before.push(bench.games(day).await);
    }
    assert_eq!(
        bench.events("game.play").await,
        4,
        "150 min en trois, 25 en une"
    );
    let rebuilt = replay::rebuild(&bench.db.admin, INTERPRETERS, false)
        .await
        .unwrap();
    assert!(rebuilt.switched);
    assert!(rebuilt.comparison.differences.is_empty());
    for (day, before) in days.iter().zip(before) {
        assert_eq!(bench.games(*day).await, before);
    }
}

// ─── La carte du jour et les lectures génériques ─────────────────────────────

/// Les relevés et les succès envoyés tels quels, à dates fixes, avec le token de la source.
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

#[tokio::test]
async fn the_day_card_gives_each_game_its_minutes_its_total_and_its_achievements() {
    let bench = Bench::new().await;
    let fixture = read_json(&repo_path("contract/fixtures/valid/steam.json"));
    send(&bench, fixture["records"].as_array().unwrap().clone()).await;
    // Un autre jeu, les jours d'avant : un point de départ le 2 nov., un succès le 3 sans partie ce
    // jour-là, puis une heure de jeu le 4, qui ne compte pas dans le total du 3.
    let other = OwnedGame {
        appid: 777,
        name: Some("Autre Jeu".to_owned()),
        playtime_forever: 300,
        playtime_disconnected: 0,
        rtime_last_played: None,
        stats: true,
    };
    let mut skipped = 0;
    let mut records: Vec<Value> = steam::unlocked(
        777,
        Some("Autre Jeu"),
        &[achievement("BOSS", Some(at("2026-11-03T09:30:00Z")))],
        &mut skipped,
    )
    .into_iter()
    .map(|u| u.record)
    .collect();
    records.push(steam::playtime_record(
        &other,
        None,
        at("2026-11-02T12:00:00Z"),
    ));
    let later = OwnedGame {
        playtime_forever: 360,
        ..other.clone()
    };
    records.push(steam::playtime_record(
        &later,
        Some(Previous {
            playtime_forever: 300,
            observed_at: at("2026-11-02T12:00:00Z"),
        }),
        at("2026-11-04T12:00:00Z"),
    ));
    send(&bench, records).await;
    assert_eq!(
        bench
            .db
            .count("SELECT count(*) FROM derived.interpretation_errors")
            .await,
        0
    );

    let card = bench
        .games(NaiveDate::from_ymd_opt(2026, 11, 6).unwrap())
        .await;
    assert_eq!(
        card,
        json!({
            "date": "2026-11-06", "minutes": 40, "segments": 1, "achievements": 1,
            "discoveries": [], "matches": [],
            "games": [{
                "key": "4242", "name": "Jeu Fictif", "minutes": 40,
                "first_time": "21:15", "last_time": "21:55", "total_minutes": 160,
                "achievements": [{"key": "4242:ACH_FICTIF", "name": "Succès fictif", "time": "21:50"}],
            }],
        })
    );
    let card = bench
        .games(NaiveDate::from_ymd_opt(2026, 11, 3).unwrap())
        .await;
    assert_eq!(
        card,
        json!({
            "date": "2026-11-03", "minutes": 0, "segments": 0, "achievements": 1,
            "discoveries": [], "matches": [],
            "games": [{
                "key": "777", "name": "Autre Jeu", "minutes": 0,
                "first_time": null, "last_time": null, "total_minutes": 300,
                "achievements": [{"key": "777:BOSS", "name": "Succès BOSS", "time": "10:30"}],
            }],
        }),
        "un succès sans partie ce jour-là, avec le total à la fin du jour"
    );
    let card = bench
        .games(NaiveDate::from_ymd_opt(2026, 11, 4).unwrap())
        .await;
    assert_eq!(card["minutes"], 60);
    assert_eq!(card["games"][0]["total_minutes"], 360);
    let empty = bench
        .games(NaiveDate::from_ymd_opt(2026, 11, 20).unwrap())
        .await;
    assert_eq!(empty["games"], json!([]));
    assert_eq!(empty["minutes"], 0);

    // Le calendrier : les minutes du jour, pas les succès (sans valeur du jour, ils ne se lisent que
    // dans la carte et par les lectures génériques) ; avant la première donnée, rien, pas zéro.
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
    assert_eq!(values("2026-11-06")["game.play"], 40.0);
    assert!(
        month["metrics"]
            .as_array()
            .unwrap()
            .iter()
            .all(|m| m["metric"] != "game.achievement"),
        "{}",
        month["metrics"]
    );
    assert_eq!(values("2026-11-06").get("game.achievement"), None);
    assert_eq!(values("2026-11-03").get("game.achievement"), None);
    assert_eq!(values("2026-11-04")["game.play"], 60.0);
    assert_eq!(
        values("2026-11-05")["game.play"],
        0.0,
        "après la première donnée"
    );
    assert_eq!(values("2026-11-01")["game.play"], Value::Null);

    // Le MCP : les minutes et le classement par jeu, les seuls détails partagés, pas la carte.
    let (status, top) = bench
        .as_mcp("/v1/top?metric=game.play&level=game&from=2026-11-01&to=2026-11-30")
        .await;
    assert_eq!(status, 200, "{top}");
    assert_eq!(top["items"][0]["key"], "777");
    assert_eq!(top["items"][0]["minutes"], 60);
    assert_eq!(top["items"][1]["name"], "Jeu Fictif");
    let (_, occurrences) = bench
        .as_mcp("/v1/occurrences?metric=game.play&from=2026-11-06&to=2026-11-06&form=list")
        .await;
    assert_eq!(
        occurrences["occurrences"][0]["details"],
        json!({"played_ms": 40 * MINUTE})
    );
    let (status, minutes) = bench
        .as_mcp("/v1/durations?metric=game.play&from=2026-11-01&to=2026-11-30")
        .await;
    assert_eq!(status, 200);
    assert_eq!(minutes["total_minutes"], 100);
    assert_eq!(bench.as_mcp("/v1/days/2026-11-06/games").await.0, 401);
    // Les succès restent lisibles par les lectures génériques.
    let (status, unlocked) = bench
        .as_mcp("/v1/occurrences?metric=game.achievement&from=2026-11-01&to=2026-11-30&form=list")
        .await;
    assert_eq!(status, 200, "{unlocked}");
    assert_eq!(unlocked["occurrences"].as_array().unwrap().len(), 2);
}

/// Un relevé d'un jeu fictif, avec son précédent s'il y en a un.
fn reading(appid: u64, minutes: i64, at_: &str, previous: Option<(i64, &str)>) -> Value {
    let owned = OwnedGame {
        appid,
        name: Some(format!("Jeu {appid}")),
        playtime_forever: minutes,
        playtime_disconnected: 0,
        rtime_last_played: None,
        stats: true,
    };
    steam::playtime_record(
        &owned,
        previous.map(|(playtime_forever, observed_at)| Previous {
            playtime_forever,
            observed_at: at(observed_at),
        }),
        at(at_),
    )
}

#[tokio::test]
async fn only_a_first_play_after_tracking_started_is_a_discovery() {
    let bench = Bench::new().await;
    send(
        &bench,
        vec![
            // 4242 : déjà 120 min au premier relevé, joué avant le suivi ; une partie le 5 nov.
            reading(4242, 120, "2026-11-04T10:00:00Z", None),
            reading(
                4242,
                150,
                "2026-11-05T19:00:00Z",
                Some((120, "2026-11-04T10:00:00Z")),
            ),
            // 777 : jamais lancé au premier relevé, joué pour la première fois le 5, puis le 6.
            reading(777, 0, "2026-11-04T10:00:00Z", None),
            reading(
                777,
                30,
                "2026-11-05T20:00:00Z",
                Some((0, "2026-11-04T10:00:00Z")),
            ),
            reading(
                777,
                50,
                "2026-11-06T20:00:00Z",
                Some((30, "2026-11-05T20:00:00Z")),
            ),
            // 888 : acheté après le début du suivi, il part de zéro au passage d'avant.
            reading(
                888,
                25,
                "2026-11-05T21:00:00Z",
                Some((0, "2026-11-05T20:45:00Z")),
            ),
            // 999 : jamais lancé, puis l'état du connecteur est perdu pendant qu'on y joue : un nouveau
            // point de départ, déjà à 40 min, sans temps compté ; la partie suivante n'est pas une
            // découverte (on ne sait plus quand elle a eu lieu).
            reading(999, 0, "2026-11-04T10:00:00Z", None),
            reading(999, 40, "2026-11-05T09:00:00Z", None),
            reading(
                999,
                70,
                "2026-11-05T22:00:00Z",
                Some((40, "2026-11-05T09:00:00Z")),
            ),
        ],
    )
    .await;
    let day = |d: u32| NaiveDate::from_ymd_opt(2026, 11, d).unwrap();
    let discovered = |card: &Value| {
        card["discoveries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| {
                (
                    d["key"].as_str().unwrap().to_owned(),
                    d["name"].as_str().unwrap().to_owned(),
                )
            })
            .collect::<Vec<_>>()
    };
    let card = bench.games(day(5)).await;
    assert_eq!(card["games"].as_array().unwrap().len(), 4, "{card}");
    assert_eq!(
        discovered(&card),
        [
            ("777".to_owned(), "Jeu 777".to_owned()),
            ("888".to_owned(), "Jeu 888".to_owned())
        ],
        "par première heure"
    );
    assert_eq!(card["discoveries"][0]["level"], "game");
    assert_eq!(discovered(&bench.games(day(6)).await), []);
    assert_eq!(discovered(&bench.games(day(4)).await), []);

    // Le MCP lit les mêmes découvertes, et compte les jeux déjà connus à part.
    let (status, found) = bench
        .as_mcp("/v1/discoveries?metric=game.play&level=game&from=2026-11-01&to=2026-11-30")
        .await;
    assert_eq!(status, 200, "{found}");
    let keys: Vec<&str> = found["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["key"].as_str().unwrap())
        .collect();
    assert_eq!(keys, ["777", "888"]);
    assert_eq!(found["marked_known"], 2, "4242 et 999");
}

// ─── Le vrai binaire : ce qui sort dans les logs ──────────────────────────────

fn collect_process(fake: &Fake, bench: &Bench) -> (std::process::Child, PathBuf) {
    let log = bench.state_dir.join("collector.log");
    let file = std::fs::File::create(&log).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_life-tracker"))
        .arg("collect")
        .env_clear()
        .env("RUST_LOG", "trace")
        .env("STEAM_API_URL", &fake.url)
        .env("STEAM_API_KEY", KEY)
        .env("STEAM_ID", STEAM_ID)
        .env("STEAM_TOKEN", &bench.token)
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
async fn logs_never_cite_the_key_the_account_or_a_game() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    library(&fake);
    let (child, log) = collect_process(&fake, &bench);
    let deadline = Instant::now() + Duration::from_secs(120);
    let status = loop {
        if let Some(status) = bench.status().await {
            break status;
        }
        assert!(Instant::now() < deadline, "aucun état rapporté");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(status["state"], "ok");
    let logs = logs_until(child, &log, "steam pass completed").await;
    assert!(logs.contains("\"accepted\":4"), "{logs}");
    for secret in [
        KEY,
        STEAM_ID,
        &bench.token,
        "Jeu Sentinelle",
        "Succès ACH",
        &ALPHA.to_string(),
    ] {
        assert!(!logs.contains(secret), "fuite de {secret} : {logs}");
    }
    assert!(bench.state_path().exists());
    assert!(
        !bench.state_dir.join("state.json").exists(),
        "Last.fm n'est pas configuré"
    );
    let state = std::fs::read_to_string(bench.state_path()).unwrap();
    assert!(!state.contains(KEY) && !state.contains(STEAM_ID));

    fake.with(|d| d.owned_status = 403);
    let (child, log) = collect_process(&fake, &bench);
    let logs = logs_until(child, &log, "clé Steam refusée").await;
    assert!(!logs.contains(KEY) && !logs.contains(STEAM_ID), "{logs}");
}

#[test]
fn what_the_connector_sends_respects_the_contract() {
    let schema = validator(None);
    let game = steam::owned_game(&game(ALPHA, 120, 1_700_000_000)).unwrap();
    let mut records = vec![
        steam::playtime_record(&game, None, Utc::now()),
        steam::playtime_record(
            &game,
            Some(Previous {
                playtime_forever: 60,
                observed_at: Utc::now() - TimeDelta::minutes(15),
            }),
            Utc::now(),
        ),
    ];
    let mut skipped = 0;
    records.extend(
        steam::unlocked(
            ALPHA,
            Some(&name(ALPHA)),
            &[achievement("ACH_ONE", Some(at("2025-03-30T12:01:24Z")))],
            &mut skipped,
        )
        .into_iter()
        .map(|u| u.record),
    );
    let batch = json!({ "records": records });
    let errors: Vec<_> = schema.iter_errors(&batch).map(|e| e.to_string()).collect();
    assert!(errors.is_empty(), "{errors:?}");
}
