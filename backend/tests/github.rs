//! GitHub de bout en bout, sans aucun réseau extérieur : le connecteur contre un GitHub GraphQL simulé
//! et la vraie API d'ingestion, puis le vrai worker et les lectures (carte du jour, calendrier, lectures
//! génériques du MCP). Les réponses suivent la structure relevée sur le vrai compte le 7 nov. 2026
//! (essai du chantier) ; tous les ids, comptes, dépôts, titres et le token sont fictifs.
//!
//! Le GitHub simulé ne comprend pas GraphQL : il reconnaît chaque requête à son nom d'opération et lit
//! ses variables, comme le connecteur les envoie.

mod common;
use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use chrono::{DateTime, Days, NaiveDate, TimeDelta, Timelike, Utc};
use common::*;
use life_tracker::api;
use life_tracker::collector::garmin::{paris_offset, paris_today};
use life_tracker::collector::github::{self, Collector, Config, RepoRef, ServiceError, TickError};
use life_tracker::interpreters::INTERPRETERS;
use life_tracker::{admin, engine, replay};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::watch;

const PASSWORD: &str = "un mot de passe assez long";
const PAT: &str = "ghp_JETONFICTIF0123456789abcdefABCDEF";
const VIEWER: &str = "U_kgDOVIEWERFICTIF";
const LOGIN: &str = "compte-sentinelle";
const OTHER: &str = "autre-compte";
const ORG: &str = "orga-sentinelle";
const SAML_ORG: &str = "orga-saml-sentinelle";
/// Des dépôts fictifs, aux noms reconnaissables : jamais dans les logs.
const REPO_A: &str = "R_kgDOAAAA";
const REPO_B: &str = "R_kgDOBBBB";
const REPO_C: &str = "R_kgDOCCCC";
const FORK: &str = "R_kgDOFORK";
const EMPTY: &str = "R_kgDOVIDE";
const SAML_REPO: &str = "R_kgDOSAML";
const HISTORY_A: usize = 250;

fn at(text: &str) -> DateTime<Utc> {
    text.parse().unwrap()
}

fn paris_day(at: DateTime<Utc>) -> NaiveDate {
    at.with_timezone(&paris_offset(at)).date_naive()
}

fn stamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

// ─── Le GitHub simulé ────────────────────────────────────────────────────────

#[derive(Clone)]
struct FakeCommit {
    oid: String,
    author: String,
    authored: DateTime<Utc>,
    committed: DateTime<Utc>,
    headline: String,
}

#[derive(Clone)]
struct FakeRepo {
    id: String,
    name: String,
    fork: bool,
    empty: bool,
    pushed_at: DateTime<Utc>,
    commits: Vec<FakeCommit>,
}

#[derive(Clone)]
struct FakeReview {
    id: String,
    author: String,
    state: String,
    submitted: Option<DateTime<Utc>>,
}

#[derive(Clone)]
struct FakeItem {
    id: String,
    pr: bool,
    number: i64,
    title: String,
    repo: String,
    author: String,
    created: DateTime<Utc>,
    updated: DateTime<Utc>,
    merged: Option<DateTime<Utc>>,
    reviews: Vec<FakeReview>,
}

#[derive(Clone)]
struct FakeComment {
    id: String,
    repo: String,
    /// L'issue ou la pull request commentée, par son id ; `None` : un commentaire de commit.
    on: Option<String>,
    created: DateTime<Utc>,
    updated: DateTime<Utc>,
}

#[derive(Default)]
struct Data {
    organizations: Vec<String>,
    saml: HashSet<String>,
    repos: Vec<FakeRepo>,
    viewer_repos: Vec<String>,
    org_repos: HashMap<String, Vec<String>>,
    contributed: Vec<String>,
    items: Vec<FakeItem>,
    issue_comments: Vec<FakeComment>,
    commit_comments: Vec<FakeComment>,
    /// Statut de chaque réponse : 200, ou une erreur.
    status: u16,
    /// Une réponse 200 dont l'erreur GraphQL dit la limite atteinte.
    rate_limited_body: bool,
    /// Le reste de points annoncé.
    remaining: i64,
    /// Au-delà de ce nombre de lectures d'historique, une 502.
    history_budget: Option<u32>,
    /// Les opérations reçues, avec leurs variables, dans l'ordre.
    calls: Vec<(String, Value)>,
}

#[derive(Clone)]
struct Fake {
    url: String,
    data: Arc<Mutex<Data>>,
}

fn repo_ref(data: &Data, id: &str) -> Value {
    let repo = data
        .repos
        .iter()
        .find(|r| r.id == id)
        .expect("dépôt simulé");
    json!({"id": repo.id, "nameWithOwner": repo.name})
}

fn repo_node(repo: &FakeRepo) -> Value {
    json!({
        "id": repo.id, "nameWithOwner": repo.name, "isFork": repo.fork,
        "pushedAt": stamp(repo.pushed_at), "description": "Description sentinelle",
        "defaultBranchRef": if repo.empty { Value::Null } else { json!({"name": "main"}) },
    })
}

/// Une page de 100 à partir du curseur (un rang, en texte).
fn paged(all: Vec<Value>, after: &Value) -> (Vec<Value>, Value) {
    let start: usize = after.as_str().map_or(0, |a| a.parse().unwrap());
    let end = (start + 100).min(all.len());
    let page = all[start.min(all.len())..end].to_vec();
    let info = json!({"hasNextPage": end < all.len(), "endCursor": end.to_string()});
    (page, info)
}

fn connection(all: Vec<Value>, after: &Value) -> Value {
    let (nodes, info) = paged(all, after);
    json!({"pageInfo": info, "nodes": nodes})
}

/// Les qualificatifs de recherche que le connecteur emploie.
fn matches(item: &FakeItem, query: &str) -> bool {
    query.split_whitespace().all(|term| {
        if term == "is:pr" {
            item.pr
        } else if term == "is:issue" {
            !item.pr
        } else if let Some(login) = term.strip_prefix("-author:") {
            item.author != login
        } else if let Some(login) = term.strip_prefix("author:") {
            item.author == login
        } else if let Some(login) = term.strip_prefix("reviewed-by:") {
            item.reviews.iter().any(|r| r.author == login)
        } else if let Some(range) = term.strip_prefix("created:") {
            let (from, to) = range.split_once("..").unwrap();
            let day = item.created.date_naive().to_string();
            from <= day.as_str() && day.as_str() <= to
        } else if let Some(since) = term.strip_prefix("updated:>=") {
            item.updated >= at(since)
        } else {
            panic!("qualificatif inconnu : {term}")
        }
    })
}

