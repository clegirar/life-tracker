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
//! - **sans les points pris pendant un lieu** (voir plus bas) : à l'arrêt, iOS continue de relever des
//!   positions qui sautent de quelques dizaines de mètres, et leur somme faisait des dizaines de
//!   kilomètres pour une journée à la maison. Seuls les points strictement entre l'arrivée et le départ
//!   d'un lieu tombent : le point d'arrivée finit le trajet qui y mène, celui du départ ouvre le
//!   suivant, et la trace ne les relie jamais ;
//! - les lieux qui touchent ce jour-là, arrivés la veille compris (une nuit à la maison) : les visites
//!   iOS complètes, et les séjours détectés dans les points (voir plus bas), dont un éventuel séjour en
//!   cours.
//!
//! **Séjours détectés.** Une visite iOS n'entre au journal qu'une fois terminée, et jamais si iOS ne
//! connaît pas son arrivée : le lieu où je suis en ce moment n'en a pas. Un séjour est une suite
//! maximale de points consécutifs (précision de `MAX_ACCURACY_M` au plus) qui restent à moins de
//! `STAY_RADIUS_M` de leur centre courant, et dont le premier et le dernier sont à `MIN_STAY_SECONDS`
//! ou plus l'un de l'autre ; Overland, qui écarte les points à l'arrêt, en envoie un en s'arrêtant puis
//! un par heure. L'arrivée est le premier point ; le départ, le premier point qui suit le séjour s'il
//! vient dans les `MAX_DEPARTURE_GAP_SECONDS`, sinon son dernier point ; ses points peuvent venir des
//! jours voisins (lus par tranches d'un jour, jusqu'à `MAX_VISIT_DAYS`), d'où `arrived_before` et
//! `departed_after`. Un séjour qui recouvre dans le temps une visite iOS s'efface devant elle, qui a de
//! meilleures heures. C'est un calcul à la lecture, qui croise des enregistrements : rien n'entre au
//! journal, et le MCP ne le voit pas (route de session).
//!
//! **Séjour en cours**, aujourd'hui à Paris seulement, quand l'iPhone (la source des points) s'est
//! manifesté depuis moins de `ONGOING_SILENCE_SECONDS` (`last_seen_at`, rafraîchi par chaque envoi,
//! même vide) : le dernier séjour, s'il contient le dernier point de la source ; ou, sans séjour à la
//! fin, la dernière suite de points proches (un seul point, souvent) quand son dernier point a
//! `ONGOING_MIN_AGE_SECONDS` ou plus et que l'iPhone s'est manifesté depuis. Son départ est nul, sa
//! durée court jusqu'à maintenant.
//!
//! **Sans données ou sans déplacement.** `covered` dit si le jour est dans la période suivie : des
//! points ou des lieux existent ce jour-là ou avant, et ce jour-là ou après. Un jour couvert sans point
//! est un jour sans déplacement enregistré ; un jour non couvert est un jour sans données.

use std::ops::Range;

use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::Serialize;
use sqlx::PgPool;

use crate::geo::{distance_m, douglas_peucker};

/// La tolérance de la trace d'une séance (`collector::garmin::TRACK_TOLERANCE_M`).
const TOLERANCE_M: f64 = 5.0;
/// Au-delà de ce silence entre deux points, la trace se coupe.
const MAX_GAP_SECONDS: i64 = 15 * 60;
/// Au-delà, une position vient des antennes ou d'un Wi-Fi lointain, pas du GPS : elle ferait des pics.
const MAX_ACCURACY_M: f64 = 200.0;
/// Un lieu commencé plus tôt n'est pas cherché : borne la lecture par l'index des dates.
const MAX_VISIT_DAYS: i32 = 7;
/// Un séjour : des points à moins de 150 m de leur centre…
const STAY_RADIUS_M: f64 = 150.0;
/// … pendant 15 min au moins.
const MIN_STAY_SECONDS: i64 = 15 * 60;
/// Un séjour terminé part au premier point qui le suit, s'il vient dans les 2 h : Overland en note un
/// dès que je bouge, bien plus près du départ que le dernier point horaire. Au-delà, le téléphone était
/// éteint ou hors suivi, et le séjour finit à son dernier point.
const MAX_DEPARTURE_GAP_SECONDS: i64 = 2 * 60 * 60;
/// Un séjour n'est en cours que si l'iPhone s'est manifesté depuis moins de 70 min : avec un point par
/// heure à l'arrêt, plus de marge qu'un envoi manqué.
const ONGOING_SILENCE_SECONDS: i64 = 70 * 60;
/// Un dernier point seul vaut un séjour en cours à partir de 10 min : avant, je repars peut-être.
const ONGOING_MIN_AGE_SECONDS: i64 = 10 * 60;

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

