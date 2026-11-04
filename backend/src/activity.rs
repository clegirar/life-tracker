//! La carte Activité de la page du jour : pas, calories, sommeil et séances, avec leurs détails, et
//! la trace d'une séance pour sa carte. Des lectures de la web app seulement, derrière la session :
//! elles donnent des coordonnées, que le MCP ne voit jamais (hors de `READ_ROUTES`, voir l'architecture).
//!
//! Calculées à la lecture sur `derived.live_events`, comme le reste : une révision de Garmin annule
//! la version précédente, donc chaque mesure n'a qu'une version vivante. Si l'annulation manquait
//! (état du connecteur perdu), la plus récente l'emporte, comme dans le calendrier (`last`).

use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use serde_json::Value;
use sqlx::PgPool;

/// Une mesure du jour : sa valeur et ses détails tels que l'interprète les a écrits.
#[derive(Serialize)]
pub struct Measure {
    value: Option<f64>,
    details: Value,
}

#[derive(Serialize)]
pub struct Workout {
    /// Heure de Paris du début, `HH:MM`.
    time: String,
    occurred_at: DateTime<Utc>,
    /// Les détails de la séance : type, lieu, durées, distance, allure, cardio, calories, coordonnées
    /// de début et de fin.
    details: Value,
    /// Une trace existe pour cette séance : la carte peut la demander.
    has_track: bool,
}

#[derive(Serialize)]
pub struct DayActivity {
    date: NaiveDate,
    /// `null` : pas de données ce jour-là (montre pas portée), jamais zéro.
    steps: Option<Measure>,
    calories: Option<Measure>,
    sleep: Option<Measure>,
    workouts: Vec<Workout>,
}

/// Le fuseau en sous-requête et la génération fixée, comme toutes les lectures (voir `api::get_day`).
pub async fn day(pool: &PgPool, date: NaiveDate) -> Result<DayActivity, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT e.metric AS "metric!", e.value::float8 AS value, e.details AS "details!",
                  e.occurred_at AS "occurred_at!",
                  to_char(e.occurred_at AT TIME ZONE (SELECT timezone FROM core.settings), 'HH24:MI') AS "time!",
                  e.metric = 'workout' AND EXISTS (
                    SELECT 1 FROM derived.live_events t
                    WHERE t.generation = e.generation AND t.metric = 'workout.track'
                      AND t.details->'activity_id' = e.details->'activity_id') AS "has_track!"
           FROM derived.live_events e
           WHERE e.generation = (SELECT id FROM derived.generations WHERE state = 'active')
             AND e.metric IN ('steps', 'calories.burned', 'sleep', 'workout')
             AND e.occurred_at >= ($1::date::timestamp AT TIME ZONE (SELECT timezone FROM core.settings))
             AND e.occurred_at < (($1::date + 1)::timestamp AT TIME ZONE (SELECT timezone FROM core.settings))
           ORDER BY e.occurred_at, e.seq, e.ordinal"#,
        date,
    )
    .fetch_all(pool)
    .await?;
    let mut day = DayActivity {
        date,
        steps: None,
        calories: None,
        sleep: None,
        workouts: Vec::new(),
    };
    for row in rows {
        let measure = || {
            Some(Measure {
                value: row.value,
                details: row.details.clone(),
            })
        };
        // Dans l'ordre du journal : la dernière version écrase les précédentes.
        match row.metric.as_str() {
            "steps" => day.steps = measure(),
            "calories.burned" => day.calories = measure(),
            "sleep" => day.sleep = measure(),
            _ => day.workouts.push(Workout {
                time: row.time,
                occurred_at: row.occurred_at,
                details: row.details,
                has_track: row.has_track,
            }),
        }
    }
    Ok(day)
}

#[derive(Serialize)]
pub struct Track {
    activity_id: u64,
    /// `[lat, lon, secondes depuis le début]`, simplifiés par le connecteur.
    points: Value,
}

/// La trace vivante d'une séance, la plus récente si plusieurs restaient vivantes.
pub async fn track(pool: &PgPool, activity_id: u64) -> Result<Option<Track>, sqlx::Error> {
    let id = Value::from(activity_id);
    let points = sqlx::query_scalar!(
        r#"SELECT e.details->'points' AS "points!"
           FROM derived.live_events e
           WHERE e.generation = (SELECT id FROM derived.generations WHERE state = 'active')
             AND e.metric = 'workout.track'
             AND e.details->'activity_id' = $1
           ORDER BY e.seq DESC
           LIMIT 1"#,
        id,
    )
    .fetch_optional(pool)
    .await?;
    Ok(points.map(|points| Track {
        activity_id,
        points,
    }))
}