fn item_node(data: &Data, item: &FakeItem, login: &str, with_reviews: bool) -> Value {
    let mut node = json!({
        "id": item.id, "number": item.number, "title": item.title,
        "url": format!("https://github.com/x/y/{}/{}", if item.pr { "pull" } else { "issues" }, item.number),
        "createdAt": stamp(item.created), "repository": repo_ref(data, &item.repo),
        "body": "Corps sentinelle",
    });
    if item.pr {
        node["mergedAt"] = item.merged.map_or(Value::Null, |m| json!(stamp(m)));
        node["additions"] = json!(30);
        node["deletions"] = json!(4);
        if with_reviews {
            let reviews: Vec<Value> = item
                .reviews
                .iter()
                .filter(|r| r.author == login)
                .map(|r| {
                    json!({"id": r.id, "state": r.state,
                       "submittedAt": r.submitted.map(stamp), "url": "https://github.com/x/y/pull/1",
                       "body": "Corps sentinelle"})
                })
                .collect();
            node["reviews"] = json!({"nodes": reviews});
        }
    }
    node
}

fn answer(data: &mut Data, operation: &str, vars: &Value) -> Result<Value, Value> {
    let rate =
        json!({"remaining": data.remaining, "resetAt": stamp(Utc::now() + TimeDelta::minutes(10))});
    let after = &vars["after"];
    let body = match operation {
        "Viewer" => json!({"viewer": {
            "id": VIEWER, "login": LOGIN, "createdAt": "2019-03-01T10:00:00Z",
            "organizations": {"nodes": data.organizations.iter().map(|o| json!({"login": o})).collect::<Vec<_>>()},
        }}),
        "ViewerRepositories" => {
            let all = data
                .viewer_repos
                .iter()
                .map(|id| repo_node(data.repos.iter().find(|r| &r.id == id).unwrap()))
                .collect();
            json!({"viewer": {"repositories": connection(all, after)}})
        }
        "OrganizationRepositories" => {
            let login = vars["login"].as_str().unwrap();
            if data.saml.contains(login) {
                return Err(json!({
                    "data": {"organization": null, "rateLimit": rate},
                    "errors": [{"type": "FORBIDDEN", "path": ["organization"],
                                "message": "Resource protected by organization SAML enforcement. You must grant your Personal Access token access to this organization."}],
                }));
            }
            let all = data.org_repos[login]
                .iter()
                .map(|id| repo_node(data.repos.iter().find(|r| &r.id == id).unwrap()))
                .collect();
            json!({"organization": {"repositories": connection(all, after)}})
        }
        "ContributedRepositories" => {
            let all = data
                .contributed
                .iter()
                .map(|id| repo_node(data.repos.iter().find(|r| &r.id == id).unwrap()))
                .collect();
            json!({"viewer": {"repositoriesContributedTo": connection(all, after)}})
        }
        "History" => {
            if let Some(budget) = data.history_budget.as_mut() {
                if *budget == 0 {
                    return Err(Value::Null);
                }
                *budget -= 1;
            }
            let repo = data
                .repos
                .iter()
                .find(|r| r.id == vars["id"].as_str().unwrap())
                .unwrap();
            assert!(!repo.fork, "une fourche n'est jamais lue");
            let since = vars["since"].as_str().map(at);
            let mut commits: Vec<&FakeCommit> = repo
                .commits
                .iter()
                .filter(|c| c.author == vars["author"].as_str().unwrap())
                .filter(|c| since.is_none_or(|s| c.committed >= s))
                .collect();
            commits.sort_by_key(|c| std::cmp::Reverse(c.committed));
            let all = commits
                .iter()
                .map(|c| {
                    json!({"oid": c.oid, "authoredDate": stamp(c.authored),
                       "committedDate": stamp(c.committed), "messageHeadline": c.headline,
                       "message": format!("{}\n\nCorps sentinelle", c.headline),
                       "additions": 10, "deletions": 1,
                       "url": format!("https://github.com/x/y/commit/{}", c.oid)})
                })
                .collect();
            json!({"node": {"defaultBranchRef": {"target": {"history": connection(all, after)}}}})
        }
        "Search" => {
            let query = vars["q"].as_str().unwrap();
            let login = vars["login"].as_str().unwrap();
            let with_reviews = vars["withReviews"].as_bool().unwrap();
            let found: Vec<Value> = data
                .items
                .iter()
                .filter(|i| matches(i, query))
                .map(|i| item_node(data, i, login, with_reviews))
                .collect();
            let count = found.len();
            let mut search = connection(found, after);
            search["issueCount"] = json!(count);
            json!({"search": search})
        }
        "IssueComments" => {
            let mut comments = data.issue_comments.clone();
            comments.sort_by_key(|c| std::cmp::Reverse(c.updated));
            let all = comments
                .iter()
                .map(|c| {
                    let item = data
                        .items
                        .iter()
                        .find(|i| Some(&i.id) == c.on.as_ref())
                        .unwrap();
                    let subject = json!({"number": item.number, "title": item.title});
                    json!({"id": c.id, "createdAt": stamp(c.created), "updatedAt": stamp(c.updated),
                       "url": "https://github.com/x/y/issues/1#issuecomment-1",
                       "body": "Corps sentinelle", "repository": repo_ref(data, &c.repo),
                       "issue": subject,
                       "pullRequest": if item.pr { subject.clone() } else { Value::Null }})
                })
                .collect();
            json!({"viewer": {"issueComments": connection(all, after)}})
        }
        "CommitComments" => {
            let all = data
                .commit_comments
                .iter()
                .map(|c| {
                    json!({"id": c.id, "createdAt": stamp(c.created),
                       "url": "https://github.com/x/y/commit/abc#commitcomment-1",
                       "body": "Corps sentinelle", "repository": repo_ref(data, &c.repo),
                       "commit": {"messageHeadline": "Commit sentinelle commenté"}})
                })
                .collect();
            json!({"viewer": {"commitComments": connection(all, after)}})
        }
        other => panic!("opération inconnue : {other}"),
    };
    let mut body = body;
    body["rateLimit"] = rate;
    Ok(json!({ "data": body }))
}

