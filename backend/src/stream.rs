//! La carte Twitch de la page du jour : le temps passé, par chaîne, et les nouvelles chaînes. Une
//! lecture de la web app seulement, derrière la session : le MCP lit les mêmes faits par les lectures
//! génériques (durée cumulée, classement, découvertes).
//!
//! Calculée à la lecture sur `derived.live_events`, comme le reste. Le temps est celui de la forme
//! `duration` (`read::measured`) : l'union des segments, coupée aux minuits de Paris, si bien que deux
//! streams regardés en même temps ne comptent qu'une fois dans le total, et chacun en entier pour sa
//! chaîne.

use chrono::NaiveDate;
use serde::Serialize;
use sqlx::PgPool;

use crate::interpreters::MAX_MEASURED_MS;
use crate::read::{self, DayDiscovery, Period};

const METRIC: &str = "stream.watch";
const LEVEL: &str = "channel";

#[derive(Serialize)]
pub struct Channel {
    /// Le login, pour le lien `https://www.twitch.tv/<login>`, construit par la web app.
    key: String,
    name: String,
    /// L'union des segments de la chaîne ce jour-là.
    minutes: i64,
    /// `live`, `vod`, `clip`, dans cet ordre.
    contents: Vec<String>,
    /// Heure de Paris, `HH:MM`, du début du premier segment, qui peut être la veille.
    first_time: String,
    /// Heure de Paris, `HH:MM`, de la fin du dernier segment, qui peut être le lendemain.
    last_time: String,
}

#[derive(Serialize)]
pub struct DayStream {
    date: NaiveDate,
    /// L'union de tous les segments : le temps passé sur Twitch, deux streams à la fois comptant une fois.
    minutes: i64,
    /// La somme des temps de chaque chaîne (et du temps sans chaîne) : `summed_minutes - minutes` est
    /// le temps passé à regarder deux streams à la fois, ou plus.
    summed_minutes: i64,
    /// Les segments qui touchent ce jour-là, commencés la veille compris.
    segments: i64,
    /// Par temps décroissant, puis par nom.
    channels: Vec<Channel>,
    /// Le temps des segments dont l'extension n'a pas su la chaîne.
    unknown_minutes: i64,
    /// Les chaînes regardées ce jour-là pour la première fois.
    discoveries: Vec<DayDiscovery>,
}

/// Le fuseau en sous-requête et la génération fixée, comme toutes les lectures (voir `api::get_day`).
pub async fn day(pool: &PgPool, date: NaiveDate) -> Result<DayStream, sqlx::Error> {
    let period = Period {
        from: date,
        to: date,
    };
    let total = read::measured(pool, METRIC, period, None, None, false).await?;
    let by_channel = read::measured(pool, METRIC, period, Some(LEVEL), None, true).await?;
    // Les segments qui touchent le jour, par chaîne : leurs contenus et leurs heures.
    let rows = sqlx::query!(
        r#"WITH spans AS (
             SELECT i.item_key, e.details->>'content' AS content, e.occurred_at,
                    e.occurred_at + (e.details->>'played_ms')::bigint * interval '1 millisecond' AS ended_at
             FROM derived.live_events e
             LEFT JOIN derived.event_items i
               ON i.generation = e.generation AND i.seq = e.seq AND i.ordinal = e.ordinal
              AND i.level = $2
             WHERE e.generation = (SELECT id FROM derived.generations WHERE state = 'active')
               AND e.metric = $3
               AND e.occurred_at >= ($1::date::timestamp AT TIME ZONE (SELECT timezone FROM core.settings))
                                    - $4::bigint * interval '1 millisecond'
               AND e.occurred_at < (($1::date + 1)::timestamp AT TIME ZONE (SELECT timezone FROM core.settings))
               AND (e.details->>'played_ms')::bigint > 0
           )
           SELECT item_key AS "item_key?", count(*) AS "segments!",
                  array_agg(DISTINCT content) FILTER (WHERE content IS NOT NULL) AS contents,
                  to_char(min(occurred_at) AT TIME ZONE (SELECT timezone FROM core.settings), 'HH24:MI') AS "first_time!",
                  to_char(max(ended_at) AT TIME ZONE (SELECT timezone FROM core.settings), 'HH24:MI') AS "last_time!"
           FROM spans
           WHERE ended_at > ($1::date::timestamp AT TIME ZONE (SELECT timezone FROM core.settings))
           GROUP BY item_key"#,
        date,
        LEVEL,
        METRIC,
        MAX_MEASURED_MS,
    )
    .fetch_all(pool)
    .await?;
    let discoveries = read::day_discoveries(pool, METRIC, &[LEVEL], date).await?;

    let minutes = |seconds: f64| (seconds / 60.0).round() as i64;
    let mut unknown_seconds = 0.0;
    let mut summed_seconds = 0.0;
    let mut channels = Vec::new();
    for measured in by_channel {
        summed_seconds += measured.seconds;
        let Some(key) = measured.item_key else {
            unknown_seconds += measured.seconds;
            continue;
        };
        let Some(row) = rows
            .iter()
            .find(|r| r.item_key.as_deref() == Some(key.as_str()))
        else {
            continue;
        };
        let mut contents = row.contents.clone().unwrap_or_default();
        contents.sort_by_key(|c| ["live", "vod", "clip"].iter().position(|k| k == c));
        channels.push(Channel {
            name: measured.name.unwrap_or_else(|| key.clone()),
            key,
            minutes: minutes(measured.seconds),
            contents,
            first_time: row.first_time.clone(),
            last_time: row.last_time.clone(),
        });
    }
    channels.sort_by(|a, b| {
        b.minutes
            .cmp(&a.minutes)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.key.cmp(&b.key))
    });
    Ok(DayStream {
        date,
        minutes: minutes(total.iter().map(|d| d.seconds).sum()),
        summed_minutes: minutes(summed_seconds),
        segments: rows.iter().map(|r| r.segments).sum(),
        channels,
        unknown_minutes: minutes(unknown_seconds),
        discoveries,
    })
}
