//! Moteur d'interprétation et rejeu sur un vrai PostgreSQL : une base neuve par test, en tracker_app
//! pour le worker et en tracker_admin pour le rejeu.

mod common;
use common::*;
use life_tracker::engine::{self, Target, backoff, ensure_active, pass, pass_batch};
use life_tracker::interpreters::{
    Event, INTERPRETERS, InterpretError, Interpreter, Item, Output, Record,
};
use life_tracker::replay::{
    self, SwitchOutcome, begin_building, catch_up, retry_errors, try_switch,
};
use serde_json::{Value, json};
use sqlx::{PgPool, Row};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::io::copy_bidirectional;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

const SENTINEL: &str = "SENTINELfd38a1";
const KINDS: &[&str] = &["coffee.cup", "record.cancel"];

async fn drain(pool: &PgPool, registry: &[Interpreter]) {
    while pass(pool, registry, Target::Active).await.unwrap() > 0 {}
}

async fn active_id(db: &TestDb) -> i32 {
    sqlx::query_scalar("SELECT id FROM derived.generations WHERE state = 'active'")
        .fetch_one(&db.app)
        .await
        .unwrap()
}

async fn cursor(db: &TestDb, kind: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT c.last_seq FROM derived.cursors c
         JOIN derived.generations g ON g.id = c.generation AND g.state = 'active'
         WHERE c.kind = $1",
    )
    .bind(kind)
    .fetch_one(&db.app)
    .await
    .unwrap()
}

async fn errors(db: &TestDb) -> Vec<(i64, i32, String)> {
    sqlx::query(
        "SELECT e.seq, e.interpreter_version, e.error FROM derived.interpretation_errors e
         JOIN derived.generations g ON g.id = e.generation AND g.state = 'active' ORDER BY e.seq",
    )
    .fetch_all(&db.app)
    .await
    .unwrap()
    .iter()
    .map(|r| (r.get(0), r.get(1), r.get(2)))
    .collect()
}

async fn live_seqs(db: &TestDb) -> Vec<i64> {
    sqlx::query_scalar("SELECT seq FROM derived.live_events ORDER BY seq, ordinal")
        .fetch_all(&db.app)
        .await
        .unwrap()
}

fn cup(n: usize) -> String {
    format!("cup-{n}")
}

fn cancel_of(source: &str, key: &str) -> Value {
    json!({"source": source, "key": key})
}

// ── Critères de l'étape ─────────────────────────────────────────────────────

#[tokio::test]
async fn a_coffee_gives_one_event() {
    let db = TestDb::new().await;
    let (source, _) = db.source("ios", KINDS).await;
    let seq = db
        .insert(source, "coffee.cup", "c1", json!({"ignored": SENTINEL}))
        .await;
    drain(&db.app, INTERPRETERS).await;

    let rows = sqlx::query(
        "SELECT seq, ordinal, metric, source_id, occurred_at::text AS at, utc_offset_minutes, value::text AS value,
                details::text AS details, interpreter_version
         FROM derived.live_events",
    )
    .fetch_all(&db.app)
    .await
    .unwrap();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.get::<i64, _>("seq"), seq);
    assert_eq!(row.get::<i16, _>("ordinal"), 0);
    assert_eq!(row.get::<String, _>("metric"), "coffee");
    assert_eq!(row.get::<i16, _>("source_id"), source);
    assert!(
        row.get::<String, _>("at")
            .starts_with("2026-10-13 07:42:10")
    );
    assert_eq!(row.get::<Option<i16>, _>("utc_offset_minutes"), Some(120));
    assert_eq!(row.get::<Option<String>, _>("value"), None);
    assert_eq!(row.get::<String, _>("details"), "{}");
    assert_eq!(row.get::<i32, _>("interpreter_version"), 1);
    assert_eq!(cursor(&db, "coffee.cup").await, seq);
    assert!(errors(&db).await.is_empty());
}

#[tokio::test]
async fn a_cancellation_works_before_and_after_its_target() {
    let db = TestDb::new().await;
    let (source, _) = db.source("ios", KINDS).await;
    let kept = db.insert(source, "coffee.cup", "kept", json!({})).await;

    // La cible d'abord, interprétée, puis son annulation.
    db.insert(source, "coffee.cup", "target-first", json!({}))
        .await;
    drain(&db.app, INTERPRETERS).await;
    assert_eq!(live_seqs(&db).await.len(), 2);
    db.insert(
        source,
        "record.cancel",
        "x1",
        cancel_of("ios", "target-first"),
    )
    .await;
    drain(&db.app, INTERPRETERS).await;
    assert_eq!(live_seqs(&db).await, vec![kept]);

    // L'annulation d'abord, interprétée, puis sa cible.
    db.insert(source, "record.cancel", "x2", cancel_of("ios", "late"))
        .await;
    drain(&db.app, INTERPRETERS).await;
    assert_eq!(live_seqs(&db).await, vec![kept]);
    db.insert(source, "coffee.cup", "late", json!({})).await;
    drain(&db.app, INTERPRETERS).await;
    assert_eq!(live_seqs(&db).await, vec![kept]);

    // Le fait reste stocké : c'est la vue qui l'écarte.
    assert_eq!(db.count("SELECT count(*) FROM derived.events").await, 3);
}

/// Une source munie du droit `record.cancel` annule n'importe quelle source, nommée par son nom.
#[tokio::test]
async fn a_cancellation_targets_source_and_key_across_sources() {
    let db = TestDb::new().await;
    let (ios, _) = db.source("ios", &["coffee.cup"]).await;
    let (other, _) = db.source("other", &["coffee.cup"]).await;
    let (web, _) = db.source("web", &["record.cancel"]).await;
    db.insert(ios, "coffee.cup", "same-key", json!({})).await;
    let kept = db.insert(other, "coffee.cup", "same-key", json!({})).await;
    db.insert(web, "record.cancel", "x", cancel_of("ios", "same-key"))
        .await;
    drain(&db.app, INTERPRETERS).await;
    assert_eq!(live_seqs(&db).await, vec![kept]);
    assert_eq!(
        db.count("SELECT count(*) FROM derived.cancellations WHERE target_key = 'same-key'")
            .await,
        1
    );

    // Une révocation ne change pas l'histoire : le rejeu ne dépend pas des droits actuels.
    life_tracker::admin::revoke_kind(&db.admin, "web", "record.cancel")
        .await
        .unwrap();
    let report = replay::rebuild(&db.admin, INTERPRETERS, false)
        .await
        .unwrap();
    assert!(report.switched);
    assert_eq!(live_seqs(&db).await, vec![kept]);
}