async fn graphql(State(fake): State<Fake>, headers: HeaderMap, body: String) -> Response {
    assert_eq!(
        headers["authorization"].to_str().unwrap(),
        format!("bearer {PAT}")
    );
    let request: Value = serde_json::from_str(&body).unwrap();
    let query = request["query"].as_str().unwrap();
    let operation = query
        .strip_prefix("query ")
        .unwrap()
        .split(|c: char| c == '(' || c.is_whitespace())
        .next()
        .unwrap()
        .to_owned();
    let mut data = fake.data.lock().unwrap();
    data.calls
        .push((operation.clone(), request["variables"].clone()));
    match data.status {
        200 => {}
        401 => {
            return (
                StatusCode::UNAUTHORIZED,
                axum::Json(json!({"message": "Bad credentials"})),
            )
                .into_response();
        }
        429 => return (StatusCode::TOO_MANY_REQUESTS, [("retry-after", "120")]).into_response(),
        403 => {
            let reset = (Utc::now() + TimeDelta::seconds(300))
                .timestamp()
                .to_string();
            return (
                StatusCode::FORBIDDEN,
                [
                    ("x-ratelimit-remaining", "0".to_owned()),
                    ("x-ratelimit-reset", reset),
                ],
                "API rate limit exceeded",
            )
                .into_response();
        }
        other => return StatusCode::from_u16(other).unwrap().into_response(),
    }
    if data.rate_limited_body {
        return axum::Json(json!({"data": null, "errors": [{"type": "RATE_LIMITED",
            "message": "API rate limit exceeded for user ID 1."}]}))
        .into_response();
    }
    match answer(&mut data, &operation, &request["variables"]) {
        Ok(body) => axum::Json(body).into_response(),
        Err(Value::Null) => StatusCode::BAD_GATEWAY.into_response(),
        Err(body) => axum::Json(body).into_response(),
    }
}

impl Fake {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let fake = Self {
            url: format!("http://{}/graphql", listener.local_addr().unwrap()),
            data: Arc::new(Mutex::new(Data {
                status: 200,
                remaining: 4999,
                ..Data::default()
            })),
        };
        let app = Router::new()
            .route("/graphql", post(graphql))
            .with_state(fake.clone());
        tokio::spawn(async move { axum::serve(listener, app).await });
        fake
    }

    fn with<T>(&self, change: impl FnOnce(&mut Data) -> T) -> T {
        change(&mut self.data.lock().unwrap())
    }

    /// Les opérations reçues depuis le dernier appel.
    fn calls(&self) -> Vec<(String, Value)> {
        self.with(|d| std::mem::take(&mut d.calls))
    }

    fn repo(&self, id: &str, change: impl FnOnce(&mut FakeRepo)) {
        self.with(|d| change(d.repos.iter_mut().find(|r| r.id == id).unwrap()));
    }
}

fn commit(oid: String, author: &str, authored: DateTime<Utc>) -> FakeCommit {
    FakeCommit {
        headline: format!("Commit sentinelle {oid}"),
        oid,
        author: author.to_owned(),
        authored,
        committed: authored + TimeDelta::minutes(5),
    }
}

fn oid(prefix: char, n: usize) -> String {
    format!("{prefix}{n:039x}")
}

fn repo(id: &str, name: &str, commits: Vec<FakeCommit>) -> FakeRepo {
    FakeRepo {
        id: id.to_owned(),
        name: name.to_owned(),
        fork: false,
        empty: false,
        pushed_at: Utc::now() - TimeDelta::days(2),
        commits,
    }
}

/// Un compte fictif : A, à moi, 250 commits sur 250 jours (et 3 d'un autre) ; B, d'une organisation et
/// aussi parmi les contributions ; C, public d'un autre ; une fourche et un dépôt vide, jamais lus. Une
/// pull request ancienne fusionnée, une récente fusionnée, une ouverte ; une review sur la pull request
/// d'un autre (et une en attente) ; une issue ; deux commentaires d'issue ou de pull request et un de
/// commit.
fn world(fake: &Fake) {
    let now = Utc::now();
    fake.with(|d| {
        let mut history: Vec<FakeCommit> = (0..HISTORY_A)
            .map(|n| {
                commit(
                    oid('a', n),
                    VIEWER,
                    now - TimeDelta::days(3 + n as i64) - TimeDelta::hours(1),
                )
            })
            .collect();
        history.extend((0..3).map(|n| commit(oid('e', n), "U_AUTRE", now - TimeDelta::days(4))));
        d.repos = vec![
            repo(REPO_A, "compte-sentinelle/depot-sentinelle-a", history),
            repo(
                REPO_B,
                "orga-sentinelle/depot-sentinelle-b",
                (0..5)
                    .map(|n| commit(oid('b', n), VIEWER, now - TimeDelta::days(20 + n as i64)))
                    .collect(),
            ),
            repo(
                REPO_C,
                "autre-compte/depot-sentinelle-c",
                (0..2)
                    .map(|n| commit(oid('c', n), VIEWER, now - TimeDelta::days(60 + n as i64)))
                    .collect(),
            ),
            FakeRepo {
                fork: true,
                ..repo(
                    FORK,
                    "compte-sentinelle/fourche-sentinelle",
                    (0..4)
                        .map(|n| commit(oid('f', n), VIEWER, now - TimeDelta::days(5)))
                        .collect(),
                )
            },
            FakeRepo {
                empty: true,
                ..repo(EMPTY, "compte-sentinelle/vide-sentinelle", Vec::new())
            },
            repo(
                SAML_REPO,
                "orga-saml-sentinelle/depot-saml",
                vec![commit(oid('d', 0), VIEWER, now - TimeDelta::days(7))],
            ),
        ];
        d.organizations = vec![ORG.to_owned()];
        d.viewer_repos = vec![REPO_A.to_owned(), FORK.to_owned(), EMPTY.to_owned()];
        d.org_repos = HashMap::from([
            (ORG.to_owned(), vec![REPO_B.to_owned()]),
            (SAML_ORG.to_owned(), vec![SAML_REPO.to_owned()]),
        ]);
        d.contributed = vec![REPO_B.to_owned(), REPO_C.to_owned()];
        let item = |id: &str, pr: bool, number, repo: &str, author: &str, created| FakeItem {
            id: id.to_owned(),
            pr,
            number,
            title: format!("Titre sentinelle {id}"),
            repo: repo.to_owned(),
            author: author.to_owned(),
            created,
            updated: created,
            merged: None,
            reviews: Vec::new(),
        };
        let old = at("2020-05-01T10:00:00Z");
        d.items = vec![
            FakeItem {
                merged: Some(old + TimeDelta::days(1)),
                updated: old + TimeDelta::days(1),
                ..item("PR_OLD", true, 1, REPO_A, LOGIN, old)
            },
            FakeItem {
                merged: Some(now - TimeDelta::days(9)),
                updated: now - TimeDelta::days(9),
                ..item("PR_ONE", true, 2, REPO_A, LOGIN, now - TimeDelta::days(10))
            },
            item("PR_OPEN", true, 3, REPO_B, LOGIN, now - TimeDelta::days(3)),
            FakeItem {
                reviews: vec![
                    FakeReview {
                        id: "PRR_MINE".to_owned(),
                        author: LOGIN.to_owned(),
                        state: "APPROVED".to_owned(),
                        submitted: Some(now - TimeDelta::days(2)),
                    },
                    FakeReview {
                        id: "PRR_PENDING".to_owned(),
                        author: LOGIN.to_owned(),
                        state: "PENDING".to_owned(),
                        submitted: None,
                    },
                    FakeReview {
                        id: "PRR_OTHER".to_owned(),
                        author: OTHER.to_owned(),
                        state: "COMMENTED".to_owned(),
                        submitted: Some(now - TimeDelta::days(2)),
                    },
                ],
                updated: now - TimeDelta::days(2),
                ..item("PR_OTHER", true, 7, REPO_C, OTHER, now - TimeDelta::days(4))
            },
            item("I_MINE", false, 8, REPO_C, LOGIN, now - TimeDelta::days(5)),
            item("I_OTHER", false, 9, REPO_C, OTHER, now - TimeDelta::days(5)),
        ];
        d.issue_comments = vec![
            FakeComment {
                id: "IC_ONE".to_owned(),
                repo: REPO_C.to_owned(),
                on: Some("I_MINE".to_owned()),
                created: now - TimeDelta::days(5),
                updated: now - TimeDelta::days(5),
            },
            FakeComment {
                id: "IC_TWO".to_owned(),
                repo: REPO_C.to_owned(),
                on: Some("PR_OTHER".to_owned()),
                created: now - TimeDelta::days(400),
                updated: now - TimeDelta::days(400),
            },
        ];
        d.commit_comments = vec![FakeComment {
            id: "CC_ONE".to_owned(),
            repo: REPO_A.to_owned(),
            on: None,
            created: now - TimeDelta::days(30),
            updated: now - TimeDelta::days(30),
        }];
    });
}

