#![allow(dead_code)]
//! Outils partagés. Les URL viennent de backend/test.sh : jamais d'une autre base.

use life_tracker::{admin, api, db};
use reqwest::Client;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

pub fn env_url(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} absent : lancer backend/test.sh"))
}
pub fn app_url() -> String {
    env_url("LT_TEST_APP_URL")
}
pub fn admin_url() -> String {
    env_url("LT_TEST_ADMIN_URL")
}
pub fn owner_url() -> String {
    env_url("LT_TEST_OWNER_URL")
}

pub fn unique_name() -> String {
    format!("t-{}", Uuid::now_v7().simple())
}

/// Chaque test a sa source et son serveur : le journal n'efface rien, seul le nom isole.
pub struct Ctx {
    pub base: String,
    pub http: Client,
    pub token: String,
    pub source: String,
    pub source_id: i16,
    /// Connexion tracker_app : les vérifications passent par les mêmes droits que le serveur.
    pub app: PgPool,
    pub admin: PgPool,
}

pub async fn serve(pool: PgPool) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, api::router(pool)).await });
    format!("http://{addr}")
}

impl Ctx {
    pub async fn new(kinds: &[&str]) -> Self {
        let app = db::pool(&app_url()).unwrap();
        let admin_pool = db::pool(&admin_url()).unwrap();
        let source = unique_name();
        let token = admin::create_source(&admin_pool, &source, None)
            .await
            .unwrap();
        for kind in kinds {
            admin::grant_kind(&admin_pool, &source, kind).await.unwrap();
        }
        let source_id: i16 = sqlx::query_scalar("SELECT id FROM core.sources WHERE name = $1")
            .bind(&source)
            .fetch_one(&app)
            .await
            .unwrap();
        let base = serve(app.clone()).await;
        Self {
            base,
            http: Client::new(),
            token,
            source,
            source_id,
            app,
            admin: admin_pool,
        }
    }

    pub fn post(&self, path: &str) -> reqwest::RequestBuilder {
        self.http
            .post(format!("{}{path}", self.base))
            .bearer_auth(&self.token)
    }

    pub async fn send(&self, records: Value) -> (u16, Value) {
        let response = self
            .post("/v1/records")
            .json(&json!({ "records": records }))
            .send()
            .await
            .unwrap();
        let status = response.status().as_u16();
        (status, response.json().await.unwrap())
    }

    /// Envoie un lot qui doit réussir et renvoie ses résultats.
    pub async fn results(&self, records: Value) -> Vec<Value> {
        let (status, body) = self.send(records).await;
        assert_eq!(status, 200, "{body}");
        body["results"].as_array().unwrap().clone()
    }

    pub async fn journal_count(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM core.records WHERE source_id = $1")
            .bind(self.source_id)
            .fetch_one(&self.app)
            .await
            .unwrap()
    }
}

pub fn record(key: &str) -> Value {
    json!({
        "key": key,
        "kind": "coffee.cup",
        "schema_version": 1,
        "occurred_at": "2026-10-13T09:42:10+02:00",
        "payload": {},
    })
}

pub fn statuses(results: &[Value]) -> Vec<&str> {
    results
        .iter()
        .map(|r| r["status"].as_str().unwrap())
        .collect()
}

pub fn repo_path(relative: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(relative)
}

pub fn read_json(path: &std::path::Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// Validateur du schéma du contrat, pour sa racine ou l'une de ses définitions.
pub fn validator(def: Option<&str>) -> jsonschema::Validator {
    let mut schema = read_json(&repo_path("contract/ingestion-v1.schema.json"));
    if let Some(def) = def {
        schema["$ref"] = json!(format!("#/$defs/{def}"));
    }
    jsonschema::options()
        .should_validate_formats(true)
        .build(&schema)
        .unwrap()
}

/// Fichiers d'un dossier de fixtures, triés.
pub fn fixtures(dir: &str) -> Vec<(String, Value)> {
    let mut paths: Vec<_> = std::fs::read_dir(repo_path(&format!("contract/fixtures/{dir}")))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    paths.sort();
    assert!(!paths.is_empty());
    paths
        .iter()
        .map(|p| {
            (
                p.file_name().unwrap().to_str().unwrap().to_owned(),
                read_json(p),
            )
        })
        .collect()
}