/// Limite assumée : annuler une annulation ne rétablit pas sa cible. On saisit de nouveau la donnée.
#[tokio::test]
async fn cancelling_a_cancellation_has_no_effect() {
    let db = TestDb::new().await;
    let (source, _) = db.source("ios", KINDS).await;
    db.insert(source, "coffee.cup", "c", json!({})).await;
    db.insert(source, "record.cancel", "x", cancel_of("ios", "c"))
        .await;
    db.insert(source, "record.cancel", "y", cancel_of("ios", "x"))
        .await;
    drain(&db.app, INTERPRETERS).await;
    assert!(live_seqs(&db).await.is_empty());
}

#[tokio::test]
async fn a_record_in_error_does_not_stop_the_following_ones() {
    let db = TestDb::new().await;
    let (source, _) = db.source("ios", KINDS).await;
    let cases = [
        json!({}),
        json!(SENTINEL),
        json!({"source": SENTINEL}),
        json!({"key": SENTINEL}),
        json!({"source": 3, "key": SENTINEL}),
        json!({"source": SENTINEL, "key": 3}),
        json!({"source": "", "key": SENTINEL}),
        json!({"source": SENTINEL, "key": ""}),
        json!([SENTINEL]),
    ];
    let mut bad = Vec::new();
    for (n, payload) in cases.iter().enumerate() {
        bad.push(
            db.insert(
                source,
                "record.cancel",
                &format!("bad-{n}"),
                payload.clone(),
            )
            .await,
        );
    }
    let unknown = db
        .insert(source, "record.cancel", "unknown", cancel_of(SENTINEL, "k"))
        .await;
    let coffee = db.insert(source, "coffee.cup", "c", json!({})).await;
    let good = db
        .insert(source, "record.cancel", "good", cancel_of("ios", "absent"))
        .await;
    drain(&db.app, INTERPRETERS).await;

    let stored = errors(&db).await;
    assert_eq!(stored.len(), cases.len() + 1);
    assert!(stored.iter().all(|(_, version, _)| *version == 1));
    assert!(
        stored
            .iter()
            .all(|(_, _, message)| !message.contains(SENTINEL))
    );
    assert_eq!(stored.last().unwrap().0, unknown);
    assert_eq!(stored.last().unwrap().2, "unknown target source");
    assert_eq!(cursor(&db, "record.cancel").await, good);
    assert_eq!(live_seqs(&db).await, vec![coffee]);
    assert_eq!(
        db.count("SELECT count(*) FROM derived.cancellations").await,
        1
    );
    // Pas de nouvel essai automatique : un tour de plus ne change rien.
    drain(&db.app, INTERPRETERS).await;
    assert_eq!(errors(&db).await, stored);
}

#[tokio::test]
async fn an_unknown_schema_version_is_an_error_not_a_crash_nor_a_skip() {
    let db = TestDb::new().await;
    let (source, _) = db.source("ios", KINDS).await;
    let old = db
        .insert_version(source, "coffee.cup", "v2", json!({"s": SENTINEL}), 2)
        .await;
    let next = db.insert(source, "coffee.cup", "v1", json!({})).await;
    drain(&db.app, INTERPRETERS).await;
    assert_eq!(
        errors(&db).await,
        vec![(old, 1, "unsupported schema_version 2".to_owned())]
    );
    assert_eq!(live_seqs(&db).await, vec![next]);
}

#[tokio::test]
async fn purged_records_produce_nothing() {
    let db = TestDb::new().await;
    let (source, _) = db.source("ios", KINDS).await;
    let purged = db.insert(source, "coffee.cup", "p", json!({})).await;
    let cancel = db
        .insert(source, "record.cancel", "x", json!("malformed"))
        .await;
    let kept = db.insert(source, "coffee.cup", "k", json!({})).await;
    db.purge(purged).await;
    db.purge(cancel).await;
    drain(&db.app, INTERPRETERS).await;
    assert_eq!(live_seqs(&db).await, vec![kept]);
    assert!(errors(&db).await.is_empty(), "ni événement, ni erreur");
    assert_eq!(cursor(&db, "coffee.cup").await, kept);
    assert_eq!(cursor(&db, "record.cancel").await, cancel);
}

// ── Reprise, atomicité, concurrence ─────────────────────────────────────────

#[tokio::test]
async fn a_restart_resumes_at_the_right_cursor() {
    let db = TestDb::new().await;
    let (source, _) = db.source("ios", KINDS).await;
    let mut seqs = Vec::new();
    for n in 0..5 {
        seqs.push(db.insert(source, "coffee.cup", &cup(n), json!({})).await);
    }

    // Le premier tour crée la génération ; deux tours traitent ensuite deux enregistrements.
    pass(&db.app, INTERPRETERS, Target::Active).await.unwrap();
    pass(&db.app, INTERPRETERS, Target::Active).await.unwrap();
    pass(&db.app, INTERPRETERS, Target::Active).await.unwrap();
    assert_eq!(cursor(&db, "coffee.cup").await, seqs[1]);
    assert_eq!(live_seqs(&db).await, seqs[..2]);

    // « Redémarrage » : un autre pool, sans aucune mémoire de ce qui précède.
    let restarted = life_tracker::db::pool(&db.app_url()).unwrap();
    drain(&restarted, INTERPRETERS).await;
    assert_eq!(live_seqs(&db).await, seqs);
    assert_eq!(cursor(&db, "coffee.cup").await, seqs[4]);
    assert!(errors(&db).await.is_empty());
}

