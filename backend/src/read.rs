//! Lectures génériques de l'API, pour les outils du serveur MCP (étape 9) : inventaire, recherche,
//! occurrences, durée cumulée, un élément (découverte et dernière fois), classement, découvertes.
//!
//! Comme le reste de la lecture : calculé à chaque appel, jamais stocké (principe 3), sur
//! `derived.live_*` de la génération active, les jours découpés à minuit heure de Paris d'après
//! `core.settings` (règle 10). Tout vient des registres : la métrique est un paramètre, ses niveaux
//! d'éléments, sa découverte et la source de sa durée sont lus dans `core.metrics` (principe 12).
//! Le fuseau en sous-requête et le numéro de génération fixé, comme partout : voir `api::get_day`.

use std::collections::HashMap;

use chrono::{DateTime, Datelike, NaiveDate, Utc};
use serde::Serialize;
use sqlx::PgPool;

use crate::music::fold;
use crate::privacy::Zones;

pub const MAX_LIST: i64 = 1000;
pub const MAX_TOP: i64 = 200;
pub const MAX_SEARCH: i64 = 50;

pub enum ReadError {
    /// Un paramètre absent ou invalide : le nom du paramètre, jamais sa valeur.
    Invalid(&'static str),
    NotFound,
    Db(sqlx::Error),
}

impl From<sqlx::Error> for ReadError {
    fn from(err: sqlx::Error) -> Self {
        Self::Db(err)
    }
}

type Result<T> = std::result::Result<T, ReadError>;

/// Ce que le registre dit d'une métrique, et qui décide des lectures possibles.
pub struct Metric {
    pub metric: String,
    pub shape: String,
    pub item_levels: Vec<String>,
    pub discovery: bool,
    pub duration_from: Option<String>,
    /// Les champs des détails qu'une lecture générique peut rendre (registre, `shared_details`).
    pub shared_details: Vec<String>,
}

/// Ce qu'une lecture rend des coordonnées d'un événement : tout pour la session de la web app,
/// et pour le jeton de service, ce qui est hors des zones de confidentialité (`privacy`).
#[derive(Clone, Copy)]
pub enum Exposure<'a> {
    Full,
    Zoned(&'a Zones),
}

impl Exposure<'_> {
    /// Les détails partagés d'un événement : les seuls champs que le registre liste pour sa métrique,
    /// coordonnées masquées selon l'exposition. Aucun champ listé : rien, comme pour la musique.
    pub fn details(
        self,
        metric: &Metric,
        details: &serde_json::Value,
    ) -> Option<serde_json::Value> {
        if metric.shared_details.is_empty() {
            return None;
        }
        let mut shared: serde_json::Map<String, serde_json::Value> = metric
            .shared_details
            .iter()
            .filter_map(|field| Some((field.clone(), details.get(field)?.clone())))
            .collect();
        if let Self::Zoned(zones) = self {
            zones.hide_coordinates(&mut shared);
        }
        Some(serde_json::Value::Object(shared))
    }
}

impl Metric {
    pub fn check_level(&self, level: &str) -> Result<()> {
        if self.item_levels.iter().any(|l| l == level) {
            Ok(())
        } else {
            Err(ReadError::Invalid("level"))
        }
    }
}

pub async fn metric(pool: &PgPool, name: &str) -> Result<Metric> {
    sqlx::query_as!(
        Metric,
        r#"SELECT metric, shape::text AS "shape!", item_levels, discovery, duration_from,
                  shared_details
           FROM core.metrics WHERE metric = $1"#,
        name,
    )
    .fetch_optional(pool)
    .await?
    .ok_or(ReadError::Invalid("metric"))
}

/// Une période en jours de Paris, bornes comprises.
#[derive(Clone, Copy)]
pub struct Period {
    pub from: NaiveDate,
    pub to: NaiveDate,
}

impl Period {
    pub fn parse(from: Option<&str>, to: Option<&str>) -> Result<Self> {
        let from = from.and_then(parse_day).ok_or(ReadError::Invalid("from"))?;
        let to = to.and_then(parse_day).ok_or(ReadError::Invalid("to"))?;
        if to < from {
            return Err(ReadError::Invalid("to"));
        }
        Ok(Self { from, to })
    }
}

/// Ce qu'une source couvre d'une métrique : sa première et sa dernière donnée vivante, et leurs jours
/// de Paris.
pub struct SourceSpan {
    pub metric: String,
    pub source: String,
    pub first_at: DateTime<Utc>,
    pub last_at: DateTime<Utc>,
    pub first_day: NaiveDate,
    pub last_day: NaiveDate,
}

/// La couverture de chaque source pour ces métriques, de la plus ancienne à la plus récente. Lue sur
/// `derived.live_events`, donc ce que la lecture montre vraiment : une source qui ne fait plus foi
/// s'arrête là où la suivante commence, sans faux trou. Chaque couple (métrique, source) se lit au bord
/// de l'index `(generation, metric, source_id, occurred_at)`, au premier et au dernier événement vivant :
/// quelques millisecondes, au lieu d'un parcours de tout l'historique (journal des décisions de
/// l'architecture, 8 nov. 2026). Un couple sans événement ne donne rien.
pub async fn source_spans(
    pool: &PgPool,
    metrics: &[String],
) -> std::result::Result<Vec<SourceSpan>, sqlx::Error> {
    sqlx::query_as!(
        SourceSpan,
        r#"SELECT m.metric AS "metric!", s.name AS "source!",
                  f.at AS "first_at!", l.at AS "last_at!",
                  (f.at AT TIME ZONE (SELECT timezone FROM core.settings))::date AS "first_day!",
                  (l.at AT TIME ZONE (SELECT timezone FROM core.settings))::date AS "last_day!"
           FROM unnest($1::text[]) AS m(metric)
           CROSS JOIN core.sources s
           CROSS JOIN LATERAL (
             SELECT e.occurred_at AS at
             FROM derived.live_events e
             WHERE e.generation = (SELECT id FROM derived.generations WHERE state = 'active')
               AND e.metric = m.metric AND e.source_id = s.id
             ORDER BY e.occurred_at
             LIMIT 1
           ) f
           CROSS JOIN LATERAL (
             SELECT e.occurred_at AS at
             FROM derived.live_events e
             WHERE e.generation = (SELECT id FROM derived.generations WHERE state = 'active')
               AND e.metric = m.metric AND e.source_id = s.id
             ORDER BY e.occurred_at DESC
             LIMIT 1
           ) l
           ORDER BY f.at, m.metric, s.name"#,
        metrics,
    )
    .fetch_all(pool)
    .await
}

/// `AAAA-MM-JJ` strict.
pub fn parse_day(value: &str) -> Option<NaiveDate> {
    (value.len() == 10)
        .then(|| NaiveDate::parse_from_str(value, "%Y-%m-%d").ok())
        .flatten()
}

