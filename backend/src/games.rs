//! La carte Steam de la page du jour : le temps joué, par jeu, avec le total d'heures de chaque jeu à
//! la fin du jour, les succès débloqués ce jour-là et les jeux joués ce jour-là pour la première fois. Une lecture de la web app seulement, derrière la
//! session : le MCP lit les mêmes faits par les lectures génériques (durée cumulée, classement,
//! occurrences).
//!
//! Le temps est celui de la forme `duration`, par la partie commune de la carte Twitch
//! (`stream::measured_day`) : l'union des tranches de `game.play`, coupée aux minuits de Paris. Le
//! total d'un jeu est le dernier `game.total_playtime` vivant avant le minuit qui finit le jour. Les
//! découvertes sont celles de `read::day_discoveries`, sans les jeux déclarés « déjà connu »
//! (`game.known`, donné par l'interprète quand le premier relevé a déjà du temps de jeu).
//!
//! League of Legends, Teamfight Tactics et VALORANT (sources `riot` et `valorant`) y entrent comme des
//! jeux sous leurs clés `riot:lol`, `riot:tft` et `riot:valorant`, avec leur temps du jour (sans total
//! d'heures ni succès), et y ajoutent leurs parties (`game.match`), rangées au jour de leur début.

use chrono::NaiveDate;
use serde::Serialize;
use sqlx::PgPool;

use crate::read::{self, DayDiscovery};
use crate::stream::measured_day;

const METRIC: &str = "game.play";
const TOTAL: &str = "game.total_playtime";
const ACHIEVEMENT: &str = "game.achievement";
const MATCH: &str = "game.match";
const LEVEL: &str = "game";

#[derive(Serialize)]
pub struct Achievement {
    /// `<appid>:<apiname>`.
    key: String,
    name: String,
    /// Heure de Paris, `HH:MM`, du déblocage.
    time: String,
}

#[derive(Serialize)]
pub struct Game {
    /// L'appid, en texte.
    key: String,
    name: String,
    /// Le temps joué ce jour-là ; 0 pour un jeu qui n'a que des succès ce jour-là.
    minutes: i64,
    /// Heure de Paris, `HH:MM`, du début de la première tranche, qui peut être la veille ; `null` sans
    /// temps joué ce jour-là.
    first_time: Option<String>,
    /// Heure de Paris, `HH:MM`, de la fin de la dernière tranche ; `null` sans temps joué.
    last_time: Option<String>,
    /// Le total d'heures du jeu à la fin du jour, en minutes : le dernier relevé d'ici minuit. `null`
    /// sans relevé jusque-là.
    total_minutes: Option<i64>,
    /// Les succès débloqués ce jour-là, par heure.
    achievements: Vec<Achievement>,
}

/// Une partie de League of Legends, Teamfight Tactics ou VALORANT, commencée ce jour-là.
#[derive(Serialize)]
pub struct Match {
    /// `<jeu>:<identifiant de la partie>`, unique.
    key: String,
    /// Heure de Paris, `HH:MM`, du début.
    time: String,
    /// La clé de l'élément `game` (`riot:lol`, `riot:tft`, `riot:valorant`).
    game: String,
    game_name: String,
    /// Le nom de la file (« Classée solo/duo », « Double Up », « Competitive »…), `null` s'il manque.
    queue: Option<String>,
    /// `win`, `loss`, `remake`, `draw` ; `null` en TFT (la place en tient lieu) ou sans équipe.
    result: Option<String>,
    /// Le champion (LoL) ou l'agent (VALORANT) ; `null` en TFT.
    character: Option<String>,
    /// Le poste en LoL (`TOP`, `JUNGLE`, `MIDDLE`, `BOTTOM`, `UTILITY`), sinon `null`.
    position: Option<String>,
    /// La carte en VALORANT.
    map: Option<String>,
    /// La place en TFT, de 1 à 8.
    placement: Option<i64>,
    rounds_won: Option<i64>,
    rounds_lost: Option<i64>,
    /// Le temps joué, en minutes arrondies (en TFT, jusqu'à mon élimination).
    minutes: i64,
}

#[derive(Serialize)]
pub struct DayGames {
    date: NaiveDate,
    /// L'union de toutes les tranches : le temps joué ce jour-là.
    minutes: i64,
    /// Les tranches qui touchent ce jour-là, commencées la veille comprises.
    segments: i64,
    /// Les jeux joués, par temps décroissant puis par nom, puis ceux qui n'ont que des succès, par nom.
    games: Vec<Game>,
    /// Le nombre de succès débloqués ce jour-là.
    achievements: i64,
    /// Les jeux joués ce jour-là pour la première fois, par première heure : `{level: "game", key,
    /// name}`. Ni ceux qui avaient déjà du temps au premier relevé, ni un jeu qui n'a qu'un succès.
    discoveries: Vec<DayDiscovery>,
    /// Les parties commencées ce jour-là (LoL, TFT, VALORANT), par heure de début. Une partie à cheval
    /// sur minuit est ici au jour de son début ; son temps se partage entre les deux jours (`minutes`).
    matches: Vec<Match>,
}