/// La panne survient après l'écriture des événements, au moment d'avancer le curseur (un trigger
/// jetable, dans la base de ce seul test). Rien ne doit rester, et ce n'est pas une erreur d'interprétation.
#[tokio::test]
async fn a_failure_before_commit_leaves_no_partial_output() {
    let db = TestDb::new().await;
    let (source, _) = db.source("ios", KINDS).await;
    let first = db.insert(source, "coffee.cup", "a", json!({})).await;
    let second = db.insert(source, "coffee.cup", "b", json!({})).await;
    TestDb::exec(
        &db.owner,
        &format!(
            "CREATE FUNCTION derived.inject() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN RAISE EXCEPTION 'injected' USING ERRCODE = '57P03'; END $$;
         CREATE TRIGGER inject BEFORE UPDATE ON derived.cursors FOR EACH ROW
           WHEN (NEW.last_seq = {second}) EXECUTE FUNCTION derived.inject();"
        ),
    )
    .await;

    ensure_active(&db.app, INTERPRETERS).await.unwrap();
    pass(&db.app, INTERPRETERS, Target::Active).await.unwrap();
    let failed = pass(&db.app, INTERPRETERS, Target::Active).await;
    assert!(failed.is_err(), "la panne remonte au worker");
    assert_eq!(
        live_seqs(&db).await,
        vec![first],
        "aucun événement du second"
    );
    assert_eq!(cursor(&db, "coffee.cup").await, first);
    assert!(errors(&db).await.is_empty());

    sqlx::query("DROP TRIGGER inject ON derived.cursors")
        .execute(&db.owner)
        .await
        .unwrap();
    drain(&db.app, INTERPRETERS).await;
    assert_eq!(live_seqs(&db).await, vec![first, second]);
    assert!(errors(&db).await.is_empty());
}

/// Le lot du worker : tout ou rien par enregistrement, et une panne reprend tout le lot au curseur.
#[tokio::test]
async fn a_batch_isolates_each_record_and_a_failure_restarts_from_the_cursor() {
    let db = TestDb::new().await;
    let (source, _) = db.source("ios", KINDS).await;
    let mut seqs = Vec::new();
    for key in ["a", "b", "c", "d"] {
        seqs.push(db.insert(source, "coffee.cup", key, json!({})).await);
    }
    let bad = db
        .insert(source, "record.cancel", "x", json!({"s": SENTINEL}))
        .await;
    let last = seqs[3];
    TestDb::exec(
        &db.owner,
        &format!(
            "CREATE FUNCTION derived.inject() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN RAISE EXCEPTION 'injected' USING ERRCODE = '57P03'; END $$;
         CREATE TRIGGER inject BEFORE UPDATE ON derived.cursors FOR EACH ROW
           WHEN (NEW.last_seq = {last}) EXECUTE FUNCTION derived.inject();"
        ),
    )
    .await;
    ensure_active(&db.app, INTERPRETERS).await.unwrap();
    assert!(
        pass_batch(&db.app, INTERPRETERS, Target::Active, 100)
            .await
            .is_err()
    );
    assert_eq!(
        db.count("SELECT count(*) FROM derived.cursors WHERE last_seq > 0")
            .await,
        0,
        "le curseur n'a pas bougé"
    );
    assert_eq!(db.count("SELECT count(*) FROM derived.events").await, 0);

    sqlx::query("DROP TRIGGER inject ON derived.cursors")
        .execute(&db.owner)
        .await
        .unwrap();
    assert!(
        pass_batch(&db.app, INTERPRETERS, Target::Active, 3)
            .await
            .unwrap()
            >= 3
    );
    assert_eq!(
        live_seqs(&db).await,
        seqs[..3],
        "la limite du lot est tenue"
    );
    while pass_batch(&db.app, INTERPRETERS, Target::Active, 3)
        .await
        .unwrap()
        > 0
    {}
    assert_eq!(live_seqs(&db).await, seqs);
    assert_eq!(cursor(&db, "coffee.cup").await, last);
    let stored = errors(&db).await;
    assert_eq!(stored.len(), 1, "l'erreur n'arrête pas le reste");
    assert_eq!(stored[0].0, bad);
}

fn test_interpreter(interpret: fn(&Record) -> Result<Vec<Output>, InterpretError>) -> Interpreter {
    Interpreter {
        kind: "lastfm.track_info",
        version: 7,
        schema_version: 1,
        interpret,
    }
}

fn track(record: &Record, metric: &'static str, level: &'static str) -> Output {
    Output::Event(Event {
        value: Some("215.5".to_owned()),
        details: json!({"d": 1}),
        items: vec![Item {
            level,
            position: 0,
            item_key: "a::t".to_owned(),
            display: "A - T".to_owned(),
        }],
        ..Event::at_envelope(record, metric)
    })
}

/// Le second événement est refusé par la base (métrique inconnue) : le premier ne reste pas, et
/// l'enregistrement passe en erreur avec le code seul.
#[tokio::test]
async fn a_record_gives_all_its_events_or_none() {
    let db = TestDb::new().await;
    let (source, _) = db.source("lastfm", &["lastfm.track_info"]).await;
    let registry = [test_interpreter(|r| {
        Ok(vec![
            track(r, "music.track_duration", "track"),
            track(r, "no.such.metric", "track"),
        ])
    })];
    let bad = db
        .insert(source, "lastfm.track_info", "t", json!({"s": SENTINEL}))
        .await;
    drain(&db.app, &registry).await;
    assert_eq!(db.count("SELECT count(*) FROM derived.events").await, 0);
    assert_eq!(
        db.count("SELECT count(*) FROM derived.event_items").await,
        0
    );
    assert_eq!(
        errors(&db).await,
        vec![(bad, 7, "output refused by the database (23503)".to_owned())]
    );
    assert_eq!(cursor(&db, "lastfm.track_info").await, bad);
}

