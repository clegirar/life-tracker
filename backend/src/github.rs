//! La carte GitHub de la page du jour : par dépôt, les commits, les pull requests ouvertes et
//! fusionnées, les reviews, les issues et les commentaires du jour, avec les totaux et les dépôts sur
//! lesquels j'ai commité ce jour-là pour la première fois. Une lecture de la web app seulement,
//! derrière la session : elle donne les titres, que le MCP ne voit pas (il lit les mêmes faits par les
//! lectures génériques, sans eux).
//!
//! Un dépôt est désigné par son id GraphQL (l'élément `repo`), qui survit à un renommage ; son nom est
//! celui de son dernier fait du jour. Un jour sans fait donne des listes vides et des zéros, comme les
//! autres cartes : c'est le calendrier qui distingue un jour sans données d'un jour sans activité.

use chrono::NaiveDate;
use serde::Serialize;
use serde_json::Value;
use sqlx::PgPool;

use crate::read::{self, DayDiscovery};

const COMMIT: &str = "code.commit";
const PULL_REQUEST: &str = "code.pull_request";
const MERGED: &str = "code.pull_request_merged";
const REVIEW: &str = "code.review";
const ISSUE: &str = "code.issue";
const COMMENT: &str = "code.comment";
const LEVEL: &str = "repo";

#[derive(Serialize)]
pub struct Commit {
    sha: String,
    /// Heure de Paris, `HH:MM`, de la date d'auteur.
    time: String,
    title: Option<String>,
    /// `null` quand GitHub ne les donne pas (très gros commit).
    additions: Option<i64>,
    deletions: Option<i64>,
    url: Option<String>,
}

#[derive(Serialize)]
pub struct PullRequest {
    /// `opened` (à la création) ou `merged` (à la fusion).
    action: &'static str,
    number: Option<i64>,
    title: Option<String>,
    time: String,
    url: Option<String>,
    /// Les lignes de la pull request à la fusion ; `null` pour une ouverture.
    additions: Option<i64>,
    deletions: Option<i64>,
}

#[derive(Serialize)]
pub struct Review {
    /// `approved`, `changes_requested`, `commented` ou `dismissed`.
    state: Option<String>,
    number: Option<i64>,
    title: Option<String>,
    time: String,
    url: Option<String>,
}

#[derive(Serialize)]
pub struct Issue {
    number: Option<i64>,
    title: Option<String>,
    time: String,
    url: Option<String>,
}

#[derive(Serialize)]
pub struct Comment {
    /// `issue`, `pull_request` ou `commit`.
    target: Option<String>,
    /// `null` pour un commentaire de commit.
    number: Option<i64>,
    /// Le titre de l'issue ou de la pull request, la première ligne du message du commit.
    title: Option<String>,
    time: String,
    url: Option<String>,
}

#[derive(Serialize)]
pub struct Repo {
    /// L'id GraphQL du dépôt.
    key: String,
    /// `propriétaire/nom`, au dernier fait du jour.
    name: String,
    url: String,
    /// Chaque liste par heure.
    commits: Vec<Commit>,
    /// Les lignes des commits du jour.
    additions: i64,
    deletions: i64,
    pull_requests: Vec<PullRequest>,
    reviews: Vec<Review>,
    issues: Vec<Issue>,
    comments: Vec<Comment>,
}

#[derive(Serialize)]
pub struct DayGithub {
    date: NaiveDate,
    commits: i64,
    /// Les lignes ajoutées et supprimées par les commits du jour (sans ceux dont GitHub ne les donne pas).
    additions: i64,
    deletions: i64,
    /// Les pull requests ouvertes ce jour-là.
    pull_requests: i64,
    /// Les pull requests fusionnées ce jour-là.
    merged: i64,
    reviews: i64,
    issues: i64,
    comments: i64,
    /// Par nombre de commits décroissant, puis par nom.
    repos: Vec<Repo>,
    /// Les dépôts sur lesquels j'ai commité ce jour-là pour la première fois, par première heure :
    /// `{level: "repo", key, name}`.
    discoveries: Vec<DayDiscovery>,
}

