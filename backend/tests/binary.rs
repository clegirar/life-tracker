//! Le vrai binaire : migrate, admin, serve (arrêt propre, contenu des logs).

mod common;
use common::*;
use serde_json::{Value, json};
use sqlx::Row;
use std::process::{Child, Command, Output, Stdio};
use std::time::Duration;

fn bin() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_life-tracker"));
    command.env_clear();
    command
}

fn migrate(url: &str) -> Output {
    bin()
        .arg("migrate")
        .env("MIGRATE_DATABASE_URL", url)
        .output()
        .unwrap()
}

fn admin_cmd(args: &[&str]) -> Output {
    bin()
        .arg("admin")
        .args(args)
        .env("ADMIN_DATABASE_URL", admin_url())
        .output()
        .unwrap()
}

async fn snapshot(pool: &sqlx::PgPool) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT version::text || ':' || success::text || ':' || encode(checksum, 'hex') || ':' || installed_on::text
         FROM _sqlx_migrations
         UNION ALL
         SELECT format('%s.%s.%s %s', table_schema, table_name, column_name, data_type)
         FROM information_schema.columns WHERE table_schema IN ('core', 'derived')
         UNION ALL
         SELECT format('%s.%s', schemaname, indexname) FROM pg_indexes WHERE schemaname IN ('core', 'derived')
         UNION ALL
         SELECT 'settings:' || count(*) FROM core.settings
         UNION ALL
         SELECT 'kinds:' || count(*) FROM core.kinds
         UNION ALL
         SELECT 'webapp:' || count(*) FROM core.sources WHERE name = 'webapp'
         ORDER BY 1",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

/// test.sh a déjà appliqué les migrations par `migrate` sur la base vide. Ici : elles sont toutes
/// là, par sqlx, et un second passage ne change rien.
#[tokio::test]
async fn second_migrate_changes_nothing() {
    let owner = life_tracker::db::pool(&owner_url()).unwrap();
    let files = std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/migrations"))
        .unwrap()
        .count() as i64;
    let applied: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations WHERE success")
        .fetch_one(&owner)
        .await
        .unwrap();
    assert_eq!(applied, files);

    let before = snapshot(&owner).await;
    let output = migrate(&owner_url());
    assert!(output.status.success(), "{output:?}");
    assert_eq!(snapshot(&owner).await, before);
}

#[tokio::test]
async fn app_role_cannot_migrate_nor_read_the_migration_table() {
    let app = life_tracker::db::pool(&app_url()).unwrap();
    assert!(
        sqlx::query("SELECT 1 FROM _sqlx_migrations")
            .execute(&app)
            .await
            .is_err()
    );
    // Le schéma existe déjà et tracker_app n'y peut rien : migrate échoue dès sa table de suivi.
    assert!(!migrate(&app_url()).status.success());
}

#[test]
fn each_command_requires_its_own_url() {
    let serve = bin()
        .arg("serve")
        .env("MIGRATE_DATABASE_URL", owner_url())
        .env("ADMIN_DATABASE_URL", admin_url())
        .output()
        .unwrap();
    assert!(!serve.status.success());
    assert!(String::from_utf8_lossy(&serve.stderr).contains("DATABASE_URL"));
    let migrate = bin()
        .arg("migrate")
        .env("DATABASE_URL", app_url())
        .output()
        .unwrap();
    assert!(!migrate.status.success());
    let admin = bin()
        .args(["admin", "revoke-source", "x"])
        .env("DATABASE_URL", app_url())
        .output()
        .unwrap();
    assert!(!admin.status.success());
}