pub fn parse_limit(
    value: Option<&str>,
    default: i64,
    max: i64,
    field: &'static str,
) -> Result<i64> {
    match value {
        None => Ok(default),
        Some(v) => v
            .parse::<i64>()
            .ok()
            .filter(|n| (1..=max).contains(n))
            .ok_or(ReadError::Invalid(field)),
    }
}

/// Un élément désigné par son niveau et sa clé ; la clé seule n'a pas de sens.
pub fn item_filter<'a>(
    metric: &Metric,
    level: Option<&'a str>,
    key: Option<&'a str>,
) -> Result<(Option<&'a str>, Option<&'a str>)> {
    match (level, key) {
        (None, None) => Ok((None, None)),
        (Some(level), Some(key)) if !key.is_empty() => {
            metric.check_level(level)?;
            Ok((Some(level), Some(key)))
        }
        (None, Some(_)) => Err(ReadError::Invalid("level")),
        _ => Err(ReadError::Invalid("key")),
    }
}

/// Les éléments d'un événement : niveau et nom, rangés par niveau puis par position.
type Contexts = HashMap<(i64, i16), Vec<(String, String)>>;

/// Les autres éléments de l'événement le plus récent d'un élément, par niveau : l'artiste d'un titre
/// ou d'un album. Générique : un titre « Intro » ne se comprend qu'avec son contexte.
async fn contexts(
    pool: &PgPool,
    events: &[(i64, i16)],
) -> std::result::Result<Contexts, sqlx::Error> {
    if events.is_empty() {
        return Ok(HashMap::new());
    }
    let seqs: Vec<i64> = events.iter().map(|e| e.0).collect();
    let ordinals: Vec<i16> = events.iter().map(|e| e.1).collect();
    let rows = sqlx::query!(
        r#"SELECT o.seq, o.ordinal, o.level, o.display
           FROM derived.event_items o
           JOIN unnest($1::bigint[], $2::smallint[]) AS w(seq, ordinal)
             ON w.seq = o.seq AND w.ordinal = o.ordinal
           WHERE o.generation = (SELECT id FROM derived.generations WHERE state = 'active')
           ORDER BY o.seq, o.ordinal, o.level, o.position"#,
        &seqs,
        &ordinals,
    )
    .fetch_all(pool)
    .await?;
    let mut map: Contexts = HashMap::new();
    for row in rows {
        map.entry((row.seq, row.ordinal))
            .or_default()
            .push((row.level, row.display));
    }
    Ok(map)
}

/// Les noms d'un même niveau, dans l'ordre : « A », « A & B », « A, B & C ».
fn join_names(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [one] => (*one).to_owned(),
        [head @ .., last] => format!("{} & {last}", head.join(", ")),
    }
}

/// Sans niveau propre, tous les éléments de l'événement ; sinon, ceux des niveaux plus larges que le
/// sien dans l'ordre du registre (l'artiste d'un album, l'artiste et l'album d'un titre). Plusieurs
/// éléments d'un même niveau (les artistes crédités d'une écoute) se joignent par position.
fn context_of(
    map: &Contexts,
    event: (i64, i16),
    own_level: Option<&str>,
    levels: &[String],
) -> Option<HashMap<String, String>> {
    let rank = |level: &str| levels.iter().position(|l| l == level);
    let own = own_level.map(rank);
    let mut grouped: HashMap<&str, Vec<&str>> = HashMap::new();
    for (level, display) in map.get(&event)? {
        let wider = match own {
            None => true,
            Some(own) => matches!((rank(level), own), (Some(r), Some(o)) if r < o),
        };
        if wider {
            grouped.entry(level).or_default().push(display);
        }
    }
    let items: HashMap<String, String> = grouped
        .into_iter()
        .map(|(level, names)| (level.to_owned(), join_names(&names)))
        .collect();
    (!items.is_empty()).then_some(items)
}

// ─── Inventaire ──────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct InventoryMetric {
    metric: String,
    label: String,
    shape: String,
    unit: Option<String>,
    domain: String,
    daily_aggregation: String,
    item_levels: Vec<String>,
    discovery: bool,
    applies_to: Option<String>,
    duration_from: Option<String>,
    freshness_threshold_seconds: Option<f64>,
    description: Option<String>,
    /// Les champs des détails que rendent les occurrences ; une paire `<x>_lat`/`<x>_lon` est une
    /// coordonnée, vue à travers les zones de confidentialité.
    shared_details: Vec<String>,
    events: i64,
    first_at: Option<DateTime<Utc>>,
    last_at: Option<DateTime<Utc>>,
    sources: Vec<InventorySource>,
}

#[derive(Serialize)]
pub struct InventorySource {
    source: String,
    events: i64,
    first_at: DateTime<Utc>,
    last_at: DateTime<Utc>,
}

#[derive(Serialize)]
pub struct Inventory {
    timezone: String,
    today: NaiveDate,
    /// Date de mise en ligne : un élément marqué « déjà connu » l'était avant elle.
    launch_date: Option<NaiveDate>,
    metrics: Vec<InventoryMetric>,
}

/// Ce qui est suivi, depuis quand et par quelles sources, d'après les registres et la vue.
pub async fn inventory(pool: &PgPool) -> Result<Inventory> {
    let settings = sqlx::query!(
        r#"SELECT timezone, launch_date,
                  (now() AT TIME ZONE timezone)::date AS "today!"
           FROM core.settings"#
    )
    .fetch_one(pool)
    .await?;
    let metrics = sqlx::query!(
        r#"SELECT metric, label, shape::text AS "shape!", unit, domain,
                  daily_aggregation::text AS "aggregation!", item_levels, discovery, applies_to,
                  duration_from, extract(epoch FROM freshness_threshold)::float8 AS freshness,
                  description, shared_details
           FROM core.metrics ORDER BY display_order NULLS LAST, metric"#
    )
    .fetch_all(pool)
    .await?;
    let sources = sqlx::query!(
        r#"SELECT e.metric AS "metric!", src.name AS source, count(*) AS "events!",
                  min(e.occurred_at) AS "first_at!", max(e.occurred_at) AS "last_at!"
           FROM derived.live_events e
           JOIN core.sources src ON src.id = e.source_id
           WHERE e.generation = (SELECT id FROM derived.generations WHERE state = 'active')
           GROUP BY e.metric, src.name
           ORDER BY 1, 4"#
    )
    .fetch_all(pool)
    .await?;
    Ok(Inventory {
        timezone: settings.timezone,
        today: settings.today,
        launch_date: settings.launch_date,
        metrics: metrics
            .into_iter()
            .map(|m| {
                let own: Vec<InventorySource> = sources
                    .iter()
                    .filter(|s| s.metric == m.metric)
                    .map(|s| InventorySource {
                        source: s.source.clone(),
                        events: s.events,
                        first_at: s.first_at,
                        last_at: s.last_at,
                    })
                    .collect();
                InventoryMetric {
                    events: own.iter().map(|s| s.events).sum(),
                    first_at: own.iter().map(|s| s.first_at).min(),
                    last_at: own.iter().map(|s| s.last_at).max(),
                    sources: own,
                    metric: m.metric,
                    label: m.label,
                    shape: m.shape,
                    unit: m.unit,
                    domain: m.domain,
                    daily_aggregation: m.aggregation,
                    item_levels: m.item_levels,
                    discovery: m.discovery,
                    applies_to: m.applies_to,
                    duration_from: m.duration_from,
                    freshness_threshold_seconds: m.freshness,
                    description: m.description,
                    shared_details: m.shared_details,
                }
            })
            .collect(),
    })
}

