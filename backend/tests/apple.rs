//! L'historique Apple, de l'outil d'import à la page du jour. Les fixtures de `tests/fixtures/apple/`
//! sont entièrement fictives, écrites à la main ; les colonnes sensibles portent des sentinelles.

mod common;
use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::post;
use common::*;
use life_tracker::apple::{self, Report};
use life_tracker::interpreters::INTERPRETERS;
use life_tracker::{admin, api, engine, replay};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::sync::watch;
use uuid::Uuid;

const PASSWORD: &str = "un mot de passe assez long";
const SENSITIVE: [&str; 11] = [
    "SENSIBLE",
    "203.0.113.9",
    "48.8566",
    "2.3522",
    "Container",
    "Client",
    "IP City",
    "Apple ID",
    "Device",
    "Subscription",
    "Event ID",
];
/// Exactement ce qui peut entrer au journal : ajouter une colonne doit passer par ce test.
const PAYLOAD_FIELDS: [&str; 16] = [
    "album",
    "archive_sha256",
    "artist",
    "artist_source",
    "end_position_ms",
    "end_reason",
    "ended_at",
    "item_type",
    "media_duration_ms",
    "media_type",
    "offline",
    "play_duration_ms",
    "song",
    "start_position_ms",
    "started_at",
    "utc_offset_seconds",
];

fn archive() -> PathBuf {
    repo_path("backend/tests/fixtures/apple")
}

async fn spawn_router(router: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    format!("http://{addr}")
}

struct App {
    db: TestDb,
    base: String,
    cookie: String,
    apple: String,
    lastfm: String,
    http: reqwest::Client,
    _stop: watch::Sender<bool>,
}

impl App {
    async fn new() -> Self {
        let db = TestDb::new().await;
        admin::set_password(&db.admin, "moi", PASSWORD)
            .await
            .unwrap();
        let apple = admin::create_source(&db.admin, "apple", None)
            .await
            .unwrap();
        admin::grant_kind(&db.admin, "apple", apple::KIND)
            .await
            .unwrap();
        let lastfm = admin::create_source(&db.admin, "lastfm", Some("6 hours"))
            .await
            .unwrap();
        admin::grant_kind(&db.admin, "lastfm", "lastfm.scrobble")
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
        Self {
            db,
            base,
            cookie,
            apple,
            lastfm,
            http,
            _stop: stop,
        }
    }

    async fn import(&self, dir: &Path) -> anyhow::Result<Report> {
        self.import_to(dir, &self.base).await
    }

    async fn import_to(&self, dir: &Path, url: &str) -> anyhow::Result<Report> {
        apple::run(apple::Config {
            archive_dir: dir.to_owned(),
            ingest_url: url.to_owned(),
            token: self.apple.clone(),
            max_attempts: 1,
            retry_base: Duration::ZERO,
        })
        .await
    }