#[tokio::test]
async fn admin_token_lifecycle() {
    let app = life_tracker::db::pool(&app_url()).unwrap();
    let name = unique_name();

    let created = admin_cmd(&["create-source", &name, "--silence-threshold", "6 hours"]);
    assert!(created.status.success(), "{created:?}");
    let stdout = String::from_utf8(created.stdout).unwrap();
    let token = stdout.trim();
    assert_eq!(token.len(), 64, "32 octets en hexadécimal");
    assert!(!String::from_utf8_lossy(&created.stderr).contains(token));

    let row = sqlx::query(
        "SELECT auth::text AS auth, token_hash, extract(epoch FROM silence_threshold)::float8 AS silence
         FROM core.sources WHERE name = $1",
    )
    .bind(&name)
    .fetch_one(&app)
    .await
    .unwrap();
    assert_eq!(row.get::<String, _>("auth"), "token");
    let hash: Vec<u8> = row.get("token_hash");
    assert_eq!(hash, life_tracker::admin::token_hash(token));
    assert_ne!(hash, token.as_bytes());
    assert_eq!(row.get::<f64, _>("silence"), 21600.0);

    // Deux créations ne donnent jamais le même token, et un nom déjà pris n'en affiche aucun.
    let other = admin_cmd(&["create-source", &unique_name()]);
    assert_ne!(String::from_utf8(other.stdout).unwrap().trim(), token);
    let taken = admin_cmd(&["create-source", &name]);
    assert!(!taken.status.success());
    assert!(taken.stdout.is_empty());

    let base = serve(app.clone()).await;
    let http = reqwest::Client::new();
    let send = async |key: &str| -> (u16, Value) {
        let response = http
            .post(format!("{base}/v1/records"))
            .bearer_auth(token)
            .json(&json!({"records": [record(key)]}))
            .send()
            .await
            .unwrap();
        (response.status().as_u16(), response.json().await.unwrap())
    };

    let (status, body) = send("a").await;
    assert_eq!(status, 200);
    assert_eq!(body["results"][0]["reason"], "kind_not_declared");

    assert!(
        admin_cmd(&["grant-kind", &name, "coffee.cup"])
            .status
            .success()
    );
    assert!(
        admin_cmd(&["grant-kind", &name, "coffee.cup"])
            .status
            .success(),
        "idempotent"
    );
    assert_eq!(send("b").await.1["results"][0]["status"], "accepted");

    assert!(
        admin_cmd(&["revoke-kind", &name, "coffee.cup"])
            .status
            .success()
    );
    assert!(
        !admin_cmd(&["revoke-kind", &name, "coffee.cup"])
            .status
            .success()
    );
    assert_eq!(
        send("c").await.1["results"][0]["reason"],
        "kind_not_declared"
    );

    assert!(
        admin_cmd(&["grant-kind", &name, "coffee.cup"])
            .status
            .success()
    );
    assert_eq!(send("d").await.1["results"][0]["status"], "accepted");

    assert!(
        !admin_cmd(&["grant-kind", &name, "no.such.kind"])
            .status
            .success()
    );
    assert!(
        !admin_cmd(&["grant-kind", "no-such-source", "coffee.cup"])
            .status
            .success()
    );
    assert!(
        !admin_cmd(&["create-source", "Invalid Name"])
            .status
            .success()
    );

    assert!(admin_cmd(&["revoke-source", &name]).status.success());
    assert!(!admin_cmd(&["revoke-source", &name]).status.success());
    assert_eq!(send("e").await.0, 401);
}

struct Server {
    child: Child,
    base: String,
    log: std::path::PathBuf,
}