/// Ce que le premier passage sur `world` envoie : 257 commits (ni ceux d'un autre, ni la fourche), 5
/// faits de pull requests (2 fusionnées, 1 ouverte), 1 review, 1 issue, 3 commentaires.
const FIRST_PASS: u32 = 257 + 5 + 1 + 1 + 3;

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

const KINDS: [&str; 5] = [
    "github.commit",
    "github.pull_request",
    "github.review",
    "github.issue",
    "github.comment",
];

impl Bench {
    async fn new() -> Self {
        let db = TestDb::new().await;
        admin::set_password(&db.admin, "moi", PASSWORD)
            .await
            .unwrap();
        let service = admin::create_service_token(&db.admin, "mcp").await.unwrap();
        let token = admin::create_source(&db.admin, "github", Some("6 hours"), Some("GitHub"))
            .await
            .unwrap();
        for kind in KINDS {
            admin::grant_kind(&db.admin, "github", kind).await.unwrap();
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
            api_token: PAT.to_owned(),
            ingest_url: self.base.clone(),
            token: self.token.clone(),
            state_dir: self.state_dir.clone(),
            interval: Duration::from_secs(900),
            token_expires_at: Some(at("2027-11-07T23:00:00Z")),
        })
        .unwrap()
    }

    fn state_path(&self) -> PathBuf {
        self.state_dir.join("github-state.json")
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

    async fn card(&self, date: NaiveDate) -> Value {
        self.get(&format!("/v1/days/{date}/github")).await
    }

    async fn last_seen(&self) -> Option<DateTime<Utc>> {
        sqlx::query_scalar("SELECT last_seen_at FROM core.sources WHERE name = 'github'")
            .fetch_one(&self.db.app)
            .await
            .unwrap()
    }

    async fn status(&self) -> Option<Value> {
        let text: Option<String> =
            sqlx::query_scalar("SELECT status::text FROM core.sources WHERE name = 'github'")
                .fetch_one(&self.db.app)
                .await
                .unwrap();
        text.map(|t| serde_json::from_str(&t).unwrap())
    }

    async fn payload(&self, key: &str) -> Value {
        sqlx::query_scalar("SELECT payload FROM core.records WHERE key = $1")
            .bind(key)
            .fetch_one(&self.db.app)
            .await
            .unwrap()
    }

    async fn occurred(&self, key: &str) -> DateTime<Utc> {
        sqlx::query_scalar("SELECT occurred_at FROM core.records WHERE key = $1")
            .bind(key)
            .fetch_one(&self.db.app)
            .await
            .unwrap()
    }
}

fn operations(calls: &[(String, Value)], name: &str) -> Vec<Value> {
    calls
        .iter()
        .filter(|(op, _)| op == name)
        .map(|(_, vars)| vars.clone())
        .collect()
}

// ─── Le connecteur ───────────────────────────────────────────────────────────

