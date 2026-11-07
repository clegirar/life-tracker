//! La carte Déplacements de la page du jour : la trace de l'iPhone et les lieux où je suis resté. Une
//! lecture de la web app seulement, derrière la session : elle donne des coordonnées, que le MCP ne voit
//! jamais (hors de `READ_ROUTES`, voir l'architecture). Le MCP ne lit des lieux que leurs détails
//! partagés, à travers les zones de confidentialité.
//!
//! Calculée à la lecture sur `derived.live_events`, comme le reste :
//! - les points du jour de Paris, sans ceux dont la précision dépasse `MAX_ACCURACY_M` (positions par
//!   les antennes), coupés en morceaux là où deux points s'écartent de plus de `MAX_GAP_SECONDS` (le
//!   téléphone immobile ou hors suivi : on ne trace pas de ligne à travers), puis chaque morceau
//!   simplifié par Douglas–Peucker à `TOLERANCE_M`, comme la trace d'une séance ;
//! - les lieux qui touchent ce jour-là, arrivés la veille compris (une nuit à la maison).
//!
//! **Sans données ou sans déplacement.** `covered` dit si le jour est dans la période suivie : des
//! points ou des lieux existent ce jour-là ou avant, et ce jour-là ou après. Un jour couvert sans point
//! est un jour sans déplacement enregistré ; un jour non couvert est un jour sans données.

use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use sqlx::PgPool;

use crate::geo::douglas_peucker;

/// La tolérance de la trace d'une séance (`collector::garmin::TRACK_TOLERANCE_M`).
const TOLERANCE_M: f64 = 5.0;
/// Au-delà de ce silence entre deux points, la trace se coupe.
const MAX_GAP_SECONDS: i64 = 15 * 60;
/// Au-delà, une position vient des antennes ou d'un Wi-Fi lointain, pas du GPS : elle ferait des pics.
const MAX_ACCURACY_M: f64 = 200.0;
/// Un lieu commencé plus tôt n'est pas cherché : borne la lecture par l'index des dates.
const MAX_VISIT_DAYS: i32 = 7;

/// Un morceau de trace, sans silence de plus de 15 min.
#[derive(Serialize)]
pub struct Segment {
    /// Heure de Paris, `HH:MM`, du premier et du dernier point.
    start_time: String,
    end_time: String,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    /// `[lat, lon]` au millionième, simplifiés ; un seul si le morceau n'a qu'un point.
    points: Vec<[f64; 2]>,
}

/// Un lieu où je suis resté.
#[derive(Serialize)]
pub struct Visit {
    lat: f64,
    lon: f64,
    arrival: DateTime<Utc>,
    departure: DateTime<Utc>,
    /// Heure de Paris, `HH:MM`, de l'arrivée et du départ, qui peuvent être la veille ou le lendemain.
    arrival_time: String,
    departure_time: String,
    /// L'arrivée est d'un jour précédent, le départ d'un jour suivant.
    arrived_before: bool,
    departed_after: bool,
    duration_s: i64,
    /// En mètres, selon iOS ; `null` si inconnue.
    horizontal_accuracy: Option<f64>,
}

#[derive(Serialize)]
pub struct DayLocations {
    date: NaiveDate,
    /// Le jour est dans la période suivie (voir l'en-tête du module).
    covered: bool,
    /// Les points du jour avant simplification, ceux trop imprécis compris.
    point_count: i64,
    /// Dans l'ordre du temps.
    segments: Vec<Segment>,
    /// Par arrivée.
    visits: Vec<Visit>,
}

struct Point {
    at: DateTime<Utc>,
    time: String,
    lat: f64,
    lon: f64,
}