// ─── Recherche ───────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct Found {
    metric: String,
    level: String,
    key: String,
    /// Le nom le plus récent : une même clé peut couvrir plusieurs orthographes.
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<HashMap<String, String>>,
    occurrences: i64,
    first_at: DateTime<Utc>,
    last_at: DateTime<Utc>,
}

fn like_pattern(text: &str) -> String {
    let escaped = text
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("%{escaped}%")
}

/// Un nom approximatif : la clé normalisée ou le nom affiché (en minuscules) qui le contient, ou des
/// mots proches (trigrammes, les index `gin_trgm_ops` de `event_items`). Le nom affiché sert aux clés
/// qui sont des identifiants (YouTube). Les déclarations et références ne sont jamais des résultats.
pub async fn search(
    pool: &PgPool,
    query: &str,
    metric: Option<&str>,
    level: Option<&str>,
    limit: i64,
) -> Result<Vec<Found>> {
    let folded = fold(query);
    if folded.chars().count() < 2 {
        return Err(ReadError::Invalid("q"));
    }
    let rows = sqlx::query!(
        r#"WITH candidates AS (
             SELECT i.metric, i.level, i.item_key,
                    max(greatest(word_similarity($1, i.item_key),
                                 word_similarity($1, lower(i.display)))) AS score
             FROM derived.event_items i
             JOIN core.metrics m ON m.metric = i.metric
             WHERE i.generation = (SELECT id FROM derived.generations WHERE state = 'active')
               AND m.shape NOT IN ('declaration', 'reference')
               AND ($2::text IS NULL OR i.metric = $2)
               AND ($3::text IS NULL OR i.level = $3)
               AND (i.item_key LIKE $4 OR $1 <% i.item_key
                    OR lower(i.display) LIKE $4 OR $1 <% lower(i.display))
             GROUP BY i.metric, i.level, i.item_key
             ORDER BY score DESC, i.item_key
             LIMIT $5::int8 * 4
           )
           SELECT c.metric, c.level, c.item_key, s.occurrences AS "occurrences!",
                  s.first_at AS "first_at!", s.last_at AS "last_at!", s.name AS "name!",
                  s.seq AS "seq!", s.ordinal AS "ordinal!"
           FROM candidates c
           CROSS JOIN LATERAL (
             SELECT count(*) AS occurrences, min(x.occurred_at) AS first_at,
                    max(x.occurred_at) AS last_at,
                    (array_agg(x.display ORDER BY x.occurred_at DESC, x.seq DESC))[1] AS name,
                    (array_agg(x.seq ORDER BY x.occurred_at DESC, x.seq DESC))[1] AS seq,
                    (array_agg(x.ordinal ORDER BY x.occurred_at DESC, x.seq DESC))[1] AS ordinal
             FROM derived.live_event_items x
             WHERE x.generation = (SELECT id FROM derived.generations WHERE state = 'active')
               AND x.metric = c.metric AND x.level = c.level AND x.item_key = c.item_key
           ) s
           WHERE s.occurrences > 0
           ORDER BY c.score DESC, s.occurrences DESC, c.item_key
           LIMIT $5"#,
        folded,
        metric,
        level,
        like_pattern(&folded),
        limit,
    )
    .fetch_all(pool)
    .await?;
    let events: Vec<(i64, i16)> = rows.iter().map(|r| (r.seq, r.ordinal)).collect();
    let contexts = contexts(pool, &events).await?;
    let levels: HashMap<String, Vec<String>> =
        sqlx::query!("SELECT metric, item_levels FROM core.metrics")
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(|m| (m.metric, m.item_levels))
            .collect();
    Ok(rows
        .into_iter()
        .map(|r| Found {
            context: context_of(
                &contexts,
                (r.seq, r.ordinal),
                Some(&r.level),
                levels.get(&r.metric).map_or(&[][..], Vec::as_slice),
            ),
            metric: r.metric,
            level: r.level,
            key: r.item_key,
            name: r.name,
            occurrences: r.occurrences,
            first_at: r.first_at,
            last_at: r.last_at,
        })
        .collect())
}

// ─── Occurrences ─────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct Occurrence {
    /// L'identifiant de l'occurrence, `<numéro du journal>:<rang>`, pour en demander le parcours.
    id: String,
    at: DateTime<Utc>,
    /// Heure de Paris, `AAAA-MM-JJ HH:MM`.
    local: String,
    /// La valeur de l'événement (pas d'un jour, minutes de sommeil, km d'une séance), si la métrique
    /// en a une.
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<f64>,
    /// Les détails que le registre partage (`Exposure::details`) ; absents pour une métrique qui n'en
    /// partage aucun.
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    items: Option<HashMap<String, String>>,
}

#[derive(Serialize)]
pub struct DayCount {
    date: NaiveDate,
    count: i64,
}

#[derive(Serialize)]
pub struct DayFirstLast {
    date: NaiveDate,
    /// Heure de Paris, `HH:MM`.
    first: String,
    last: String,
    count: i64,
}

#[derive(Serialize)]
#[serde(tag = "form", rename_all = "snake_case")]
pub enum Occurrences {
    List {
        total: i64,
        truncated: bool,
        occurrences: Vec<Occurrence>,
    },
    Daily {
        total: i64,
        days: Vec<DayCount>,
    },
    FirstLast {
        total: i64,
        days: Vec<DayFirstLast>,
    },
}

