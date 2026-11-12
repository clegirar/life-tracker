//! Le collecteur contre un Last.fm simulé et la vraie API d'ingestion, sans aucun réseau extérieur.
//!
//! Les fixtures de `tests/fixtures/lastfm/` sont RECONSTITUÉES d'après la documentation de Last.fm
//! (user.getRecentTracks, track.getInfo, codes d'erreur), pas capturées : aucune clé n'était
//! disponible. Noms fictifs. À confronter à de vraies réponses lors du premier passage sur le NAS.

mod common;
use axum::Router;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get};
use common::*;
use life_tracker::collector::lastfm::ServiceError;
use life_tracker::collector::{
    Collector, Config, TickError, TickReport, scrobble_records, track_info_record,
};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn fixture(name: &str) -> Value {
    read_json(&repo_path(&format!(
        "backend/tests/fixtures/lastfm/{name}.json"
    )))
}

type Params = HashMap<String, String>;
/// Une réponse scriptée : statut, délai demandé, corps.
type Reply = (u16, Option<&'static str>, String);
type Script = Box<dyn FnMut(&Params) -> Reply + Send>;

fn ok(name: &str) -> Reply {
    (200, None, fixture(name).to_string())
}

/// Un Last.fm simulé : note chaque requête et répond selon le script.
#[derive(Clone)]
struct Mock {
    url: String,
    requests: Arc<Mutex<Vec<Params>>>,
    script: Arc<Mutex<Script>>,
}

async fn mock_reply(State(mock): State<Mock>, Query(params): Query<Params>) -> Response {
    mock.requests.lock().unwrap().push(params.clone());
    let (status, retry_after, body) = (mock.script.lock().unwrap())(&params);
    let mut response = (StatusCode::from_u16(status).unwrap(), body).into_response();
    if let Some(value) = retry_after {
        response
            .headers_mut()
            .insert("retry-after", value.parse().unwrap());
    }
    response
}

impl Mock {
    async fn start(script: impl FnMut(&Params) -> Reply + Send + 'static) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mock = Self {
            url: format!("http://{}/2.0/", listener.local_addr().unwrap()),
            requests: Arc::default(),
            script: Arc::new(Mutex::new(Box::new(script))),
        };
        let app = Router::new()
            .route("/2.0/", get(mock_reply))
            .with_state(mock.clone());
        tokio::spawn(async move { axum::serve(listener, app).await });
        mock
    }

    fn set(&self, script: impl FnMut(&Params) -> Reply + Send + 'static) {
        *self.script.lock().unwrap() = Box::new(script);
    }

    fn calls(&self, method: &str) -> Vec<Params> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|p| p["method"] == method)
            .cloned()
            .collect()
    }

    fn pages(&self) -> Vec<String> {
        self.calls("user.getrecenttracks")
            .iter()
            .map(|p| p["page"].clone())
            .collect()
    }
}

fn is_recent(params: &Params) -> bool {
    params["method"] == "user.getrecenttracks"
}

/// Une liste selon la page demandée, et une fiche pour chaque titre.
fn two_pages(params: &Params) -> Reply {
    if !is_recent(params) {
        return ok("track_info");
    }
    match params["page"].as_str() {
        "1" => ok("recent_page_1_of_2"),
        "2" => ok("recent_page_2_of_2"),
        other => panic!("page inattendue {other}"),
    }
}

fn one_page(params: &Params) -> Reply {
    if is_recent(params) {
        ok("recent_nowplaying")
    } else {
        ok("track_info")
    }
}

struct Bench {
    db: TestDb,
    base: String,
    token: String,
    source_id: i16,
    state_dir: tempdir::Dir,
}

/// Un dossier d'état jetable, sans dépendance de plus.
mod tempdir {
    pub struct Dir(pub std::path::PathBuf);
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

impl Bench {
    async fn new() -> Self {
        let db = TestDb::new().await;
        let (source_id, token) = db
            .source("lastfm", &["lastfm.scrobble", "lastfm.track_info"])
            .await;
        let base = serve(db.app.clone()).await;
        let dir = std::env::temp_dir().join(unique_name());
        std::fs::create_dir_all(&dir).unwrap();
        Self {
            db,
            base,
            token,
            source_id,
            state_dir: tempdir::Dir(dir),
        }
    }