impl Server {
    /// Démarre `serve` avec pour seule URL celle donnée, logs dans un fichier.
    async fn start(database_url: &str, rust_log: &str) -> Self {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let log = std::env::temp_dir().join(format!("{}.log", unique_name()));
        let file = std::fs::File::create(&log).unwrap();
        let mut child = bin()
            .arg("serve")
            .env("DATABASE_URL", database_url)
            .env("LISTEN_ADDR", format!("127.0.0.1:{port}"))
            .env("RUST_LOG", rust_log)
            .stdout(Stdio::from(file.try_clone().unwrap()))
            .stderr(Stdio::from(file))
            .spawn()
            .unwrap();
        let base = format!("http://127.0.0.1:{port}");
        for _ in 0..100 {
            if reqwest::get(format!("{base}/health")).await.is_ok() {
                return Self { child, base, log };
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        child.kill().unwrap();
        child.wait().unwrap();
        panic!("le serveur n'a pas démarré");
    }

    /// SIGKILL, sans laisser au processus la moindre chance de finir proprement.
    fn kill(mut self) {
        self.child.kill().unwrap();
        self.child.wait().unwrap();
        let _ = std::fs::remove_file(&self.log);
    }

    /// SIGTERM, attend la fin, renvoie les logs. Échoue si l'arrêt n'est pas propre.
    fn stop(mut self) -> String {
        let pid = self.child.id().to_string();
        assert!(
            Command::new("kill")
                .args(["-TERM", &pid])
                .status()
                .unwrap()
                .success()
        );
        for _ in 0..100 {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success(), "{status}");
                let logs = std::fs::read_to_string(&self.log).unwrap();
                std::fs::remove_file(&self.log).unwrap();
                return logs;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("pas d'arrêt après SIGTERM");
    }
}

// Un test qui échoue ne laisse pas de serveur derrière lui.
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[tokio::test]
async fn graceful_shutdown_with_only_the_app_url() {
    let server = Server::start(&app_url(), "info").await;
    let logs = server.stop();
    assert!(logs.contains("stopped"), "{logs}");
}

const SENTINEL: &str = "SENTINELfd38a1";

/// Le niveau trace laisse parler sqlx, hyper et les autres : même là, rien de ce qu'envoie le client.
#[tokio::test]
async fn logs_contain_no_payload_key_or_token() {
    let ctx = Ctx::new(&["coffee.cup"]).await;
    let http = &ctx.http;
    let secret = |what: &str| format!("{SENTINEL}-{what}");
    let batch = json!({"records": [
        {"key": secret("key-accepted"), "kind": "coffee.cup", "schema_version": 1,
         "occurred_at": "2026-10-13T09:42:10Z", "payload": {"secret": secret("payload")}},
        {"key": secret("key-accepted"), "kind": "coffee.cup", "schema_version": 1,
         "occurred_at": "2026-10-13T09:42:10Z", "payload": {"secret": secret("payload")}},
        {"key": secret("key-rejected"), "kind": secret("kind"), "schema_version": 1,
         "occurred_at": "2026-10-13T09:42:10Z", "payload": {"secret": secret("payload")}},
        {"key": secret("key-rejected-2"), "kind": "coffee.cup", "schema_version": 1,
         "occurred_at": secret("date"), "payload": {"secret": secret("payload")}},
        {"key": secret("key-unstorable"), "kind": "coffee.cup", "schema_version": 1,
         "occurred_at": "2026-10-13T09:42:10Z", "payload": {"secret": format!("{}\u{0}", secret("payload"))}},
    ]});

    let exercise = async |base: &str, expected: u16| {
        let post = |path: &str| {
            http.post(format!("{base}{path}"))
                .bearer_auth(&ctx.token)
                .header("X-Client-Version", secret("version"))
        };
        let response = post("/v1/records").json(&batch).send().await.unwrap();
        assert_eq!(response.status(), expected);
        let id = response.headers()["x-request-id"]
            .to_str()
            .unwrap()
            .to_owned();
        post("/v1/records")
            .body(format!("{{{SENTINEL}"))
            .send()
            .await
            .unwrap();
        post("/v1/heartbeat")
            .body(secret("heartbeat"))
            .send()
            .await
            .unwrap();
        http.post(format!("{base}/v1/records"))
            .bearer_auth(secret("bad-token"))
            .send()
            .await
            .unwrap();
        http.post(format!("{base}/{}", secret("path")))
            .send()
            .await
            .unwrap();
        id
    };

    // Chemin normal, puis écriture refusée par la base (tracker_admin n'a pas INSERT).
    let server = Server::start(&app_url(), "trace").await;
    let id_ok = exercise(&server.base, 200).await;
    let logs_ok = server.stop();
    let server = Server::start(&admin_url(), "trace").await;
    let id_failed = exercise(&server.base, 500).await;
    let logs_failed = server.stop();

    for logs in [&logs_ok, &logs_failed] {
        assert!(!logs.contains(SENTINEL), "fuite : {}", leak(logs));
        assert!(!logs.contains(&ctx.token), "token dans les logs");
    }
    // Le test ne prouve rien si les logs sont vides : ils doivent raconter la requête.
    assert!(
        logs_ok
            .lines()
            .any(|l| l.contains(&id_ok) && l.contains("batch processed"))
    );
    assert!(
        logs_ok.contains("\"level\":\"TRACE\""),
        "le niveau trace doit être actif"
    );
    assert!(logs_ok.contains("record rejected"));
    assert!(logs_ok.contains("\"status\":401"));
    assert!(
        logs_failed
            .lines()
            .any(|l| l.contains(&id_failed) && l.contains("database error"))
    );
    assert!(logs_failed.contains("42501"));
}

fn admin_on(db: &TestDb, args: &[&str]) -> Output {
    bin()
        .arg("admin")
        .args(args)
        .env("ADMIN_DATABASE_URL", db.admin_url())
        .output()
        .unwrap()
}

async fn post_records(base: &str, token: &str, records: Value) -> Vec<Value> {
    let response = reqwest::Client::new()
        .post(format!("{base}/v1/records"))
        .bearer_auth(token)
        .json(&json!({ "records": records }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.unwrap();
    body["results"].as_array().unwrap().clone()
}

fn cup(key: &str) -> Value {
    record(key)
}

fn cancel(key: &str, source: &str, target: &str) -> Value {
    json!({"key": key, "kind": "record.cancel", "schema_version": 1,
           "occurred_at": "2026-10-13T09:50:00+02:00",
           "payload": {"source": source, "key": target}})
}

async fn wait_until_count(db: &TestDb, sql: &str, expected: i64) {
    for _ in 0..1200 {
        if db.count(sql).await == expected {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("{sql} : {} au lieu de {expected}", db.count(sql).await);
}

const LIVE: &str = "SELECT count(*) FROM derived.live_events";

/// De l'API au calendrier, avec le vrai binaire sur une base neuve : `serve` établit la génération,
/// `admin rebuild` et `admin retry-errors` passent par la ligne de commande.
#[tokio::test]
async fn ingestion_to_interpretation_end_to_end() {
    let db = TestDb::new().await;
    let (_, token) = db.source("ios", &["coffee.cup", "record.cancel"]).await;
    let server = Server::start(&db.app_url(), "info").await;

    let results = post_records(
        &server.base,
        &token,
        json!([
            cup("c1"),
            cup("c2"),
            cancel("x1", "ios", "c2"),
            cancel("x2", "ios", "c3"),
            cup("c3"),
            cancel("x3", "later", "c1"),
            {"key": "bad", "kind": "record.cancel", "schema_version": 1,
             "occurred_at": "2026-10-13T09:50:00Z", "payload": "malformed"},
        ]),
    )
    .await;
    assert!(
        results.iter().all(|r| r["status"] == "accepted"),
        "{results:?}"
    );
    wait_until_count(&db, "SELECT count(*) FROM derived.interpretation_errors", 2).await;
    wait_until_count(&db, LIVE, 1).await;
    let at: String = sqlx::query_scalar(
        "SELECT to_char(occurred_at AT TIME ZONE 'UTC', 'HH24:MI:SS') FROM derived.live_events",
    )
    .fetch_one(&db.app)
    .await
    .unwrap();
    assert_eq!(at, "07:42:10");

    // Une rebuild, pendant que le serveur tourne.
    let rebuilt = admin_on(&db, &["rebuild"]);
    assert!(rebuilt.status.success(), "{rebuilt:?}");
    let stdout = String::from_utf8(rebuilt.stdout).unwrap();
    assert!(stdout.contains("0 différence(s), active"), "{stdout}");
    assert!(
        stdout.contains("coffee.cup : comparé jusqu'au numéro"),
        "{stdout}"
    );

    // Une différence : code de sortie non nul, pas de bascule, rien que des numéros à l'écran.
    sqlx::query(
        "UPDATE derived.events SET details = '{\"x\":\"SENTINELfd38a1\"}'
         WHERE seq = (SELECT min(seq) FROM derived.live_events)
           AND generation = (SELECT id FROM derived.generations WHERE state = 'active')",
    )
    .execute(&db.admin)
    .await
    .unwrap();
    let refused = admin_on(&db, &["rebuild"]);
    assert!(!refused.status.success());
    let shown = String::from_utf8_lossy(&refused.stdout);
    assert!(shown.contains("différence : events numéro"), "{shown}");
    assert!(!shown.contains(SENTINEL));
    let forced = admin_on(&db, &["rebuild", "--allow-differences"]);
    assert!(forced.status.success(), "{forced:?}");

    // La source visée existe maintenant : l'une des deux erreurs se lève.
    db.source("later", &["coffee.cup"]).await;
    let retried = admin_on(&db, &["retry-errors"]);
    assert!(retried.status.success(), "{retried:?}");
    let shown = String::from_utf8(retried.stdout).unwrap();
    assert!(
        shown.starts_with("1 levée(s)") && shown.contains("1 restante(s)"),
        "{shown}"
    );
    assert_eq!(
        db.count("SELECT count(*) FROM derived.interpretation_errors")
            .await,
        1
    );

    assert_eq!(
        db.count("SELECT count(*) FROM derived.cancellations").await,
        3
    );

    // Un nouveau café passe encore après tout cela.
    post_records(&server.base, &token, json!([cup("c4")])).await;
    wait_until_count(&db, LIVE, 2).await;
    server.stop();
}

/// SIGKILL en pleine interprétation, sur le vrai binaire : ce qui est validé est complet, et le
/// redémarrage ne perd ni ne double rien.
#[tokio::test]
async fn a_killed_process_resumes_without_loss_or_double_interpretation() {
    let db = TestDb::new().await;
    let (source, token) = db.source("ios", &["coffee.cup"]).await;
    // Le journal d'abord, sans worker : l'API seule.
    let base = serve(db.app.clone()).await;
    const BATCHES: usize = 3;
    for batch in 0..BATCHES {
        let records: Vec<Value> = (0..500).map(|n| cup(&format!("{batch}-{n}"))).collect();
        post_records(&base, &token, json!(records)).await;
    }
    let total = (BATCHES * 500) as i64;

    let progress = async || -> (i64, i64, i64) {
        let row = sqlx::query(
            "SELECT COALESCE((SELECT last_seq FROM derived.cursors WHERE kind = 'coffee.cup'), 0) AS cursor,
                    (SELECT count(*) FROM derived.events) AS events,
                    (SELECT count(DISTINCT seq) FROM derived.events) AS seqs",
        )
        .fetch_one(&db.app)
        .await
        .unwrap();
        (row.get("cursor"), row.get("events"), row.get("seqs"))
    };

    let server = Server::start(&db.app_url(), "warn").await;
    let mut interrupted = false;
    for _ in 0..2000 {
        if progress().await.1 >= 100 {
            server.kill();
            interrupted = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert!(interrupted, "le worker n'a pas démarré");

    let (cursor, events, seqs) = progress().await;
    assert!(
        events < total,
        "tué trop tard pour prouver quoi que ce soit"
    );
    assert_eq!(events, seqs);
    let covered: i64 =
        sqlx::query_scalar("SELECT count(*) FROM core.records WHERE source_id = $1 AND seq <= $2")
            .bind(source)
            .bind(cursor)
            .fetch_one(&db.app)
            .await
            .unwrap();
    assert_eq!(
        events, covered,
        "chaque enregistrement sous le curseur, et rien d'autre"
    );

    let server = Server::start(&db.app_url(), "warn").await;
    wait_until_count(&db, "SELECT count(*) FROM derived.events", total).await;
    let logs = server.stop();
    let (_, events, seqs) = progress().await;
    assert_eq!((events, seqs), (total, total));
    assert_eq!(
        db.count("SELECT count(*) FROM derived.interpretation_errors")
            .await,
        0
    );
    assert!(!logs.contains(&token));
}

/// Aucun journal, à aucun niveau, ne cite une charge utile, une clé ou le nom d'une cible, y compris
/// quand l'interprétation échoue ; ils citent bien les numéros et le suivi de requête.
#[tokio::test]
async fn interpretation_logs_contain_only_identifiers() {
    let db = TestDb::new().await;
    let (_, token) = db.source("ios", &["coffee.cup", "record.cancel"]).await;
    let server = Server::start(&db.app_url(), "trace").await;
    let secret = |what: &str| format!("{SENTINEL}-{what}");
    let results = post_records(
        &server.base,
        &token,
        json!([
            {"key": secret("k1"), "kind": "coffee.cup", "schema_version": 1,
             "occurred_at": "2026-10-13T09:42:10Z", "payload": {"note": secret("p1")}},
            {"key": secret("k2"), "kind": "coffee.cup", "schema_version": 9,
             "occurred_at": "2026-10-13T09:42:10Z", "payload": {"note": secret("p2")}},
            {"key": secret("k3"), "kind": "record.cancel", "schema_version": 1,
             "occurred_at": "2026-10-13T09:42:10Z", "payload": secret("p3")},
            {"key": secret("k4"), "kind": "record.cancel", "schema_version": 1,
             "occurred_at": "2026-10-13T09:42:10Z",
             "payload": {"source": secret("p4"), "key": secret("p5")}},
            {"key": secret("k5"), "kind": "record.cancel", "schema_version": 1,
             "occurred_at": "2026-10-13T09:42:10Z",
             "payload": {"source": "ios", "key": secret("p6")}},
        ]),
    )
    .await;
    assert_eq!(results.len(), 5);
    wait_until_count(&db, "SELECT count(*) FROM derived.interpretation_errors", 3).await;
    wait_until_count(&db, "SELECT count(*) FROM derived.cancellations", 1).await;
    let messages: Vec<String> =
        sqlx::query_scalar("SELECT error FROM derived.interpretation_errors")
            .fetch_all(&db.app)
            .await
            .unwrap();
    assert!(
        messages.iter().all(|m| !m.contains(SENTINEL)),
        "{messages:?}"
    );

    let request_id: String =
        sqlx::query_scalar("SELECT request_id::text FROM core.records LIMIT 1")
            .fetch_one(&db.app)
            .await
            .unwrap();
    let logs = server.stop();
    assert!(!logs.contains(SENTINEL), "fuite : {}", leak(&logs));
    assert!(
        logs.contains("record interpreted"),
        "les logs racontent l'interprétation"
    );
    assert!(
        logs.lines()
            .any(|l| l.contains("record in error") && l.contains(&request_id)),
        "le suivi de requête va jusqu'à l'interprétation"
    );
}

fn leak(logs: &str) -> &str {
    logs.lines().find(|l| l.contains(SENTINEL)).unwrap_or("")
}
