//! Les vues du mois et de l'année : ce que chaque journée contient, et ce que l'on sait de sa complétude.
//!
//! Tout se calcule à la lecture, rien n'est stocké (principe 3) : un enregistrement tardif, un import
//! ou une annulation changent l'apparence d'un jour passé dès la lecture suivante (règle 7). Générique :
//! les métriques affichées, leurs agrégats et les seuils viennent des registres (principe 12).
//!
//! Pour chaque jour, trois signaux, jamais confondus avec une journée sans activité (principe 10) :
//! - `errors` : des enregistrements de ce jour sont en erreur d'interprétation, la journée est incomplète ;
//! - `pending` : une source muette n'a pas encore couvert ce jour, il est en attente de son rattrapage ;
//! - `to_check` : la métrique ne reçoit plus rien depuis son seuil de fraîcheur, à vérifier.
//!
//! Et pour chaque métrique, une valeur ou `null` : `null` dit « pas de données » (avant la première
//! donnée de la métrique, ou dans un trou entre deux sources), jamais zéro.

use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Datelike, Days, Months, NaiveDate, Utc};
use serde::Serialize;
use sqlx::PgPool;

#[derive(Serialize)]
pub struct MonthMetric {
    metric: String,
    label: String,
    unit: Option<String>,
}

#[derive(Serialize)]
pub struct MonthDay {
    date: NaiveDate,
    /// Par métrique : l'agrégat du jour, ou `null` s'il n'y a pas de données ce jour-là.
    values: BTreeMap<String, Option<f64>>,
    /// Enregistrements de ce jour, heure de Paris, en erreur d'interprétation.
    errors: i64,
    /// Sources muettes qui n'ont pas encore couvert ce jour.
    pending: Vec<String>,
    /// Métriques sans nouvelle donnée depuis leur seuil de fraîcheur.
    to_check: Vec<String>,
}

/// Une période, jour par jour : la forme commune du mois et de l'année.
#[derive(Serialize)]
pub struct Period {
    today: NaiveDate,
    metrics: Vec<MonthMetric>,
    days: Vec<MonthDay>,
}

#[derive(Serialize)]
pub struct Month {
    month: String,
    #[serde(flatten)]
    period: Period,
}

#[derive(Serialize)]
pub struct Year {
    year: i32,
    #[serde(flatten)]
    period: Period,
}

/// `AAAA-MM` strict : le premier jour du mois.
pub fn parse_month(value: &str) -> Option<NaiveDate> {
    let bytes = value.as_bytes();
    let shaped = bytes.len() == 7
        && bytes[4] == b'-'
        && bytes[..4].iter().chain(&bytes[5..]).all(u8::is_ascii_digit);
    shaped
        .then(|| NaiveDate::parse_from_str(&format!("{value}-01"), "%Y-%m-%d").ok())
        .flatten()
}

/// `AAAA` strict : le 1er janvier de l'année.
pub fn parse_year(value: &str) -> Option<NaiveDate> {
    let shaped = value.len() == 4 && value.bytes().all(|b| b.is_ascii_digit());
    shaped
        .then(|| NaiveDate::from_ymd_opt(value.parse().ok()?, 1, 1))
        .flatten()
}

/// Les jours couverts d'une métrique : de sa première donnée à aujourd'hui, sauf les trous entre
/// deux sources. Chaque source couvre de sa première à sa dernière donnée ; après la dernière
/// couverture, la métrique est suivie jusqu'à aujourd'hui (un zéro y est un vrai zéro, et le seuil
/// de fraîcheur dira s'il faut le vérifier). Deux couvertures qui se touchent ne laissent aucun trou.
#[derive(Default)]
struct Coverage {
    spans: Vec<(NaiveDate, NaiveDate)>,
}

impl Coverage {
    fn add(&mut self, first: NaiveDate, last: NaiveDate) {
        self.spans.push((first, last));
    }

    fn merge(mut self) -> Self {
        self.spans.sort();
        let mut merged: Vec<(NaiveDate, NaiveDate)> = Vec::new();
        for (first, last) in self.spans {
            match merged.last_mut() {
                Some(current)
                    if first
                        <= current
                            .1
                            .checked_add_days(Days::new(1))
                            .unwrap_or(current.1) =>
                {
                    current.1 = current.1.max(last);
                }
                _ => merged.push((first, last)),
            }
        }
        Self { spans: merged }
    }

    fn covers(&self, day: NaiveDate) -> bool {
        let Some(&(start, _)) = self.spans.first() else {
            return false;
        };
        let (_, end) = *self.spans.last().expect("non vide");
        day >= start && (day > end || self.spans.iter().any(|&(f, l)| f <= day && day <= l))
    }
}

pub async fn month(pool: &PgPool, start: NaiveDate) -> Result<Month, sqlx::Error> {
    let end = start
        .checked_add_months(Months::new(1))
        .unwrap_or(NaiveDate::MAX);
    Ok(Month {
        month: start.format("%Y-%m").to_string(),
        period: range(pool, start, end).await?,
    })
}