    fn state_path(&self) -> PathBuf {
        self.state_dir.0.clone()
    }

    fn collector_to(&self, mock: &Mock, ingest_url: &str) -> Collector {
        Collector::new(Config {
            lastfm_url: mock.url.clone(),
            api_key: "cle-fictive".to_owned(),
            user: "utilisateur-fictif".to_owned(),
            ingest_url: ingest_url.to_owned(),
            token: self.token.clone(),
            state_dir: self.state_path(),
            interval: Duration::from_secs(300),
            pause: Duration::ZERO,
        })
        .unwrap()
    }

    fn collector(&self, mock: &Mock) -> Collector {
        self.collector_to(mock, &self.base)
    }

    async fn journal(&self, kind: &str) -> i64 {
        self.db
            .count(&format!(
                "SELECT count(*) FROM core.records WHERE kind = '{kind}'"
            ))
            .await
    }

    async fn occurred(&self) -> Vec<i64> {
        sqlx::query_scalar(
            "SELECT extract(epoch FROM occurred_at)::bigint FROM core.records
             WHERE kind = 'lastfm.scrobble' ORDER BY seq",
        )
        .fetch_all(&self.db.app)
        .await
        .unwrap()
    }

    async fn last_seen(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        sqlx::query_scalar("SELECT last_seen_at FROM core.sources WHERE id = $1")
            .bind(self.source_id)
            .fetch_one(&self.db.app)
            .await
            .unwrap()
    }