/// Les occurrences d'une métrique, ou d'un de ses éléments, sur une période, en trois formes. Un jour
/// absent n'a aucune occurrence ; dire s'il a des données est le rôle de la complétude.
#[allow(clippy::too_many_arguments)]
pub async fn occurrences(
    pool: &PgPool,
    metric: &Metric,
    period: Period,
    level: Option<&str>,
    key: Option<&str>,
    form: &str,
    limit: i64,
    exposure: Exposure<'_>,
) -> Result<Occurrences> {
    match form {
        "list" => {
            let rows = sqlx::query!(
                r#"SELECT e.seq AS "seq!", e.ordinal AS "ordinal!", e.occurred_at AS "occurred_at!",
                          e.value::float8 AS value, e.details AS "details!",
                          to_char(e.occurred_at AT TIME ZONE (SELECT timezone FROM core.settings),
                                  'YYYY-MM-DD HH24:MI') AS "local!",
                          count(*) OVER () AS "total!"
                   FROM derived.live_events e
                   WHERE e.generation = (SELECT id FROM derived.generations WHERE state = 'active')
                     AND e.metric = $1
                     AND e.occurred_at >= ($2::date::timestamp AT TIME ZONE (SELECT timezone FROM core.settings))
                     AND e.occurred_at < (($3::date + 1)::timestamp AT TIME ZONE (SELECT timezone FROM core.settings))
                     AND ($4::text IS NULL OR (e.seq, e.ordinal) IN (
                           SELECT i.seq, i.ordinal FROM derived.event_items i
                           WHERE i.generation = e.generation AND i.metric = $1
                             AND i.level = $4 AND i.item_key = $5))
                   ORDER BY e.occurred_at, e.seq
                   LIMIT $6"#,
                metric.metric,
                period.from,
                period.to,
                level,
                key,
                limit,
            )
            .fetch_all(pool)
            .await?;
            let total = rows.first().map_or(0, |r| r.total);
            let events: Vec<(i64, i16)> = rows.iter().map(|r| (r.seq, r.ordinal)).collect();
            let contexts = contexts(pool, &events).await?;
            Ok(Occurrences::List {
                total,
                truncated: total > rows.len() as i64,
                occurrences: rows
                    .into_iter()
                    .map(|r| Occurrence {
                        id: format!("{}:{}", r.seq, r.ordinal),
                        items: context_of(&contexts, (r.seq, r.ordinal), None, &[]),
                        details: exposure.details(metric, &r.details),
                        at: r.occurred_at,
                        local: r.local,
                        value: r.value,
                    })
                    .collect(),
            })
        }
        "daily" | "first_last" => {
            let rows = sqlx::query!(
                r#"SELECT (e.occurred_at AT TIME ZONE (SELECT timezone FROM core.settings))::date AS "day!",
                          count(*) AS "count!",
                          to_char(min(e.occurred_at) AT TIME ZONE (SELECT timezone FROM core.settings), 'HH24:MI') AS "first!",
                          to_char(max(e.occurred_at) AT TIME ZONE (SELECT timezone FROM core.settings), 'HH24:MI') AS "last!"
                   FROM derived.live_events e
                   WHERE e.generation = (SELECT id FROM derived.generations WHERE state = 'active')
                     AND e.metric = $1
                     AND e.occurred_at >= ($2::date::timestamp AT TIME ZONE (SELECT timezone FROM core.settings))
                     AND e.occurred_at < (($3::date + 1)::timestamp AT TIME ZONE (SELECT timezone FROM core.settings))
                     AND ($4::text IS NULL OR (e.seq, e.ordinal) IN (
                           SELECT i.seq, i.ordinal FROM derived.event_items i
                           WHERE i.generation = e.generation AND i.metric = $1
                             AND i.level = $4 AND i.item_key = $5))
                   GROUP BY 1
                   ORDER BY 1"#,
                metric.metric,
                period.from,
                period.to,
                level,
                key,
            )
            .fetch_all(pool)
            .await?;
            let total = rows.iter().map(|r| r.count).sum();
            Ok(if form == "daily" {
                Occurrences::Daily {
                    total,
                    days: rows
                        .into_iter()
                        .map(|r| DayCount {
                            date: r.day,
                            count: r.count,
                        })
                        .collect(),
                }
            } else {
                Occurrences::FirstLast {
                    total,
                    days: rows
                        .into_iter()
                        .map(|r| DayFirstLast {
                            date: r.day,
                            first: r.first,
                            last: r.last,
                            count: r.count,
                        })
                        .collect(),
                }
            })
        }
        _ => Err(ReadError::Invalid("form")),
    }
}

// ─── Temps mesuré : la forme `duration` ──────────────────────────────────────

/// Le temps mesuré d'un jour de Paris, pour une métrique de forme `duration`, en tout ou pour un
/// élément.
pub struct Measured {
    pub day: NaiveDate,
    /// La clé de l'élément au niveau demandé (`by_item`), `None` pour les événements qui n'en ont pas,
    /// ou pour le total du jour.
    pub item_key: Option<String>,
    /// Le dernier nom affiché de l'élément ce jour-là.
    pub name: Option<String>,
    /// L'événement le plus récent du groupe, pour le contexte de l'élément.
    pub latest: Option<(i64, i16)>,
    /// Les événements commencés ce jour-là ; un intervalle commencé la veille compte dans le temps du
    /// jour, pas ici.
    pub events: i64,
    /// L'union des intervalles, coupée aux minuits de Paris : ce qui se chevauche compte une fois.
    pub seconds: f64,
}

/// Le temps mesuré d'une métrique de forme `duration`, jour de Paris par jour (voir le journal des
/// décisions de l'architecture, 6 nov. 2026). Chaque événement est l'intervalle `[occurred_at,
/// occurred_at + details.played_ms]` ; le temps d'un jour est l'union de ces intervalles (`range_agg`),
/// coupée à ses minuits, donc juste les jours de 23 et 25 heures. `level` et `key` restreignent aux
/// événements de cet élément ; `by_item` donne une ligne par élément du niveau `level` (et une pour les
/// événements sans élément), son temps étant l'union de ses seuls intervalles. Un événement dure
/// `MAX_MEASURED_MS` au plus : chercher ceux commencés ce temps avant la période suffit, et garde
/// l'index `(generation, metric, occurred_at)`. Chaque intervalle est rapproché des seuls jours de Paris
/// qu'il touche (un ou deux), et non de chaque jour de la période : une année de Twitch passe de 1,9 s à
/// 37 ms (journal des décisions de l'architecture, 8 nov. 2026). Seuls les jours qui ont du temps sont
/// rendus.
pub async fn measured(
    pool: &PgPool,
    metric: &str,
    period: Period,
    level: Option<&str>,
    key: Option<&str>,
    by_item: bool,
) -> std::result::Result<Vec<Measured>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"WITH spans AS (
             SELECT e.seq, e.ordinal, e.occurred_at, i.item_key, i.display,
                    tstzrange(e.occurred_at,
                              e.occurred_at + (e.details->>'played_ms')::bigint * interval '1 millisecond') AS span
             FROM derived.live_events e
             LEFT JOIN derived.event_items i
               ON i.generation = e.generation AND i.seq = e.seq AND i.ordinal = e.ordinal
              AND i.level = $4
             WHERE e.generation = (SELECT id FROM derived.generations WHERE state = 'active')
               AND e.metric = $1
               AND e.occurred_at >= ($2::date::timestamp AT TIME ZONE (SELECT timezone FROM core.settings))
                                    - $7::bigint * interval '1 millisecond'
               AND e.occurred_at < (($3::date + 1)::timestamp AT TIME ZONE (SELECT timezone FROM core.settings))
               AND (e.details->>'played_ms')::bigint > 0
               AND ($5::text IS NULL OR i.item_key = $5)
           ),
           covered AS (
             SELECT d.day, CASE WHEN $6::bool THEN s.item_key END AS item_key,
                    range_agg(s.span * d.bounds) AS covered,
                    count(DISTINCT (s.seq, s.ordinal)) FILTER (WHERE s.occurred_at <@ d.bounds) AS events,
                    (array_agg(s.display ORDER BY s.occurred_at DESC, s.seq DESC))[1] AS name,
                    (array_agg(s.seq ORDER BY s.occurred_at DESC, s.seq DESC))[1] AS seq,
                    (array_agg(s.ordinal ORDER BY s.occurred_at DESC, s.seq DESC))[1] AS ordinal
             FROM spans s
             CROSS JOIN LATERAL (
               SELECT x::date AS day,
                      tstzrange(x AT TIME ZONE (SELECT timezone FROM core.settings),
                                (x + interval '1 day') AT TIME ZONE (SELECT timezone FROM core.settings)) AS bounds
               FROM generate_series((lower(s.span) AT TIME ZONE (SELECT timezone FROM core.settings))::date::timestamp,
                                    (upper(s.span) AT TIME ZONE (SELECT timezone FROM core.settings))::date::timestamp,
                                    interval '1 day') AS x
             ) d
             WHERE d.day BETWEEN $2::date AND $3::date
               AND s.span && d.bounds
             GROUP BY 1, 2
           )
           SELECT day AS "day!", item_key AS "item_key?", name AS "name?", seq AS "seq?", ordinal AS "ordinal?", events AS "events!",
                  (SELECT sum(extract(epoch FROM upper(r) - lower(r))) FROM unnest(covered) AS r)::float8 AS "seconds!"
           FROM covered
           ORDER BY day, item_key"#,
        metric,
        period.from,
        period.to,
        level,
        key,
        by_item,
        crate::interpreters::MAX_MEASURED_MS,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| Measured {
            day: r.day,
            item_key: r.item_key,
            name: r.name,
            latest: r.seq.zip(r.ordinal),
            events: r.events,
            seconds: r.seconds,
        })
        .collect())
}