/// L'année d'un seul calcul : les couvertures portent sur tout l'historique, douze mois les
/// referaient douze fois. `None` pour une année future.
pub async fn year(pool: &PgPool, start: NaiveDate) -> Result<Option<Year>, sqlx::Error> {
    let end = start
        .checked_add_months(Months::new(12))
        .unwrap_or(NaiveDate::MAX);
    let period = range(pool, start, end).await?;
    Ok((start.year() <= period.today.year()).then(|| Year {
        year: start.year(),
        period,
    }))
}

/// Les jours de `start` inclus à `end` exclu. Le fuseau en sous-requête et le numéro de génération
/// fixé, comme toutes les lectures : voir `api::get_day`.
pub async fn range(pool: &PgPool, start: NaiveDate, end: NaiveDate) -> Result<Period, sqlx::Error> {
    let clock = sqlx::query!(
        r#"SELECT now() AS "now!",
                  (now() AT TIME ZONE (SELECT timezone FROM core.settings))::date AS "today!""#
    )
    .fetch_one(pool)
    .await?;
    let (now, today): (DateTime<Utc>, NaiveDate) = (clock.now, clock.today);

    // Les déclarations et les références éclairent une autre métrique : jamais affichées seules.
    let metrics = sqlx::query!(
        r#"SELECT metric, label, unit, daily_aggregation::text AS "aggregation!",
                  extract(epoch FROM freshness_threshold)::float8 AS freshness
           FROM core.metrics
           WHERE shape NOT IN ('declaration', 'reference') AND daily_aggregation <> 'none'
           ORDER BY metric"#
    )
    .fetch_all(pool)
    .await?;
    let names: Vec<String> = metrics.iter().map(|m| m.metric.clone()).collect();

    // Par source : `live_events` arrête déjà une source là où une autre fait foi, sans faux trou.
    let spans = sqlx::query!(
        r#"SELECT e.metric AS "metric!",
                  (min(e.occurred_at) AT TIME ZONE (SELECT timezone FROM core.settings))::date AS "first_day!",
                  (max(e.occurred_at) AT TIME ZONE (SELECT timezone FROM core.settings))::date AS "last_day!",
                  max(e.occurred_at) AS "last_at!"
           FROM derived.live_events e
           WHERE e.generation = (SELECT id FROM derived.generations WHERE state = 'active')
             AND e.metric = ANY($1)
           GROUP BY e.metric, e.source_id"#,
        &names,
    )
    .fetch_all(pool)
    .await?;

    let values = sqlx::query!(
        r#"SELECT (e.occurred_at AT TIME ZONE (SELECT timezone FROM core.settings))::date AS "day!",
                  e.metric AS "metric!",
                  count(*)::float8 AS "count!",
                  sum(e.value)::float8 AS sum,
                  avg(e.value)::float8 AS avg,
                  max(e.value)::float8 AS max,
                  (array_agg(e.value ORDER BY e.occurred_at DESC, e.seq DESC))[1]::float8 AS last
           FROM derived.live_events e
           WHERE e.generation = (SELECT id FROM derived.generations WHERE state = 'active')
             AND e.metric = ANY($3)
             AND e.occurred_at >= ($1::date::timestamp AT TIME ZONE (SELECT timezone FROM core.settings))
             AND e.occurred_at < ($2::date::timestamp AT TIME ZONE (SELECT timezone FROM core.settings))
           GROUP BY 1, 2"#,
        start,
        end,
        &names,
    )
    .fetch_all(pool)
    .await?;

    // Le jour d'une erreur est celui de son enregistrement : la donnée manquante est de ce jour-là.
    let errors = sqlx::query!(
        r#"SELECT (r.occurred_at AT TIME ZONE (SELECT timezone FROM core.settings))::date AS "day!",
                  count(*) AS "errors!"
           FROM derived.interpretation_errors x
           JOIN core.records r ON r.seq = x.seq
           WHERE x.generation = (SELECT id FROM derived.generations WHERE state = 'active')
             AND r.occurred_at >= ($1::date::timestamp AT TIME ZONE (SELECT timezone FROM core.settings))
             AND r.occurred_at < ($2::date::timestamp AT TIME ZONE (SELECT timezone FROM core.settings))
           GROUP BY 1"#,
        start,
        end,
    )
    .fetch_all(pool)
    .await?;

    // La même règle que `/v1/sources/health` : dernier contact réussi, ou création s'il n'y en a pas eu.
    // Le jour de ce contact n'est couvert qu'en partie : il est en attente lui aussi.
    let silent = sqlx::query!(
        r#"SELECT name,
                  (COALESCE(last_seen_at, created_at) AT TIME ZONE (SELECT timezone FROM core.settings))::date AS "since!"
           FROM core.sources
           WHERE silence_threshold IS NOT NULL AND revoked_at IS NULL
             AND now() - COALESCE(last_seen_at, created_at) > silence_threshold
           ORDER BY name"#
    )
    .fetch_all(pool)
    .await?;

    let mut coverage: HashMap<&str, Coverage> = HashMap::new();
    let mut last_at: HashMap<&str, DateTime<Utc>> = HashMap::new();
    let mut last_day: HashMap<&str, NaiveDate> = HashMap::new();
    for span in &spans {
        let metric = span.metric.as_str();
        coverage
            .entry(metric)
            .or_default()
            .add(span.first_day, span.last_day);
        let latest = last_at.entry(metric).or_insert(span.last_at);
        if span.last_at >= *latest {
            *latest = span.last_at;
            last_day.insert(metric, span.last_day);
        }
    }
    let coverage: HashMap<&str, Coverage> =
        coverage.into_iter().map(|(m, c)| (m, c.merge())).collect();

    // À vérifier : la métrique a un seuil de fraîcheur, et rien n'est arrivé depuis. Une métrique
    // sans aucune donnée n'est pas à vérifier : elle n'a pas commencé.
    let stale: Vec<(&str, NaiveDate)> = metrics
        .iter()
        .filter_map(|m| {
            let threshold = m.freshness?;
            let last = *last_at.get(m.metric.as_str())?;
            ((now - last).as_seconds_f64() > threshold)
                .then(|| (m.metric.as_str(), last_day[m.metric.as_str()]))
        })
        .collect();

    let mut by_day: HashMap<(NaiveDate, &str), f64> = HashMap::new();
    for row in &values {
        let aggregation = metrics
            .iter()
            .find(|m| m.metric == row.metric)
            .map_or("count", |m| m.aggregation.as_str());
        let value = match aggregation {
            "count" => Some(row.count),
            "sum" => row.sum,
            "avg" => row.avg,
            "max" => row.max,
            "last" => row.last,
            _ => None,
        };
        if let Some(value) = value {
            by_day.insert((row.day, row.metric.as_str()), value);
        }
    }
    let errors: HashMap<NaiveDate, i64> = errors.iter().map(|e| (e.day, e.errors)).collect();

    let mut days = Vec::new();
    let mut day = start;
    while day < end {
        let past = day <= today;
        let values = metrics
            .iter()
            .map(|m| {
                let metric = m.metric.as_str();
                let covered = past && coverage.get(metric).is_some_and(|c| c.covers(day));
                let empty = matches!(m.aggregation.as_str(), "count" | "sum").then_some(0.0);
                let value = match by_day.get(&(day, metric)) {
                    Some(&value) => Some(value),
                    None if covered => empty,
                    None => None,
                };
                (m.metric.clone(), value)
            })
            .collect();
        days.push(MonthDay {
            date: day,
            values,
            errors: errors.get(&day).copied().unwrap_or(0),
            pending: silent
                .iter()
                .filter(|s| past && s.since <= day)
                .map(|s| s.name.clone())
                .collect(),
            to_check: stale
                .iter()
                .filter(|&&(_, last)| past && day > last)
                .map(|&(metric, _)| metric.to_owned())
                .collect(),
        });
        match day.succ_opt() {
            Some(next) => day = next,
            None => break,
        }
    }

    Ok(Period {
        today,
        metrics: metrics
            .into_iter()
            .map(|m| MonthMetric {
                metric: m.metric,
                label: m.label,
                unit: m.unit,
            })
            .collect(),
        days,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(value: &str) -> NaiveDate {
        NaiveDate::parse_from_str(value, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn months_are_strict() {
        assert_eq!(parse_month("2026-10"), Some(d("2026-10-01")));
        for bad in [
            "2026-1",
            "2026-13",
            "2026-00",
            "26-10-1",
            "2026/10",
            "+202-10",
            "2026-10-01",
            "",
        ] {
            assert_eq!(parse_month(bad), None, "{bad}");
        }
    }

    #[test]
    fn years_are_strict() {
        assert_eq!(parse_year("2026"), Some(d("2026-01-01")));
        assert_eq!(parse_year("0001"), Some(d("0001-01-01")));
        for bad in ["26", "20266", "2026-01", "+202", "abcd", ""] {
            assert_eq!(parse_year(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_gap_between_two_sources_is_not_covered() {
        let mut c = Coverage::default();
        c.add(d("2026-10-20"), d("2026-11-02"));
        c.add(d("2022-01-03"), d("2026-10-10"));
        let c = c.merge();
        assert!(!c.covers(d("2022-01-02")));
        assert!(c.covers(d("2022-01-03")));
        assert!(c.covers(d("2026-10-10")));
        assert!(!c.covers(d("2026-10-11")));
        assert!(!c.covers(d("2026-10-19")));
        assert!(c.covers(d("2026-10-20")));
        // Après la dernière donnée : suivi jusqu'à aujourd'hui.
        assert!(c.covers(d("2026-12-25")));
    }

    #[test]
    fn touching_or_overlapping_sources_leave_no_gap() {
        let mut c = Coverage::default();
        c.add(d("2022-01-03"), d("2026-10-12"));
        c.add(d("2026-10-13"), d("2026-10-20"));
        c.add(d("2026-10-15"), d("2026-11-01"));
        let c = c.merge();
        assert_eq!(c.spans, vec![(d("2022-01-03"), d("2026-11-01"))]);
        assert!(!Coverage::default().merge().covers(d("2026-10-13")));
    }
}