#[tokio::test]
async fn the_first_pass_reads_the_whole_history_and_the_next_one_only_what_moved() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    world(&fake);
    let mut collector = bench.collector(&fake);
    let report = collector.tick().await.unwrap();
    assert_eq!(
        report.repos, 5,
        "union par id : A, B (deux fois), C, fourche, vide"
    );
    assert_eq!(report.repos_read, 3, "ni la fourche ni le dépôt vide");
    assert_eq!(report.commits, 257);
    assert_eq!(report.pull_requests, 5);
    assert_eq!(
        report.reviews, 1,
        "ni la review en attente, ni celle d'un autre"
    );
    assert_eq!(report.issues, 1);
    assert_eq!(report.comments, 3);
    assert_eq!(
        (report.accepted, report.duplicate, report.rejected),
        (FIRST_PASS, 0, 0)
    );
    assert!(!report.sso_required);
    assert!(bench.last_seen().await.is_some(), "signal de vie");
    assert_eq!(collector.complete_repos(), 3);

    let calls = fake.calls();
    let history = operations(&calls, "History");
    assert_eq!(history.len(), 3 + 1 + 1, "A en trois pages de 100, B, C");
    assert!(
        history
            .iter()
            .all(|v| v["since"].is_null() && v["author"] == VIEWER)
    );
    assert_eq!(
        history
            .iter()
            .filter(|v| v["id"] == REPO_A)
            .map(|v| v["after"].clone())
            .collect::<Vec<_>>(),
        [Value::Null, json!("100"), json!("200")]
    );
    // Les recherches, par années de création depuis celle du compte.
    let searches: Vec<String> = operations(&calls, "Search")
        .iter()
        .map(|v| v["q"].as_str().unwrap().to_owned())
        .collect();
    let years = (2019..=Utc::now().format("%Y").to_string().parse::<i32>().unwrap()).count();
    assert_eq!(searches.len(), 3 * years);
    assert_eq!(
        searches[0],
        format!("author:{LOGIN} is:pr created:2019-01-01..2019-12-31")
    );
    assert!(searches.contains(&format!(
        "reviewed-by:{LOGIN} is:pr -author:{LOGIN} created:2020-01-01..2020-12-31"
    )));

    // Les faits, à leurs dates : le commit à sa date d'auteur, la fusion à la sienne.
    assert_eq!(bench.events("code.commit").await, 257);
    assert_eq!(bench.events("code.pull_request").await, 3);
    assert_eq!(bench.events("code.pull_request_merged").await, 2);
    assert_eq!(bench.events("code.review").await, 1);
    assert_eq!(bench.events("code.issue").await, 1);
    assert_eq!(bench.events("code.comment").await, 3);
    assert_eq!(
        bench.occurred("pr:PR_OLD:merged").await,
        at("2020-05-02T10:00:00Z")
    );
    let first = bench.payload(&format!("commit:{}", oid('a', 0))).await;
    assert_eq!(first["repo"]["id"], REPO_A);
    assert_eq!(first["title"], format!("Commit sentinelle {}", oid('a', 0)));
    let journal: String =
        sqlx::query_scalar("SELECT string_agg(payload::text, '') FROM core.records")
            .fetch_one(&bench.db.app)
            .await
            .unwrap();
    for absent in [
        "Corps sentinelle",
        "Description sentinelle",
        "fourche-sentinelle",
        "PRR_PENDING",
        "PRR_OTHER",
        "I_OTHER",
        PAT,
    ] {
        assert!(!journal.contains(absent), "{absent} au journal");
    }
    assert_eq!(
        bench
            .db
            .count("SELECT count(*) FROM derived.interpretation_errors")
            .await,
        0
    );

    // Rien n'a bougé : aucun historique relu, les recherches par mise à jour, rien ne part.
    let report = collector.tick().await.unwrap();
    assert_eq!((report.accepted, report.duplicate), (0, 0));
    let calls = fake.calls();
    assert!(operations(&calls, "History").is_empty());
    let searches = operations(&calls, "Search");
    assert_eq!(searches.len(), 3);
    assert!(
        searches
            .iter()
            .all(|v| v["q"].as_str().unwrap().contains(" updated:>=")),
        "{searches:?}"
    );
}

#[tokio::test]
async fn a_cut_history_resumes_at_the_noted_page_without_duplicates() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    world(&fake);
    fake.with(|d| d.history_budget = Some(1));
    assert_eq!(
        bench.collector(&fake).tick().await.unwrap_err(),
        TickError::Service(ServiceError::Failed {
            status: 502,
            call: "history"
        })
    );
    assert_eq!(
        bench.journal("github.commit").await,
        100,
        "la première page"
    );
    let state = std::fs::read_to_string(bench.state_path()).unwrap();
    assert!(state.contains("\"after\":\"100\""), "{state}");
    fake.calls();

    // Un nouveau processus : il reprend à la page notée, sans rien renvoyer.
    fake.with(|d| d.history_budget = None);
    let report = bench.collector(&fake).tick().await.unwrap();
    let history = operations(&fake.calls(), "History");
    assert_eq!(history[0]["id"], REPO_A);
    assert_eq!(history[0]["after"], "100");
    assert_eq!(report.duplicate, 0);
    assert_eq!(report.accepted, FIRST_PASS - 100);
    assert_eq!(bench.journal("github.commit").await, 257);
}

#[tokio::test]
async fn a_lost_state_rereads_everything_as_duplicates() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    world(&fake);
    bench.collector(&fake).tick().await.unwrap();
    std::fs::remove_file(bench.state_path()).unwrap();
    let report = bench.collector(&fake).tick().await.unwrap();
    assert_eq!((report.accepted, report.duplicate), (0, FIRST_PASS));
    for (kind, count) in [
        ("github.commit", 257),
        ("github.pull_request", 5),
        ("github.review", 1),
        ("github.issue", 1),
        ("github.comment", 3),
    ] {
        assert_eq!(bench.journal(kind).await, count, "{kind}");
    }
}

#[tokio::test]
async fn a_late_push_rereads_30_days_and_dates_the_commit_at_its_author_date() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    world(&fake);
    let mut collector = bench.collector(&fake);
    collector.tick().await.unwrap();
    fake.calls();
    // Un commit écrit il y a 40 jours, rebasé et poussé aujourd'hui.
    let now = Utc::now();
    let authored = now - TimeDelta::days(40);
    let late = oid('9', 1);
    fake.repo(REPO_A, |r| {
        r.commits.push(FakeCommit {
            committed: now - TimeDelta::hours(1),
            ..commit(late.clone(), VIEWER, authored)
        });
        r.pushed_at = now;
    });
    let report = collector.tick().await.unwrap();
    assert_eq!((report.accepted, report.duplicate), (1, 0));
    let history = operations(&fake.calls(), "History");
    assert_eq!(history.len(), 1, "seul A a été poussé");
    assert_eq!(history[0]["id"], REPO_A);
    let since = at(history[0]["since"].as_str().unwrap());
    assert!((since - (now - TimeDelta::days(30))).num_minutes().abs() < 5);
    assert_eq!(
        bench.occurred(&format!("commit:{late}")).await,
        authored.with_nanosecond(0).unwrap()
    );
    let card = bench.card(paris_day(authored)).await;
    let shas: Vec<&str> = card["repos"][0]["commits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["sha"].as_str().unwrap())
        .collect();
    assert!(shas.contains(&late.as_str()), "{card}");

    // Le même dépôt poussé encore : la fenêtre est relue, sans rien renvoyer.
    fake.repo(REPO_A, |r| r.pushed_at = now + TimeDelta::seconds(1));
    let report = bench.collector(&fake).tick().await.unwrap();
    assert_eq!(report.accepted, 0);
    assert!(
        report.duplicate > 0,
        "la fenêtre relue revient en duplicate"
    );
}