fn round_minutes(seconds: f64) -> i64 {
    (seconds / 60.0).round() as i64
}

/// Le libellé d'un jour dans un regroupement : le jour, la semaine ISO de son lundi, ou le mois.
fn period_label(day: NaiveDate, group: &str) -> String {
    match group {
        "day" => day.format("%Y-%m-%d").to_string(),
        "week" => {
            let week = day.iso_week();
            format!("{}-W{:02}", week.year(), week.week())
        }
        _ => day.format("%Y-%m").to_string(),
    }
}

/// La durée cumulée d'une métrique de forme `duration` : la somme, jour par jour, des unions.
async fn measured_durations(
    pool: &PgPool,
    metric: &Metric,
    period: Period,
    level: Option<&str>,
    key: Option<&str>,
    group: &str,
) -> Result<Durations> {
    let days = measured(pool, &metric.metric, period, level, key, false).await?;
    let mut periods: Vec<(String, f64, i64)> = Vec::new();
    for day in days {
        let label = period_label(day.day, group);
        match periods.last_mut() {
            Some(last) if last.0 == label => {
                last.1 += day.seconds;
                last.2 += day.events;
            }
            _ => periods.push((label, day.seconds, day.events)),
        }
    }
    let total: f64 = periods.iter().map(|p| p.1).sum();
    let events: i64 = periods.iter().map(|p| p.2).sum();
    Ok(Durations {
        group: group.to_owned(),
        total_minutes: (!periods.is_empty()).then(|| round_minutes(total)),
        events,
        events_without_duration: 0,
        periods: periods
            .into_iter()
            .map(|(period, seconds, events)| DurationPeriod {
                period,
                minutes: Some(round_minutes(seconds)),
                events,
                events_without_duration: 0,
            })
            .collect(),
    })
}

// ─── Durée cumulée ───────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct DurationPeriod {
    period: String,
    minutes: Option<i64>,
    events: i64,
    /// Sans durée connue : non comptés dans les minutes, et cela doit se voir.
    events_without_duration: i64,
}

#[derive(Serialize)]
pub struct Durations {
    group: String,
    total_minutes: Option<i64>,
    events: i64,
    events_without_duration: i64,
    periods: Vec<DurationPeriod>,
}

/// Le temps passé, par jour, semaine (lundi, `AAAA-Wss` ISO) ou mois. La durée d'un événement est celle
/// mesurée par la source (`played_ms` dans ses détails) si elle existe, sinon la dernière valeur connue
/// de la référence que le registre désigne (`duration_from`) pour son élément au niveau de cette
/// référence : la même règle que les minutes de la page du jour. Une jointure, donc en vue. Une
/// métrique de forme `duration` a son propre calcul : l'union des intervalles de chaque jour
/// (`measured`), où ce qui se chevauche compte une fois.
#[allow(clippy::too_many_arguments)]
pub async fn durations(
    pool: &PgPool,
    metric: &Metric,
    period: Period,
    level: Option<&str>,
    key: Option<&str>,
    group: &str,
) -> Result<Durations> {
    if !matches!(group, "day" | "week" | "month") {
        return Err(ReadError::Invalid("group"));
    }
    if metric.shape == "duration" {
        return measured_durations(pool, metric, period, level, key, group).await;
    }
    let reference = metric
        .duration_from
        .as_deref()
        .ok_or(ReadError::Invalid("metric"))?;
    let reference = self::metric(pool, reference).await?;
    let reference_level = reference
        .item_levels
        .first()
        .ok_or(ReadError::Invalid("metric"))?;
    let rows = sqlx::query!(
        r#"WITH timed AS (
             SELECT e.occurred_at AT TIME ZONE (SELECT timezone FROM core.settings) AS local,
                    COALESCE((e.details->>'played_ms')::numeric / 1000, d.value)::float8 AS seconds
             FROM derived.live_events e
             LEFT JOIN derived.event_items it
               ON it.generation = e.generation AND it.seq = e.seq AND it.ordinal = e.ordinal
              AND it.level = $7
             LEFT JOIN LATERAL (
               SELECT known.value
               FROM derived.event_items i
               JOIN derived.live_events known USING (generation, seq, ordinal)
               WHERE i.generation = e.generation AND i.metric = $6 AND i.level = $7
                 AND i.item_key = it.item_key
               ORDER BY i.occurred_at DESC, i.seq DESC
               LIMIT 1
             ) d ON true
             WHERE e.generation = (SELECT id FROM derived.generations WHERE state = 'active')
               AND e.metric = $1
               AND e.occurred_at >= ($2::date::timestamp AT TIME ZONE (SELECT timezone FROM core.settings))
               AND e.occurred_at < (($3::date + 1)::timestamp AT TIME ZONE (SELECT timezone FROM core.settings))
               AND ($4::text IS NULL OR (e.seq, e.ordinal) IN (
                     SELECT i.seq, i.ordinal FROM derived.event_items i
                     WHERE i.generation = e.generation AND i.metric = $1
                       AND i.level = $4 AND i.item_key = $5))
           )
           SELECT CASE $8::text
                    WHEN 'day' THEN to_char(local, 'YYYY-MM-DD')
                    WHEN 'week' THEN to_char(date_trunc('week', local), 'IYYY-"W"IW')
                    ELSE to_char(local, 'YYYY-MM')
                  END AS "period!",
                  count(*) AS "events!",
                  count(seconds) AS "known!",
                  sum(seconds) AS seconds
           FROM timed
           GROUP BY 1
           ORDER BY 1"#,
        metric.metric,
        period.from,
        period.to,
        level,
        key,
        reference.metric,
        reference_level,
        group,
    )
    .fetch_all(pool)
    .await?;
    let minutes = |seconds: f64| (seconds / 60.0).round() as i64;
    let known: i64 = rows.iter().map(|r| r.known).sum();
    let total_seconds: f64 = rows.iter().filter_map(|r| r.seconds).sum();
    let events: i64 = rows.iter().map(|r| r.events).sum();
    Ok(Durations {
        group: group.to_owned(),
        total_minutes: (known > 0).then(|| minutes(total_seconds)),
        events,
        events_without_duration: events - known,
        periods: rows
            .into_iter()
            .map(|r| DurationPeriod {
                period: r.period,
                minutes: r.seconds.map(minutes),
                events: r.events,
                events_without_duration: r.events - r.known,
            })
            .collect(),
    })
}