fn string(details: &Value, field: &str) -> Option<String> {
    details
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn number(details: &Value, field: &str) -> Option<i64> {
    details.get(field).and_then(Value::as_i64)
}

/// Le fuseau en sous-requête et la génération fixée, comme toutes les lectures (voir `api::get_day`).
pub async fn day(pool: &PgPool, date: NaiveDate) -> Result<DayGithub, sqlx::Error> {
    let metrics: Vec<String> = [COMMIT, PULL_REQUEST, MERGED, REVIEW, ISSUE, COMMENT]
        .iter()
        .map(|m| (*m).to_owned())
        .collect();
    let rows = sqlx::query!(
        r#"SELECT e.metric AS "metric!", r.item_key AS "key!", r.display AS "name!",
                  e.details AS "details!",
                  to_char(e.occurred_at AT TIME ZONE (SELECT timezone FROM core.settings), 'HH24:MI') AS "time!"
           FROM derived.live_events e
           JOIN derived.event_items r
             ON r.generation = e.generation AND r.seq = e.seq AND r.ordinal = e.ordinal AND r.level = $2
           WHERE e.generation = (SELECT id FROM derived.generations WHERE state = 'active')
             AND e.metric = ANY($3)
             AND e.occurred_at >= ($1::date::timestamp AT TIME ZONE (SELECT timezone FROM core.settings))
             AND e.occurred_at < (($1::date + 1)::timestamp AT TIME ZONE (SELECT timezone FROM core.settings))
           ORDER BY e.occurred_at, e.seq, e.ordinal"#,
        date,
        LEVEL,
        &metrics,
    )
    .fetch_all(pool)
    .await?;
    let discoveries = read::day_discoveries(pool, COMMIT, &[LEVEL], date).await?;

    let mut repos: Vec<Repo> = Vec::new();
    for row in rows {
        let index = match repos.iter().position(|r| r.key == row.key) {
            Some(index) => index,
            None => {
                repos.push(Repo {
                    key: row.key.clone(),
                    name: String::new(),
                    url: String::new(),
                    commits: Vec::new(),
                    additions: 0,
                    deletions: 0,
                    pull_requests: Vec::new(),
                    reviews: Vec::new(),
                    issues: Vec::new(),
                    comments: Vec::new(),
                });
                repos.len() - 1
            }
        };
        let repo = &mut repos[index];
        repo.url = format!("https://github.com/{}", row.name);
        repo.name = row.name;
        let d = &row.details;
        match row.metric.as_str() {
            COMMIT => {
                let commit = Commit {
                    sha: string(d, "sha").unwrap_or_default(),
                    time: row.time,
                    title: string(d, "title"),
                    additions: number(d, "additions"),
                    deletions: number(d, "deletions"),
                    url: string(d, "url"),
                };
                repo.additions += commit.additions.unwrap_or(0);
                repo.deletions += commit.deletions.unwrap_or(0);
                repo.commits.push(commit);
            }
            PULL_REQUEST | MERGED => {
                let merged = row.metric == MERGED;
                repo.pull_requests.push(PullRequest {
                    action: if merged { "merged" } else { "opened" },
                    number: number(d, "number"),
                    title: string(d, "title"),
                    time: row.time,
                    url: string(d, "url"),
                    additions: number(d, "additions").filter(|_| merged),
                    deletions: number(d, "deletions").filter(|_| merged),
                });
            }
            REVIEW => repo.reviews.push(Review {
                state: string(d, "state"),
                number: number(d, "number"),
                title: string(d, "title"),
                time: row.time,
                url: string(d, "url"),
            }),
            ISSUE => repo.issues.push(Issue {
                number: number(d, "number"),
                title: string(d, "title"),
                time: row.time,
                url: string(d, "url"),
            }),
            _ => repo.comments.push(Comment {
                target: string(d, "target"),
                number: number(d, "number"),
                title: string(d, "title"),
                time: row.time,
                url: string(d, "url"),
            }),
        }
    }
    repos.sort_by(|a, b| {
        b.commits
            .len()
            .cmp(&a.commits.len())
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.key.cmp(&b.key))
    });

    let total = |f: fn(&Repo) -> usize| repos.iter().map(f).sum::<usize>() as i64;
    let opened = |merged: bool| {
        repos
            .iter()
            .flat_map(|r| &r.pull_requests)
            .filter(|p| (p.action == "merged") == merged)
            .count() as i64
    };
    Ok(DayGithub {
        date,
        commits: total(|r| r.commits.len()),
        additions: repos.iter().map(|r| r.additions).sum(),
        deletions: repos.iter().map(|r| r.deletions).sum(),
        pull_requests: opened(false),
        merged: opened(true),
        reviews: total(|r| r.reviews.len()),
        issues: total(|r| r.issues.len()),
        comments: total(|r| r.comments.len()),
        repos,
        discoveries,
    })
}
