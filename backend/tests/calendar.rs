//! La vue du mois et la complétude : erreurs, sources muettes et journées en attente, à vérifier,
//! sans données. De l'API d'ingestion à l'API de lecture, avec le vrai worker.

mod common;
use common::*;
use life_tracker::interpreters::INTERPRETERS;
use life_tracker::{admin, api, engine};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::sync::watch;

const PASSWORD: &str = "un mot de passe assez long";

struct App {
    db: TestDb,
    base: String,
    cookie: String,
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
        let lastfm = admin::create_source(&db.admin, "lastfm", Some("6 hours"), None)
            .await
            .unwrap();
        for kind in ["lastfm.scrobble", "lastfm.track_info"] {
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
            lastfm,
            http,
            _stop: stop,
        }
    }

    /// Une seconde source d'écoutes, sans seuil de silence : un historique importé une fois.
    async fn history_source(&self) -> String {
        let token = admin::create_source(&self.db.admin, "history", None, None)
            .await
            .unwrap();
        admin::grant_kind(&self.db.admin, "history", "lastfm.scrobble")
            .await
            .unwrap();
        token
    }

    async fn send_as(&self, token: &str, records: Vec<Value>) {
        let response = self
            .http
            .post(format!("{}/v1/records", self.base))
            .bearer_auth(token)
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

    async fn send(&self, records: Vec<Value>) {
        self.send_as(&self.lastfm, records).await;
    }

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
    }

    /// Le worker est asynchrone : attend qu'il ait lu tout le journal.
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

    async fn response(&self, path: &str) -> reqwest::Response {
        self.settled().await;
        self.http
            .get(format!("{}{path}", self.base))
            .header("cookie", &self.cookie)
            .send()
            .await
            .unwrap()
    }

    async fn month(&self, month: &str) -> Value {
        let response = self.response(&format!("/v1/months/{month}")).await;
        assert_eq!(response.status(), 200, "{month}");
        response.json().await.unwrap()
    }

    /// Un jour de la vue du mois.
    async fn day(&self, date: &str) -> Value {
        let month = self.month(&date[..7]).await;
        month["days"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["date"] == date)
            .unwrap()
            .clone()
    }

    async fn year(&self, year: i32) -> Value {
        let response = self.response(&format!("/v1/years/{year}")).await;
        assert_eq!(response.status(), 200, "{year}");
        assert_eq!(response.headers()["cache-control"], "no-store");
        response.json().await.unwrap()
    }

    async fn sql_date(&self, sql: &str) -> String {
        let date: chrono::NaiveDate = sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.db.app)
            .await
            .unwrap();
        date.to_string()
    }
}

fn scrobble(at: &str, track: &str) -> Value {
    json!({
        "key": format!("{at}|{track}"),
        "kind": "lastfm.scrobble",
        "schema_version": 1,
        "occurred_at": at,
        "payload": {
            "artist": {"mbid": "", "#text": "Artiste"},
            "album": {"mbid": "", "#text": ""},
            "name": track,
            "date": {"uts": "0"},
        },
    })
}

/// Un scrobble que l'interprète refuse : il reste au journal, en erreur.
fn broken(at: &str) -> Value {
    json!({
        "key": format!("{at}|cassé"),
        "kind": "lastfm.scrobble",
        "schema_version": 1,
        "occurred_at": at,
        "payload": {"unexpected": true},
    })
}

fn cup(key: &str, at: &str) -> Value {
    json!({"key": key, "kind": "coffee.cup", "schema_version": 1, "occurred_at": at, "payload": {}})
}

fn add_days(date: &str, days: i64) -> String {
    (chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap() + chrono::Duration::days(days))
        .to_string()
}