/// Principe 12 : un nouveau type, c'est une entrée de plus. Ses éléments, sa valeur et l'historique
/// déjà présent dans le journal suivent sans rien changer au moteur.
#[tokio::test]
async fn a_new_interpreter_is_one_entry_and_catches_up_on_the_journal() {
    let db = TestDb::new().await;
    let (source, _) = db
        .source("mixed", &["coffee.cup", "lastfm.track_info"])
        .await;
    let old_track = db
        .insert(source, "lastfm.track_info", "before", json!({}))
        .await;
    db.insert(source, "coffee.cup", "c", json!({})).await;
    // Le registre d'avant l'interprète : le café et les annulations.
    drain(&db.app, &INTERPRETERS[..2]).await;
    assert_eq!(
        live_seqs(&db).await.len(),
        1,
        "sans interprète : en attente, sans erreur"
    );
    assert!(errors(&db).await.is_empty());

    let registry = [
        INTERPRETERS[0],
        INTERPRETERS[1],
        test_interpreter(|r| Ok(vec![track(r, "music.track_duration", "track")])),
    ];
    drain(&db.app, &registry).await;
    let row = sqlx::query(
        "SELECT e.value::text AS value, e.details::text AS details, e.interpreter_version,
                i.level, i.item_key, i.display, i.metric
         FROM derived.live_events e JOIN derived.live_event_items i USING (generation, seq, ordinal)",
    )
    .fetch_one(&db.app)
    .await
    .unwrap();
    assert_eq!(row.get::<String, _>("value"), "215.5");
    assert_eq!(row.get::<i32, _>("interpreter_version"), 7);
    assert_eq!(row.get::<String, _>("level"), "track");
    assert_eq!(row.get::<String, _>("item_key"), "a::t");
    assert_eq!(row.get::<String, _>("metric"), "music.track_duration");
    assert_eq!(cursor(&db, "lastfm.track_info").await, old_track);
}

#[tokio::test]
async fn a_panicking_interpreter_is_an_error_and_the_engine_continues() {
    let db = TestDb::new().await;
    let (source, _) = db.source("lastfm", &["lastfm.track_info"]).await;
    let registry = [test_interpreter(|r| {
        if r.payload.get("boom").is_some() {
            panic!("boom");
        }
        Ok(vec![])
    })];
    let boom = db
        .insert(source, "lastfm.track_info", "a", json!({"boom": 1}))
        .await;
    let fine = db.insert(source, "lastfm.track_info", "b", json!({})).await;
    drain(&db.app, &registry).await;
    assert_eq!(
        errors(&db).await,
        vec![(boom, 7, "interpreter panicked".to_owned())]
    );
    assert_eq!(cursor(&db, "lastfm.track_info").await, fine);
}

#[tokio::test]
async fn concurrent_workers_do_not_double_produce() {
    let db = TestDb::new().await;
    let (source, _) = db.source("ios", KINDS).await;
    const COUNT: usize = 300;
    for n in 0..COUNT {
        db.insert(source, "coffee.cup", &cup(n), json!({})).await;
    }
    let mut workers = Vec::new();
    for _ in 0..4 {
        let pool = life_tracker::db::pool(&db.app_url()).unwrap();
        workers.push(tokio::spawn(async move {
            let mut processed = 0;
            loop {
                let progress = pass(&pool, INTERPRETERS, Target::Active).await.unwrap();
                if progress == 0 {
                    return processed;
                }
                processed += progress;
            }
        }));
    }
    let mut totals = Vec::new();
    for worker in workers {
        totals.push(worker.await.unwrap());
    }
    assert!(
        totals.iter().filter(|t| **t > 10).count() >= 2,
        "{totals:?}"
    );
    // Le dernier arrivé peut avoir vu le journal vide : il ne reste rien à faire.
    drain(&db.app, INTERPRETERS).await;
    assert_eq!(
        db.count("SELECT count(*) FROM derived.live_events").await,
        COUNT as i64
    );
    assert_eq!(
        db.count("SELECT count(DISTINCT seq) FROM derived.live_events")
            .await,
        COUNT as i64
    );
    assert!(errors(&db).await.is_empty(), "aucune collision déguisée");
}

#[tokio::test]
async fn concurrent_starters_establish_one_active_generation() {
    let db = TestDb::new().await;
    let mut starters = Vec::new();
    for _ in 0..8 {
        let pool = life_tracker::db::pool(&db.app_url()).unwrap();
        starters.push(tokio::spawn(async move {
            ensure_active(&pool, INTERPRETERS).await.unwrap();
            pass(&pool, INTERPRETERS, Target::Active).await.unwrap();
        }));
    }
    for starter in starters {
        starter.await.unwrap();
    }
    assert_eq!(
        db.count("SELECT count(*) FROM derived.generations").await,
        1
    );
    let versions: String =
        sqlx::query_scalar("SELECT interpreter_versions::text FROM derived.generations")
            .fetch_one(&db.app)
            .await
            .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&versions).unwrap(),
        json!({"coffee.cup": 1, "record.cancel": 1, "lastfm.scrobble": 2, "lastfm.track_info": 2,
               "music.mark_known": 2, "apple.play_history_row": 2, "garmin.daily_summary": 1,
               "garmin.sleep": 2, "garmin.activity": 1, "garmin.activity_track": 1,
               "youtube.watch": 1, "youtube.video_info": 1, "twitch.watch": 1,
               "steam.playtime": 1, "steam.achievement": 1,
               "overland.location": 1, "overland.visit": 1})
    );
}

/// Proxy TCP vers le PostgreSQL de test : l'éteindre rend la base indisponible pour de vrai.
struct Proxy {
    port: u16,
    target: String,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    alive: Arc<AtomicBool>,
}

impl Proxy {
    async fn start(target: String, port: u16) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let alive = Arc::new(AtomicBool::new(true));
        let flag = alive.clone();
        let upstream = target.clone();
        let task = tokio::spawn(async move {
            while let Ok((mut client, _)) = listener.accept().await {
                let upstream = upstream.clone();
                let flag = flag.clone();
                tokio::spawn(async move {
                    let mut server = TcpStream::connect(&upstream).await.unwrap();
                    tokio::select! {
                        _ = copy_bidirectional(&mut client, &mut server) => {}
                        _ = async {
                            while flag.load(Ordering::Relaxed) {
                                tokio::time::sleep(Duration::from_millis(20)).await;
                            }
                        } => {}
                    }
                });
            }
        });
        Self {
            port,
            target,
            tasks: vec![task],
            alive,
        }
    }

    async fn stop(self) -> (String, u16) {
        self.alive.store(false, Ordering::Relaxed);
        for task in &self.tasks {
            task.abort();
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        (self.target, self.port)
    }
}