// ─── Un élément : découverte et dernière fois ────────────────────────────────

#[derive(Serialize)]
pub struct KnownMark {
    at: DateTime<Utc>,
    source: String,
}

#[derive(Serialize)]
pub struct Item {
    metric: String,
    level: String,
    key: String,
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<HashMap<String, String>>,
    occurrences: i64,
    first_at: Option<DateTime<Utc>>,
    first_local: Option<String>,
    last_at: Option<DateTime<Utc>>,
    last_local: Option<String>,
    /// La métrique a un sens de « découverte » (registre).
    discovery: bool,
    /// Déclarations « déjà connu » vivantes de cet élément, à son niveau : il était alors connu avant
    /// la date de mise en ligne, et sa première occurrence n'est pas une découverte.
    known_marks: Vec<KnownMark>,
    launch_date: Option<NaiveDate>,
}

pub async fn item(pool: &PgPool, metric: &Metric, level: &str, key: &str) -> Result<Item> {
    metric.check_level(level)?;
    let seen = sqlx::query!(
        r#"SELECT count(*) AS "occurrences!", min(x.occurred_at) AS first_at,
                  max(x.occurred_at) AS last_at,
                  to_char(min(x.occurred_at) AT TIME ZONE (SELECT timezone FROM core.settings), 'YYYY-MM-DD HH24:MI') AS first_local,
                  to_char(max(x.occurred_at) AT TIME ZONE (SELECT timezone FROM core.settings), 'YYYY-MM-DD HH24:MI') AS last_local,
                  (array_agg(x.display ORDER BY x.occurred_at DESC, x.seq DESC))[1] AS name,
                  (array_agg(x.seq ORDER BY x.occurred_at DESC, x.seq DESC))[1] AS seq,
                  (array_agg(x.ordinal ORDER BY x.occurred_at DESC, x.seq DESC))[1] AS ordinal
           FROM derived.live_event_items x
           WHERE x.generation = (SELECT id FROM derived.generations WHERE state = 'active')
             AND x.metric = $1 AND x.level = $2 AND x.item_key = $3"#,
        metric.metric,
        level,
        key,
    )
    .fetch_one(pool)
    .await?;
    let marks = sqlx::query!(
        r#"SELECT k.occurred_at AS "at!", src.name AS "source!",
                  k.display AS "display!"
           FROM derived.live_event_items k
           JOIN core.records r ON r.seq = k.seq
           JOIN core.sources src ON src.id = r.source_id
           WHERE k.generation = (SELECT id FROM derived.generations WHERE state = 'active')
             AND k.metric IN (SELECT m.metric FROM core.metrics m
                              WHERE m.applies_to = $1 AND m.shape = 'declaration')
             AND k.level = $2 AND k.item_key = $3
           ORDER BY k.occurred_at, k.seq"#,
        metric.metric,
        level,
        key,
    )
    .fetch_all(pool)
    .await?;
    if seen.occurrences == 0 && marks.is_empty() {
        return Err(ReadError::NotFound);
    }
    let launch_date = sqlx::query_scalar!("SELECT launch_date FROM core.settings")
        .fetch_one(pool)
        .await?;
    let event = seen.seq.zip(seen.ordinal);
    let contexts = contexts(pool, &event.into_iter().collect::<Vec<_>>()).await?;
    Ok(Item {
        metric: metric.metric.clone(),
        level: level.to_owned(),
        key: key.to_owned(),
        name: seen
            .name
            .or_else(|| marks.first().map(|m| m.display.clone())),
        context: event.and_then(|e| context_of(&contexts, e, Some(level), &metric.item_levels)),
        occurrences: seen.occurrences,
        first_at: seen.first_at,
        first_local: seen.first_local,
        last_at: seen.last_at,
        last_local: seen.last_local,
        discovery: metric.discovery,
        known_marks: marks
            .into_iter()
            .map(|m| KnownMark {
                at: m.at,
                source: m.source,
            })
            .collect(),
        launch_date,
    })
}

// ─── Classement ──────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct Ranked {
    key: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<HashMap<String, String>>,
    occurrences: i64,
    /// Une métrique de forme `duration` : le temps de l'élément sur la période, qui fait le classement.
    #[serde(skip_serializing_if = "Option::is_none")]
    minutes: Option<i64>,
}

#[derive(Serialize)]
pub struct Top {
    level: String,
    /// Éléments différents sur la période, au-delà de la limite compris.
    distinct_items: i64,
    /// Occurrences qui ont un élément à ce niveau (une écoute sans artiste n'en a pas). Une écoute à
    /// trois artistes compte une fois ici, et pour chacun de ses artistes dans `items`.
    occurrences_with_item: i64,
    items: Vec<Ranked>,
}