#[tokio::test]
async fn an_error_marks_its_day_in_paris() {
    let app = App::new().await;
    // 22:30 UTC le 16 : 00:30 le 17 à Paris. La bonne écoute du même instant passe quand même.
    app.send(vec![
        broken("2026-10-16T22:30:00Z"),
        scrobble("2026-10-16T22:31:00Z", "Bon"),
    ])
    .await;
    // 23:30 UTC le 28 mars : 00:30 le 29, jour de 23 heures.
    app.send(vec![broken("2026-03-28T23:30:00Z")]).await;

    let october = app.month("2026-10").await;
    let errors: Vec<(&str, i64)> = october["days"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["errors"] != 0)
        .map(|d| (d["date"].as_str().unwrap(), d["errors"].as_i64().unwrap()))
        .collect();
    assert_eq!(errors, [("2026-10-17", 1)]);
    assert_eq!(app.day("2026-10-17").await["values"]["music.listen"], 1.0);
    assert_eq!(app.day("2026-03-29").await["errors"], 1);
    assert_eq!(app.day("2026-03-28").await["errors"], 0);

    // Le compteur global, sur la santé que lit aussi la page du jour.
    let health = app.response("/v1/sources/health").await;
    assert_eq!(health.json::<Value>().await.unwrap()["errors"], 2);
}

#[tokio::test]
async fn a_silent_source_marks_the_days_it_has_not_covered() {
    let app = App::new().await;
    app.send(vec![scrobble("2026-10-01T10:00:00Z", "Premier")])
        .await;
    let pending = async |date: &str| app.day(date).await["pending"].clone();
    let today = app
        .sql_date("SELECT (now() AT TIME ZONE 'Europe/Paris')::date")
        .await;

    // Jamais muette : rien n'est en attente, pas même aujourd'hui.
    assert_eq!(pending(&today).await, json!([]));

    sqlx::query(
        "UPDATE core.sources SET last_seen_at = now() - interval '3 days' WHERE name = 'lastfm'",
    )
    .execute(&app.db.app)
    .await
    .unwrap();
    let since = app
        .sql_date("SELECT ((now() - interval '3 days') AT TIME ZONE 'Europe/Paris')::date")
        .await;
    // Le jour du dernier contact n'est couvert qu'en partie : en attente lui aussi, jusqu'à aujourd'hui.
    assert_eq!(pending(&add_days(&since, -1)).await, json!([]));
    for offset in 0..=3 {
        let date = add_days(&since, offset);
        assert!(date <= today);
        assert_eq!(pending(&date).await, json!(["lastfm"]), "{date}");
    }
    // Ni le futur, ni ce qui précède le dernier contact.
    assert_eq!(pending(&add_days(&today, 1)).await, json!([]));
    assert_eq!(pending("2026-10-01").await, json!([]));

    // Un battement : la source a rattrapé, plus rien n'est en attente.
    let beat = app
        .http
        .post(format!("{}/v1/heartbeat", app.base))
        .bearer_auth(&app.lastfm)
        .send()
        .await
        .unwrap();
    assert_eq!(beat.status(), 200);
    assert_eq!(pending(&today).await, json!([]));
}

#[tokio::test]
async fn a_source_without_threshold_is_never_pending() {
    let app = App::new().await;
    let history = app.history_source().await;
    sqlx::query(
        "UPDATE core.sources SET created_at = now() - interval '1 year' WHERE name = 'history'",
    )
    .execute(&app.db.owner)
    .await
    .unwrap();
    app.send_as(&history, vec![scrobble("2026-09-01T10:00:00Z", "A")])
        .await;
    let today = app
        .sql_date("SELECT (now() AT TIME ZONE 'Europe/Paris')::date")
        .await;
    let day = app.day(&today).await;
    // Seule lastfm, tout juste créée, a un seuil ; elle n'est pas encore muette.
    assert_eq!(day["pending"], json!([]));
}