/// Un lieu où je suis resté : une visite iOS ou un séjour détecté dans les points.
#[derive(Serialize)]
pub struct Place {
    /// `visit` (une visite iOS) ou `points` (un séjour détecté).
    origin: &'static str,
    lat: f64,
    lon: f64,
    arrival: DateTime<Utc>,
    /// `null` pour un séjour en cours.
    departure: Option<DateTime<Utc>>,
    /// Heure de Paris, `HH:MM`, de l'arrivée et du départ, qui peuvent être la veille ou le lendemain.
    arrival_time: String,
    departure_time: Option<String>,
    /// L'arrivée est d'un jour précédent, le départ d'un jour suivant.
    arrived_before: bool,
    departed_after: bool,
    /// Jusqu'à maintenant pour un séjour en cours.
    duration_s: i64,
    ongoing: bool,
    /// En mètres, selon iOS ; `null` si inconnue, et pour un séjour détecté.
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
    places: Vec<Place>,
}

struct Point {
    at: DateTime<Utc>,
    time: String,
    lat: f64,
    lon: f64,
}

/// Un point pour la détection des séjours.
struct StayPoint {
    at: DateTime<Utc>,
    time: String,
    lat: f64,
    lon: f64,
    source: i16,
}

/// Le fuseau en sous-requête et la génération fixée, comme toutes les lectures (voir `api::get_day`).
/// `now` ne sert qu'au séjour en cours : la route passe l'heure courante, les tests une heure fixée.
pub async fn day(
    pool: &PgPool,
    date: NaiveDate,
    now: DateTime<Utc>,
) -> Result<DayLocations, sqlx::Error> {
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

    let visits: Vec<Place> = sqlx::query!(
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
    .map(|v| Place {
        origin: "visit",
        lat: v.lat,
        lon: v.lon,
        arrival: v.arrival,
        departure: Some(v.departure),
        arrival_time: v.arrival_time,
        departure_time: Some(v.departure_time),
        arrived_before: v.arrived_before,
        departed_after: v.departed_after,
        duration_s: v.duration_s,
        ongoing: false,
        horizontal_accuracy: v.accuracy,
    })
    .collect();
    let places = with_stays(pool, date, now, visits).await?;

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
        segments: segments(&points, &stops(&places, now)),
        places,
    })
}