    /// Le dernier état rapporté pour la page d'état.
    async fn status(&self) -> Option<Value> {
        let text: Option<String> =
            sqlx::query_scalar("SELECT status::text FROM core.sources WHERE id = $1")
                .bind(self.source_id)
                .fetch_one(&self.db.app)
                .await
                .unwrap();
        text.map(|t| serde_json::from_str(&t).unwrap())
    }
}

/// `next_run_at` : maintenant plus le délai, à quelques secondes près.
fn assert_next_run(status: &Value, delay: Duration) {
    let next: chrono::DateTime<chrono::Utc> =
        status["next_run_at"].as_str().unwrap().parse().unwrap();
    let expected = chrono::Utc::now() + chrono::TimeDelta::from_std(delay).unwrap();
    assert!(
        (next - expected).num_seconds().abs() < 10,
        "{status} au lieu de {expected}"
    );
}

#[tokio::test]
async fn a_page_keeps_the_raw_scrobbles_and_skips_the_track_playing_now() {
    let bench = Bench::new().await;
    let mock = Mock::start(one_page).await;
    let report = bench.collector(&mock).tick().await.unwrap();
    assert_eq!(
        (report.accepted, report.duplicate, report.rejected),
        (2, 0, 0)
    );
    assert_eq!(bench.journal("lastfm.scrobble").await, 2);

    let row: (String, String, Option<i16>) = sqlx::query_as(
        "SELECT payload::text, to_char(occurred_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI'), utc_offset_minutes
         FROM core.records WHERE kind = 'lastfm.scrobble' AND occurred_at = to_timestamp(1792306800)",
    )
    .fetch_one(&bench.db.app)
    .await
    .unwrap();
    let stored: Value = serde_json::from_str(&row.0).unwrap();
    let raw = &fixture("recent_nowplaying")["recenttracks"]["track"][2];
    assert_eq!(&stored, raw, "le brut, entier");
    assert_eq!(row.1, "2026-10-18 07:00");
    assert_eq!(row.2, None, "Last.fm ne donne pas de décalage");

    let call = &mock.calls("user.getrecenttracks")[0];
    assert_eq!(call["limit"], "200");
    assert_eq!(call["format"], "json");
    assert_eq!(call["api_key"], "cle-fictive");
    assert_eq!(call["user"], "utilisateur-fictif");
    assert!(
        !call.contains_key("from"),
        "aucun curseur au premier passage"
    );
}

#[tokio::test]
async fn a_single_track_object_and_empty_lists_are_understood() {
    let bench = Bench::new().await;
    let mock = Mock::start(|p| {
        if is_recent(p) {
            ok("recent_single")
        } else {
            ok("track_info")
        }
    })
    .await;
    let report = bench.collector(&mock).tick().await.unwrap();
    assert_eq!(report.accepted, 1);

    for empty in ["recent_empty", "recent_empty_text"] {
        let bench = Bench::new().await;
        let mock = Mock::start(move |_| ok(empty)).await;
        let report = bench.collector(&mock).tick().await.unwrap();
        assert_eq!(report.accepted, 0, "{empty}");
        assert!(
            bench.last_seen().await.is_some(),
            "une liste vide est un succès"
        );
    }
}

#[tokio::test]
async fn pages_are_read_from_the_oldest_with_a_closed_window() {
    let bench = Bench::new().await;
    let mock = Mock::start(two_pages).await;
    let mut collector = bench.collector(&mock);
    let before = chrono::Utc::now().timestamp();
    let report = collector.tick().await.unwrap();
    assert_eq!(report.pages, 2);
    assert_eq!(
        mock.pages(),
        ["1", "2", "1"],
        "le total, puis du plus ancien au plus récent"
    );
    assert_eq!(
        bench.occurred().await[0],
        1792220400,
        "le plus ancien entre le premier"
    );

    // La fenêtre est la même pour toute la pagination, et le curseur va jusqu'à sa fin.
    let calls = mock.calls("user.getrecenttracks");
    assert!(calls.iter().all(|c| c["to"] == calls[0]["to"]));
    let to: i64 = calls[0]["to"].parse().unwrap();
    assert!(to >= before);
    assert_eq!(collector.cursor(), Some(to));
}

#[tokio::test]
async fn the_next_pass_rereads_48_hours_and_sends_nothing_again() {
    let bench = Bench::new().await;
    let mock = Mock::start(one_page).await;
    let mut collector = bench.collector(&mock);
    collector.tick().await.unwrap();
    let cursor = collector.cursor().unwrap();
    let report = collector.tick().await.unwrap();
    assert_eq!(
        report,
        TickReport {
            pages: 1,
            ..Default::default()
        }
    );
    let calls = mock.calls("user.getrecenttracks");
    assert_eq!(calls[1]["from"], (cursor - 48 * 3600).to_string());
    assert_eq!(bench.journal("lastfm.scrobble").await, 2);
}

#[tokio::test]
async fn a_late_scrobble_older_than_the_cursor_still_enters() {
    let bench = Bench::new().await;
    let mock = Mock::start(|p| {
        if is_recent(p) {
            ok("recent_single")
        } else {
            ok("track_info")
        }
    })
    .await;
    let mut collector = bench.collector(&mock);
    collector.tick().await.unwrap();
    assert_eq!(bench.journal("lastfm.scrobble").await, 1);
    // Le scrobbler rattrape : une écoute datée d'avant le curseur apparaît dans la fenêtre relue.
    mock.set(one_page);
    assert_eq!(collector.tick().await.unwrap().accepted, 2);
    assert_eq!(bench.journal("lastfm.scrobble").await, 3);
}

#[tokio::test]
async fn an_interrupted_sync_resumes_without_any_duplicate() {
    let bench = Bench::new().await;
    // Coupure à la dernière lecture : la page la plus ancienne est reçue, pas la plus récente.
    let calls = Arc::new(Mutex::new(0));
    let counter = calls.clone();
    let mock = Mock::start(move |p| {
        if is_recent(p) {
            *counter.lock().unwrap() += 1;
            if *counter.lock().unwrap() == 3 {
                return (503, None, "indisponible".to_owned());
            }
        }
        two_pages(p)
    })
    .await;
    let mut collector = bench.collector(&mock);
    let failure = collector.tick().await.unwrap_err();
    assert_eq!(
        failure,
        TickError::Service(ServiceError::Failed {
            status: 503,
            code: None
        })
    );
    assert_eq!(bench.occurred().await, [1792220400]);
    assert_eq!(
        collector.cursor(),
        Some(1792220400),
        "le curseur reste au dernier accusé"
    );

    // Le processus disparaît : tout repart du fichier d'état, comme après un redémarrage.
    drop(collector);
    mock.set(two_pages);
    let mut resumed = bench.collector(&mock);
    assert_eq!(resumed.cursor(), Some(1792220400));
    let report = resumed.tick().await.unwrap();
    assert_eq!((report.accepted, report.duplicate), (2, 1));
    assert_eq!(bench.occurred().await, [1792220400, 1792306800, 1792310400]);
    let from = mock.calls("user.getrecenttracks").last().unwrap()["from"].clone();
    assert_eq!(from, (1792220400 - 48 * 3600).to_string());
}

#[tokio::test]
async fn a_lost_state_only_gives_duplicates() {
    let bench = Bench::new().await;
    let mock = Mock::start(two_pages).await;
    bench.collector(&mock).tick().await.unwrap();
    let scrobbles = bench.journal("lastfm.scrobble").await;
    let infos = bench.journal("lastfm.track_info").await;

    std::fs::remove_file(bench.state_path().join("state.json")).unwrap();
    let report = bench.collector(&mock).tick().await.unwrap();
    assert_eq!(report.accepted, 0);
    assert_eq!(report.duplicate, scrobbles as u32);
    assert_eq!(bench.journal("lastfm.scrobble").await, scrobbles);
    assert_eq!(bench.journal("lastfm.track_info").await, infos);

    // Un fichier illisible vaut un état perdu.
    std::fs::write(bench.state_path().join("state.json"), "{pas du json").unwrap();
    assert_eq!(bench.collector(&mock).cursor(), None);
}

#[tokio::test]
async fn late_listens_that_shift_the_pages_stop_at_the_last_safe_point() {
    let bench = Bench::new().await;
    let mut reads = 0;
    let mock = Mock::start(move |p| {
        if !is_recent(p) {
            return ok("track_info");
        }
        reads += 1;
        match reads {
            1 => ok("recent_page_1_of_2"),
            2 => ok("recent_page_2_of_2"),
            // Le total passe de 3 à 4 entre deux pages.
            _ => {
                let mut page = fixture("recent_page_1_of_2");
                page["recenttracks"]["@attr"]["total"] = json!("4");
                (200, None, page.to_string())
            }
        }
    })
    .await;
    let mut collector = bench.collector(&mock);
    let report = collector.tick().await.unwrap();
    assert!(report.interrupted);
    assert_eq!(report.accepted, 1);
    assert_eq!(
        collector.cursor(),
        Some(1792220400),
        "pas jusqu'à la fin de la fenêtre"
    );
}

#[tokio::test]
async fn a_rate_limit_is_respected_and_moves_nothing() {
    let bench = Bench::new().await;
    for (status, retry_after, expected) in [
        (429, Some("120"), 120),
        (429, None, 300),
        (200, None, 300), // Last.fm peut aussi répondre 200 avec le code 29.
    ] {
        let mock =
            Mock::start(move |_| (status, retry_after, fixture("error_rate_limit").to_string()))
                .await;
        let mut collector = bench.collector(&mock);
        assert_eq!(
            collector.tick().await.unwrap_err(),
            TickError::Service(ServiceError::RateLimited {
                retry_after: Duration::from_secs(expected)
            })
        );
        assert_eq!(collector.cursor(), None);
    }
    assert_eq!(bench.journal("lastfm.scrobble").await, 0);
    assert_eq!(
        bench.last_seen().await,
        None,
        "aucun signal de vie sans succès"
    );
}

#[tokio::test]
async fn no_sign_of_life_until_lastfm_has_answered() {
    let bench = Bench::new().await;
    let mock = Mock::start(|_| (200, None, fixture("error_offline").to_string())).await;
    let failure = bench.collector(&mock).tick().await.unwrap_err();
    assert_eq!(
        failure,
        TickError::Service(ServiceError::Failed {
            status: 200,
            code: Some(11)
        })
    );
    assert_eq!(bench.last_seen().await, None);

    for garbage in [
        (502, "<html>Bad Gateway</html>"),
        (200, "<html>?</html>"),
        (200, "{}"),
    ] {
        let mock = Mock::start(move |_| (garbage.0, None, garbage.1.to_owned())).await;
        assert!(bench.collector(&mock).tick().await.is_err(), "{garbage:?}");
        assert_eq!(bench.last_seen().await, None, "{garbage:?}");
    }

    // Service injoignable : une adresse où plus rien n'écoute.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    drop(listener);
    let mut gone = Mock::start(|_| ok("recent_empty")).await;
    gone.url = url;
    assert_eq!(
        bench.collector(&gone).tick().await.unwrap_err(),
        TickError::Service(ServiceError::Unavailable { reason: "connect" })
    );
    assert_eq!(bench.last_seen().await, None);

    let mock = Mock::start(|_| ok("recent_empty")).await;
    bench.collector(&mock).tick().await.unwrap();
    assert!(bench.last_seen().await.is_some());
}

#[tokio::test]
async fn each_pass_reports_its_status_which_is_never_a_sign_of_life() {
    let bench = Bench::new().await;
    let invalid_key =
        r#"{"error":10,"message":"Invalid API key - You must be granted a valid key by last.fm"}"#;
    let mock = Mock::start(move |_| (403, None, invalid_key.to_owned())).await;
    let mut collector = bench.collector(&mock);
    let delay = collector.pass().await;
    assert_eq!(delay, Duration::from_secs(60));
    let status = bench.status().await.unwrap();
    assert_eq!(status["state"], "action_required");
    assert_eq!(status["reason"], "key_rejected");
    assert_eq!(status["expires_at"], Value::Null);
    assert_next_run(&status, delay);
    assert_eq!(
        bench.last_seen().await,
        None,
        "le seuil de silence alertera"
    );

    mock.set(|_| (429, Some("90"), fixture("error_rate_limit").to_string()));
    let delay = collector.pass().await;
    assert_eq!(delay, Duration::from_secs(90));
    let status = bench.status().await.unwrap();
    assert_eq!(
        (&status["state"], &status["reason"]),
        (&json!("error"), &json!("rate_limited"))
    );
    assert_next_run(&status, delay);

    mock.set(|_| (200, None, fixture("error_offline").to_string()));
    collector.pass().await;
    let status = bench.status().await.unwrap();
    assert_eq!(status["reason"], "service_unavailable");

    mock.set(one_page);
    let delay = collector.pass().await;
    assert_eq!(delay, Duration::from_secs(300));
    let status = bench.status().await.unwrap();
    assert_eq!(status["state"], "ok");
    assert_eq!(status["reason"], Value::Null);
    assert_next_run(&status, delay);
    assert!(bench.last_seen().await.is_some());
    assert_eq!(bench.journal("lastfm.scrobble").await, 2);
}

/// Une API qui refuse les rapports d'état, et rien d'autre : la passe n'en voit rien.
#[tokio::test]
async fn a_refused_status_report_never_fails_the_pass() {
    let bench = Bench::new().await;
    let mock = Mock::start(one_page).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = life_tracker::api::router(bench.db.app.clone()).layer(axum::middleware::from_fn(
        |request: axum::extract::Request, next: axum::middleware::Next| async move {
            if request.uri().path() == "/v1/sources/me/status" {
                return StatusCode::SERVICE_UNAVAILABLE.into_response();
            }
            next.run(request).await
        },
    ));
    tokio::spawn(async move { axum::serve(listener, app).await });
    let mut collector = bench.collector_to(&mock, &base);
    assert_eq!(collector.pass().await, Duration::from_secs(300));
    assert_eq!(bench.journal("lastfm.scrobble").await, 2);
    assert!(bench.last_seen().await.is_some());
    assert_eq!(bench.status().await, None);
    assert_eq!(
        collector.pass().await,
        Duration::from_secs(300),
        "pas d'échec compté"
    );
}

#[tokio::test]
async fn an_ingestion_failure_keeps_the_cursor_and_the_batch_is_sent_again() {
    let bench = Bench::new().await;
    let mock = Mock::start(one_page).await;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let broken = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new().route(
        "/{*path}",
        any(|| async { StatusCode::SERVICE_UNAVAILABLE }),
    );
    tokio::spawn(async move { axum::serve(listener, app).await });
    let mut collector = bench.collector_to(&mock, &broken);
    assert_eq!(
        collector.tick().await.unwrap_err(),
        TickError::Ingest {
            reason: "status",
            status: Some(503)
        }
    );
    assert_eq!(collector.cursor(), None);
    assert_eq!(mock.calls("track.getInfo").len(), 0);

    let report = bench.collector(&mock).tick().await.unwrap();
    assert_eq!(report.accepted, 2);
}

#[tokio::test]
async fn a_refused_token_is_a_failure_not_a_success() {
    let bench = Bench::new().await;
    let mock = Mock::start(one_page).await;
    let mut collector = Bench {
        token: "mauvais".to_owned(),
        ..Bench::new().await
    }
    .collector_to(&mock, &bench.base);
    assert_eq!(
        collector.tick().await.unwrap_err(),
        TickError::Ingest {
            reason: "status",
            status: Some(401)
        }
    );
    assert_eq!(collector.cursor(), None);
}

/// Un droit oublié ne perd rien : les fiches attendent, et repartent seules une fois le droit accordé.
#[tokio::test]
async fn a_missing_grant_keeps_the_enrichments_pending_until_it_is_granted() {
    let bench = Bench::new().await;
    admin_revoke_info(&bench).await;
    let mock = Mock::start(one_page).await;
    let mut collector = bench.collector(&mock);
    assert_eq!(
        collector.tick().await.unwrap_err(),
        TickError::Ingest {
            reason: "kind_not_declared",
            status: None
        }
    );
    assert_eq!(bench.journal("lastfm.scrobble").await, 2);
    assert_eq!(bench.journal("lastfm.track_info").await, 0);
    assert_eq!(collector.pending_enrichments(), 2);

    life_tracker::admin::grant_kind(&bench.db.admin, "lastfm", "lastfm.track_info")
        .await
        .unwrap();
    let report = collector.tick().await.unwrap();
    assert_eq!((report.enriched, report.rejected), (2, 0));
    assert_eq!(collector.pending_enrichments(), 0);
    assert_eq!(bench.journal("lastfm.track_info").await, 2);
}

async fn admin_revoke_info(bench: &Bench) {
    life_tracker::admin::revoke_kind(&bench.db.admin, "lastfm", "lastfm.track_info")
        .await
        .unwrap();
}

#[tokio::test]
async fn each_normalized_track_is_enriched_once() {
    let bench = Bench::new().await;
    let mock = Mock::start(two_pages).await;
    let mut collector = bench.collector(&mock);
    let report = collector.tick().await.unwrap();
    // « Récent » et « Récent - Remastered 2011 » sont un seul titre ; « Ancien titre » l'autre.
    assert_eq!(report.enriched, 2);
    assert_eq!(mock.calls("track.getInfo").len(), 2);
    assert_eq!(bench.journal("lastfm.track_info").await, 2);
    collector.tick().await.unwrap();
    assert_eq!(mock.calls("track.getInfo").len(), 2);

    // Même après un redémarrage.
    let mut restarted = bench.collector(&mock);
    restarted.tick().await.unwrap();
    assert_eq!(mock.calls("track.getInfo").len(), 2);

    let asked = mock.calls("track.getInfo");
    assert!(
        asked
            .iter()
            .all(|c| c["autocorrect"] == "1" && !c.contains_key("username"))
    );
}

#[tokio::test]
async fn unknown_tracks_and_missing_durations_are_not_asked_again() {
    for (reply, records) in [
        (("error_track_not_found", 200), 0),
        (("error_track_not_found", 400), 0),
        (("track_info_zero_duration", 200), 2),
        (("track_info_no_duration", 200), 2),
    ] {
        let bench = Bench::new().await;
        let mock = Mock::start(move |p| {
            if is_recent(p) {
                ok("recent_nowplaying")
            } else {
                (reply.1, None, fixture(reply.0).to_string())
            }
        })
        .await;
        let mut collector = bench.collector(&mock);
        collector.tick().await.unwrap();
        collector.tick().await.unwrap();
        assert_eq!(collector.pending_enrichments(), 0, "{reply:?}");
        assert_eq!(mock.calls("track.getInfo").len(), 2, "{reply:?}");
        assert_eq!(
            bench.journal("lastfm.track_info").await,
            records,
            "{reply:?}"
        );
    }
}

#[tokio::test]
async fn a_rate_limit_during_enrichment_keeps_what_remains_to_ask() {
    let bench = Bench::new().await;
    let mut answers = 0;
    let mock = Mock::start(move |p| {
        if is_recent(p) {
            return ok("recent_nowplaying");
        }
        answers += 1;
        if answers == 1 {
            ok("track_info")
        } else {
            (429, Some("60"), fixture("error_rate_limit").to_string())
        }
    })
    .await;
    let mut collector = bench.collector(&mock);
    assert_eq!(
        collector.tick().await.unwrap_err(),
        TickError::Service(ServiceError::RateLimited {
            retry_after: Duration::from_secs(60)
        })
    );
    assert_eq!(collector.pending_enrichments(), 1);
    assert!(
        collector.cursor().is_some(),
        "les écoutes, elles, sont passées"
    );
    assert!(bench.last_seen().await.is_some(), "Last.fm a bien répondu");

    // La fiche manquante reste à demander après un redémarrage, même hors de la fenêtre relue.
    drop(collector);
    mock.set(|p| {
        if is_recent(p) {
            ok("recent_empty")
        } else {
            ok("track_info")
        }
    });
    let mut resumed = bench.collector(&mock);
    assert_eq!(resumed.pending_enrichments(), 1);
    assert_eq!(resumed.tick().await.unwrap().enriched, 1);
    assert_eq!(bench.journal("lastfm.track_info").await, 2);
}

#[test]
fn what_the_collector_sends_respects_the_contract() {
    let schema = validator(None);
    let tracks = fixture("recent_nowplaying")["recenttracks"]["track"].clone();
    let mut records = scrobble_records(tracks.as_array().unwrap());
    assert_eq!(records.len(), 2);
    records.push(track_info_record(
        "A",
        "T",
        &fixture("track_info"),
        chrono::Utc::now(),
    ));
    let batch = json!({ "records": records });
    let errors: Vec<_> = schema.iter_errors(&batch).map(|e| e.to_string()).collect();
    assert!(errors.is_empty(), "{errors:?}");
}

// ─── Le vrai binaire : ce qui sort dans les logs ──────────────────────────────

const SENTINEL: &str = "SENTINELLE";

fn collect_process(
    mock_url: &str,
    ingest: &str,
    token: &str,
    state: &std::path::Path,
) -> (std::process::Child, PathBuf) {
    let log = state.join("collector.log");
    let file = std::fs::File::create(&log).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_life-tracker"))
        .arg("collect")
        .env_clear()
        .env("RUST_LOG", "trace")
        .env("LASTFM_API_URL", mock_url)
        .env("LASTFM_API_KEY", format!("{SENTINEL}-key"))
        .env("LASTFM_USER", format!("{SENTINEL}-user"))
        .env("COLLECTOR_TOKEN", token)
        .env("INGEST_URL", ingest)
        .env("COLLECTOR_STATE_DIR", state)
        .stdout(Stdio::from(file))
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    (child, log)
}

/// Attend une ligne de log : le premier résultat du collecteur, quel que soit le temps qu'il y met.
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
    // Arrêt demandé par SIGTERM : le processus doit s'arrêter proprement.
    Command::new("kill")
        .arg(child.id().to_string())
        .status()
        .unwrap();
    assert!(child.wait().unwrap().success());
    logs
}

