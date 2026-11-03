//! Lectures génériques de l'API, pour les outils du serveur MCP (étape 9) : inventaire, recherche,
//! occurrences, durée cumulée, un élément (découverte et dernière fois), classement, découvertes.
//!
//! Comme le reste de la lecture : calculé à chaque appel, jamais stocké (principe 3), sur
//! `derived.live_*` de la génération active, les jours découpés à minuit heure de Paris d'après
//! `core.settings` (règle 10). Tout vient des registres : la métrique est un paramètre, ses niveaux
//! d'éléments, sa découverte et la source de sa durée sont lus dans `core.metrics` (principe 12).
//! Le fuseau en sous-requête et le numéro de génération fixé, comme partout : voir `api::get_day`.

use std::collections::HashMap;

use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use sqlx::PgPool;

use crate::music::fold;

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
        r#"SELECT metric, shape::text AS "shape!", item_levels, discovery, duration_from
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
                  description
           FROM core.metrics ORDER BY metric"#
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

/// Un nom approximatif : la clé normalisée qui le contient, ou des mots proches (trigrammes, l'index
/// `gin_trgm_ops` de `event_items`). Les déclarations et références ne sont jamais des résultats.
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
             SELECT i.metric, i.level, i.item_key, max(word_similarity($1, i.item_key)) AS score
             FROM derived.event_items i
             JOIN core.metrics m ON m.metric = i.metric
             WHERE i.generation = (SELECT id FROM derived.generations WHERE state = 'active')
               AND m.shape NOT IN ('declaration', 'reference')
               AND ($2::text IS NULL OR i.metric = $2)
               AND ($3::text IS NULL OR i.level = $3)
               AND (i.item_key LIKE $4 OR $1 <% i.item_key)
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
    at: DateTime<Utc>,
    /// Heure de Paris, `AAAA-MM-JJ HH:MM`.
    local: String,
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
) -> Result<Occurrences> {
    match form {
        "list" => {
            let rows = sqlx::query!(
                r#"SELECT e.seq AS "seq!", e.ordinal AS "ordinal!", e.occurred_at AS "occurred_at!",
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
                        items: context_of(&contexts, (r.seq, r.ordinal), None, &[]),
                        at: r.occurred_at,
                        local: r.local,
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
/// référence : la même règle que les minutes de la page du jour. Une jointure, donc en vue.
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