/// Les visites iOS, plus les séjours détectés qui ne recouvrent aucune d'elles, par arrivée.
async fn with_stays(
    pool: &PgPool,
    date: NaiveDate,
    now: DateTime<Utc>,
    mut places: Vec<Place>,
) -> Result<Vec<Place>, sqlx::Error> {
    let day = sqlx::query!(
        r#"SELECT ($1::date::timestamp AT TIME ZONE (SELECT timezone FROM core.settings)) AS "start!",
                  (($1::date + 1)::timestamp AT TIME ZONE (SELECT timezone FROM core.settings)) AS "end!",
                  ($2::timestamptz AT TIME ZONE (SELECT timezone FROM core.settings))::date = $1 AS "today!""#,
        date,
        now,
    )
    .fetch_one(pool)
    .await?;

    // Une tranche d'un jour de chaque côté ; une de plus, jusqu'à `MAX_VISIT_DAYS`, tant qu'un séjour
    // qui touche le jour commence au premier point lu ou finit au dernier.
    let (mut back, mut ahead) = (1, 1);
    let (mut back_done, mut ahead_done) = (false, false);
    let mut previous: Option<(DateTime<Utc>, DateTime<Utc>)> = None;
    let (points, found, trailing) = loop {
        let from = day.start - Duration::days(back);
        let to = day.end + Duration::days(ahead);
        let points = stay_points(pool, from, to).await?;
        let (found, trailing) = stays(&points);
        let touching: Vec<&Range<usize>> = found
            .iter()
            .filter(|r| points[r.end - 1].at >= day.start && points[r.start].at < day.end)
            .collect();
        let ends = points.first().zip(points.last()).map(|(f, l)| (f.at, l.at));
        if let (Some((first, last)), Some((was_first, was_last))) = (ends, previous) {
            back_done |= first == was_first;
            ahead_done |= last == was_last;
        }
        previous = ends;
        let grow_back = !back_done
            && back < i64::from(MAX_VISIT_DAYS)
            && touching.first().is_some_and(|r| r.start == 0);
        let grow_ahead = !ahead_done
            && ahead < i64::from(MAX_VISIT_DAYS)
            && to < now
            && touching.last().is_some_and(|r| r.end == points.len());
        if !grow_back && !grow_ahead {
            break (points, found, trailing);
        }
        back += i64::from(grow_back);
        ahead += i64::from(grow_ahead);
    };

    // Le séjour en cours : celui qui finit au dernier point, s'il est le dernier de sa source.
    let at_end = found
        .last()
        .filter(|r| r.end == points.len())
        .cloned()
        .map(|r| (r, true))
        .or(trailing.map(|r| (r, false)));
    let mut ongoing = None;
    if let (true, Some((range, long_enough))) = (day.today, &at_end) {
        let last = &points[range.end - 1];
        let source = sqlx::query!(
            r#"SELECT s.last_seen_at,
                      (SELECT max(e.occurred_at) FROM derived.live_events e
                       WHERE e.generation = (SELECT id FROM derived.generations WHERE state = 'active')
                         AND e.metric = 'location.point' AND e.source_id = s.id
                         AND COALESCE((e.details->>'horizontal_accuracy')::float8, 0) <= $2) AS latest
               FROM core.sources s WHERE s.id = $1"#,
            last.source,
            MAX_ACCURACY_M,
        )
        .fetch_optional(pool)
        .await?;
        if let Some(source) = source
            && let Some(seen) = source.last_seen_at
            && source.latest == Some(last.at)
            && (now - seen).num_seconds() <= ONGOING_SILENCE_SECONDS
            && (*long_enough
                || (seen > last.at && (now - last.at).num_seconds() >= ONGOING_MIN_AGE_SECONDS))
        {
            ongoing = Some(range.clone());
        }
    }

    let visits = places.len();
    let mut detected: Vec<Range<usize>> = found;
    if let Some(range) = &ongoing
        && !detected.contains(range)
    {
        detected.push(range.clone());
    }
    for range in detected {
        let run = &points[range.clone()];
        let (first, last) = (&run[0], &run[run.len() - 1]);
        let is_ongoing = ongoing.as_ref() == Some(&range);
        let leaving = points
            .get(range.end)
            .filter(|next| (next.at - last.at).num_seconds() <= MAX_DEPARTURE_GAP_SECONDS)
            .unwrap_or(last);
        let end = if is_ongoing { now } else { leaving.at };
        let touches = end >= day.start && first.at < day.end;
        let covered_by_visit = places[..visits]
            .iter()
            .any(|v| v.departure.is_some_and(|d| first.at < d && v.arrival < end));
        if !touches || covered_by_visit {
            continue;
        }
        let n = run.len() as f64;
        let round = |v: f64| (v * 1e6).round() / 1e6;
        places.push(Place {
            origin: "points",
            lat: round(run.iter().map(|p| p.lat).sum::<f64>() / n),
            lon: round(run.iter().map(|p| p.lon).sum::<f64>() / n),
            arrival: first.at,
            departure: (!is_ongoing).then_some(leaving.at),
            arrival_time: first.time.clone(),
            departure_time: (!is_ongoing).then(|| leaving.time.clone()),
            arrived_before: first.at < day.start,
            departed_after: !is_ongoing && leaving.at >= day.end,
            duration_s: (end - first.at).num_seconds(),
            ongoing: is_ongoing,
            horizontal_accuracy: None,
        });
    }
    places.sort_by_key(|p| p.arrival);
    Ok(places)
}