#[tokio::test]
async fn logs_never_cite_the_key_the_user_the_token_or_a_name() {
    let bench = Bench::new().await;
    let names = |_: &Params| -> Reply {
        let page = json!({"recenttracks": {
            "track": [{
                "artist": {"#text": format!("{SENTINEL} artiste")},
                "album": {"#text": format!("{SENTINEL} album")},
                "name": format!("{SENTINEL} titre"),
                "date": {"uts": "1792310400"},
            }],
            "@attr": {"total": "1", "totalPages": "1"},
        }});
        (200, None, page.to_string())
    };

    // Réussite : titres, artistes et fiches dans les réponses.
    let mock = Mock::start(move |p| {
        if is_recent(p) {
            names(p)
        } else {
            ok("track_info")
        }
    })
    .await;
    let dir = bench.state_path();
    let (child, log) = collect_process(&mock.url, &bench.base, &bench.token, &dir);
    let logs = logs_until(child, &log, "collect pass completed").await;
    assert!(logs.contains("\"accepted\":1"), "{logs}");
    assert!(!logs.contains(SENTINEL), "fuite : {logs}");
    assert!(!logs.contains(&bench.token), "token dans les logs");
    assert_eq!(bench.journal("lastfm.scrobble").await, 1);

    // Chaque échec, y compris un message de Last.fm qui citerait l'utilisateur.
    let echo = format!("{{\"error\":6,\"message\":\"User {SENTINEL} not found\"}}");
    let cases: Vec<(Mock, &str)> = vec![
        (
            Mock::start(move |_| (400, None, echo.clone())).await,
            "lastfm refused",
        ),
        (
            Mock::start(|_| (429, Some("1"), fixture("error_rate_limit").to_string())).await,
            "lastfm rate limit",
        ),
        (
            Mock::start(move |_| (200, None, format!("<html>{SENTINEL}</html>"))).await,
            "lastfm unavailable",
        ),
    ];
    for (mock, line) in cases {
        let (child, log) = collect_process(&mock.url, &bench.base, &bench.token, &dir);
        let logs = logs_until(child, &log, line).await;
        assert!(!logs.contains(SENTINEL), "fuite : {logs}");
        assert!(!logs.contains(&bench.token));
    }

    // Une coupure : c'est là qu'une erreur de reqwest citerait son URL entière.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let gone = format!("http://{}/", listener.local_addr().unwrap());
    drop(listener);
    let (child, log) = collect_process(&gone, &bench.base, &bench.token, &dir);
    let logs = logs_until(child, &log, "lastfm unavailable").await;
    assert!(!logs.contains(SENTINEL), "fuite : {logs}");
    assert!(logs.contains("\"reason\":\"connect\""), "{logs}");

    // L'ingestion injoignable.
    let (child, log) = collect_process(&mock.url, gone.trim_end_matches('/'), &bench.token, &dir);
    let logs = logs_until(child, &log, "ingestion unavailable").await;
    assert!(!logs.contains(SENTINEL), "fuite : {logs}");
    assert!(!logs.contains(&bench.token));
}