#[tokio::test]
async fn an_outage_retries_and_never_records_an_interpretation_error() {
    let db = TestDb::new().await;
    let (source, _) = db.source("ios", KINDS).await;
    let first = db.insert(source, "coffee.cup", "a", json!({})).await;

    let direct = db.app_url();
    let authority = direct.split('@').nth(1).unwrap().split('/').next().unwrap();
    let proxy = Proxy::start(authority.to_owned(), 0).await;
    let through_proxy = direct.replace(authority, &format!("127.0.0.1:{}", proxy.port));
    let pool = life_tracker::db::pool(&through_proxy).unwrap();
    let (stop, stopped) = watch::channel(false);
    let worker = tokio::spawn(engine::run(
        pool.clone(),
        INTERPRETERS,
        Duration::from_millis(20),
        stopped,
    ));

    async fn wait_for(db: &TestDb, seq: i64) {
        for _ in 0..600 {
            if db.count("SELECT count(*) FROM derived.live_events").await > 0
                && live_seqs(db).await.contains(&seq)
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("{seq} jamais interprété");
    }
    wait_for(&db, first).await;

    let (target, port) = proxy.stop().await;
    let during = db.insert(source, "coffee.cup", "b", json!({})).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(live_seqs(&db).await, vec![first], "base injoignable");
    assert!(errors(&db).await.is_empty());

    let proxy = Proxy::start(target, port).await;
    wait_for(&db, during).await;
    assert!(errors(&db).await.is_empty());

    stop.send_replace(true);
    tokio::time::timeout(Duration::from_secs(10), worker)
        .await
        .expect("arrêt propre")
        .unwrap();
    proxy.stop().await;
}

#[test]
fn the_delay_grows_up_to_a_minute() {
    let delays: Vec<u64> = (1..=9).map(|n| backoff(n).as_secs()).collect();
    assert_eq!(delays, [1, 2, 4, 8, 16, 32, 60, 60, 60]);
}

#[tokio::test]
async fn the_registry_matches_the_declared_kinds() {
    let db = TestDb::new().await;
    for interpreter in INTERPRETERS {
        let current: i16 =
            sqlx::query_scalar("SELECT current_schema_version FROM core.kinds WHERE kind = $1")
                .bind(interpreter.kind)
                .fetch_one(&db.app)
                .await
                .unwrap();
        assert_eq!(current, interpreter.schema_version, "{}", interpreter.kind);
    }
}

// ── retry-errors ────────────────────────────────────────────────────────────

#[tokio::test]
async fn retry_errors_clears_what_succeeds_and_keeps_the_rest() {
    let db = TestDb::new().await;
    let (source, _) = db.source("ios", KINDS).await;
    let target = db.insert(source, "coffee.cup", "target", json!({})).await;
    // Échoue tant que la source visée n'existe pas : se corrige sans changer d'interprète.
    let fixable = db
        .insert(source, "record.cancel", "x1", cancel_of("later", "k"))
        .await;
    let malformed = db
        .insert(source, "record.cancel", "x2", json!({"bad": true}))
        .await;
    let purged = db
        .insert(source, "record.cancel", "x3", json!({"bad": true}))
        .await;
    let cancel_target = db
        .insert(source, "record.cancel", "x4", cancel_of("nope", "target"))
        .await;
    drain(&db.app, INTERPRETERS).await;
    assert_eq!(errors(&db).await.len(), 4);
    let cursor_before = cursor(&db, "record.cancel").await;

    let (later, _) = db.source("later", &["coffee.cup"]).await;
    // Rédaction sans passer par le nettoyage : l'erreur existe encore quand la purge est arrivée.
    let purge: i64 = sqlx::query_scalar(
        "INSERT INTO core.purges (selector, reason, record_count) VALUES ('{}', 't', 1) RETURNING id",
    )
    .fetch_one(&db.admin)
    .await
    .unwrap();
    sqlx::query("UPDATE core.records SET payload = NULL, purge_id = $1 WHERE seq = $2")
        .bind(purge)
        .bind(purged)
        .execute(&db.admin)
        .await
        .unwrap();

    let report = retry_errors(&db.admin, INTERPRETERS).await.unwrap();
    assert_eq!(report.cleared, vec![fixable, purged]);
    assert_eq!(report.kept, vec![malformed, cancel_target]);
    let left: Vec<i64> = errors(&db).await.iter().map(|e| e.0).collect();
    assert_eq!(left, vec![malformed, cancel_target]);
    assert_eq!(
        sqlx::query_scalar::<_, i16>(
            "SELECT target_source_id FROM derived.cancellations WHERE cancel_seq = $1"
        )
        .bind(fixable)
        .fetch_one(&db.app)
        .await
        .unwrap(),
        later
    );
    assert_eq!(
        cursor(&db, "record.cancel").await,
        cursor_before,
        "le curseur n'a pas à bouger"
    );
    assert_eq!(live_seqs(&db).await, vec![target]);

    // La rebuild retrouve le même état : retry-errors n'a rien produit d'autre qu'un vrai rejeu.
    let report = replay::rebuild(&db.admin, INTERPRETERS, false)
        .await
        .unwrap();
    assert!(report.switched, "{:?}", report.comparison.differences);
    let again = retry_errors(&db.admin, INTERPRETERS).await.unwrap();
    assert!(again.cleared.is_empty());
    assert_eq!(again.kept, vec![malformed, cancel_target]);
}

// ── rebuild ─────────────────────────────────────────────────────────────────

/// Un peu de tout : café, annulation avant et après, erreur, purge.
async fn seed_mixed(db: &TestDb) -> i16 {
    let (source, _) = db.source("ios", KINDS).await;
    db.insert(source, "coffee.cup", "c1", json!({})).await;
    db.insert(source, "coffee.cup", "c2", json!({})).await;
    db.insert(source, "record.cancel", "x1", cancel_of("ios", "c2"))
        .await;
    db.insert(source, "record.cancel", "x2", cancel_of("ios", "c3"))
        .await;
    db.insert(source, "coffee.cup", "c3", json!({})).await;
    db.insert(source, "record.cancel", "bad", json!("malformed"))
        .await;
    let purged = db.insert(source, "coffee.cup", "p", json!({})).await;
    db.purge(purged).await;
    source
}

async fn core_fingerprint(db: &TestDb) -> String {
    sqlx::query_scalar(
        "SELECT (SELECT md5(string_agg(r::text, ',' ORDER BY seq)) FROM core.records r)
             || (SELECT md5(string_agg(s::text, ',' ORDER BY id)) FROM core.sources s)",
    )
    .fetch_one(&db.app)
    .await
    .unwrap()
}

#[tokio::test]
async fn a_rebuild_produces_an_identical_generation() {
    let db = TestDb::new().await;
    seed_mixed(&db).await;
    drain(&db.app, INTERPRETERS).await;
    let (old, core_before, live_before) = (
        active_id(&db).await,
        core_fingerprint(&db).await,
        live_seqs(&db).await,
    );

    let report = replay::rebuild(&db.admin, INTERPRETERS, false)
        .await
        .unwrap();
    assert!(report.switched);
    assert_eq!(report.comparison.differences, vec![], "règle 16");
    assert!(
        report
            .comparison
            .compared_up_to
            .iter()
            .filter(|(kind, _)| KINDS.contains(&kind.as_str()))
            .all(|(_, seq)| *seq > 0),
        "la comparaison a bien porté sur quelque chose : {:?}",
        report.comparison.compared_up_to
    );
    assert_ne!(report.generation, old);
    assert_eq!(active_id(&db).await, report.generation);
    assert_eq!(live_seqs(&db).await, live_before);
    assert_eq!(
        errors(&db).await.len(),
        1,
        "l'erreur est reproduite, pas perdue"
    );

    // L'ancienne génération est supprimée, ses lignes parties en cascade.
    assert_eq!(
        db.count("SELECT count(*) FROM derived.generations").await,
        1
    );
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM derived.events WHERE generation = {old}"
        ))
        .await,
        0
    );
    // Règle 14 : un rejeu ne touche à rien d'autre que derived.
    assert_eq!(core_fingerprint(&db).await, core_before);
}