#[tokio::test]
async fn a_renamed_repository_keeps_its_key_and_is_discovered_once() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    world(&fake);
    let mut collector = bench.collector(&fake);
    collector.tick().await.unwrap();
    let now = Utc::now();
    fake.repo(REPO_B, |r| {
        r.name = "orga-sentinelle/depot-renomme".to_owned();
        r.commits
            .push(commit(oid('b', 99), VIEWER, now - TimeDelta::minutes(30)));
        r.pushed_at = now;
    });
    let report = collector.tick().await.unwrap();
    assert_eq!(report.accepted, 1);
    let new = bench.payload(&format!("commit:{}", oid('b', 99))).await;
    assert_eq!(
        new["repo"],
        json!({"id": REPO_B, "name": "orga-sentinelle/depot-renomme"})
    );
    bench.settled().await;
    let names: Vec<(String, i64)> = sqlx::query_as(
        "SELECT display, count(*) FROM derived.live_event_items
         WHERE metric = 'code.commit' AND level = 'repo' AND item_key = $1
         GROUP BY display ORDER BY display",
    )
    .bind(REPO_B)
    .fetch_all(&bench.db.app)
    .await
    .unwrap();
    assert_eq!(
        names,
        [
            ("orga-sentinelle/depot-renomme".to_owned(), 1),
            ("orga-sentinelle/depot-sentinelle-b".to_owned(), 5)
        ]
    );
    let card = bench.card(paris_day(now - TimeDelta::minutes(30))).await;
    let repo = card["repos"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["key"] == REPO_B)
        .unwrap()
        .clone();
    assert_eq!(repo["name"], "orga-sentinelle/depot-renomme");
    assert_eq!(
        repo["url"],
        "https://github.com/orga-sentinelle/depot-renomme"
    );
    assert!(
        card["discoveries"]
            .as_array()
            .unwrap()
            .iter()
            .all(|d| d["key"] != REPO_B),
        "déjà commité avant le renommage : {card}"
    );
    let (_, found) = bench
        .as_mcp(&format!(
            "/v1/discoveries?metric=code.commit&level=repo&from=2000-01-01&to={}",
            paris_today(Utc::now()) + Days::new(1)
        ))
        .await;
    let keys: Vec<&str> = found["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["key"].as_str().unwrap())
        .collect();
    assert_eq!(keys.iter().filter(|k| **k == REPO_B).count(), 1, "{found}");
    assert_eq!(keys.len(), 3, "A, B, C : {found}");
}

#[tokio::test]
async fn an_opened_pull_request_is_merged_later_without_counting_twice() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    world(&fake);
    let mut collector = bench.collector(&fake);
    collector.tick().await.unwrap();
    let now = Utc::now();
    fake.with(|d| {
        let pr = d.items.iter_mut().find(|i| i.id == "PR_OPEN").unwrap();
        pr.merged = Some(now - TimeDelta::minutes(10));
        pr.updated = now - TimeDelta::minutes(10);
    });
    let report = collector.tick().await.unwrap();
    assert_eq!(
        (report.accepted, report.pull_requests),
        (1, 2),
        "l'ouverture relue, déjà accusée ; la fusion part"
    );
    assert_eq!(bench.journal("github.pull_request").await, 6);
    let merged = bench.payload("pr:PR_OPEN:merged").await;
    assert_eq!(merged["action"], "merged");
    assert_eq!(merged["additions"], 30);
    assert_eq!(bench.events("code.pull_request").await, 3);
    assert_eq!(bench.events("code.pull_request_merged").await, 3);

    // Le calendrier compte les ouvertures, à leur jour ; une fusion n'y compte jamais.
    let created = paris_day(now - TimeDelta::days(3));
    let month = bench
        .get(&format!("/v1/months/{}", created.format("%Y-%m")))
        .await;
    let day = month["days"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["date"] == created.to_string())
        .unwrap()
        .clone();
    assert_eq!(day["values"]["code.pull_request"], 1.0, "{day}");
    assert_eq!(day["values"].get("code.pull_request_merged"), None);
}

#[tokio::test]
async fn a_rejected_token_sends_nothing_and_no_sign_of_life() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    world(&fake);
    fake.with(|d| d.status = 401);
    let mut collector = bench.collector(&fake);
    assert_eq!(
        collector.tick().await.unwrap_err(),
        TickError::Service(ServiceError::KeyRejected { status: 401 })
    );
    assert_eq!(collector.pass().await, Duration::from_secs(60));
    let state = bench.status().await.unwrap();
    assert_eq!(
        (&state["state"], &state["reason"]),
        (&json!("action_required"), &json!("key_rejected"))
    );
    assert_eq!(
        at(state["expires_at"].as_str().unwrap()),
        at("2027-11-07T23:00:00Z")
    );
    assert_eq!(state["last_collected_at"], Value::Null);
    assert_eq!(
        bench.last_seen().await,
        None,
        "le seuil de silence alertera"
    );
    assert_eq!(bench.journal("github.commit").await, 0);

    // Le token remplacé : tout repart, et l'état revient à `ok`, avec l'échéance.
    fake.with(|d| d.status = 200);
    bench.collector(&fake).pass().await;
    let state = bench.status().await.unwrap();
    assert_eq!(state["state"], "ok");
    assert!(state["last_collected_at"].is_string());
    assert!(state["expires_at"].is_string());
    assert_eq!(bench.journal("github.commit").await, 257);
}

#[tokio::test]
async fn every_rate_limit_waits_and_keeps_the_state() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    world(&fake);
    // Une limite secondaire, avec son délai.
    fake.with(|d| d.status = 429);
    let mut collector = bench.collector(&fake);
    assert_eq!(collector.pass().await, Duration::from_secs(120));
    let state = bench.status().await.unwrap();
    assert_eq!(
        (&state["state"], &state["reason"]),
        (&json!("error"), &json!("rate_limited"))
    );
    // La limite horaire épuisée : jusqu'à sa remise à zéro.
    fake.with(|d| d.status = 403);
    let TickError::Service(ServiceError::RateLimited { retry_after }) =
        collector.tick().await.unwrap_err()
    else {
        panic!("limite attendue");
    };
    assert!(
        (290..=300).contains(&retry_after.as_secs()),
        "{retry_after:?}"
    );
    // La même, dite dans le corps d'une réponse 200.
    fake.with(|d| {
        d.status = 200;
        d.rate_limited_body = true;
    });
    assert!(matches!(
        collector.tick().await.unwrap_err(),
        TickError::Service(ServiceError::RateLimited { .. })
    ));
    // Sous 100 points restants, le passage s'arrête de lui-même jusqu'à la remise à zéro.
    fake.with(|d| {
        d.rate_limited_body = false;
        d.remaining = 50;
    });
    fake.calls();
    let TickError::Service(ServiceError::RateLimited { retry_after }) =
        collector.tick().await.unwrap_err()
    else {
        panic!("limite attendue");
    };
    assert!(
        (590..=600).contains(&retry_after.as_secs()),
        "{retry_after:?}"
    );
    assert_eq!(fake.calls().len(), 1, "seulement le compte, puis l'arrêt");
    // La limite retombée : le passage aboutit.
    fake.with(|d| d.remaining = 4999);
    let report = bench.collector(&fake).tick().await.unwrap();
    assert_eq!(report.accepted, FIRST_PASS);
}