/// Le fuseau en sous-requête et la génération fixée, comme toutes les lectures (voir `api::get_day`).
pub async fn day(pool: &PgPool, date: NaiveDate) -> Result<DayLocations, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT e.occurred_at AS "occurred_at!",
                  to_char(e.occurred_at AT TIME ZONE (SELECT timezone FROM core.settings), 'HH24:MI') AS "time!",
                  (e.details->>'point_lat')::float8 AS "lat!",
                  (e.details->>'point_lon')::float8 AS "lon!",
                  (e.details->>'horizontal_accuracy')::float8 AS accuracy
           FROM derived.live_events e
           WHERE e.generation = (SELECT id FROM derived.generations WHERE state = 'active')
             AND e.metric = 'location.point'
             AND e.occurred_at >= ($1::date::timestamp AT TIME ZONE (SELECT timezone FROM core.settings))
             AND e.occurred_at < (($1::date + 1)::timestamp AT TIME ZONE (SELECT timezone FROM core.settings))
           ORDER BY e.occurred_at, e.seq, e.ordinal"#,
        date,
    )
    .fetch_all(pool)
    .await?;
    let point_count = rows.len() as i64;
    let points: Vec<Point> = rows
        .into_iter()
        .filter(|r| r.accuracy.is_none_or(|a| a <= MAX_ACCURACY_M))
        .map(|r| Point {
            at: r.occurred_at,
            time: r.time,
            lat: r.lat,
            lon: r.lon,
        })
        .collect();

    let visits = sqlx::query!(
        r#"WITH bounds AS (
             SELECT ($1::date::timestamp AT TIME ZONE (SELECT timezone FROM core.settings)) AS day_start,
                    (($1::date + 1)::timestamp AT TIME ZONE (SELECT timezone FROM core.settings)) AS day_end
           ), visits AS (
             SELECT e.occurred_at AS arrival, (e.details->>'departure')::timestamptz AS departure,
                    (e.details->>'visit_lat')::float8 AS lat, (e.details->>'visit_lon')::float8 AS lon,
                    (e.details->>'duration_s')::int8 AS duration_s,
                    (e.details->>'horizontal_accuracy')::float8 AS accuracy, e.seq
             FROM derived.live_events e, bounds b
             WHERE e.generation = (SELECT id FROM derived.generations WHERE state = 'active')
               AND e.metric = 'place.visit'
               AND e.occurred_at >= b.day_start - make_interval(days => $2)
               AND e.occurred_at < b.day_end
           )
           SELECT v.arrival AS "arrival!", v.departure AS "departure!", v.lat AS "lat!",
                  v.lon AS "lon!", v.duration_s AS "duration_s!", v.accuracy,
                  to_char(v.arrival AT TIME ZONE (SELECT timezone FROM core.settings), 'HH24:MI') AS "arrival_time!",
                  to_char(v.departure AT TIME ZONE (SELECT timezone FROM core.settings), 'HH24:MI') AS "departure_time!",
                  v.arrival < b.day_start AS "arrived_before!",
                  v.departure >= b.day_end AS "departed_after!"
           FROM visits v, bounds b
           WHERE v.departure > b.day_start OR v.arrival >= b.day_start
           ORDER BY v.arrival, v.seq"#,
        date,
        MAX_VISIT_DAYS,
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|v| Visit {
        lat: v.lat,
        lon: v.lon,
        arrival: v.arrival,
        departure: v.departure,
        arrival_time: v.arrival_time,
        departure_time: v.departure_time,
        arrived_before: v.arrived_before,
        departed_after: v.departed_after,
        duration_s: v.duration_s,
        horizontal_accuracy: v.accuracy,
    })
    .collect();

    let covered = sqlx::query_scalar!(
        r#"WITH bounds AS (
             SELECT ($1::date::timestamp AT TIME ZONE (SELECT timezone FROM core.settings)) AS day_start,
                    (($1::date + 1)::timestamp AT TIME ZONE (SELECT timezone FROM core.settings)) AS day_end
           )
           SELECT EXISTS (SELECT 1 FROM derived.live_events e, bounds b
                          WHERE e.generation = (SELECT id FROM derived.generations WHERE state = 'active')
                            AND e.metric IN ('location.point', 'place.visit')
                            AND e.occurred_at < b.day_end)
              AND EXISTS (SELECT 1 FROM derived.live_events e, bounds b
                          WHERE e.generation = (SELECT id FROM derived.generations WHERE state = 'active')
                            AND e.metric IN ('location.point', 'place.visit')
                            AND e.occurred_at >= b.day_start) AS "covered!""#,
        date,
    )
    .fetch_one(pool)
    .await?;

    Ok(DayLocations {
        date,
        covered,
        point_count,
        segments: segments(&points),
        visits,
    })
}

/// Les morceaux de la trace : coupés aux silences, puis simplifiés.
fn segments(points: &[Point]) -> Vec<Segment> {
    points
        .chunk_by(|a, b| (b.at - a.at).num_seconds() <= MAX_GAP_SECONDS)
        .map(|chunk| {
            let (first, last) = (&chunk[0], &chunk[chunk.len() - 1]);
            let raw: Vec<[f64; 3]> = chunk
                .iter()
                .map(|p| [p.lat, p.lon, (p.at - first.at).num_seconds() as f64])
                .collect();
            let round = |v: f64| (v * 1e6).round() / 1e6;
            Segment {
                start_time: first.time.clone(),
                end_time: last.time.clone(),
                start: first.at,
                end: last.at,
                points: douglas_peucker(&raw, TOLERANCE_M)
                    .into_iter()
                    .map(|[lat, lon, _]| [round(lat), round(lon)])
                    .collect(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(minutes: i64, lat: f64, lon: f64) -> Point {
        let start: DateTime<Utc> = "2026-10-20T08:00:00Z".parse().unwrap();
        Point {
            at: start + chrono::Duration::minutes(minutes),
            time: String::new(),
            lat,
            lon,
        }
    }

    #[test]
    fn a_silence_cuts_the_trace_and_a_straight_line_keeps_its_ends() {
        let mut points: Vec<Point> = (0..10)
            .map(|i| at(i, 43.0, 5.0 + i as f64 * 1e-3))
            .collect();
        points.push(at(40, 43.1, 5.1));
        let cut = segments(&points);
        assert_eq!(cut.len(), 2);
        assert_eq!(cut[0].points, [[43.0, 5.0], [43.0, 5.009]]);
        assert_eq!(cut[1].points, [[43.1, 5.1]]);
        assert_eq!((cut[0].end - cut[0].start).num_minutes(), 9);
        assert!(segments(&[]).is_empty());
    }
}