pub async fn top(
    pool: &PgPool,
    metric: &Metric,
    level: &str,
    period: Period,
    limit: i64,
) -> Result<Top> {
    metric.check_level(level)?;
    if metric.shape == "duration" {
        return measured_top(pool, metric, level, period, limit).await;
    }
    let rows = sqlx::query!(
        r#"WITH period AS (
             SELECT i.*
             FROM derived.live_event_items i
             WHERE i.generation = (SELECT id FROM derived.generations WHERE state = 'active')
               AND i.metric = $1 AND i.level = $2
               AND i.occurred_at >= ($3::date::timestamp AT TIME ZONE (SELECT timezone FROM core.settings))
               AND i.occurred_at < (($4::date + 1)::timestamp AT TIME ZONE (SELECT timezone FROM core.settings))
           )
           SELECT i.item_key AS "item_key!", count(*) AS "occurrences!",
                  (array_agg(i.display ORDER BY i.occurred_at DESC, i.seq DESC))[1] AS "name!",
                  (array_agg(i.seq ORDER BY i.occurred_at DESC, i.seq DESC))[1] AS "seq!",
                  (array_agg(i.ordinal ORDER BY i.occurred_at DESC, i.seq DESC))[1] AS "ordinal!",
                  count(*) OVER () AS "distinct!",
                  (SELECT count(*) FROM (SELECT DISTINCT seq, ordinal FROM period) e) AS "with_item!"
           FROM period i
           GROUP BY i.item_key
           ORDER BY 2 DESC, 1
           LIMIT $5"#,
        metric.metric,
        level,
        period.from,
        period.to,
        limit,
    )
    .fetch_all(pool)
    .await?;
    let events: Vec<(i64, i16)> = rows.iter().map(|r| (r.seq, r.ordinal)).collect();
    let contexts = contexts(pool, &events).await?;
    Ok(Top {
        level: level.to_owned(),
        distinct_items: rows.first().map_or(0, |r| r.distinct),
        occurrences_with_item: rows.first().map_or(0, |r| r.with_item),
        items: rows
            .into_iter()
            .map(|r| Ranked {
                context: context_of(
                    &contexts,
                    (r.seq, r.ordinal),
                    Some(level),
                    &metric.item_levels,
                ),
                key: r.item_key,
                name: r.name,
                occurrences: r.occurrences,
                minutes: None,
            })
            .collect(),
    })
}

/// Le classement d'une métrique de forme `duration` : par temps sur la période (la somme, jour par jour,
/// des unions de l'élément), puis par clé. `occurrences` compte les événements commencés sur la période.
async fn measured_top(
    pool: &PgPool,
    metric: &Metric,
    level: &str,
    period: Period,
    limit: i64,
) -> Result<Top> {
    struct Total {
        key: String,
        name: String,
        latest: Option<(i64, i16)>,
        events: i64,
        seconds: f64,
    }
    let mut totals: HashMap<String, Total> = HashMap::new();
    // Jour par jour, du plus ancien au plus récent : le nom et l'événement gardés sont les derniers.
    for day in measured(pool, &metric.metric, period, Some(level), None, true).await? {
        let Some(key) = day.item_key else { continue };
        let total = totals.entry(key.clone()).or_insert_with(|| Total {
            key,
            name: String::new(),
            latest: None,
            events: 0,
            seconds: 0.0,
        });
        if let Some(name) = day.name {
            total.name = name;
        }
        total.latest = day.latest.or(total.latest);
        total.events += day.events;
        total.seconds += day.seconds;
    }
    let with_item = sqlx::query_scalar!(
        r#"SELECT count(DISTINCT (i.seq, i.ordinal)) AS "count!"
           FROM derived.live_event_items i
           WHERE i.generation = (SELECT id FROM derived.generations WHERE state = 'active')
             AND i.metric = $1 AND i.level = $2
             AND i.occurred_at >= ($3::date::timestamp AT TIME ZONE (SELECT timezone FROM core.settings))
             AND i.occurred_at < (($4::date + 1)::timestamp AT TIME ZONE (SELECT timezone FROM core.settings))"#,
        metric.metric,
        level,
        period.from,
        period.to,
    )
    .fetch_one(pool)
    .await?;
    let mut ranked: Vec<Total> = totals.into_values().collect();
    ranked.sort_by(|a, b| {
        b.seconds
            .total_cmp(&a.seconds)
            .then_with(|| a.key.cmp(&b.key))
    });
    let distinct_items = ranked.len() as i64;
    ranked.truncate(usize::try_from(limit).unwrap_or(0));
    let events: Vec<(i64, i16)> = ranked.iter().filter_map(|t| t.latest).collect();
    let contexts = contexts(pool, &events).await?;
    Ok(Top {
        level: level.to_owned(),
        distinct_items,
        occurrences_with_item: with_item,
        items: ranked
            .into_iter()
            .map(|t| Ranked {
                context: t
                    .latest
                    .and_then(|e| context_of(&contexts, e, Some(level), &metric.item_levels)),
                key: t.key,
                name: t.name,
                occurrences: t.events,
                minutes: Some(round_minutes(t.seconds)),
            })
            .collect(),
    })
}

// ─── Découvertes ─────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct Discovered {
    key: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<HashMap<String, String>>,
    first_at: DateTime<Utc>,
    first_local: String,
}

#[derive(Serialize)]
pub struct Discoveries {
    level: String,
    /// Découvertes de la période, au-delà de la limite comprises, sans les éléments marqués.
    total: i64,
    /// Éléments entendus pour la première fois sur la période, mais marqués « déjà connu ».
    marked_known: i64,
    truncated: bool,
    items: Vec<Discovered>,
}

/// Les éléments dont la première occurrence de toute la vie tombe dans la période, à leur niveau, sans
/// ceux qui sont marqués « déjà connu » à ce niveau : la règle de la page du jour, sur une période.
pub async fn discoveries(
    pool: &PgPool,
    metric: &Metric,
    level: &str,
    period: Period,
    limit: i64,
) -> Result<Discoveries> {
    if !metric.discovery {
        return Err(ReadError::Invalid("metric"));
    }
    metric.check_level(level)?;
    let rows = sqlx::query!(
        r#"WITH seen AS (
             SELECT i.item_key, min(i.occurred_at) AS first_at,
                    (array_agg(i.display ORDER BY i.occurred_at, i.seq))[1] AS name,
                    (array_agg(i.seq ORDER BY i.occurred_at, i.seq))[1] AS seq,
                    (array_agg(i.ordinal ORDER BY i.occurred_at, i.seq))[1] AS ordinal
             FROM derived.live_event_items i
             WHERE i.generation = (SELECT id FROM derived.generations WHERE state = 'active')
               AND i.metric = $1 AND i.level = $2
               AND i.occurred_at >= ($3::date::timestamp AT TIME ZONE (SELECT timezone FROM core.settings))
               AND i.occurred_at < (($4::date + 1)::timestamp AT TIME ZONE (SELECT timezone FROM core.settings))
             GROUP BY i.item_key
           ),
           first AS (
             SELECT s.*,
                    EXISTS (SELECT 1 FROM derived.live_event_items k
                            WHERE k.generation = (SELECT id FROM derived.generations WHERE state = 'active')
                              AND k.metric IN (SELECT m.metric FROM core.metrics m
                                               WHERE m.applies_to = $1 AND m.shape = 'declaration')
                              AND k.level = $2 AND k.item_key = s.item_key) AS marked
             FROM seen s
             LEFT JOIN LATERAL (
               SELECT 1 AS found FROM derived.live_event_items x
               WHERE x.generation = (SELECT id FROM derived.generations WHERE state = 'active')
                 AND x.metric = $1 AND x.level = $2 AND x.item_key = s.item_key
                 AND x.occurred_at < ($3::date::timestamp AT TIME ZONE (SELECT timezone FROM core.settings))
               LIMIT 1
             ) earlier ON true
             WHERE earlier.found IS NULL
           )
           SELECT item_key AS "key!", first_at AS "first_at!", name AS "name!", seq AS "seq!",
                  ordinal AS "ordinal!", marked AS "marked!",
                  to_char(first_at AT TIME ZONE (SELECT timezone FROM core.settings), 'YYYY-MM-DD HH24:MI') AS "first_local!"
           FROM first
           ORDER BY first_at, item_key"#,
        metric.metric,
        level,
        period.from,
        period.to,
    )
    .fetch_all(pool)
    .await?;
    let marked_known = rows.iter().filter(|r| r.marked).count() as i64;
    let discovered: Vec<_> = rows.into_iter().filter(|r| !r.marked).collect();
    let total = discovered.len() as i64;
    let kept: Vec<_> = discovered.into_iter().take(limit as usize).collect();
    let events: Vec<(i64, i16)> = kept.iter().map(|r| (r.seq, r.ordinal)).collect();
    let contexts = contexts(pool, &events).await?;
    Ok(Discoveries {
        level: level.to_owned(),
        total,
        marked_known,
        truncated: total > kept.len() as i64,
        items: kept
            .into_iter()
            .map(|r| Discovered {
                context: context_of(
                    &contexts,
                    (r.seq, r.ordinal),
                    Some(level),
                    &metric.item_levels,
                ),
                key: r.key,
                name: r.name,
                first_at: r.first_at,
                first_local: r.first_local,
            })
            .collect(),
    })
}