#[tokio::test]
async fn an_organization_that_requires_saml_is_skipped_and_asks_for_an_action() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    world(&fake);
    fake.with(|d| {
        d.organizations.push(SAML_ORG.to_owned());
        d.saml.insert(SAML_ORG.to_owned());
    });
    let mut collector = bench.collector(&fake);
    let report = collector.tick().await.unwrap();
    assert!(report.sso_required);
    assert_eq!(report.accepted, FIRST_PASS, "le reste est collecté");
    collector.pass().await;
    let state = bench.status().await.unwrap();
    assert_eq!(
        (&state["state"], &state["reason"]),
        (&json!("action_required"), &json!("sso_required"))
    );
    assert!(state["last_collected_at"].is_string());

    // L'organisation autorisée : ses dépôts arrivent, l'état revient à `ok`.
    fake.with(|d| d.saml.clear());
    let report = collector.tick().await.unwrap();
    assert!(!report.sso_required);
    assert_eq!(report.accepted, 1, "le commit du dépôt SAML");
    collector.pass().await;
    assert_eq!(bench.status().await.unwrap()["state"], "ok");
}

#[tokio::test]
async fn a_rebuild_gives_no_difference() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    world(&fake);
    bench.collector(&fake).tick().await.unwrap();
    let today = paris_today(Utc::now());
    let days: Vec<NaiveDate> = (2..12).map(|n| today - Days::new(n)).collect();
    let mut before = Vec::new();
    for day in &days {
        before.push(bench.card(*day).await);
    }
    let rebuilt = replay::rebuild(&bench.db.admin, INTERPRETERS, false)
        .await
        .unwrap();
    assert!(rebuilt.switched);
    assert!(rebuilt.comparison.differences.is_empty());
    for (day, before) in days.iter().zip(before) {
        assert_eq!(bench.card(*day).await, before);
    }
}

// ─── La carte du jour et les lectures génériques ─────────────────────────────

/// Les enregistrements envoyés tels quels, à dates fixes, avec le token de la source.
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
async fn the_day_card_gives_each_repository_its_activity() {
    let bench = Bench::new().await;
    let fixture = read_json(&repo_path("contract/fixtures/valid/github.json"));
    send(&bench, fixture["records"].as_array().unwrap().clone()).await;
    // Le lendemain, un autre commit sur le même dépôt : plus une découverte.
    let repo = RepoRef {
        id: "R_kgDOFICTIF1".to_owned(),
        name: "moi/depot-fictif".to_owned(),
    };
    let next = github::commit_record(
        &repo,
        &json!({"oid": "fedcba9", "authoredDate": "2026-11-07T08:00:00Z",
                "messageHeadline": "Encore", "additions": null, "deletions": null}),
    )
    .unwrap();
    send(&bench, vec![next]).await;
    assert_eq!(
        bench
            .db
            .count("SELECT count(*) FROM derived.interpretation_errors")
            .await,
        0
    );

    let card = bench
        .card(NaiveDate::from_ymd_opt(2026, 11, 6).unwrap())
        .await;
    assert_eq!(
        card,
        json!({
            "date": "2026-11-06", "commits": 1, "additions": 40, "deletions": 2,
            "pull_requests": 1, "merged": 1, "reviews": 1, "issues": 1, "comments": 1,
            "repos": [
                {
                    "key": "R_kgDOFICTIF1", "name": "moi/depot-fictif",
                    "url": "https://github.com/moi/depot-fictif",
                    "commits": [{
                        "sha": "0123456789abcdef0123456789abcdef01234567", "time": "21:15",
                        "title": "Ajoute le compteur fictif", "additions": 40, "deletions": 2,
                        "url": "https://github.com/moi/depot-fictif/commit/0123456789abcdef0123456789abcdef01234567",
                    }],
                    "additions": 40, "deletions": 2,
                    "pull_requests": [
                        {"action": "opened", "number": 12, "title": "Compteur fictif", "time": "21:30",
                         "url": "https://github.com/moi/depot-fictif/pull/12",
                         "additions": null, "deletions": null},
                        {"action": "merged", "number": 12, "title": "Compteur fictif", "time": "22:00",
                         "url": "https://github.com/moi/depot-fictif/pull/12",
                         "additions": 40, "deletions": 2},
                    ],
                    "reviews": [], "issues": [], "comments": [],
                },
                {
                    "key": "R_kgDOFICTIF2", "name": "orga-fictive/autre-depot",
                    "url": "https://github.com/orga-fictive/autre-depot",
                    "commits": [], "additions": 0, "deletions": 0, "pull_requests": [],
                    "reviews": [{"state": "approved", "number": 3, "title": "La PR d'un autre",
                                 "time": "11:05",
                                 "url": "https://github.com/orga-fictive/autre-depot/pull/3#pullrequestreview-1"}],
                    "issues": [{"number": 4, "title": "Un bogue fictif", "time": "10:00",
                                "url": "https://github.com/orga-fictive/autre-depot/issues/4"}],
                    "comments": [{"target": "issue", "number": 4, "title": "Un bogue fictif",
                                  "time": "10:30",
                                  "url": "https://github.com/orga-fictive/autre-depot/issues/4#issuecomment-1"}],
                },
            ],
            "discoveries": [{"level": "repo", "key": "R_kgDOFICTIF1", "name": "moi/depot-fictif"}],
        })
    );
    let card = bench
        .card(NaiveDate::from_ymd_opt(2026, 11, 7).unwrap())
        .await;
    assert_eq!(card["commits"], 1);
    assert_eq!(card["additions"], 0, "sans lignes connues");
    assert_eq!(card["repos"][0]["commits"][0]["additions"], Value::Null);
    assert_eq!(card["discoveries"], json!([]));
    let empty = bench
        .card(NaiveDate::from_ymd_opt(2026, 11, 20).unwrap())
        .await;
    assert_eq!(empty["repos"], json!([]));
    assert_eq!(
        (&empty["commits"], &empty["comments"]),
        (&json!(0), &json!(0))
    );

    // Le calendrier : commits et pull requests ouvertes ; ni fusions, ni reviews, ni issues, ni
    // commentaires ; avant la première donnée, rien, pas zéro.
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
    assert_eq!(values("2026-11-06")["code.commit"], 1.0);
    assert_eq!(values("2026-11-06")["code.pull_request"], 1.0);
    for absent in [
        "code.pull_request_merged",
        "code.review",
        "code.issue",
        "code.comment",
    ] {
        assert_eq!(values("2026-11-06").get(absent), None, "{absent}");
    }
    assert_eq!(values("2026-11-07")["code.pull_request"], 0.0);
    assert_eq!(values("2026-11-01")["code.commit"], Value::Null);

    // Le MCP : comptes, dépôts et lignes, jamais un titre ; pas la carte.
    let (status, list) = bench
        .as_mcp("/v1/occurrences?metric=code.commit&from=2026-11-06&to=2026-11-06&form=list")
        .await;
    assert_eq!(status, 200, "{list}");
    assert_eq!(
        list["occurrences"][0]["details"],
        json!({"additions": 40, "deletions": 2})
    );
    assert_eq!(list["occurrences"][0]["items"]["repo"], "moi/depot-fictif");
    let (_, daily) = bench
        .as_mcp("/v1/occurrences?metric=code.commit&from=2026-11-01&to=2026-11-30&form=daily")
        .await;
    assert_eq!(
        daily["days"],
        json!([{"date": "2026-11-06", "count": 1}, {"date": "2026-11-07", "count": 1}])
    );
    let (status, by_repo) = bench
        .as_mcp("/v1/occurrences?metric=code.commit&level=repo&key=R_kgDOFICTIF1&from=2026-11-01&to=2026-11-30&form=daily")
        .await;
    assert_eq!(status, 200, "{by_repo}");
    assert_eq!(by_repo["days"].as_array().unwrap().len(), 2);
    for (metric, details) in [
        // Sans détail partagé : aucun détail, ni numéro ni titre.
        ("code.pull_request", Value::Null),
        (
            "code.pull_request_merged",
            json!({"additions": 40, "deletions": 2}),
        ),
        ("code.review", json!({"state": "approved"})),
        ("code.issue", Value::Null),
        ("code.comment", json!({"target": "issue"})),
    ] {
        let (status, found) = bench
            .as_mcp(&format!(
                "/v1/occurrences?metric={metric}&from=2026-11-06&to=2026-11-06&form=list"
            ))
            .await;
        assert_eq!(status, 200, "{found}");
        assert_eq!(found["occurrences"][0]["details"], details, "{metric}");
    }
    let (_, everything) = bench
        .as_mcp("/v1/occurrences?metric=code.commit&from=2026-11-01&to=2026-11-30&form=list")
        .await;
    for title in ["Ajoute le compteur", "Encore", "Compteur fictif"] {
        assert!(!everything.to_string().contains(title), "{everything}");
    }
    assert_eq!(bench.as_mcp("/v1/days/2026-11-06/github").await.0, 401);
}