#[test]
fn collect_never_receives_a_database_url_and_requires_a_connector() {
    let run = |envs: &[(&str, &str)]| {
        let output = Command::new(env!("CARGO_BIN_EXE_life-tracker"))
            .arg("collect")
            .env_clear()
            .env("DATABASE_URL", "postgres://ignored")
            .env("INGEST_URL", "http://127.0.0.1:9")
            .envs(envs.iter().copied())
            .output()
            .unwrap();
        assert!(!output.status.success());
        String::from_utf8_lossy(&output.stderr).into_owned()
    };
    let none = run(&[]);
    assert!(
        none.contains("LASTFM_API_KEY")
            && none.contains("GARMIN_API_KEY")
            && none.contains("YOUTUBE_CLIENT_ID")
            && none.contains("STEAM_API_KEY")
            && none.contains("GITHUB_API_TOKEN")
            && none.contains("RIOT_LOL_API_KEY")
            && none.contains("HENRIK_API_KEY"),
        "{none}"
    );
    let half = run(&[("RIOT_LOL_API_KEY", "cle-fictive"), ("RIOT_ID", "Nom#TAG")]);
    assert!(
        half.contains("RIOT_TFT_API_KEY") && half.contains("RIOT_TOKEN"),
        "{half}"
    );
    assert!(
        !half.contains("cle-fictive") && !half.contains("Nom#TAG"),
        "{half}"
    );
    let half = run(&[("HENRIK_API_KEY", "cle-fictive")]);
    assert!(half.contains("VALORANT_TOKEN"), "{half}");
    assert!(!half.contains("cle-fictive"), "{half}");
    let half = run(&[("GARMIN_API_KEY", "cle-fictive")]);
    assert!(half.contains("GARMIN_TOKEN"), "{half}");
    assert!(!half.contains("cle-fictive"), "{half}");
    let half = run(&[("LASTFM_API_KEY", "cle-fictive"), ("LASTFM_USER", "u")]);
    assert!(half.contains("COLLECTOR_TOKEN"), "{half}");
    let half = run(&[
        ("STEAM_API_KEY", "cle-fictive"),
        ("STEAM_ID", "76500000000000001"),
    ]);
    assert!(half.contains("STEAM_TOKEN"), "{half}");
    assert!(
        !half.contains("cle-fictive") && !half.contains("76500000000000001"),
        "{half}"
    );
    let half = run(&[("GITHUB_API_TOKEN", "ghp_jetonfictif")]);
    assert!(half.contains("GITHUB_INGEST_TOKEN"), "{half}");
    assert!(!half.contains("ghp_jetonfictif"), "{half}");
    let unreadable = run(&[
        ("GITHUB_API_TOKEN", "ghp_jetonfictif"),
        ("GITHUB_INGEST_TOKEN", "token-fictif"),
        ("GITHUB_TOKEN_EXPIRES_ON", "dans un an"),
    ]);
    assert!(
        unreadable.contains("GITHUB_TOKEN_EXPIRES_ON"),
        "{unreadable}"
    );
    assert!(!unreadable.contains("ghp_jetonfictif"), "{unreadable}");
}