    async fn scrobble(&self, at: &str, artist: &str, track: &str, album: &str) {
        let key = Uuid::now_v7().to_string();
        let response = self
            .http
            .post(format!("{}/v1/records", self.base))
            .bearer_auth(&self.lastfm)
            .json(&json!({"records": [{
                "key": key, "kind": "lastfm.scrobble", "schema_version": 1, "occurred_at": at,
                "payload": {"artist": {"#text": artist}, "name": track, "album": {"#text": album}},
            }]}))
            .send()
            .await
            .unwrap();
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["results"][0]["status"], "accepted");
    }

    /// Attend que le worker ait lu tout le journal, sans dépendre de sa vitesse.
    async fn settled(&self) {
        for _ in 0..6000 {
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

    async fn keys(&self) -> Vec<String> {
        sqlx::query_scalar("SELECT key FROM core.records ORDER BY key")
            .fetch_all(&self.db.app)
            .await
            .unwrap()
    }
}

fn names(list: &Value) -> Vec<&str> {
    list.as_array()
        .unwrap()
        .iter()
        .map(|item| item["name"].as_str().unwrap())
        .collect()
}

/// Un dossier temporaire, effacé à la fin du test.
struct TempDir(PathBuf);

impl TempDir {
    /// Une copie des fixtures, dont Play Activity passe d'abord par `change`.
    fn with_activity(change: impl Fn(&str) -> String) -> Self {
        let dir = std::env::temp_dir().join(format!("lt-apple-{}", Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in [apple::DAILY_TRACKS, apple::LIBRARY_TRACKS] {
            std::fs::copy(archive().join(name), dir.join(name)).unwrap();
        }
        let original = std::fs::read_to_string(archive().join(apple::PLAY_ACTIVITY)).unwrap();
        std::fs::write(dir.join(apple::PLAY_ACTIVITY), change(&original)).unwrap();
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn counts(report: &Report) -> (u64, u64, u64) {
    (
        report.accepted,
        report.duplicate,
        report.rejected.values().sum(),
    )
}

#[tokio::test]
async fn the_import_counts_everything_and_keeps_each_kind_of_row_where_it_belongs() {
    let app = App::new().await;
    let report = app.import(&archive()).await.unwrap();
    assert_eq!(counts(&report), (7, 0, 0));
    assert_eq!(report.rows, 13);
    let ignored: Vec<_> = report.ignored.iter().map(|(k, v)| (*k, *v)).collect();
    assert_eq!(
        ignored,
        [
            ("lyric_display", 1),
            ("no_playback", 1),
            ("no_song_name", 1),
            ("no_start_timestamp", 1),
            ("play_start", 1),
            ("video", 1),
        ]
    );
    let attributed: Vec<_> = report.attribution.iter().map(|(k, v)| (*k, *v)).collect();
    assert_eq!(
        attributed,
        [
            ("ambiguous", 1),
            ("daily_hour", 1),
            ("library_album", 3),
            ("unknown", 2),
        ]
    );
    assert_eq!(report.activity_sha256.len(), 64);

    // Une écoute pour la lecture entière, une seule pour la lecture reprise, aucune pour le titre passé.
    let day = app.music("2023-03-01").await;
    assert_eq!(day["listens"], 2);
    assert_eq!(names(&day["tracks"]), ["Titre fictif", "Titre repris"]);
    // 200 s + les 140 s du morceau qui franchit la moitié : pas besoin d'une fiche Last.fm.
    assert_eq!(day["listens_without_duration"], 0);
    assert_eq!(day["minutes"], 6);
    assert_eq!(names(&day["artists"]), ["ARTISTE FICTIF feat. Invité"]);
    assert_eq!(day["artists"][0]["listens"], 2);
    assert_eq!(names(&day["albums"]), ["Album fictif"]);
}

#[tokio::test]
async fn reimporting_the_same_archive_gives_only_duplicates() {
    let app = App::new().await;
    assert_eq!(counts(&app.import(&archive()).await.unwrap()), (7, 0, 0));
    assert_eq!(counts(&app.import(&archive()).await.unwrap()), (0, 7, 0));
    assert_eq!(
        app.db.count("SELECT count(*) FROM core.records").await,
        7,
        "aucun doublon au journal"
    );
}

/// Un export ultérieur couvrant la même période : autre ordre, autres identifiants et autres valeurs
/// partout où Apple peut changer d'avis, sauf ce qui décrit la lecture elle-même.
#[tokio::test]
async fn a_later_export_of_the_same_period_gives_only_duplicates() {
    let app = App::new().await;
    assert_eq!(counts(&app.import(&archive()).await.unwrap()), (7, 0, 0));
    let before = app.keys().await;

    let later = TempDir::with_activity(|original| {
        let mut lines: Vec<&str> = original.lines().collect();
        let header = lines.remove(0);
        lines.reverse();
        // Un autre appareil, une autre ville, d'autres identifiants, et les dates écrites sans millisecondes.
        let body = lines
            .join("\n")
            .replace("SENSIBLE-EVENT", "AUTRE-EVENT")
            .replace("SENSIBLE-DEV-ID", "AUTRE-APPAREIL")
            .replace("SENSIBLE-CITY", "AUTRE-VILLE")
            .replace("203.0.113.9", "203.0.113.200")
            .replace(".000Z", "Z");
        format!("{header}\n{body}\n")
    });
    // La bibliothèque a changé aussi : l'artiste déduit serait autre, la clé ne doit pas le voir.
    std::fs::write(later.0.join(apple::LIBRARY_TRACKS), "[]").unwrap();
    assert_eq!(counts(&app.import(&later.0).await.unwrap()), (0, 7, 0));
    assert_eq!(app.keys().await, before);
    // Le premier reçu reste : les faits déjà au journal ne changent pas.
    assert_eq!(app.music("2023-03-01").await["listens"], 2);
}

/// La sentinelle couvre chaque colonne hors liste de la fixture : aucune n'atteint le journal, par aucun
/// chemin. Le nom d'une colonne n'y entre pas non plus, et la liste des champs est figée.
#[tokio::test]
async fn a_column_outside_the_whitelist_never_reaches_the_journal() {
    let app = App::new().await;
    app.import(&archive()).await.unwrap();
    app.settled().await;
    let journal: Vec<(String, String)> =
        sqlx::query_as("SELECT key, payload::text FROM core.records")
            .fetch_all(&app.db.app)
            .await
            .unwrap();
    assert_eq!(journal.len(), 7);
    for (key, payload) in &journal {
        for sentinel in SENSITIVE {
            assert!(!payload.contains(sentinel), "colonne {sentinel} au journal");
            assert!(!key.contains(sentinel));
        }
        let value: Value = serde_json::from_str(payload).unwrap();
        let mut fields: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        fields.sort_unstable();
        assert_eq!(fields, PAYLOAD_FIELDS);
    }
    let derived: Vec<String> = sqlx::query_scalar("SELECT details::text FROM derived.events")
        .fetch_all(&app.db.app)
        .await
        .unwrap();
    assert!(!derived.is_empty());
    assert!(
        derived
            .iter()
            .all(|details| SENSITIVE.iter().all(|s| !details.contains(s)))
    );
}

/// Une écoute dont l'artiste reste inconnu existe, compte, se voit, mais ne devient ni artiste, ni album,
/// ni découverte. Même chose si plusieurs artistes sont possibles.
#[tokio::test]
async fn a_listen_without_a_known_artist_is_counted_and_shown_without_inventing_anything() {
    let app = App::new().await;
    app.import(&archive()).await.unwrap();

    let day = app.music("2023-03-02").await;
    assert_eq!(day["listens"], 2);
    assert_eq!(day["listens_without_artist"], 1);
    let unknown = day["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|track| track["name"] == "Titre sans artiste")
        .unwrap();
    assert_eq!(unknown["artist"], Value::Null);
    assert_eq!(names(&day["artists"]), ["Groupe Quotidien"]);
    assert_eq!(names(&day["albums"]), ["Album quotidien"]);
    assert_eq!(day["first_listens"].as_array().unwrap().len(), 3);
    assert!(
        day["first_listens"]
            .as_array()
            .unwrap()
            .iter()
            .all(|find| find["artist"] == "Groupe Quotidien")
    );

    let ambiguous = app.music("2023-03-03").await;
    assert_eq!(ambiguous["listens"], 1);
    assert_eq!(ambiguous["listens_without_artist"], 1);
    assert_eq!(ambiguous["artist_of_day"], Value::Null);
    assert_eq!(ambiguous["discoveries"], 0);
    assert_eq!(ambiguous["artists"], json!([]));
    let journal: String = sqlx::query_scalar(
        "SELECT payload->>'artist_source' FROM core.records WHERE payload->>'song' = 'Titre ambigu'",
    )
    .fetch_one(&app.db.app)
    .await
    .unwrap();
    assert_eq!(journal, "ambiguous");
}

#[tokio::test]
async fn the_overlap_stays_in_the_journal_and_the_views_ignore_it() {
    let app = App::new().await;
    app.import(&archive()).await.unwrap();
    // La même écoute, vue par Last.fm, le jour où il démarre.
    app.scrobble(
        "2023-03-02T09:29:55Z",
        "Groupe Quotidien",
        "Titre du jour",
        "Album quotidien",
    )
    .await;
    app.settled().await;
    assert_eq!(
        app.music("2023-03-02").await["listens"],
        3,
        "sans priorité, la même écoute compte deux fois"
    );

    let start = admin::set_precedence(&app.db.admin, "music.listen", "lastfm", None)
        .await
        .unwrap();
    assert_eq!(start.to_rfc3339(), "2023-03-02T09:29:55+00:00");

    let day = app.music("2023-03-02").await;
    // L'écoute Apple de 08:00 précède le démarrage et reste ; celle de 09:30:00 s'efface devant Last.fm.
    assert_eq!(day["listens"], 2);
    assert_eq!(day["listens_without_artist"], 1);
    assert_eq!(
        app.db
            .count("SELECT count(*) FROM core.records WHERE kind = 'apple.play_history_row'")
            .await,
        7,
        "le journal garde le chevauchement"
    );
    assert_eq!(
        app.music("2023-03-03").await["listens"],
        0,
        "après le démarrage"
    );
    assert_eq!(
        app.music("2023-03-01").await["listens"],
        2,
        "avant le démarrage"
    );

    // Une couverture par source, sans chevauchement ni trou inventé.
    let health = app.get("/v1/sources/health").await;
    let metric = &health["metrics"][0];
    assert_eq!(metric["coverage"][0]["source"], "apple");
    assert_eq!(metric["coverage"][1]["source"], "lastfm");
    assert_eq!(metric["coverage"][1]["first_at"], "2023-03-02T09:29:55Z");

    let report = replay::rebuild(&app.db.admin, INTERPRETERS, false)
        .await
        .unwrap();
    assert!(report.switched);
    assert_eq!(report.comparison.differences, vec![]);
    assert_eq!(app.music("2023-03-02").await["listens"], 2);
}

#[tokio::test]
async fn discoveries_move_back_to_the_dates_of_the_history() {
    let app = App::new().await;
    app.scrobble(
        "2026-10-20T09:00:00Z",
        "Artiste Fictif",
        "Titre fictif (Live)",
        "Album fictif",
    )
    .await;
    app.scrobble(
        "2026-10-20T10:00:00Z",
        "Artiste Fictif",
        "Vraiment nouveau",
        "Album fictif",
    )
    .await;
    let before = app.music("2026-10-20").await;
    assert_eq!(before["discoveries"], 4, "artiste, album et deux titres");

    app.import(&archive()).await.unwrap();
    admin::set_precedence(&app.db.admin, "music.listen", "lastfm", None)
        .await
        .unwrap();

    let after = app.music("2026-10-20").await;
    let found: Vec<&str> = after["first_listens"]
        .as_array()
        .unwrap()
        .iter()
        .map(|find| find["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        found,
        ["Vraiment nouveau"],
        "seul ce que l'historique ne connaît pas reste une découverte"
    );
    let then = app.music("2023-03-01").await;
    let levels: Vec<(&str, &str)> = then["first_listens"]
        .as_array()
        .unwrap()
        .iter()
        .map(|find| {
            (
                find["level"].as_str().unwrap(),
                find["name"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        levels,
        [
            ("artist", "ARTISTE FICTIF feat. Invité"),
            ("album", "Album fictif"),
            ("track", "Titre fictif"),
            ("track", "Titre repris"),
        ]
    );
    let report = replay::rebuild(&app.db.admin, INTERPRETERS, false)
        .await
        .unwrap();
    assert_eq!(report.comparison.differences, vec![]);
    let same = app.music("2023-03-01").await;
    assert_eq!(same["first_listens"], then["first_listens"]);
}

#[tokio::test]
async fn a_lone_old_scrobble_cannot_silently_take_over_the_whole_history() {
    let app = App::new().await;
    app.import(&archive()).await.unwrap();
    app.scrobble("2021-01-01T10:00:00Z", "Artiste Vieux", "Essai", "")
        .await;
    app.scrobble("2026-10-20T09:00:00Z", "Artiste Fictif", "Récent", "")
        .await;
    app.settled().await;

    let refused = admin::set_precedence(&app.db.admin, "music.listen", "lastfm", None)
        .await
        .unwrap_err()
        .to_string();
    assert!(refused.contains("isolée"), "{refused}");
    assert_eq!(
        app.db
            .count("SELECT count(*) FROM core.source_precedence")
            .await,
        0
    );

    let from = "2026-10-20T00:00:00Z".parse().unwrap();
    admin::set_precedence(&app.db.admin, "music.listen", "lastfm", Some(from))
        .await
        .unwrap();
    assert_eq!(
        app.music("2023-03-01").await["listens"],
        2,
        "l'historique reste"
    );
    // Le changement de date remplace la précédente : la base refuserait deux périodes qui se chevauchent.
    let later = "2026-10-25T00:00:00Z".parse().unwrap();
    admin::set_precedence(&app.db.admin, "music.listen", "lastfm", Some(later))
        .await
        .unwrap();
    assert_eq!(
        app.db
            .count("SELECT count(*) FROM core.source_precedence")
            .await,
        1
    );

    let covered = admin::coverage(&app.db.admin, "music.listen")
        .await
        .unwrap();
    assert_eq!(covered[0].source, "lastfm", "la plus ancienne d'abord");
    assert_eq!(covered[0].events, 2);
    assert_eq!(covered[1].source, "apple");
    assert_eq!(covered[1].events, 5);
    assert!(
        admin::set_precedence(&app.db.admin, "music.listen", "inconnue", Some(from))
            .await
            .is_err()
    );
}

/// Du bruit : une répétition se compresserait et tiendrait dans l'index.
fn long_name(chars: usize) -> String {
    let mut out = String::new();
    let mut seed = Sha256::digest(b"fictif").to_vec();
    while out.len() < chars {
        seed = Sha256::digest(&seed).to_vec();
        out.push_str(&hex::encode(&seed));
    }
    out.truncate(chars);
    out
}

/// Un nom trop long pour l'index des éléments : une erreur d'interprétation, jamais une panne qui
/// bloquerait le worker. Le suivant passe. Un nom long qui tient reste une écoute, y compris au rebuild.
#[tokio::test]
async fn a_name_too_long_for_the_index_is_an_error_and_the_worker_goes_on() {
    let app = App::new().await;
    let too_long = long_name(20_000);
    app.scrobble("2023-05-01T09:00:00Z", "Avant", "Titre", "")
        .await;
    app.scrobble("2023-05-01T09:01:00Z", &too_long, "Titre", "")
        .await;
    app.scrobble("2023-05-01T09:02:00Z", "Artiste", &too_long, "")
        .await;
    let long_but_fine = long_name(1_500);
    app.scrobble("2023-05-01T09:03:00Z", &long_but_fine, "Titre", "")
        .await;
    app.scrobble("2023-05-01T09:04:00Z", "Après", "Titre", "")
        .await;

    let dir = TempDir::with_activity(|original| original.replace("Titre sans artiste", &too_long));
    let report = app.import(&dir.0).await.unwrap();
    assert_eq!(counts(&report), (7, 0, 0));

    let day = app.music("2023-05-01").await;
    assert_eq!(day["listens"], 3);
    let messages: Vec<String> =
        sqlx::query_scalar("SELECT error FROM derived.interpretation_errors")
            .fetch_all(&app.db.app)
            .await
            .unwrap();
    assert_eq!(
        messages,
        vec!["output refused by the database (54000)"; 2],
        "Last.fm ; le titre Apple sans artiste n'a pas d'élément, donc pas de limite"
    );
    assert!(messages.iter().all(|m| !m.contains(&too_long[..20])));

    let report = replay::rebuild(&app.db.admin, INTERPRETERS, false)
        .await
        .unwrap();
    assert!(report.switched);
    assert_eq!(report.comparison.differences, vec![]);
    assert_eq!(app.music("2023-05-01").await["listens"], 3);
}

#[tokio::test]
async fn an_apple_name_too_long_for_the_index_is_set_aside_too() {
    let app = App::new().await;
    let too_long = long_name(20_000);
    let dir = TempDir::with_activity(|original| {
        original.replace("Titre fictif,Album", &format!("{too_long},Album"))
    });
    std::fs::write(
        dir.0.join(apple::LIBRARY_TRACKS),
        json!([{"Title": too_long, "Artist": "Artiste Fictif", "Album": "Album fictif"}])
            .to_string(),
    )
    .unwrap();
    assert_eq!(counts(&app.import(&dir.0).await.unwrap()), (7, 0, 0));
    app.settled().await;
    assert_eq!(
        app.db
            .count("SELECT count(*) FROM derived.interpretation_errors WHERE error LIKE '%54000%'")
            .await,
        1
    );
    assert_eq!(
        app.music("2023-03-02").await["listens"],
        2,
        "le reste continue"
    );
}

#[derive(Clone)]
struct Flaky {
    target: String,
    calls: Arc<AtomicUsize>,
    pass: usize,
    http: reqwest::Client,
}

/// Laisse passer les premiers appels vers la vraie API, puis ne répond plus que 503.
async fn flaky(
    State(flaky): State<Flaky>,
    headers: axum::http::HeaderMap,
    body: String,
) -> impl IntoResponse {
    if flaky.calls.fetch_add(1, Ordering::SeqCst) >= flaky.pass {
        return (StatusCode::SERVICE_UNAVAILABLE, String::new());
    }
    let reply = flaky
        .http
        .post(format!("{}/v1/records", flaky.target))
        .header("authorization", headers["authorization"].to_str().unwrap())
        .body(body)
        .send()
        .await
        .unwrap();
    (StatusCode::OK, reply.text().await.unwrap())
}

/// Assez de lignes pour plusieurs lots, toutes distinctes.
fn many_rows(original: &str, count: usize) -> String {
    let mut lines = original.lines();
    let header = lines.next().unwrap();
    let row = lines.find(|l| l.contains("SENSIBLE-EVENT-2,")).unwrap();
    let mut out = format!("{header}\n");
    for n in 0..count {
        let minute = n % 60;
        let hour = n / 60 % 24;
        let day = n / 1440 + 1;
        let start = format!("2023-04-{day:02}T{hour:02}:{minute:02}:00.000Z");
        let end = format!("2023-04-{day:02}T{hour:02}:{minute:02}:30.000Z");
        out.push_str(
            &row.replace("2023-03-01T10:00:00.000Z", &start)
                .replace("2023-03-01T10:03:20.000Z", &end),
        );
        out.push('\n');
    }
    out
}

/// Une coupure au milieu : l'outil s'arrête et le dit, ce qui est passé reste. Le relancer ne renvoie
/// que des `duplicate` pour ce qui était déjà là et finit le travail, sans aucun état local.
#[tokio::test]
async fn an_import_cut_in_the_middle_resumes_by_being_run_again() {
    let app = App::new().await;
    let dir = TempDir::with_activity(|original| many_rows(original, 1_200));
    let proxy = spawn_router(
        Router::new()
            .route("/v1/records", post(flaky))
            .with_state(Flaky {
                target: app.base.clone(),
                calls: Arc::new(AtomicUsize::new(0)),
                pass: 1,
                http: client(),
            }),
    )
    .await;
    let error = app.import_to(&dir.0, &proxy).await.unwrap_err().to_string();
    assert!(
        error.contains("503") && error.contains("relancer"),
        "{error}"
    );
    assert_eq!(app.db.count("SELECT count(*) FROM core.records").await, 500);

    let report = app.import(&dir.0).await.unwrap();
    assert_eq!(counts(&report), (700, 500, 0));
    assert_eq!(
        app.db.count("SELECT count(*) FROM core.records").await,
        1_200
    );
}

#[tokio::test]
async fn a_source_without_the_right_stops_at_once_and_says_why() {
    let app = App::new().await;
    admin::revoke_kind(&app.db.admin, "apple", apple::KIND)
        .await
        .unwrap();
    let error = app.import(&archive()).await.unwrap_err().to_string();
    assert!(error.contains(apple::KIND), "{error}");
    assert_eq!(app.db.count("SELECT count(*) FROM core.records").await, 0);
}

#[tokio::test]
async fn a_missing_file_is_named_and_a_missing_column_too() {
    let app = App::new().await;
    let dir = TempDir::with_activity(|original| original.to_owned());
    std::fs::remove_file(dir.0.join(apple::DAILY_TRACKS)).unwrap();
    let error = app.import(&dir.0).await.unwrap_err().to_string();
    assert!(error.contains(apple::DAILY_TRACKS), "{error}");

    let dir = TempDir::with_activity(|original| original.replacen("Event Type", "Autre", 1));
    let error = app.import(&dir.0).await.unwrap_err().to_string();
    assert!(error.contains("Event Type"), "{error}");
}

fn run_binary(base: &str, stdin: &str) -> (bool, String) {
    use std::io::Write;
    let mut child = Command::new(env!("CARGO_BIN_EXE_life-tracker"))
        .env_clear()
        .args(["import-apple", "--archive"])
        .arg(archive())
        .env("INGEST_URL", base)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    let text =
        String::from_utf8(output.stdout).unwrap() + &String::from_utf8(output.stderr).unwrap();
    (output.status.success(), text)
}

/// Le vrai binaire : le token passe par l'entrée standard, jamais en argument, et rien de la sortie ne
/// vient des lignes.
#[tokio::test(flavor = "multi_thread")]
async fn the_binary_reads_the_token_on_stdin_and_prints_only_counts() {
    let app = App::new().await;
    let binary = |stdin: String| {
        let base = app.base.clone();
        tokio::task::spawn_blocking(move || run_binary(&base, &stdin))
    };
    let (ok, first) = binary(format!("{}\n", app.apple)).await.unwrap();
    assert!(ok, "{first}");
    for line in [
        "accepted : 7",
        "duplicate : 0",
        "rejected : 0",
        "play_start : 1",
        "unknown : 2",
    ] {
        assert!(first.contains(line), "{line} : {first}");
    }
    assert!(first.contains("empreinte Apple Music Play Activity.csv : "));
    let (ok, second) = binary(format!("{}\n", app.apple)).await.unwrap();
    assert!(ok);
    assert!(
        second.contains("accepted : 0") && second.contains("duplicate : 7"),
        "{second}"
    );
    for text in [&first, &second] {
        for leak in [
            "Titre",
            "Album",
            "Artiste",
            "Groupe",
            "SENSIBLE",
            app.apple.as_str(),
        ] {
            assert!(!text.contains(leak), "{leak} dans la sortie");
        }
    }

    let (ok, empty) = binary(String::new()).await.unwrap();
    assert!(!ok);
    assert!(empty.contains("token vide"));
    let (ok, wrong) = binary("mauvais\n".to_owned()).await.unwrap();
    assert!(!ok);
    assert!(wrong.contains("token refusé") && !wrong.contains("mauvais"));

    let flag = Command::new(env!("CARGO_BIN_EXE_life-tracker"))
        .env_clear()
        .args(["import-apple", "--ingest-url", "x", "--token", "secret"])
        .output()
        .unwrap();
    assert!(!flag.status.success(), "aucun argument pour le token");
}