/// Les points assez précis entre deux instants, pour la détection des séjours.
async fn stay_points(
    pool: &PgPool,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<StayPoint>, sqlx::Error> {
    Ok(sqlx::query!(
        r#"SELECT e.occurred_at AS "occurred_at!", e.source_id AS "source_id!",
                  to_char(e.occurred_at AT TIME ZONE (SELECT timezone FROM core.settings), 'HH24:MI') AS "time!",
                  (e.details->>'point_lat')::float8 AS "lat!",
                  (e.details->>'point_lon')::float8 AS "lon!"
           FROM derived.live_events e
           WHERE e.generation = (SELECT id FROM derived.generations WHERE state = 'active')
             AND e.metric = 'location.point'
             AND e.occurred_at >= $1 AND e.occurred_at < $2
             AND COALESCE((e.details->>'horizontal_accuracy')::float8, 0) <= $3
           ORDER BY e.occurred_at, e.seq, e.ordinal"#,
        from,
        to,
        MAX_ACCURACY_M,
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|r| StayPoint {
        at: r.occurred_at,
        time: r.time,
        lat: r.lat,
        lon: r.lon,
        source: r.source_id,
    })
    .collect())
}

/// Les séjours, suites maximales de points à moins de `STAY_RADIUS_M` de leur centre courant et d'au
/// moins `MIN_STAY_SECONDS` ; puis, si aucun séjour ne finit au dernier point, la plus longue suite de
/// points proches qui y finit (un séjour peut-être en cours, trop court pour l'instant).
fn stays(points: &[StayPoint]) -> (Vec<Range<usize>>, Option<Range<usize>>) {
    let mut found = Vec::new();
    let mut i = 0;
    while i < points.len() {
        let (mut lat, mut lon, mut j) = (points[i].lat, points[i].lon, i + 1);
        while j < points.len() {
            let n = (j - i) as f64;
            if distance_m([lat / n, lon / n], [points[j].lat, points[j].lon]) > STAY_RADIUS_M {
                break;
            }
            lat += points[j].lat;
            lon += points[j].lon;
            j += 1;
        }
        if (points[j - 1].at - points[i].at).num_seconds() >= MIN_STAY_SECONDS {
            found.push(i..j);
            i = j;
        } else if j == points.len() {
            return (found, Some(i..j));
        } else {
            i += 1;
        }
    }
    (found, None)
}

/// L'arrivée et le départ de chaque lieu ; maintenant pour un séjour en cours.
fn stops(places: &[Place], now: DateTime<Utc>) -> Vec<(DateTime<Utc>, DateTime<Utc>)> {
    places
        .iter()
        .map(|p| (p.arrival, p.departure.unwrap_or(now)))
        .collect()
}

/// Les morceaux de la trace : sans les points pris pendant un lieu, coupés aux lieux et aux silences,
/// puis simplifiés.
fn segments(points: &[Point], stops: &[(DateTime<Utc>, DateTime<Utc>)]) -> Vec<Segment> {
    let moving: Vec<&Point> = points
        .iter()
        .filter(|p| !stops.iter().any(|(from, to)| *from < p.at && p.at < *to))
        .collect();
    moving
        .chunk_by(|a, b| {
            (b.at - a.at).num_seconds() <= MAX_GAP_SECONDS
                && !stops.iter().any(|(from, to)| *from <= a.at && b.at <= *to)
        })
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
        let cut = segments(&points, &[]);
        assert_eq!(cut.len(), 2);
        assert_eq!(cut[0].points, [[43.0, 5.0], [43.0, 5.009]]);
        assert_eq!(cut[1].points, [[43.1, 5.1]]);
        assert_eq!((cut[0].end - cut[0].start).num_minutes(), 9);
        assert!(segments(&[], &[]).is_empty());
    }

    #[test]
    fn the_jitter_of_a_stay_is_no_trace() {
        // Un trajet de 10 min, une heure au même endroit avec des sauts de 30 m chaque minute, puis le
        // départ : seuls les deux trajets restent, coupés au lieu.
        let mut points: Vec<Point> = (0..10)
            .map(|i| at(i, 43.0, 5.0 + i as f64 * 1e-3))
            .collect();
        points.extend((10..70).map(|i| at(i, 43.0 + (i % 2) as f64 * 3e-4, 5.01)));
        points.extend((70..80).map(|i| at(i, 43.0, 5.01 + (i - 69) as f64 * 1e-3)));
        let stop = (points[10].at, points[70].at);
        let cut = segments(&points, &[stop]);
        assert_eq!(cut.len(), 2);
        assert_eq!(cut[0].points, [[43.0, 5.0], [43.0, 5.01]]);
        assert_eq!(cut[1].points, [[43.0, 5.011], [43.0, 5.02]]);
        assert_eq!(
            (cut[0].end, cut[1].start),
            stop,
            "le point d'arrivée et celui du départ restent"
        );
        // Sans le lieu, les sauts font la trace.
        assert!(segments(&points, &[])[0].points.len() > 30);
    }

    fn stay_point(minutes: i64, lat: f64, lon: f64) -> StayPoint {
        let p = at(minutes, lat, lon);
        StayPoint {
            at: p.at,
            time: String::new(),
            lat,
            lon,
            source: 1,
        }
    }

    #[test]
    fn a_stay_needs_fifteen_minutes_within_its_radius() {
        // Un trajet, une heure au même endroit à 50 m près, un départ, puis un point seul à la fin.
        let points = [
            stay_point(0, 43.0, 5.0),
            stay_point(5, 43.01, 5.0),
            stay_point(10, 43.02, 5.0),
            stay_point(70, 43.0204, 5.0),
            stay_point(130, 43.0199, 5.0004),
            stay_point(140, 43.05, 5.0),
        ];
        let (found, trailing) = stays(&points);
        assert_eq!((found.len(), &found[0], trailing), (1, &(2..5), Some(5..6)));
        // Moins de 15 min au même endroit : pas de séjour, mais une suite à la fin.
        assert_eq!(stays(&points[..3]), (vec![], Some(2..3)));
        let (found, trailing) = stays(&points[..4]);
        assert_eq!((found.len(), &found[0], trailing), (1, &(2..4), None));
        assert_eq!(stays(&[]), (vec![], None));
    }
}