// ─── Le vrai binaire : ce qui sort dans les logs ──────────────────────────────

fn collect_process(fake: &Fake, bench: &Bench) -> (std::process::Child, PathBuf) {
    let log = bench.state_dir.join("collector.log");
    let file = std::fs::File::create(&log).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_life-tracker"))
        .arg("collect")
        .env_clear()
        .env("RUST_LOG", "trace")
        .env("GITHUB_API_URL", &fake.url)
        .env("GITHUB_API_TOKEN", PAT)
        .env("GITHUB_INGEST_TOKEN", &bench.token)
        .env("GITHUB_TOKEN_EXPIRES_ON", "2027-11-07")
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
async fn logs_never_cite_the_token_the_account_a_repository_or_a_title() {
    let bench = Bench::new().await;
    let fake = Fake::start().await;
    world(&fake);
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
    assert_eq!(
        at(status["expires_at"].as_str().unwrap()),
        at("2027-11-07T23:00:00Z"),
        "la fin du jour de Paris"
    );
    let logs = logs_until(child, &log, "github pass completed").await;
    assert!(
        logs.contains(&format!("\"accepted\":{FIRST_PASS}")),
        "{logs}"
    );
    let secrets = [
        PAT,
        bench.token.as_str(),
        LOGIN,
        VIEWER,
        ORG,
        "sentinelle",
        "Titre",
        REPO_A,
    ];
    for secret in secrets {
        assert!(!logs.contains(secret), "fuite de {secret} : {logs}");
    }
    assert!(bench.state_path().exists());
    assert!(
        !bench.state_dir.join("state.json").exists(),
        "Last.fm n'est pas configuré"
    );
    let state = std::fs::read_to_string(bench.state_path()).unwrap();
    for secret in [PAT, LOGIN, VIEWER, ORG, "sentinelle"] {
        assert!(!state.contains(secret), "{secret} dans l'état : {state}");
    }

    fake.with(|d| d.status = 401);
    let (child, log) = collect_process(&fake, &bench);
    let logs = logs_until(child, &log, "token GitHub refusé").await;
    for secret in secrets {
        assert!(!logs.contains(secret), "fuite de {secret} : {logs}");
    }
}

#[test]
fn what_the_connector_sends_respects_the_contract() {
    let schema = validator(None);
    let repo = RepoRef {
        id: REPO_A.to_owned(),
        name: "moi/depot".to_owned(),
    };
    let repository = json!({"id": REPO_A, "nameWithOwner": "moi/depot"});
    let mut records = vec![
        github::commit_record(
            &repo,
            &json!({"oid": oid('a', 1), "authoredDate": "2026-11-06T21:15:00+01:00",
                    "committedDate": "2026-11-06T21:20:00+01:00", "messageHeadline": "Titre",
                    "additions": 1, "deletions": 0, "url": "https://github.com/moi/depot/commit/a"}),
        )
        .unwrap(),
        github::issue_record(&json!({"id": "I_1", "number": 1, "title": "T",
            "createdAt": "2026-11-06T10:00:00Z", "repository": repository}))
        .unwrap(),
        github::issue_comment_record(&json!({"id": "IC_1", "createdAt": "2026-11-06T10:00:00Z",
            "repository": repository, "issue": {"number": 1, "title": "T"}, "pullRequest": null}))
        .unwrap(),
        github::commit_comment_record(&json!({"id": "CC_1", "createdAt": "2026-11-06T10:00:00Z",
            "repository": repository, "commit": {"messageHeadline": "T"}}))
        .unwrap(),
    ];
    records.extend(
        github::pull_request_records(&json!({"id": "PR_1", "number": 2, "title": "T",
            "createdAt": "2026-11-05T10:00:00Z", "mergedAt": "2026-11-06T10:00:00Z",
            "additions": 3, "deletions": 1, "repository": repository}))
        .unwrap(),
    );
    let mut skipped = 0;
    records.extend(github::review_records(
        &json!({"number": 2, "title": "T", "repository": repository,
                "reviews": {"nodes": [{"id": "PRR_1", "state": "COMMENTED",
                                       "submittedAt": "2026-11-06T10:00:00Z"}]}}),
        &mut skipped,
    ));
    assert_eq!(records.len(), 7);
    let batch = json!({ "records": records });
    let errors: Vec<_> = schema.iter_errors(&batch).map(|e| e.to_string()).collect();
    assert!(errors.is_empty(), "{errors:?}");
}