#[tokio::test]
async fn days_before_a_metric_starts_have_no_data_and_no_alert() {
    let app = App::new().await;
    app.write_as_webapp(vec![cup("premier", "2026-10-20T08:00:00+02:00")])
        .await;

    // Avant le premier café : pas de données, ni zéro ni alerte. Quatre ans de jours sans café le restent.
    let september = app.month("2026-09").await;
    for day in september["days"].as_array().unwrap() {
        assert_eq!(day["values"]["coffee"], Value::Null, "{day}");
        assert_eq!(day["values"]["music.listen"], Value::Null, "{day}");
        assert_eq!(day["errors"], 0);
        assert_eq!(day["pending"], json!([]));
        assert_eq!(day["to_check"], json!([]));
    }
    assert_eq!(app.day("2022-03-15").await["values"]["coffee"], Value::Null);
    assert_eq!(app.day("2026-10-19").await["values"]["coffee"], Value::Null);
    assert_eq!(app.day("2026-10-20").await["values"]["coffee"], 1.0);
    // Après : une journée sans café est une vraie donnée (le café n'a pas de seuil).
    let after = app.day("2026-10-21").await;
    assert_eq!(after["values"]["coffee"], 0.0);
    assert_eq!(after["to_check"], json!([]));

    let month = app.month("2026-10").await;
    assert_eq!(month["month"], "2026-10");
    assert_eq!(month["days"].as_array().unwrap().len(), 31);
    let metrics: Vec<&str> = month["metrics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["metric"].as_str().unwrap())
        .collect();
    assert_eq!(
        metrics,
        [
            "coffee",
            "music.listen",
            "steps",
            "workout",
            "sleep",
            "calories.burned",
            "video.watch",
            "stream.watch",
            "game.play",
            "place.visit"
        ],
        "ni déclarations, ni références, ni métriques sans valeur du jour (les succès Steam), dans l'ordre d'affichage du registre"
    );
    assert_eq!(month["metrics"][0]["label"], "Cafés");
}

#[tokio::test]
async fn a_gap_between_two_sources_is_no_data_and_a_stale_metric_is_to_check() {
    let app = App::new().await;
    let history = app.history_source().await;
    // L'historique couvre du 1er au 5 septembre, sans écoute le 3 ; Last.fm commence le 10.
    app.send_as(
        &history,
        vec![
            scrobble("2026-09-01T10:00:00Z", "A"),
            scrobble("2026-09-02T10:00:00Z", "B"),
            scrobble("2026-09-04T10:00:00Z", "C"),
            scrobble("2026-09-05T10:00:00Z", "D"),
        ],
    )
    .await;
    app.send(vec![
        scrobble("2026-09-10T10:00:00Z", "E"),
        scrobble("2026-09-12T10:00:00Z", "F"),
    ])
    .await;

    let september = app.month("2026-09").await;
    let listens = |date: &str| {
        september["days"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["date"] == date)
            .unwrap()
            .clone()
    };
    let value = |date: &str| listens(date)["values"]["music.listen"].clone();
    assert_eq!(value("2026-09-01"), 1.0);
    assert_eq!(
        value("2026-09-03"),
        0.0,
        "dans la couverture : un vrai zéro"
    );
    for gap in ["2026-09-06", "2026-09-07", "2026-09-08", "2026-09-09"] {
        assert_eq!(value(gap), Value::Null, "{gap} : entre deux sources");
        assert_eq!(listens(gap)["to_check"], json!([]));
    }
    assert_eq!(value("2026-09-11"), 0.0);
    assert_eq!(
        app.day("2026-08-31").await["values"]["music.listen"],
        Value::Null
    );

    // Plus rien depuis le 12 septembre : au-delà de 3 jours, les jours suivants sont à vérifier.
    assert_eq!(listens("2026-09-12")["to_check"], json!([]));
    assert_eq!(listens("2026-09-13")["to_check"], json!(["music.listen"]));
    assert_eq!(value("2026-09-13"), 0.0, "à vérifier, mais reçu comme zéro");
    assert_eq!(listens("2026-09-11")["to_check"], json!([]));

    // Une écoute récente : plus rien à vérifier.
    let now = chrono::Utc::now().to_rfc3339();
    app.send(vec![scrobble(&now, "G")]).await;
    assert_eq!(app.day("2026-09-13").await["to_check"], json!([]));
}

#[tokio::test]
async fn the_days_of_23_and_25_hours_keep_their_edges() {
    let app = App::new().await;
    app.send(vec![
        scrobble("2026-10-24T21:59:00Z", "Veille"), // 23:59 le 24, heure d'été
        scrobble("2026-10-24T22:30:00Z", "Minuit"), // 00:30 le 25
        scrobble("2026-10-25T22:30:00Z", "Fin"),    // 23:30 le 25, heure d'hiver
        scrobble("2026-03-28T23:00:00Z", "Début"),  // 00:00 le 29 mars
        scrobble("2026-03-29T21:59:00Z", "Fin courte"), // 23:59 le 29, heure d'été
    ])
    .await;
    let value = async |date: &str| app.day(date).await["values"]["music.listen"].clone();
    assert_eq!(value("2026-10-24").await, 1.0);
    assert_eq!(value("2026-10-25").await, 2.0);
    assert_eq!(value("2026-10-26").await, 0.0);
    assert_eq!(value("2026-03-28").await, Value::Null);
    assert_eq!(value("2026-03-29").await, 2.0);
}

#[tokio::test]
async fn past_days_are_never_cached_and_follow_late_data() {
    let app = App::new().await;
    app.write_as_webapp(vec![cup("debut", "2026-10-01T08:00:00+02:00")])
        .await;
    let response = app.response("/v1/months/2026-10").await;
    assert_eq!(response.headers()["cache-control"], "no-store");
    let value = async |date: &str| app.day(date).await["values"]["coffee"].clone();
    assert_eq!(value("2026-10-10").await, 0.0);

    // Un café noté après coup, puis une erreur tardive : le jour passé change d'apparence.
    app.write_as_webapp(vec![cup("tardif", "2026-10-10T09:00:00+02:00")])
        .await;
    assert_eq!(value("2026-10-10").await, 1.0);
    app.send(vec![broken("2026-10-10T12:00:00Z")]).await;
    assert_eq!(app.day("2026-10-10").await["errors"], 1);

    // Et une annulation le ramène.
    app.write_as_webapp(vec![json!({
        "key": "annule", "kind": "record.cancel", "schema_version": 1,
        "occurred_at": "2026-11-01T10:00:00+01:00",
        "payload": {"source": "webapp", "key": "tardif"},
    })])
    .await;
    assert_eq!(value("2026-10-10").await, 0.0);
}

#[tokio::test]
async fn invalid_months_are_refused_and_extreme_ones_are_empty() {
    let app = App::new().await;
    for month in ["2026-13", "2026-1", "2026-10-01", "pas-un-mois", "202610"] {
        let response = app.response(&format!("/v1/months/{month}")).await;
        assert_eq!(response.status(), 400, "{month}");
    }
    for month in ["0001-01", "2026-02", "2028-02", "9999-12"] {
        let days = app.month(month).await["days"].as_array().unwrap().len();
        assert!((28..=31).contains(&days), "{month}");
    }
    assert_eq!(
        app.month("2028-02").await["days"].as_array().unwrap().len(),
        29
    );
}

#[tokio::test]
async fn the_month_requires_a_session() {
    let app = App::new().await;
    let anonymous = app
        .http
        .get(format!("{}/v1/months/2026-10", app.base))
        .send()
        .await
        .unwrap();
    assert_eq!(anonymous.status(), 401);
    let with_token = app
        .http
        .get(format!("{}/v1/months/2026-10", app.base))
        .bearer_auth(&app.lastfm)
        .send()
        .await
        .unwrap();
    assert_eq!(with_token.status(), 401);
}

#[tokio::test]
async fn each_day_of_a_year_is_the_same_day_as_in_its_month() {
    let app = App::new().await;
    let history = app.history_source().await;
    let today = app
        .sql_date("SELECT (now() AT TIME ZONE 'Europe/Paris')::date")
        .await;
    let this_year: i32 = today[..4].parse().unwrap();
    let last = this_year - 1;
    // L'an dernier : un historique avec un vrai zéro, un trou entre deux sources, une erreur, le
    // premier café ; Last.fm s'arrête et devient à vérifier, puis se tait (en attente).
    app.send_as(
        &history,
        vec![
            scrobble(&format!("{last}-03-01T10:00:00Z"), "A"),
            scrobble(&format!("{last}-03-02T10:00:00Z"), "B"),
            scrobble(&format!("{last}-03-04T10:00:00Z"), "C"),
        ],
    )
    .await;
    app.send(vec![
        scrobble(&format!("{last}-03-10T10:00:00Z"), "D"),
        scrobble(&format!("{last}-03-10T11:00:00Z"), "E"),
        broken(&format!("{last}-06-15T12:00:00Z")),
    ])
    .await;
    app.write_as_webapp(vec![cup(
        "premier",
        &format!("{last}-08-01T08:00:00+02:00"),
    )])
    .await;
    sqlx::query(
        "UPDATE core.sources SET last_seen_at = now() - interval '3 days' WHERE name = 'lastfm'",
    )
    .execute(&app.db.app)
    .await
    .unwrap();

    for year in [last, this_year] {
        let whole = app.year(year).await;
        assert_eq!(whole["year"], year);
        let days = whole["days"].as_array().unwrap();
        let leap = chrono::NaiveDate::from_ymd_opt(year, 2, 29).is_some();
        assert_eq!(days.len(), if leap { 366 } else { 365 });
        for month in 1..=12 {
            let month = format!("{year}-{month:02}");
            let alone = app.month(&month).await;
            assert_eq!(alone["metrics"], whole["metrics"]);
            assert_eq!(alone["today"], whole["today"]);
            let part: Vec<&Value> = days
                .iter()
                .filter(|d| d["date"].as_str().unwrap().starts_with(&month))
                .collect();
            let alone: Vec<&Value> = alone["days"].as_array().unwrap().iter().collect();
            assert_eq!(part, alone, "{month}");
        }
    }

    // De quoi comparer : chaque cas apparaît bien dans l'année.
    let whole = app.year(last).await;
    let day = |date: String| {
        whole["days"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["date"] == date.as_str())
            .unwrap()
            .clone()
    };
    assert_eq!(day(format!("{last}-03-03"))["values"]["music.listen"], 0.0);
    assert_eq!(
        day(format!("{last}-03-06"))["values"]["music.listen"],
        Value::Null
    );
    assert_eq!(day(format!("{last}-03-10"))["values"]["music.listen"], 2.0);
    assert_eq!(day(format!("{last}-06-15"))["errors"], 1);
    assert_eq!(
        day(format!("{last}-07-31"))["values"]["coffee"],
        Value::Null
    );
    assert_eq!(day(format!("{last}-08-02"))["values"]["coffee"], 0.0);
    assert_eq!(
        day(format!("{last}-03-20"))["to_check"],
        json!(["music.listen"])
    );
    assert_eq!(app.day(&today).await["pending"], json!(["lastfm"]));
}

#[tokio::test]
async fn a_future_or_malformed_year_is_refused() {
    let app = App::new().await;
    let today = app
        .sql_date("SELECT (now() AT TIME ZONE 'Europe/Paris')::date")
        .await;
    let next: i32 = today[..4].parse::<i32>().unwrap() + 1;
    for year in [
        next.to_string(),
        "9999".to_owned(),
        "26".to_owned(),
        "20266".to_owned(),
        "2026-01".to_owned(),
        "deux-mille".to_owned(),
    ] {
        let response = app.response(&format!("/v1/years/{year}")).await;
        assert_eq!(response.status(), 400, "{year}");
    }
    // Une année passée, même sans aucune donnée, se lit : tout y est « pas de données ».
    let old = app.year(1999).await;
    assert_eq!(old["days"].as_array().unwrap().len(), 365);
    assert!(
        old["days"]
            .as_array()
            .unwrap()
            .iter()
            .all(|d| d["values"]["coffee"].is_null())
    );
}

#[tokio::test]
async fn the_year_requires_a_session() {
    let app = App::new().await;
    let url = format!("{}/v1/years/2026", app.base);
    assert_eq!(app.http.get(&url).send().await.unwrap().status(), 401);
    let with_token = app
        .http
        .get(&url)
        .bearer_auth(&app.lastfm)
        .send()
        .await
        .unwrap();
    assert_eq!(with_token.status(), 401);
}