/// Un élément vu pour la première fois un jour donné.
#[derive(Serialize)]
pub struct DayDiscovery {
    pub level: String,
    pub key: String,
    pub name: String,
}

/// Les éléments d'une métrique vus ce jour-là pour la première fois de la vie, aux niveaux donnés et
/// dans leur ordre, puis par première heure : aucune occurrence vivante du même niveau et de la même clé
/// avant minuit, heure de Paris, et aucune déclaration « déjà connu » vivante du même niveau et de la
/// même clé (les métriques `declaration` qui s'appliquent à celle-ci, comme `discoveries` ; pour Steam,
/// `game.known`). La musique, qui montre aussi ses éléments marqués, lit les siens à part
/// (`api::first_listens`). Le coût suit le jour demandé, pas l'historique : une recherche d'index par
/// élément, d'où les `LIMIT 1` latéraux, qu'un `NOT EXISTS` ferait devenir un parcours de tous les
/// éléments.
pub async fn day_discoveries(
    pool: &PgPool,
    metric: &str,
    levels: &[&str],
    date: NaiveDate,
) -> std::result::Result<Vec<DayDiscovery>, sqlx::Error> {
    let levels: Vec<String> = levels.iter().map(|l| (*l).to_owned()).collect();
    let rows = sqlx::query!(
        r#"WITH seen AS (
             SELECT DISTINCT ON (i.level, i.item_key)
                    e.generation, i.level, i.item_key, i.display,
                    min(e.occurred_at) OVER (PARTITION BY i.level, i.item_key) AS first_at,
                    ($1::date::timestamp AT TIME ZONE (SELECT timezone FROM core.settings)) AS day_start
             FROM derived.live_events e
               JOIN derived.event_items i USING (generation, seq, ordinal)
             WHERE e.generation = (SELECT id FROM derived.generations WHERE state = 'active')
               AND e.metric = $2
               AND i.level = ANY($3)
               AND e.occurred_at >= ($1::date::timestamp AT TIME ZONE (SELECT timezone FROM core.settings))
               AND e.occurred_at < (($1::date + 1)::timestamp AT TIME ZONE (SELECT timezone FROM core.settings))
             ORDER BY i.level, i.item_key, e.occurred_at DESC, e.seq DESC
           )
           SELECT w.level, w.item_key, w.display
           FROM seen w
           LEFT JOIN LATERAL (
             SELECT 1 AS found FROM derived.live_event_items x
             WHERE x.generation = w.generation AND x.metric = $2
               AND x.level = w.level AND x.item_key = w.item_key
               AND x.occurred_at < w.day_start
             LIMIT 1
           ) earlier ON true
           LEFT JOIN LATERAL (
             SELECT 1 AS found FROM derived.live_event_items k
             WHERE k.generation = w.generation
               AND k.metric IN (SELECT m.metric FROM core.metrics m
                                WHERE m.applies_to = $2 AND m.shape = 'declaration')
               AND k.level = w.level AND k.item_key = w.item_key
             LIMIT 1
           ) known ON true
           WHERE earlier.found IS NULL AND known.found IS NULL
           ORDER BY array_position($3, w.level), w.first_at, w.item_key"#,
        date,
        metric,
        &levels,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| DayDiscovery {
            level: row.level,
            key: row.item_key,
            name: row.display,
        })
        .collect())
}

// ─── Parcours ────────────────────────────────────────────────────────────────

/// Le parcours d'une occurrence : la référence à points (`details.points`, `[lat, lon, …]`) d'une
/// métrique qui l'éclaire (`applies_to`), rattachée par l'identifiant que les deux portent dans leurs
/// détails (`activity_id`, le seul que partagent une séance et sa trace). Générique : une autre source
/// de parcours n'aurait qu'à déclarer sa référence.
pub struct Route {
    pub metric: String,
    pub local: String,
    pub points: Vec<(f64, f64)>,
}

/// `<numéro du journal>:<rang>`, tel que le rendent les occurrences.
pub fn parse_occurrence_id(value: &str) -> Option<(i64, i16)> {
    let (seq, ordinal) = value.split_once(':')?;
    let seq = seq.parse().ok().filter(|s: &i64| *s > 0)?;
    let ordinal = ordinal.parse().ok().filter(|o: &i16| *o >= 0)?;
    Some((seq, ordinal))
}

pub async fn route(pool: &PgPool, (seq, ordinal): (i64, i16)) -> Result<Route> {
    let row = sqlx::query!(
        r#"SELECT e.metric AS "metric!",
                  to_char(e.occurred_at AT TIME ZONE (SELECT timezone FROM core.settings),
                          'YYYY-MM-DD HH24:MI') AS "local!",
                  (SELECT t.details->'points'
                   FROM derived.live_events t
                   JOIN core.metrics m ON m.metric = t.metric
                   WHERE t.generation = e.generation
                     AND m.applies_to = e.metric AND m.shape = 'reference'
                     AND t.details ? 'points'
                     AND t.details->'activity_id' = e.details->'activity_id'
                   ORDER BY t.seq DESC LIMIT 1) AS points
           FROM derived.live_events e
           WHERE e.generation = (SELECT id FROM derived.generations WHERE state = 'active')
             AND e.seq = $1 AND e.ordinal = $2"#,
        seq,
        ordinal,
    )
    .fetch_optional(pool)
    .await?
    .ok_or(ReadError::NotFound)?;
    let points: Vec<(f64, f64)> = row
        .points
        .as_ref()
        .and_then(serde_json::Value::as_array)
        .ok_or(ReadError::NotFound)?
        .iter()
        .filter_map(|p| Some((p.get(0)?.as_f64()?, p.get(1)?.as_f64()?)))
        .collect();
    Ok(Route {
        metric: row.metric,
        local: row.local,
        points,
    })
}