/// La comparaison n'est pas vide de sens : chaque altération de chaque table la fait réagir, et
/// sans le drapeau rien ne bascule. Les horodatages et la version d'interprète, eux, sont ignorés.
#[tokio::test]
async fn the_comparison_detects_a_difference_in_every_table() {
    let db = TestDb::new().await;
    let source = seed_mixed(&db).await;
    let with_items = [
        INTERPRETERS[0],
        INTERPRETERS[1],
        test_interpreter(|r| Ok(vec![track(r, "music.track_duration", "track")])),
    ];
    sqlx::query("INSERT INTO core.source_kinds (source_id, kind) VALUES ($1, 'lastfm.track_info')")
        .bind(source)
        .execute(&db.owner)
        .await
        .unwrap();
    db.insert(source, "lastfm.track_info", "t", json!({})).await;
    drain(&db.app, &with_items).await;
    let original = active_id(&db).await;

    let identical = replay::rebuild(&db.admin, &with_items, false)
        .await
        .unwrap();
    assert!(identical.switched);
    let active = identical.generation;

    let track_seq: i64 = sqlx::query_scalar("SELECT max(seq) FROM core.records")
        .fetch_one(&db.app)
        .await
        .unwrap();
    let coffee: i64 = sqlx::query_scalar("SELECT min(seq) FROM core.records WHERE key = 'c1'")
        .fetch_one(&db.app)
        .await
        .unwrap();
    assert_ne!(active, original);

    let g = active;
    let alterations: Vec<(&str, i64, String, String)> = vec![
        (
            "events",
            coffee,
            format!(
                "UPDATE derived.events SET value = 2 WHERE generation = {g} AND seq = {coffee}"
            ),
            format!(
                "UPDATE derived.events SET value = NULL WHERE generation = {g} AND seq = {coffee}"
            ),
        ),
        (
            "events",
            coffee,
            format!(
                "UPDATE derived.events SET details = '{{\"x\":1}}' WHERE generation = {g} AND seq = {coffee}"
            ),
            format!(
                "UPDATE derived.events SET details = '{{}}' WHERE generation = {g} AND seq = {coffee}"
            ),
        ),
        (
            "event_items",
            track_seq,
            format!(
                "UPDATE derived.event_items SET display = 'other' WHERE generation = {g} AND seq = {track_seq}"
            ),
            format!(
                "UPDATE derived.event_items SET display = 'A - T' WHERE generation = {g} AND seq = {track_seq}"
            ),
        ),
        (
            "event_items",
            track_seq,
            format!(
                "UPDATE derived.event_items SET position = 1 WHERE generation = {g} AND seq = {track_seq}"
            ),
            format!(
                "UPDATE derived.event_items SET position = 0 WHERE generation = {g} AND seq = {track_seq}"
            ),
        ),
        (
            "cancellations",
            0,
            format!(
                "UPDATE derived.cancellations SET target_key = 'other' WHERE generation = {g} AND target_key = 'c2'"
            ),
            format!(
                "UPDATE derived.cancellations SET target_key = 'c2' WHERE generation = {g} AND target_key = 'other'"
            ),
        ),
        (
            "interpretation_errors",
            0,
            format!(
                "UPDATE derived.interpretation_errors SET error = 'other' WHERE generation = {g}"
            ),
            format!(
                "UPDATE derived.interpretation_errors SET error = 'record.cancel: invalid source' WHERE generation = {g}"
            ),
        ),
    ];
    for (table, seq, alter, restore) in alterations {
        TestDb::exec(&db.admin, &alter).await;
        let report = replay::rebuild(&db.admin, &with_items, false)
            .await
            .unwrap();
        assert!(!report.switched, "{alter}");
        assert_eq!(active_id(&db).await, g, "pas de bascule : {alter}");
        let found = &report.comparison.differences;
        assert!(!found.is_empty(), "{alter}");
        assert!(found.iter().all(|d| d.table == table), "{found:?}");
        assert!(seq == 0 || found.iter().any(|d| d.seq == seq), "{found:?}");
        TestDb::exec(&db.admin, &restore).await;
    }

    // Une ligne en trop ou manquante se voit aussi, dans les deux sens.
    TestDb::exec(
        &db.admin,
        &format!("DELETE FROM derived.events WHERE generation = {g} AND seq = {coffee}"),
    )
    .await;
    let report = replay::rebuild(&db.admin, &with_items, false)
        .await
        .unwrap();
    assert_eq!(report.comparison.differences.len(), 1);
    assert_eq!(report.comparison.differences[0].seq, coffee);
    // Le nom de génération, les horodatages et la version d'interprète ne comptent pas.
    let touched = replay::rebuild(&db.admin, &with_items, true).await.unwrap();
    assert!(touched.switched, "--allow-differences bascule quand même");
    assert_eq!(touched.comparison.differences.len(), 1);
    let g = touched.generation;
    assert_eq!(active_id(&db).await, g);
    TestDb::exec(
        &db.admin,
        &format!("UPDATE derived.events SET interpreter_version = 99 WHERE generation = {g};"),
    )
    .await;
    TestDb::exec(&db.admin, &format!(
        "UPDATE derived.interpretation_errors SET failed_at = now() - interval '1 day', interpreter_version = 99 WHERE generation = {g}"
    )).await;
    let report = replay::rebuild(&db.admin, &with_items, false)
        .await
        .unwrap();
    assert_eq!(report.comparison.differences, vec![]);
    assert!(report.switched);
}