/// Le fuseau en sous-requête et la génération fixée, comme toutes les lectures (voir `api::get_day`).
pub async fn day(pool: &PgPool, date: NaiveDate) -> Result<DayGames, sqlx::Error> {
    let measured = measured_day(pool, METRIC, LEVEL, date).await?;
    let discoveries = read::day_discoveries(pool, METRIC, &[LEVEL], date).await?;
    let unlocked = sqlx::query!(
        r#"SELECT g.item_key AS "game_key!", g.display AS "game_name!",
                  a.item_key AS "key!", a.display AS "name!",
                  to_char(e.occurred_at AT TIME ZONE (SELECT timezone FROM core.settings), 'HH24:MI') AS "time!"
           FROM derived.live_events e
           JOIN derived.event_items g
             ON g.generation = e.generation AND g.seq = e.seq AND g.ordinal = e.ordinal AND g.level = $2
           JOIN derived.event_items a
             ON a.generation = e.generation AND a.seq = e.seq AND a.ordinal = e.ordinal
            AND a.level = 'achievement'
           WHERE e.generation = (SELECT id FROM derived.generations WHERE state = 'active')
             AND e.metric = $3
             AND e.occurred_at >= ($1::date::timestamp AT TIME ZONE (SELECT timezone FROM core.settings))
             AND e.occurred_at < (($1::date + 1)::timestamp AT TIME ZONE (SELECT timezone FROM core.settings))
           ORDER BY e.occurred_at, a.item_key"#,
        date,
        LEVEL,
        ACHIEVEMENT,
    )
    .fetch_all(pool)
    .await?;

    let mut games: Vec<Game> = measured
        .items
        .into_iter()
        .map(|item| Game {
            key: item.key,
            name: item.name,
            minutes: item.minutes,
            first_time: Some(item.first_time),
            last_time: Some(item.last_time),
            total_minutes: None,
            achievements: Vec::new(),
        })
        .collect();
    let played = games.len();
    for row in &unlocked {
        let achievement = Achievement {
            key: row.key.clone(),
            name: row.name.clone(),
            time: row.time.clone(),
        };
        match games.iter_mut().find(|g| g.key == row.game_key) {
            Some(game) => game.achievements.push(achievement),
            None => games.push(Game {
                key: row.game_key.clone(),
                name: row.game_name.clone(),
                minutes: 0,
                first_time: None,
                last_time: None,
                total_minutes: None,
                achievements: vec![achievement],
            }),
        }
    }
    games[played..].sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.key.cmp(&b.key))
    });

    let keys: Vec<String> = games.iter().map(|g| g.key.clone()).collect();
    let totals = sqlx::query!(
        r#"SELECT DISTINCT ON (i.item_key) i.item_key AS "key!", e.value::bigint AS total
           FROM derived.live_events e
           JOIN derived.event_items i
             ON i.generation = e.generation AND i.seq = e.seq AND i.ordinal = e.ordinal AND i.level = $2
           WHERE e.generation = (SELECT id FROM derived.generations WHERE state = 'active')
             AND e.metric = $3
             AND i.metric = $3 AND i.item_key = ANY($4)
             AND e.occurred_at < (($1::date + 1)::timestamp AT TIME ZONE (SELECT timezone FROM core.settings))
           ORDER BY i.item_key, e.occurred_at DESC, e.seq DESC"#,
        date,
        LEVEL,
        TOTAL,
        &keys,
    )
    .fetch_all(pool)
    .await?;
    for row in totals {
        if let Some(game) = games.iter_mut().find(|g| g.key == row.key) {
            game.total_minutes = row.total;
        }
    }

    let matches = sqlx::query!(
        r#"SELECT g.item_key AS "game!", g.display AS "game_name!", e.details AS "details!",
                  to_char(e.occurred_at AT TIME ZONE (SELECT timezone FROM core.settings), 'HH24:MI') AS "time!"
           FROM derived.live_events e
           JOIN derived.event_items g
             ON g.generation = e.generation AND g.seq = e.seq AND g.ordinal = e.ordinal AND g.level = $2
           WHERE e.generation = (SELECT id FROM derived.generations WHERE state = 'active')
             AND e.metric = $3
             AND e.occurred_at >= ($1::date::timestamp AT TIME ZONE (SELECT timezone FROM core.settings))
             AND e.occurred_at < (($1::date + 1)::timestamp AT TIME ZONE (SELECT timezone FROM core.settings))
           ORDER BY e.occurred_at, e.seq"#,
        date,
        LEVEL,
        MATCH,
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|row| {
        let details = &row.details;
        let text = |field: &str| details.get(field).and_then(|v| v.as_str()).map(str::to_owned);
        let number = |field: &str| details.get(field).and_then(serde_json::Value::as_i64);
        Match {
            key: format!(
                "{}:{}",
                text("game").unwrap_or_default(),
                text("match_id").unwrap_or_default()
            ),
            time: row.time,
            game: row.game,
            game_name: row.game_name,
            queue: text("queue"),
            result: text("result"),
            character: text("character"),
            position: text("position"),
            map: text("map"),
            placement: number("placement"),
            rounds_won: number("rounds_won"),
            rounds_lost: number("rounds_lost"),
            minutes: crate::stream::minutes(number("played_ms").unwrap_or(0) as f64 / 1000.0),
        }
    })
    .collect();

    Ok(DayGames {
        date,
        minutes: measured.minutes,
        segments: measured.segments,
        games,
        achievements: unlocked.len() as i64,
        discoveries,
        matches,
    })
}