/// Une nouvelle version d'interprète qui change le résultat : signalée, bascule seulement sur demande.
#[tokio::test]
async fn a_new_interpreter_version_needs_the_explicit_flag() {
    let db = TestDb::new().await;
    let (source, _) = db.source("ios", KINDS).await;
    let seq = db.insert(source, "coffee.cup", "c", json!({})).await;
    drain(&db.app, INTERPRETERS).await;
    let old = active_id(&db).await;

    let v2 = [
        Interpreter {
            kind: "coffee.cup",
            version: 2,
            schema_version: 1,
            interpret: |r| {
                Ok(vec![Output::Event(Event {
                    value: Some("1".to_owned()),
                    ..Event::at_envelope(r, "coffee")
                })])
            },
        },
        INTERPRETERS[1],
    ];
    let refused = replay::rebuild(&db.admin, &v2, false).await.unwrap();
    assert!(!refused.switched);
    assert_eq!(refused.comparison.differences.len(), 1);
    assert_eq!(refused.comparison.differences[0].seq, seq);
    assert_eq!(active_id(&db).await, old);
    assert_eq!(
        db.count("SELECT count(*) FROM derived.generations WHERE state = 'building'")
            .await,
        1,
        "laissée à examiner"
    );

    let accepted = replay::rebuild(&db.admin, &v2, true).await.unwrap();
    assert!(accepted.switched);
    let row = sqlx::query(
        "SELECT e.interpreter_version, g.interpreter_versions::text AS versions
         FROM derived.live_events e JOIN derived.generations g ON g.id = e.generation",
    )
    .fetch_one(&db.app)
    .await
    .unwrap();
    assert_eq!(row.get::<i32, _>("interpreter_version"), 2);
    let versions: Value = serde_json::from_str(&row.get::<String, _>("versions")).unwrap();
    assert_eq!(versions["coffee.cup"], 2);
    assert_eq!(
        db.count("SELECT count(*) FROM derived.generations").await,
        1
    );
}

#[tokio::test]
async fn a_crashed_rebuild_is_replaced_and_a_running_one_is_respected() {
    let db = TestDb::new().await;
    let (source, _) = db.source("ios", KINDS).await;
    let seq = db.insert(source, "coffee.cup", "c", json!({})).await;

    // Sans active, rien à comparer : rebuild refuse plutôt que de créer un calendrier à lui seul.
    let none = replay::rebuild(&db.admin, INTERPRETERS, false)
        .await
        .unwrap_err();
    assert!(
        none.to_string().contains("aucune génération active"),
        "{none}"
    );

    drain(&db.app, INTERPRETERS).await;
    // Reste d'une reconstruction interrompue, avec une ligne fausse.
    let crashed = begin_building(&db.admin, INTERPRETERS).await.unwrap();
    sqlx::query(
        "INSERT INTO derived.events (generation, seq, ordinal, metric, source_id, occurred_at, interpreter_version)
         VALUES ($1, $2, 0, 'coffee', $3, now(), 1)",
    )
    .bind(crashed)
    .bind(seq)
    .bind(source)
    .execute(&db.admin)
    .await
    .unwrap();
    assert_eq!(live_seqs(&db).await, vec![seq]);

    // Une rebuild tient le verrou : la seconde s'efface sans toucher à sa génération.
    let mut holder = db.admin.acquire().await.unwrap();
    sqlx::query("SELECT pg_advisory_lock(7461002)")
        .execute(&mut *holder)
        .await
        .unwrap();
    let busy = replay::rebuild(&db.admin, INTERPRETERS, false)
        .await
        .unwrap_err();
    assert!(busy.to_string().contains("déjà en cours"), "{busy}");
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM derived.generations WHERE id = {crashed} AND state = 'building'"
        ))
        .await,
        1
    );
    sqlx::query("SELECT pg_advisory_unlock(7461002)")
        .execute(&mut *holder)
        .await
        .unwrap();
    drop(holder);

    let report = replay::rebuild(&db.admin, INTERPRETERS, false)
        .await
        .unwrap();
    assert!(report.switched);
    assert_ne!(report.generation, crashed);
    assert_eq!(report.comparison.differences, vec![]);
    assert_eq!(
        db.count("SELECT count(*) FROM derived.generations").await,
        1
    );
}

/// Pendant la construction, le calendrier lit l'ancienne génération. Si le worker prend de l'avance,
/// la bascule refuse, et un enregistrement arrivé juste après la bascule n'est pas perdu.
#[tokio::test]
async fn the_switch_is_correct_when_records_arrive_during_the_rebuild() {
    let db = TestDb::new().await;
    let (source, _) = db.source("ios", KINDS).await;
    let first = db.insert(source, "coffee.cup", "a", json!({})).await;
    drain(&db.app, INTERPRETERS).await;
    let old = active_id(&db).await;

    let building = begin_building(&db.admin, INTERPRETERS).await.unwrap();
    catch_up(&db.admin, INTERPRETERS, building).await.unwrap();
    assert_eq!(active_id(&db).await, old);
    assert_eq!(live_seqs(&db).await, vec![first], "jamais vide");

    let during = db.insert(source, "coffee.cup", "b", json!({})).await;
    drain(&db.app, INTERPRETERS).await;
    assert_eq!(live_seqs(&db).await, vec![first, during]);
    assert!(matches!(
        try_switch(&db.admin, INTERPRETERS, building, false)
            .await
            .unwrap(),
        SwitchOutcome::Lagging
    ));
    assert_eq!(active_id(&db).await, old);

    catch_up(&db.admin, INTERPRETERS, building).await.unwrap();
    let after = db.insert(source, "coffee.cup", "c", json!({})).await;
    match try_switch(&db.admin, INTERPRETERS, building, false)
        .await
        .unwrap()
    {
        SwitchOutcome::Switched(comparison) => assert_eq!(comparison.differences, vec![]),
        other => panic!("{other:?}"),
    }
    assert_eq!(active_id(&db).await, building);
    assert_eq!(
        live_seqs(&db).await,
        vec![first, during],
        "aucun recul à la bascule"
    );
    drain(&db.app, INTERPRETERS).await;
    assert_eq!(live_seqs(&db).await, vec![first, during, after]);
}

/// Pour de vrai : le worker tourne, des enregistrements arrivent, plusieurs rebuild s'enchaînent.
#[tokio::test]
async fn rebuilds_with_a_running_worker_and_arriving_records_lose_nothing() {
    let db = TestDb::new().await;
    let (source, _) = db.source("ios", KINDS).await;
    for n in 0..50 {
        db.insert(source, "coffee.cup", &cup(n), json!({})).await;
    }
    ensure_active(&db.app, INTERPRETERS).await.unwrap();
    let (stop, stopped) = watch::channel(false);
    let mut workers = Vec::new();
    for _ in 0..2 {
        let pool = life_tracker::db::pool(&db.app_url()).unwrap();
        workers.push(tokio::spawn(engine::run(
            pool,
            INTERPRETERS,
            Duration::from_millis(5),
            stopped.clone(),
        )));
    }

    let writer_db = life_tracker::db::pool(&db.app_url()).unwrap();
    let (done, finished) = watch::channel(false);
    let writer = tokio::spawn(async move {
        let mut n = 1000;
        while !*finished.borrow() {
            sqlx::query(
                "INSERT INTO core.records (source_id, kind, schema_version, key, occurred_at, payload)
                 VALUES ($1, 'coffee.cup', 1, $2, now(), '{}')",
            )
            .bind(source)
            .bind(cup(n))
            .execute(&writer_db)
            .await
            .unwrap();
            n += 1;
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        n - 1000
    });

    for _ in 0..3 {
        let report = replay::rebuild(&db.admin, INTERPRETERS, false)
            .await
            .unwrap();
        assert!(report.switched);
        assert_eq!(report.comparison.differences, vec![]);
    }
    done.send_replace(true);
    let written = writer.await.unwrap() as i64;
    assert!(written > 0);

    let expected = 50 + written;
    for _ in 0..2400 {
        if db.count("SELECT count(*) FROM derived.live_events").await == expected {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    stop.send_replace(true);
    for worker in workers {
        worker.await.unwrap();
    }
    assert_eq!(
        db.count("SELECT count(*) FROM derived.live_events").await,
        expected
    );
    assert_eq!(
        db.count("SELECT count(DISTINCT seq) FROM derived.live_events")
            .await,
        expected
    );
    assert!(errors(&db).await.is_empty());
    assert_eq!(
        db.count("SELECT count(*) FROM derived.generations").await,
        1
    );

    let last = replay::rebuild(&db.admin, INTERPRETERS, false)
        .await
        .unwrap();
    assert_eq!(last.comparison.differences, vec![]);
}

async fn wait_for_more_records(db: &TestDb, than: i64) {
    while db.count("SELECT count(*) FROM core.records").await <= than {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Un journal qui avance plus vite que l'interprétation ne retient pas la reconstruction : elle vise
/// le bout du journal à son départ, et ce qui suit revient au worker.
#[tokio::test(flavor = "multi_thread")]
async fn a_rebuild_finishes_even_if_the_journal_outruns_it() {
    let db = TestDb::new().await;
    let (source, _) = db.source("ios", KINDS).await;
    db.insert(source, "coffee.cup", "first", json!({})).await;
    drain(&db.app, INTERPRETERS).await;

    let writer_db = life_tracker::db::pool(&db.app_url()).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_writer = stop.clone();
    let writer = tokio::spawn(async move {
        let mut n = 0;
        while !stop_writer.load(Ordering::Relaxed) {
            sqlx::query(
                "INSERT INTO core.records (source_id, kind, schema_version, key, occurred_at, payload)
                 VALUES ($1, 'coffee.cup', 1, $2, now(), '{}')",
            )
            .bind(source)
            .bind(cup(n))
            .execute(&writer_db)
            .await
            .unwrap();
            n += 1;
        }
    });
    // Le journal avance déjà au départ de la reconstruction...
    wait_for_more_records(&db, 1).await;
    let report = tokio::time::timeout(
        Duration::from_secs(60),
        replay::rebuild(&db.admin, INTERPRETERS, false),
    )
    .await
    .expect("la reconstruction doit finir")
    .unwrap();
    // ... et il a dépassé sa cible à l'arrivée, quelle que soit la charge de la machine.
    let at_end = db.count("SELECT count(*) FROM core.records").await;
    wait_for_more_records(&db, at_end).await;
    stop.store(true, Ordering::Relaxed);
    writer.await.unwrap();
    assert!(report.switched);
    assert_eq!(report.comparison.differences, vec![]);

    let before = db.count("SELECT count(*) FROM derived.live_events").await;
    drain(&db.app, INTERPRETERS).await;
    assert!(db.count("SELECT count(*) FROM derived.live_events").await > before);
    assert_eq!(
        db.count("SELECT count(*) FROM derived.live_events").await,
        db.count("SELECT count(*) FROM core.records").await
    );
}

/// Le rejeu tourne en tracker_admin sans droit de plus que ceux de la migration.
#[tokio::test]
async fn replay_runs_as_tracker_admin() {
    let db = TestDb::new().await;
    let who: String = sqlx::query_scalar("SELECT current_user::text")
        .fetch_one(&db.admin)
        .await
        .unwrap();
    assert_eq!(who, "tracker_admin");
    assert!(
        sqlx::query("INSERT INTO core.records (source_id, kind, schema_version, key, occurred_at, payload) VALUES (1, 'coffee.cup', 1, 'x', now(), '{}')")
            .execute(&db.admin)
            .await
            .is_err(),
        "tracker_admin n'écrit toujours pas au journal"
    );
}
