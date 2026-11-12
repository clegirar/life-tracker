//! Les interprètes : un par type de donnée, versionné. Fonctions pures, sans heure courante ni
//! accès extérieur. Ajouter un type, c'est ajouter une entrée à `INTERPRETERS` (principe 12).

use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use crate::music;

/// Un enregistrement non purgé, tel que le voit un interprète.
pub struct Record {
    pub seq: i64,
    pub source_id: i16,
    pub schema_version: i16,
    pub occurred_at: DateTime<Utc>,
    pub utc_offset_minutes: Option<i16>,
    pub payload: Value,
}

/// `&'static str` : un message stocké ou journalisé ne peut pas citer la charge utile.
#[derive(Debug, PartialEq, Eq)]
pub struct InterpretError(pub &'static str);

#[derive(Debug, PartialEq)]
pub struct Item {
    pub level: &'static str,
    /// Le rang de l'élément parmi ceux de son niveau : 0, sauf pour les artistes crédités d'une
    /// écoute, où 0 désigne l'artiste principal.
    pub position: i16,
    pub item_key: String,
    pub display: String,
}

impl Item {
    /// Le seul élément de son niveau.
    fn single(level: &'static str, item_key: String, display: &str) -> Self {
        Self {
            level,
            position: 0,
            item_key,
            display: display.to_owned(),
        }
    }
}

/// Un élément `artist` par artiste crédité, dans l'ordre du crédit.
fn artist_items(credits: &[music::Credit]) -> Vec<Item> {
    credits
        .iter()
        .zip(0..=i16::MAX)
        .map(|(credit, position)| Item {
            level: "artist",
            position,
            item_key: credit.key.clone(),
            display: credit.name.clone(),
        })
        .collect()
}

#[derive(Debug, PartialEq)]
pub struct Event {
    pub metric: &'static str,
    pub occurred_at: DateTime<Utc>,
    pub utc_offset_minutes: Option<i16>,
    /// Numeric en texte : sa conversion revient à la base.
    pub value: Option<String>,
    pub details: Value,
    pub items: Vec<Item>,
}

impl Event {
    /// Un événement sans valeur ni détail, à la date de l'enveloppe.
    pub fn at_envelope(record: &Record, metric: &'static str) -> Self {
        Self {
            metric,
            occurred_at: record.occurred_at,
            utc_offset_minutes: record.utc_offset_minutes,
            value: None,
            details: json!({}),
            items: Vec::new(),
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum Output {
    Event(Event),
    /// La source est désignée par son nom ; le moteur le résout dans le registre.
    Cancellation {
        source: String,
        key: String,
    },
}

pub type Interpret = fn(&Record) -> Result<Vec<Output>, InterpretError>;

#[derive(Clone, Copy)]
pub struct Interpreter {
    pub kind: &'static str,
    pub version: i32,
    /// Le seul format connu de l'interprète ; un autre est une erreur d'interprétation.
    pub schema_version: i16,
    pub interpret: Interpret,
}

pub static INTERPRETERS: &[Interpreter] = &[
    Interpreter {
        kind: "coffee.cup",
        version: 1,
        schema_version: 1,
        interpret: coffee_cup,
    },
    Interpreter {
        kind: "record.cancel",
        version: 1,
        schema_version: 1,
        interpret: record_cancel,
    },
    Interpreter {
        kind: "lastfm.scrobble",
        version: 2,
        schema_version: 1,
        interpret: lastfm_scrobble,
    },
    Interpreter {
        kind: "lastfm.track_info",
        version: 2,
        schema_version: 1,
        interpret: lastfm_track_info,
    },
    Interpreter {
        kind: "music.mark_known",
        version: 2,
        schema_version: 1,
        interpret: music_mark_known,
    },
    Interpreter {
        kind: "apple.play_history_row",
        version: 2,
        schema_version: 1,
        interpret: apple_play_history_row,
    },
    Interpreter {
        kind: "garmin.daily_summary",
        version: 1,
        schema_version: 1,
        interpret: garmin_daily_summary,
    },
    Interpreter {
        kind: "garmin.sleep",
        version: 2,
        schema_version: 1,
        interpret: garmin_sleep,
    },
    Interpreter {
        kind: "garmin.activity",
        version: 1,
        schema_version: 1,
        interpret: garmin_activity,
    },
    Interpreter {
        kind: "garmin.activity_track",
        version: 1,
        schema_version: 1,
        interpret: garmin_activity_track,
    },
    Interpreter {
        kind: "youtube.watch",
        version: 1,
        schema_version: 1,
        interpret: youtube_watch,
    },
    Interpreter {
        kind: "youtube.video_info",
        version: 1,
        schema_version: 1,
        interpret: youtube_video_info,
    },
    Interpreter {
        kind: "twitch.watch",
        version: 1,
        schema_version: 1,
        interpret: twitch_watch,
    },
    Interpreter {
        kind: "steam.playtime",
        version: 1,
        schema_version: 1,
        interpret: steam_playtime,
    },
    Interpreter {
        kind: "steam.achievement",
        version: 1,
        schema_version: 1,
        interpret: steam_achievement,
    },
    Interpreter {
        kind: "github.commit",
        version: 1,
        schema_version: 1,
        interpret: github_commit,
    },
    Interpreter {
        kind: "github.pull_request",
        version: 1,
        schema_version: 1,
        interpret: github_pull_request,
    },
    Interpreter {
        kind: "github.review",
        version: 1,
        schema_version: 1,
        interpret: github_review,
    },
    Interpreter {
        kind: "github.issue",
        version: 1,
        schema_version: 1,
        interpret: github_issue,
    },
    Interpreter {
        kind: "github.comment",
        version: 1,
        schema_version: 1,
        interpret: github_comment,
    },
    Interpreter {
        kind: "overland.location",
        version: 1,
        schema_version: 1,
        interpret: overland_location,
    },
    Interpreter {
        kind: "overland.visit",
        version: 1,
        schema_version: 1,
        interpret: overland_visit,
    },
    Interpreter {
        kind: "riot.lol_match",
        version: 1,
        schema_version: 1,
        interpret: riot_lol_match,
    },
    Interpreter {
        kind: "riot.tft_match",
        version: 1,
        schema_version: 1,
        interpret: riot_tft_match,
    },
    Interpreter {
        kind: "valorant.match",
        version: 1,
        schema_version: 1,
        interpret: valorant_match,
    },
];

/// Un café : un événement à l'heure de l'enveloppe. La charge utile, attendue vide, est ignorée.
fn coffee_cup(record: &Record) -> Result<Vec<Output>, InterpretError> {
    Ok(vec![Output::Event(Event::at_envelope(record, "coffee"))])
}

/// `{"source": "<nom dans core.sources>", "key": "<clé de la cible>"}`.
fn record_cancel(record: &Record) -> Result<Vec<Output>, InterpretError> {
    let text = |field| record.payload.get(field).and_then(Value::as_str);
    let source = text("source")
        .filter(|s| !s.is_empty())
        .ok_or(InterpretError("record.cancel: invalid source"))?;
    // Mêmes bornes que la clé de core.records.
    let key = text("key")
        .filter(|k| (1..=512).contains(&k.chars().count()))
        .ok_or(InterpretError("record.cancel: invalid key"))?;
    Ok(vec![Output::Cancellation {
        source: source.to_owned(),
        key: key.to_owned(),
    }])
}

/// Un nom non vide : une chaîne, ou un objet Last.fm qui le porte dans `#text` (ou `name` selon la
/// requête). Le nom gardé est tel que la source l'envoie, à part les blancs aux extrémités.
pub(crate) fn lastfm_name(value: Option<&Value>) -> Option<&str> {
    let text = match value? {
        Value::Object(object) => object
            .get("#text")
            .or_else(|| object.get("name"))?
            .as_str()?,
        other => other.as_str()?,
    };
    Some(text.trim()).filter(|t| !t.is_empty())
}

/// Le scrobble brut de `user.getRecentTracks`. La date est celle de l'enveloppe. Le morceau en cours
/// n'a pas de date et ne devrait pas arriver ici ; s'il arrive, ce n'est pas une écoute.
fn lastfm_scrobble(record: &Record) -> Result<Vec<Output>, InterpretError> {
    let scrobble = &record.payload;
    if scrobble
        .pointer("/@attr/nowplaying")
        .and_then(Value::as_str)
        == Some("true")
    {
        return Ok(Vec::new());
    }
    let artist = lastfm_name(scrobble.get("artist"))
        .ok_or(InterpretError("lastfm.scrobble: missing artist"))?;
    let track =
        lastfm_name(scrobble.get("name")).ok_or(InterpretError("lastfm.scrobble: missing name"))?;
    let album = lastfm_name(scrobble.get("album"));

    Ok(vec![Output::Event(attributed_listen(
        record, artist, album, track, None,
    ))])
}

/// Une écoute dont l'artiste est connu, avec ses éléments, sous les mêmes clés quelle que soit la source.
/// Un élément `artist` par artiste crédité (`music::credited_artists`) ; l'album et le titre prennent
/// la clé de l'artiste principal. `played_ms` n'apparaît que si la source le donne.
fn attributed_listen(
    record: &Record,
    artist: &str,
    album: Option<&str>,
    track: &str,
    played_ms: Option<i64>,
) -> Event {
    let credits = music::credited_artists(artist, track);
    let artist_key = music::artist_key(artist);
    let track_key = music::track_key(artist, track);
    let album_key = album.map(|album| music::album_key(artist, album));
    let mut items = artist_items(&credits);
    if let (Some(album), Some(key)) = (album, &album_key) {
        items.push(Item::single("album", key.clone(), album));
    }
    items.push(Item::single("track", track_key.clone(), track));
    let artists: Vec<Value> = credits
        .iter()
        .map(|credit| json!({ "key": credit.key, "name": credit.name }))
        .collect();
    let mut details = json!({
        "artist": artist,
        "artists": artists,
        "album": album,
        "track": track,
        "artist_key": artist_key,
        "album_key": album_key,
        "track_key": track_key,
    });
    if let Some(ms) = played_ms {
        details["played_ms"] = ms.into();
    }
    Event {
        details,
        items,
        ..Event::at_envelope(record, "music.listen")
    }
}

/// Un titre plus court n'est jamais scrobblé par Last.fm, dont les règles servent de référence.
const MIN_TRACK_MS: i64 = 30_000;
/// La moitié du titre, ou 4 minutes s'il est très long.
const MAX_LISTEN_THRESHOLD_MS: i64 = 240_000;

/// Une écoute au sens de Last.fm : un titre de plus de 30 s, joué jusqu'à la moitié ou jusqu'à 4 min.
/// La lecture franchit ce point de la piste une fois, quel que soit le nombre de morceaux : après une
/// pause ou un saut, une seule ligne compte, sans rien savoir des autres. Un saut par-dessus ce point
/// n'est pas une écoute ; un retour en arrière qui le refranchit en est une seconde (voir l'architecture).
fn crosses_listen_threshold(start_ms: i64, end_ms: i64, media_ms: i64) -> bool {
    let threshold = (media_ms / 2).min(MAX_LISTEN_THRESHOLD_MS);
    media_ms > MIN_TRACK_MS && start_ms < threshold && threshold <= end_ms
}

/// Une ligne `PLAY_END` de Play Activity, réduite par l'outil d'import à sa liste blanche (`apple.rs`).
/// Ligne sans artiste attribuable : l'écoute existe, mais aucun élément n'est inventé, donc elle ne
/// compte ni dans les artistes ou albums, ni dans les découvertes.
fn apple_play_history_row(record: &Record) -> Result<Vec<Output>, InterpretError> {
    let row = &record.payload;
    let number = |field| row.get(field).and_then(Value::as_i64);
    let text = |field| {
        row.get(field)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|t| !t.is_empty())
    };
    let track = text("song").ok_or(InterpretError("apple.play_history_row: missing song"))?;
    let played = number("play_duration_ms").ok_or(InterpretError(
        "apple.play_history_row: missing play_duration_ms",
    ))?;
    let (Some(start), Some(end), Some(media)) = (
        number("start_position_ms"),
        number("end_position_ms"),
        number("media_duration_ms"),
    ) else {
        return Ok(Vec::new());
    };
    if played <= 0 || !crosses_listen_threshold(start, end, media) {
        return Ok(Vec::new());
    }
    // Une ligne peut annoncer des heures pour un titre de trois minutes : jamais plus que le titre.
    let played = played.min(media);
    let album = text("album");
    let event = match text("artist") {
        Some(artist) => attributed_listen(record, artist, album, track, Some(played)),
        None => Event {
            details: json!({
                "artist": null,
                "artists": null,
                "album": album,
                "track": track,
                "artist_key": null,
                "album_key": null,
                "track_key": null,
                "played_ms": played,
            }),
            ..Event::at_envelope(record, "music.listen")
        },
    };
    Ok(vec![Output::Event(event)])
}

/// Une durée Last.fm en millisecondes, en texte ou en nombre. Zéro ou illisible : pas de durée.
fn lastfm_duration_ms(value: Option<&Value>) -> Option<u64> {
    let ms = match value? {
        Value::String(text) => text.trim().parse().ok()?,
        other => other.as_u64()?,
    };
    (ms > 0).then_some(ms)
}

/// `{"artist", "track", "info"}` : le titre tel que scrobblé, et la réponse brute de `track.getInfo`.
/// La clé vient du scrobble et non de la réponse, que Last.fm peut avoir corrigée : c'est elle qui
/// relie la durée aux écoutes. Un titre sans durée connue ne produit rien.
fn lastfm_track_info(record: &Record) -> Result<Vec<Output>, InterpretError> {
    let artist = lastfm_name(record.payload.get("artist"))
        .ok_or(InterpretError("lastfm.track_info: missing artist"))?;
    let track = lastfm_name(record.payload.get("track"))
        .ok_or(InterpretError("lastfm.track_info: missing track"))?;
    let info = record
        .payload
        .pointer("/info/track")
        .ok_or(InterpretError("lastfm.track_info: missing info"))?;
    let Some(ms) = lastfm_duration_ms(info.get("duration")) else {
        return Ok(Vec::new());
    };
    let track_key = music::track_key(artist, track);
    Ok(vec![Output::Event(Event {
        value: Some(format!("{}.{:03}", ms / 1000, ms % 1000)),
        details: json!({ "track_key": track_key }),
        items: vec![Item::single("track", track_key, track)],
        ..Event::at_envelope(record, "music.track_duration")
    })])
}

/// Assez pour tout vrai nom, et assez court pour tenir dans l'index des éléments : une clé trop longue
/// serait une panne pour la base, pas un refus, et bloquerait le worker.
const MAX_MARKED_NAME_BYTES: usize = 1024;

/// `{"level", "artist", "album", "track"}` : la déclaration « déjà connu », avec les noms tels
/// qu'affichés. Des éléments à son seul niveau : c'est ce qui fait qu'un marquage n'agit qu'à son niveau.
/// Un artiste « A & B » marque A et B ; un album ou un titre prend la clé de l'artiste principal.
/// Les clés se recalculent comme celles des écoutes, donc suivent la normalisation à chaque rebuild.
fn music_mark_known(record: &Record) -> Result<Vec<Output>, InterpretError> {
    let name = |field: &str| {
        let text = record.payload.get(field)?.as_str()?.trim();
        Some(text).filter(|t| !t.is_empty() && t.len() <= MAX_MARKED_NAME_BYTES)
    };
    let artist = name("artist").ok_or(InterpretError("music.mark_known: invalid artist"))?;
    let (album, track) = (name("album"), name("track"));
    let level = record.payload.get("level").and_then(Value::as_str);
    let (level, items) = match level {
        Some("artist") => ("artist", artist_items(&music::credited_artists(artist, ""))),
        Some("album") => {
            let album = album.ok_or(InterpretError("music.mark_known: invalid album"))?;
            let key = music::album_key(artist, album);
            ("album", vec![Item::single("album", key, album)])
        }
        Some("track") => {
            let track = track.ok_or(InterpretError("music.mark_known: invalid track"))?;
            let key = music::track_key(artist, track);
            ("track", vec![Item::single("track", key, track)])
        }
        _ => return Err(InterpretError("music.mark_known: invalid level")),
    };
    Ok(vec![Output::Event(Event {
        details: json!({ "level": level, "artist": artist, "album": album, "track": track }),
        items,
        ..Event::at_envelope(record, "music.known")
    })])
}

// ─── Garmin ──────────────────────────────────────────────────────────────────
//
// Les charges utiles sont déjà réduites à leur liste blanche par le connecteur (`collector::garmin`).
// Une révision de Garmin arrive comme un nouvel enregistrement, accompagné de l'annulation du
// précédent : l'interprète ne voit jamais qu'une version à la fois, et n'en sait rien.

/// Un nombre fini et positif de la charge utile, tel qu'envoyé ; `null` ou absent : `None`.
fn garmin_number<'a>(
    payload: &'a Value,
    field: &str,
) -> Result<Option<&'a serde_json::Number>, InterpretError> {
    match payload.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) if n.as_f64().is_some_and(|f| f.is_finite() && f >= 0.0) => {
            Ok(Some(n))
        }
        Some(_) => Err(InterpretError("garmin: invalid number")),
    }
}

/// Un nombre en texte pour la colonne numeric : sa conversion revient à la base.
fn numeric_text(n: &serde_json::Number) -> String {
    n.to_string()
}

/// Une valeur divisée et arrondie à trois décimales, en texte : mètres en km, secondes en minutes.
fn scaled(n: &serde_json::Number, divisor: f64) -> String {
    format!("{:.3}", n.as_f64().unwrap_or(0.0) / divisor)
}

fn garmin_calendar_date(payload: &Value) -> Result<&str, InterpretError> {
    payload
        .get("calendar_date")
        .and_then(Value::as_str)
        .filter(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").is_ok() && d.len() == 10)
        .ok_or(InterpretError("garmin: invalid calendar_date"))
}

/// `{calendar_date, steps, distance_m, calories_total, calories_active, calories_resting}` : les pas et
/// les calories du jour, à midi heure de Paris (l'enveloppe), deux mesures du même jour. Des calories
/// inconnues ne font pas d'événement : ce n'est pas zéro.
fn garmin_daily_summary(record: &Record) -> Result<Vec<Output>, InterpretError> {
    let payload = &record.payload;
    let date = garmin_calendar_date(payload)?;
    let steps = garmin_number(payload, "steps")?
        .ok_or(InterpretError("garmin.daily_summary: missing steps"))?;
    let mut outputs = vec![Output::Event(Event {
        value: Some(numeric_text(steps)),
        details: json!({ "calendar_date": date, "distance_m": garmin_number(payload, "distance_m")? }),
        ..Event::at_envelope(record, "steps")
    })];
    if let Some(total) = garmin_number(payload, "calories_total")? {
        outputs.push(Output::Event(Event {
            value: Some(numeric_text(total)),
            details: json!({
                "calendar_date": date,
                "active": garmin_number(payload, "calories_active")?,
                "resting": garmin_number(payload, "calories_resting")?,
            }),
            ..Event::at_envelope(record, "calories.burned")
        }));
    }
    Ok(outputs)
}

/// `heart_rate` d'une nuit, `[[secondes depuis le début, bpm], …]` : la plus basse et la moyenne
/// arrondie, `None` sans échantillon (une nuit d'avant la fréquence cardiaque n'en a pas).
fn sleep_heart_rate(payload: &Value) -> Result<Option<(&serde_json::Number, i64)>, InterpretError> {
    const INVALID: InterpretError = InterpretError("garmin.sleep: invalid heart_rate");
    let samples = match payload.get("heart_rate") {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Array(samples)) => samples,
        Some(_) => return Err(INVALID),
    };
    let mut lowest: Option<&serde_json::Number> = None;
    let mut sum = 0.0;
    for sample in samples {
        let [seconds, Value::Number(bpm)] = sample.as_array().map(Vec::as_slice).ok_or(INVALID)?
        else {
            return Err(INVALID);
        };
        let bpm_value = bpm
            .as_f64()
            .filter(|b| b.is_finite() && *b > 0.0)
            .ok_or(INVALID)?;
        seconds
            .as_f64()
            .filter(|s| s.is_finite() && *s >= 0.0)
            .ok_or(INVALID)?;
        if lowest.is_none_or(|l| bpm_value < l.as_f64().unwrap_or(f64::INFINITY)) {
            lowest = Some(bpm);
        }
        sum += bpm_value;
    }
    Ok(lowest.map(|l| (l, (sum / samples.len() as f64).round() as i64)))
}

/// `{calendar_date, start, end, sleep_seconds, deep_seconds, light_seconds, rem_seconds,
/// awake_seconds, heart_rate?}` : une nuit, rangée au jour du réveil (celui de Garmin), en minutes.
/// Avec des échantillons de fréquence cardiaque, la plus basse (`lowest_hr`) et la moyenne (`avg_hr`) ;
/// la série elle-même reste au journal.
fn garmin_sleep(record: &Record) -> Result<Vec<Output>, InterpretError> {
    let payload = &record.payload;
    let date = garmin_calendar_date(payload)?;
    let seconds = garmin_number(payload, "sleep_seconds")?
        .filter(|s| s.as_f64().is_some_and(|s| s > 0.0))
        .ok_or(InterpretError("garmin.sleep: missing sleep_seconds"))?;
    let instant = |field| -> Result<Option<String>, InterpretError> {
        match payload.get(field) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(text)) => DateTime::parse_from_rfc3339(text)
                .map(|at| Some(at.with_timezone(&Utc).to_rfc3339()))
                .map_err(|_| InterpretError("garmin.sleep: invalid instant")),
            Some(_) => Err(InterpretError("garmin.sleep: invalid instant")),
        }
    };
    let mut details = json!({
        "calendar_date": date,
        "start": instant("start")?,
        "end": instant("end")?,
        "sleep_seconds": seconds,
        "deep_seconds": garmin_number(payload, "deep_seconds")?,
        "light_seconds": garmin_number(payload, "light_seconds")?,
        "rem_seconds": garmin_number(payload, "rem_seconds")?,
        "awake_seconds": garmin_number(payload, "awake_seconds")?,
    });
    if let Some((lowest, average)) = sleep_heart_rate(payload)? {
        details["lowest_hr"] = json!(lowest);
        details["avg_hr"] = json!(average);
    }
    Ok(vec![Output::Event(Event {
        value: Some(scaled(seconds, 60.0)),
        details,
        ..Event::at_envelope(record, "sleep")
    })])
}

/// Assez pour tout type de Garmin (`running`, `trail_running`…), et court pour l'index des éléments.
const MAX_ACTIVITY_TYPE_BYTES: usize = 64;

/// La clé d'un lieu : minuscules, blancs réduits, forme NFC. « Paris » et « paris » sont un même lieu.
fn place_key(name: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    name.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .nfc()
        .collect::<String>()
        .to_lowercase()
}

/// Assez pour tout nom de ville, et court pour l'index des éléments.
const MAX_PLACE_CHARS: usize = 200;

/// Une coordonnée dans ses bornes, ou rien ; autre chose est l'erreur `error`.
fn coordinate(
    payload: &Value,
    field: &str,
    bound: f64,
    error: &'static str,
) -> Result<Value, InterpretError> {
    match payload.get(field) {
        None | Some(Value::Null) => Ok(Value::Null),
        Some(Value::Number(n)) if n.as_f64().is_some_and(|c| c.abs() <= bound) => {
            Ok(Value::Number(n.clone()))
        }
        Some(_) => Err(InterpretError(error)),
    }
}

fn garmin_coordinate(payload: &Value, field: &str, bound: f64) -> Result<Value, InterpretError> {
    coordinate(payload, field, bound, "garmin: invalid coordinate")
}

/// `{activity_id, type, start, duration_s, moving_s, distance_m, avg_speed_mps, avg_hr, max_hr,
/// calories, start_lat, start_lon, end_lat, end_lon, location_name}` : une séance, à son début
/// (l'enveloppe). Tous les types : la course se filtre à l'affichage. Valeur : la distance en km, ou
/// rien pour une séance sans distance. Éléments : son type et son lieu, s'il est connu. Les
/// coordonnées restent dans les détails, que seules les routes de session lisent.
fn garmin_activity(record: &Record) -> Result<Vec<Output>, InterpretError> {
    let payload = &record.payload;
    let id = garmin_number(payload, "activity_id")?
        .and_then(serde_json::Number::as_u64)
        .ok_or(InterpretError("garmin.activity: missing activity_id"))?;
    let kind = payload
        .get("type")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty() && t.len() <= MAX_ACTIVITY_TYPE_BYTES)
        .ok_or(InterpretError("garmin.activity: invalid type"))?;
    let distance = garmin_number(payload, "distance_m")?;
    let mut details = json!({ "activity_id": id, "type": kind });
    for field in [
        "duration_s",
        "moving_s",
        "distance_m",
        "avg_speed_mps",
        "avg_hr",
        "max_hr",
        "calories",
    ] {
        details[field] = garmin_number(payload, field)?
            .cloned()
            .map_or(Value::Null, Value::Number);
    }
    details["start_lat"] = garmin_coordinate(payload, "start_lat", 90.0)?;
    details["start_lon"] = garmin_coordinate(payload, "start_lon", 180.0)?;
    details["end_lat"] = garmin_coordinate(payload, "end_lat", 90.0)?;
    details["end_lon"] = garmin_coordinate(payload, "end_lon", 180.0)?;
    let place = match payload.get("location_name") {
        None | Some(Value::Null) => None,
        Some(Value::String(name)) => {
            Some(name.trim()).filter(|n| !n.is_empty() && n.chars().count() <= MAX_PLACE_CHARS)
        }
        Some(_) => return Err(InterpretError("garmin.activity: invalid location_name")),
    };
    details["location_name"] = place.map_or(Value::Null, |p| Value::String(p.to_owned()));
    let mut items = vec![Item::single("activity_type", kind.to_owned(), kind)];
    if let Some(place) = place {
        items.push(Item::single("place", place_key(place), place));
    }
    Ok(vec![Output::Event(Event {
        value: distance
            .filter(|d| d.as_f64().is_some_and(|d| d > 0.0))
            .map(|d| scaled(d, 1000.0)),
        details,
        items,
        ..Event::at_envelope(record, "workout")
    })])
}

/// Une trace de plusieurs heures reste loin de ce nombre une fois simplifiée par le connecteur.
const MAX_TRACK_POINTS: usize = 5000;

/// `{activity_id, points: [[lat, lon, secondes depuis le début], …]}` : la référence `workout.track`,
/// rattachée à sa séance par son identifiant, à la date de la séance.
fn garmin_activity_track(record: &Record) -> Result<Vec<Output>, InterpretError> {
    let payload = &record.payload;
    let id = garmin_number(payload, "activity_id")?
        .and_then(serde_json::Number::as_u64)
        .ok_or(InterpretError("garmin.activity_track: missing activity_id"))?;
    let points = payload
        .get("points")
        .and_then(Value::as_array)
        .filter(|p| (2..=MAX_TRACK_POINTS).contains(&p.len()))
        .ok_or(InterpretError("garmin.activity_track: invalid points"))?;
    let valid = |point: &Value| {
        let Some([lat, lon, seconds]) = point.as_array().map(Vec::as_slice) else {
            return false;
        };
        let number = |v: &Value, bound: f64| {
            v.as_f64()
                .is_some_and(|f| f.is_finite() && f.abs() <= bound)
        };
        number(lat, 90.0) && number(lon, 180.0) && seconds.as_f64().is_some_and(|s| s >= 0.0)
    };
    if !points.iter().all(valid) {
        return Err(InterpretError("garmin.activity_track: invalid points"));
    }
    Ok(vec![Output::Event(Event {
        details: json!({ "activity_id": id, "points": points }),
        ..Event::at_envelope(record, "workout.track")
    })])
}

// ─── YouTube ─────────────────────────────────────────────────────────────────
//
// Les charges utiles sont déjà réduites à leur liste blanche par le connecteur (`crate::youtube`).

/// Un id YouTube : lettres, chiffres, `-` et `_`, dans ses bornes.
fn youtube_id<'a>(
    payload: &'a Value,
    field: &str,
    lengths: std::ops::RangeInclusive<usize>,
) -> Option<&'a str> {
    payload.get(field)?.as_str().filter(|id| {
        lengths.contains(&id.len())
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    })
}

/// Un nom facultatif : absent, `null` ou vide, rien ; autre chose qu'un texte, une erreur.
fn youtube_name<'a>(payload: &'a Value, field: &str) -> Result<Option<&'a str>, InterpretError> {
    match payload.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => {
            Ok(Some(text.trim()).filter(|t| !t.is_empty() && t.len() <= MAX_MARKED_NAME_BYTES))
        }
        Some(_) => Err(InterpretError("youtube: invalid name")),
    }
}

/// `{video_id, title, channel_id, channel_name}` : une vidéo vue, à l'heure de l'enveloppe. Éléments :
/// la vidéo, sous son id, et sa chaîne si elle est connue (une vidéo supprimée n'en a plus). Le nom
/// affiché d'une vidéo sans titre est son id.
fn youtube_watch(record: &Record) -> Result<Vec<Output>, InterpretError> {
    let payload = &record.payload;
    let video = youtube_id(payload, "video_id", 11..=11)
        .ok_or(InterpretError("youtube.watch: invalid video_id"))?;
    let title = youtube_name(payload, "title")?;
    let channel = match payload.get("channel_id") {
        None | Some(Value::Null) => None,
        Some(_) => Some(
            youtube_id(payload, "channel_id", 1..=64)
                .ok_or(InterpretError("youtube.watch: invalid channel_id"))?,
        ),
    };
    let channel_name = youtube_name(payload, "channel_name")?.filter(|_| channel.is_some());
    let mut items = Vec::new();
    if let Some(channel) = channel {
        items.push(Item::single(
            "channel",
            channel.to_owned(),
            channel_name.unwrap_or(channel),
        ));
    }
    items.push(Item::single(
        "video",
        video.to_owned(),
        title.unwrap_or(video),
    ));
    Ok(vec![Output::Event(Event {
        details: json!({
            "video_id": video,
            "title": title,
            "channel_id": channel,
            "channel_name": channel_name,
        }),
        items,
        ..Event::at_envelope(record, "video.watch")
    })])
}

/// `{video_id, duration_s, title, channel_id, channel_name}` : la fiche d'une vidéo. Avec une durée
/// connue et positive, la référence `video.duration` (en secondes) de son élément `video` ; sans, rien.
fn youtube_video_info(record: &Record) -> Result<Vec<Output>, InterpretError> {
    let payload = &record.payload;
    let video = youtube_id(payload, "video_id", 11..=11)
        .ok_or(InterpretError("youtube.video_info: invalid video_id"))?;
    let seconds = match payload.get("duration_s") {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(value) => value
            .as_u64()
            .ok_or(InterpretError("youtube.video_info: invalid duration_s"))?,
    };
    if seconds == 0 {
        return Ok(Vec::new());
    }
    let title = youtube_name(payload, "title")?;
    Ok(vec![Output::Event(Event {
        value: Some(seconds.to_string()),
        details: json!({ "video_id": video, "title": title }),
        items: vec![Item::single(
            "video",
            video.to_owned(),
            title.unwrap_or(video),
        )],
        ..Event::at_envelope(record, "video.duration")
    })])
}

// ─── Twitch ──────────────────────────────────────────────────────────────────
//
// Un segment de lecture mesuré par l'extension Firefox : le temps de l'horloge pendant que la vidéo
// avance, coupé à la pause, au saut, au changement de chaîne, à la page masquée, ou au bout de 60 s.

/// La durée maximale d'un événement de forme `duration` : 1 h. Au-delà, l'événement est refusé ; les
/// lectures de cette forme cherchent donc les intervalles commencés au plus 1 h avant leur période.
pub const MAX_MEASURED_MS: i64 = 3_600_000;

/// Les noms de Twitch (affiché, catégorie) : 25 caractères et un nom de jeu, largement.
const MAX_TWITCH_NAME_BYTES: usize = 200;

/// Un login Twitch : minuscules, chiffres et `_`, 25 caractères au plus.
fn twitch_login(value: &str) -> bool {
    (1..=25).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// Un nom facultatif : absent, `null` ou vide, rien ; autre chose qu'un texte, ou trop long, une erreur.
fn twitch_name<'a>(
    payload: &'a Value,
    field: &str,
    error: &'static str,
) -> Result<Option<&'a str>, InterpretError> {
    match payload.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) if text.trim().len() <= MAX_TWITCH_NAME_BYTES => {
            Ok(Some(text.trim()).filter(|t| !t.is_empty()))
        }
        Some(_) => Err(InterpretError(error)),
    }
}

/// `{channel, channel_name, content, duration_ms, category}` : un segment de lecture, à l'heure de son
/// début. La durée jouée devient `details.played_ms`, comme pour une écoute d'Apple ; c'est elle que
/// lisent les minutes de la forme `duration`. Élément : la chaîne sous son login, avec son nom affiché
/// (le login s'il manque) ; une chaîne inconnue (`null`) n'a pas d'élément, son temps compte quand même.
fn twitch_watch(record: &Record) -> Result<Vec<Output>, InterpretError> {
    let payload = &record.payload;
    let channel = match payload.get("channel") {
        None | Some(Value::Null) => None,
        Some(Value::String(login)) if twitch_login(login) => Some(login.as_str()),
        Some(_) => return Err(InterpretError("twitch.watch: invalid channel")),
    };
    let channel_name = twitch_name(
        payload,
        "channel_name",
        "twitch.watch: invalid channel_name",
    )?
    .filter(|_| channel.is_some());
    let content = payload
        .get("content")
        .and_then(Value::as_str)
        .filter(|c| matches!(*c, "live" | "vod" | "clip"))
        .ok_or(InterpretError("twitch.watch: invalid content"))?;
    let played = payload
        .get("duration_ms")
        .and_then(Value::as_i64)
        .filter(|ms| (1..=MAX_MEASURED_MS).contains(ms))
        .ok_or(InterpretError("twitch.watch: invalid duration_ms"))?;
    let category = twitch_name(payload, "category", "twitch.watch: invalid category")?;
    let items = channel
        .map(|login| Item::single("channel", login.to_owned(), channel_name.unwrap_or(login)))
        .into_iter()
        .collect();
    Ok(vec![Output::Event(Event {
        details: json!({
            "channel": channel,
            "channel_name": channel_name,
            "content": content,
            "category": category,
            "played_ms": played,
        }),
        items,
        ..Event::at_envelope(record, "stream.watch")
    })])
}

// ─── Steam ───────────────────────────────────────────────────────────────────
//
// Steam ne donne que des compteurs cumulés. Chaque relevé porte le relevé précédent du même jeu, que le
// connecteur garde dans son état : l'interprète tire le temps joué de l'écart, sans mémoire. Les deux
// valeurs brutes restent au journal, et un rejeu peut toujours les placer autrement.

/// Le nom d'un jeu ou d'un succès : largement de quoi tenir.
const MAX_STEAM_NAME_BYTES: usize = 200;
/// L'identifiant d'un succès dans son jeu (`apiname`) : un nom technique, court.
const MAX_STEAM_APINAME_BYTES: usize = 128;
/// Au-delà de cet écart entre deux relevés, le temps n'est pas compté : le relevé ne sert que de point
/// de départ. Un jeu rendu à la bibliothèque avec des années de jeu ne devient pas des milliers
/// d'heures d'un seul coup ; deux semaines de jeu entre deux relevés restent comptées.
const MAX_STEAM_DELTA_MINUTES: i64 = 14 * 24 * 60;

/// Un appid de Steam : un entier positif.
fn steam_appid(payload: &Value, error: &'static str) -> Result<u64, InterpretError> {
    payload
        .get("appid")
        .and_then(Value::as_u64)
        .filter(|id| *id > 0)
        .ok_or(InterpretError(error))
}

/// Un nom facultatif : absent, `null` ou vide, rien ; autre chose qu'un texte, ou trop long, une erreur.
fn steam_name<'a>(
    payload: &'a Value,
    field: &str,
    error: &'static str,
) -> Result<Option<&'a str>, InterpretError> {
    match payload.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) if text.trim().len() <= MAX_STEAM_NAME_BYTES => {
            Ok(Some(text.trim()).filter(|t| !t.is_empty()))
        }
        Some(_) => Err(InterpretError(error)),
    }
}

/// Un compteur en minutes : un entier positif ou nul.
fn steam_minutes(value: Option<&Value>) -> Option<i64> {
    value
        .and_then(Value::as_i64)
        .filter(|m| (0..=i64::from(i32::MAX)).contains(m))
}

/// Le jeu d'un événement, sous son appid, affiché sous son nom (l'appid s'il manque).
fn game_item(appid: u64, name: Option<&str>) -> Item {
    let key = appid.to_string();
    let display = name.map_or_else(|| key.clone(), str::to_owned);
    Item {
        level: "game",
        position: 0,
        item_key: key,
        display,
    }
}

/// `{appid, name, playtime_forever, playtime_disconnected, rtime_last_played, previous}` : un relevé du
/// compteur d'un jeu, à l'heure du relevé. Il donne toujours le total d'heures du jeu
/// (`game.total_playtime`, en minutes). Avec un relevé précédent (`{playtime_forever, observed_at}`) et
/// un écart positif, il donne aussi le temps joué entre les deux (`game.play`) :
///
/// - la fin de la partie est `rtime_last_played` s'il tombe entre le relevé précédent (exclu) et
///   celui-ci (inclus), sinon l'heure du relevé ;
/// - l'écart est placé juste avant cette fin, découpé en tranches de `MAX_MEASURED_MS` au plus, mises
///   bout à bout, chacune avec `details.played_ms`.
///
/// Une partie hors ligne, synchronisée plus tard, se place donc à la fin. Un écart négatif ou nul ne
/// donne aucun temps, un écart de plus de deux semaines non plus (point de départ).
///
/// Un point de départ qui a déjà du temps (sans relevé précédent et à plus de zéro, ou après un écart
/// de plus de deux semaines) donne aussi `game.known` : le jeu a été joué avant le suivi, sa première
/// partie comptée n'est pas une découverte. Un premier relevé à zéro n'en donne pas : la première
/// partie qui suit en est une.
fn steam_playtime(record: &Record) -> Result<Vec<Output>, InterpretError> {
    let payload = &record.payload;
    let appid = steam_appid(payload, "steam.playtime: invalid appid")?;
    let name = steam_name(payload, "name", "steam.playtime: invalid name")?;
    let total = steam_minutes(payload.get("playtime_forever"))
        .ok_or(InterpretError("steam.playtime: invalid playtime_forever"))?;
    match payload.get("playtime_disconnected") {
        None | Some(Value::Null) => {}
        value => {
            steam_minutes(value).ok_or(InterpretError(
                "steam.playtime: invalid playtime_disconnected",
            ))?;
        }
    }
    let last_played = match payload.get("rtime_last_played") {
        None | Some(Value::Null) => None,
        Some(value) => Some(
            value
                .as_i64()
                .filter(|s| *s >= 0)
                .and_then(|s| DateTime::from_timestamp(s, 0))
                .ok_or(InterpretError("steam.playtime: invalid rtime_last_played"))?,
        )
        .filter(|at| at.timestamp() > 0),
    };
    let previous = match payload.get("previous") {
        None | Some(Value::Null) => None,
        Some(previous) => {
            let minutes = steam_minutes(previous.get("playtime_forever"))
                .ok_or(InterpretError("steam.playtime: invalid previous"))?;
            let at = previous
                .get("observed_at")
                .and_then(Value::as_str)
                .and_then(|text| DateTime::parse_from_rfc3339(text).ok())
                .map(|at| at.with_timezone(&Utc))
                .filter(|at| *at < record.occurred_at)
                .ok_or(InterpretError("steam.playtime: invalid previous"))?;
            Some((minutes, at))
        }
    };

    let mut outputs = vec![Output::Event(Event {
        value: Some(total.to_string()),
        details: json!({}),
        items: vec![game_item(appid, name)],
        ..Event::at_envelope(record, "game.total_playtime")
    })];
    // Un point de départ qui a déjà du temps de jeu (premier relevé, état perdu, jeu revenu avec plus de
    // deux semaines d'écart) : joué hors de notre vue, donc déjà connu, et jamais une découverte.
    let known = Output::Event(Event {
        details: json!({}),
        items: vec![game_item(appid, name)],
        ..Event::at_envelope(record, "game.known")
    });
    let Some((before, since)) = previous else {
        if total > 0 {
            outputs.push(known);
        }
        return Ok(outputs);
    };
    let delta = total - before;
    if delta > MAX_STEAM_DELTA_MINUTES {
        outputs.push(known);
        return Ok(outputs);
    }
    if delta <= 0 {
        return Ok(outputs);
    }
    let end = last_played
        .filter(|at| *at > since && *at <= record.occurred_at)
        .unwrap_or(record.occurred_at);
    let mut remaining = delta * 60_000;
    let mut start = end - chrono::TimeDelta::milliseconds(remaining);
    while remaining > 0 {
        let played = remaining.min(MAX_MEASURED_MS);
        outputs.push(Output::Event(Event {
            details: json!({ "played_ms": played }),
            items: vec![game_item(appid, name)],
            occurred_at: start,
            ..Event::at_envelope(record, "game.play")
        }));
        start += chrono::TimeDelta::milliseconds(played);
        remaining -= played;
    }
    Ok(outputs)
}

/// `{appid, game_name, apiname, name}` : un succès débloqué, à sa date de déblocage (l'enveloppe).
/// Éléments : le jeu, sous son appid, et le succès, sous `<appid>:<apiname>`, affiché sous son nom
/// (son identifiant s'il manque).
fn steam_achievement(record: &Record) -> Result<Vec<Output>, InterpretError> {
    let payload = &record.payload;
    let appid = steam_appid(payload, "steam.achievement: invalid appid")?;
    let game_name = steam_name(payload, "game_name", "steam.achievement: invalid game_name")?;
    let apiname = payload
        .get("apiname")
        .and_then(Value::as_str)
        .filter(|a| {
            !a.is_empty() && a.len() <= MAX_STEAM_APINAME_BYTES && !a.chars().any(char::is_control)
        })
        .ok_or(InterpretError("steam.achievement: invalid apiname"))?;
    let name = steam_name(payload, "name", "steam.achievement: invalid name")?;
    Ok(vec![Output::Event(Event {
        items: vec![
            game_item(appid, game_name),
            Item::single(
                "achievement",
                format!("{appid}:{apiname}"),
                name.unwrap_or(apiname),
            ),
        ],
        ..Event::at_envelope(record, "game.achievement")
    })])
}

// ─── GitHub ──────────────────────────────────────────────────────────────────
//
// Le connecteur réduit déjà chaque fait à sa liste blanche ; l'interprète la revérifie et ne recopie
// dans les détails que les champs listés. Jamais le corps d'un message, d'une issue ni d'un
// commentaire : seulement les titres, que `shared_details` ne donne pas au MCP.

/// Un id GraphQL (dépôt, pull request, review, issue, commentaire) : un jeton opaque et court.
const MAX_GITHUB_ID_BYTES: usize = 128;
/// `propriétaire/nom` : 39 + 1 + 100 octets chez GitHub.
const MAX_GITHUB_REPO_BYTES: usize = 200;
/// Un titre au-delà est coupé : la carte n'en montre pas plus.
const MAX_GITHUB_TITLE_CHARS: usize = 300;
const MAX_GITHUB_URL_BYTES: usize = 2048;

/// Un texte court, sans espace ni caractère de contrôle.
fn github_token(value: Option<&Value>, max: usize) -> Option<&str> {
    value.and_then(Value::as_str).filter(|t| {
        !t.is_empty() && t.len() <= max && !t.chars().any(|c| c.is_control() || c.is_whitespace())
    })
}

/// Le dépôt d'un fait, sous son id stable, affiché `propriétaire/nom` (le nom au moment du fait).
fn github_repo(payload: &Value, error: &'static str) -> Result<Item, InterpretError> {
    let repo = payload.get("repo").ok_or(InterpretError(error))?;
    let id = github_token(repo.get("id"), MAX_GITHUB_ID_BYTES).ok_or(InterpretError(error))?;
    let name =
        github_token(repo.get("name"), MAX_GITHUB_REPO_BYTES).ok_or(InterpretError(error))?;
    Ok(Item::single("repo", id.to_owned(), name))
}

/// Un titre facultatif : absent ou `null`, rien ; un texte, sans ses blancs autour ni ses caractères de
/// contrôle, coupé à `MAX_GITHUB_TITLE_CHARS` ; autre chose, une erreur.
fn github_title(payload: &Value, error: &'static str) -> Result<Option<String>, InterpretError> {
    match payload.get("title") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => {
            let clean: String = text
                .chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .collect();
            let title: String = clean.trim().chars().take(MAX_GITHUB_TITLE_CHARS).collect();
            Ok(Some(title).filter(|t| !t.is_empty()))
        }
        Some(_) => Err(InterpretError(error)),
    }
}

/// Une adresse facultative, en https seulement : la carte en fait un lien.
fn github_url<'a>(
    payload: &'a Value,
    error: &'static str,
) -> Result<Option<&'a str>, InterpretError> {
    match payload.get("url") {
        None | Some(Value::Null) => Ok(None),
        value => github_token(value, MAX_GITHUB_URL_BYTES)
            .filter(|u| u.starts_with("https://"))
            .map(Some)
            .ok_or(InterpretError(error)),
    }
}

/// Un nombre de lignes facultatif : GitHub n'en donne pas pour un très gros commit.
fn github_lines(
    payload: &Value,
    field: &str,
    error: &'static str,
) -> Result<Option<i64>, InterpretError> {
    match payload.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_i64()
            .filter(|n| (0..=i64::from(i32::MAX)).contains(n))
            .map(Some)
            .ok_or(InterpretError(error)),
    }
}

/// Le numéro d'une issue ou d'une pull request dans son dépôt.
fn github_number(value: Option<&Value>) -> Option<i64> {
    value
        .and_then(Value::as_i64)
        .filter(|n| (1..=i64::from(i32::MAX)).contains(n))
}

/// L'id d'un fait : exigé, même s'il ne sert qu'à la clé, pour qu'un fait sans identité soit vu.
fn github_require_id(payload: &Value, error: &'static str) -> Result<(), InterpretError> {
    github_token(payload.get("id"), MAX_GITHUB_ID_BYTES)
        .map(|_| ())
        .ok_or(InterpretError(error))
}

/// `{repo: {id, name}, oid, authored_at, committed_at, title, additions, deletions, url}` : un commit, à
/// sa date d'auteur (l'enveloppe). Détails : `{sha, title, additions, deletions, url}`.
fn github_commit(record: &Record) -> Result<Vec<Output>, InterpretError> {
    let payload = &record.payload;
    let repo = github_repo(payload, "github.commit: invalid repo")?;
    let sha = payload
        .get("oid")
        .and_then(Value::as_str)
        .filter(|s| (7..=64).contains(&s.len()) && s.chars().all(|c| c.is_ascii_hexdigit()))
        .ok_or(InterpretError("github.commit: invalid oid"))?;
    let title = github_title(payload, "github.commit: invalid title")?;
    let additions = github_lines(payload, "additions", "github.commit: invalid additions")?;
    let deletions = github_lines(payload, "deletions", "github.commit: invalid deletions")?;
    let url = github_url(payload, "github.commit: invalid url")?;
    Ok(vec![Output::Event(Event {
        details: json!({
            "sha": sha.to_ascii_lowercase(), "title": title, "additions": additions,
            "deletions": deletions, "url": url,
        }),
        items: vec![repo],
        ..Event::at_envelope(record, "code.commit")
    })])
}

/// `{id, action, repo, number, title, url, additions, deletions}` : une pull request ouverte
/// (`opened`, à sa création : `code.pull_request`) ou fusionnée (`merged`, à sa fusion :
/// `code.pull_request_merged`, avec les lignes de la pull request à ce moment-là). Deux métriques pour
/// que le calendrier ne compte pas deux fois une pull request fusionnée (voir la migration).
fn github_pull_request(record: &Record) -> Result<Vec<Output>, InterpretError> {
    let payload = &record.payload;
    github_require_id(payload, "github.pull_request: invalid id")?;
    let repo = github_repo(payload, "github.pull_request: invalid repo")?;
    let number = github_number(payload.get("number"))
        .ok_or(InterpretError("github.pull_request: invalid number"))?;
    let title = github_title(payload, "github.pull_request: invalid title")?;
    let url = github_url(payload, "github.pull_request: invalid url")?;
    let event = match payload.get("action").and_then(Value::as_str) {
        Some("opened") => Event {
            details: json!({ "number": number, "title": title, "url": url }),
            items: vec![repo],
            ..Event::at_envelope(record, "code.pull_request")
        },
        Some("merged") => {
            let additions = github_lines(
                payload,
                "additions",
                "github.pull_request: invalid additions",
            )?;
            let deletions = github_lines(
                payload,
                "deletions",
                "github.pull_request: invalid deletions",
            )?;
            Event {
                details: json!({
                    "number": number, "title": title, "url": url,
                    "additions": additions, "deletions": deletions,
                }),
                items: vec![repo],
                ..Event::at_envelope(record, "code.pull_request_merged")
            }
        }
        _ => return Err(InterpretError("github.pull_request: invalid action")),
    };
    Ok(vec![Output::Event(event)])
}

/// `{id, state, repo, number, title, url}` : une review soumise sur la pull request `number`, à sa
/// soumission. L'état passe en minuscules : `approved`, `changes_requested`, `commented`, `dismissed`.
fn github_review(record: &Record) -> Result<Vec<Output>, InterpretError> {
    let payload = &record.payload;
    github_require_id(payload, "github.review: invalid id")?;
    let repo = github_repo(payload, "github.review: invalid repo")?;
    let state = match payload.get("state").and_then(Value::as_str) {
        Some("APPROVED") => "approved",
        Some("CHANGES_REQUESTED") => "changes_requested",
        Some("COMMENTED") => "commented",
        Some("DISMISSED") => "dismissed",
        _ => return Err(InterpretError("github.review: invalid state")),
    };
    let number = github_number(payload.get("number"))
        .ok_or(InterpretError("github.review: invalid number"))?;
    let title = github_title(payload, "github.review: invalid title")?;
    let url = github_url(payload, "github.review: invalid url")?;
    Ok(vec![Output::Event(Event {
        details: json!({ "state": state, "number": number, "title": title, "url": url }),
        items: vec![repo],
        ..Event::at_envelope(record, "code.review")
    })])
}

/// `{id, repo, number, title, url}` : une issue ouverte, à sa création.
fn github_issue(record: &Record) -> Result<Vec<Output>, InterpretError> {
    let payload = &record.payload;
    github_require_id(payload, "github.issue: invalid id")?;
    let repo = github_repo(payload, "github.issue: invalid repo")?;
    let number = github_number(payload.get("number"))
        .ok_or(InterpretError("github.issue: invalid number"))?;
    let title = github_title(payload, "github.issue: invalid title")?;
    let url = github_url(payload, "github.issue: invalid url")?;
    Ok(vec![Output::Event(Event {
        details: json!({ "number": number, "title": title, "url": url }),
        items: vec![repo],
        ..Event::at_envelope(record, "code.issue")
    })])
}

/// `{id, repo, target, number, title, url}` : un commentaire, à sa création, sur une issue
/// (`issue`), une pull request (`pull_request`), avec le numéro et le titre de la cible, ou sur un
/// commit (`commit`), avec la première ligne de son message pour titre et sans numéro.
fn github_comment(record: &Record) -> Result<Vec<Output>, InterpretError> {
    let payload = &record.payload;
    github_require_id(payload, "github.comment: invalid id")?;
    let repo = github_repo(payload, "github.comment: invalid repo")?;
    let target = payload
        .get("target")
        .and_then(Value::as_str)
        .filter(|t| ["issue", "pull_request", "commit"].contains(t))
        .ok_or(InterpretError("github.comment: invalid target"))?;
    let number = match payload.get("number") {
        None | Some(Value::Null) if target == "commit" => None,
        value => {
            Some(github_number(value).ok_or(InterpretError("github.comment: invalid number"))?)
        }
    };
    let title = github_title(payload, "github.comment: invalid title")?;
    let url = github_url(payload, "github.comment: invalid url")?;
    Ok(vec![Output::Event(Event {
        details: json!({ "target": target, "number": number, "title": title, "url": url }),
        items: vec![repo],
        ..Event::at_envelope(record, "code.comment")
    })])
}

// ─── Position de l'iPhone (Overland) ─────────────────────────────────────────
//
// Les charges utiles sont déjà réduites à leur liste blanche par la route d'adaptation
// (`crate::overland`), qui ne garde d'une visite que sa version complète, départ connu.

/// Les états de mouvement qu'Overland tire de CoreMotion ; tout autre mot est ignoré.
const OVERLAND_MOTIONS: [&str; 5] = ["walking", "running", "cycling", "driving", "stationary"];

/// Un nombre fini de la charge utile, tel qu'envoyé ; `null` ou absent : `None`.
fn overland_number<'a>(
    payload: &'a Value,
    field: &str,
    error: &'static str,
) -> Result<Option<&'a serde_json::Number>, InterpretError> {
    match payload.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) if n.as_f64().is_some_and(f64::is_finite) => Ok(Some(n)),
        Some(_) => Err(InterpretError(error)),
    }
}

/// Une coordonnée obligatoire, dans ses bornes.
fn overland_coordinate(
    payload: &Value,
    field: &str,
    bound: f64,
    error: &'static str,
) -> Result<Value, InterpretError> {
    Some(coordinate(payload, field, bound, error)?)
        .filter(|c| !c.is_null())
        .ok_or(InterpretError(error))
}

/// `{lat, lon, horizontal_accuracy, altitude, speed, motion}` : un point `location.point` à son
/// instant (l'enveloppe), sans valeur. Une précision négative est, pour iOS, une position invalide :
/// aucun événement, comme le titre en cours de Last.fm. Une vitesse négative est inconnue : `null`.
fn overland_location(record: &Record) -> Result<Vec<Output>, InterpretError> {
    const ERROR: &str = "overland.location: invalid point";
    let payload = &record.payload;
    let lat = overland_coordinate(payload, "lat", 90.0, ERROR)?;
    let lon = overland_coordinate(payload, "lon", 180.0, ERROR)?;
    let accuracy = overland_number(payload, "horizontal_accuracy", ERROR)?;
    if accuracy.is_some_and(|a| a.as_f64().is_some_and(|a| a < 0.0)) {
        return Ok(Vec::new());
    }
    let speed =
        overland_number(payload, "speed", ERROR)?.filter(|s| s.as_f64().is_some_and(|s| s >= 0.0));
    let altitude = overland_number(payload, "altitude", ERROR)?;
    let motion: Vec<&str> = match payload.get("motion") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(states)) => states
            .iter()
            .filter_map(Value::as_str)
            .filter(|s| OVERLAND_MOTIONS.contains(s))
            .collect(),
        Some(_) => return Err(InterpretError(ERROR)),
    };
    let number = |n: Option<&serde_json::Number>| n.cloned().map_or(Value::Null, Value::Number);
    Ok(vec![Output::Event(Event {
        details: json!({
            "point_lat": lat,
            "point_lon": lon,
            "horizontal_accuracy": number(accuracy),
            "altitude": number(altitude),
            "speed": number(speed),
            "motion": motion,
        }),
        ..Event::at_envelope(record, "location.point")
    })])
}

/// `{lat, lon, arrival, departure, horizontal_accuracy}` : un lieu `place.visit` à son arrivée
/// (l'enveloppe), sans valeur (le jour compte les lieux). Détails : coordonnées, arrivée et départ tels
/// qu'envoyés (RFC 3339), durée en secondes, précision.
fn overland_visit(record: &Record) -> Result<Vec<Output>, InterpretError> {
    const ERROR: &str = "overland.visit: invalid visit";
    let payload = &record.payload;
    let lat = overland_coordinate(payload, "lat", 90.0, ERROR)?;
    let lon = overland_coordinate(payload, "lon", 180.0, ERROR)?;
    let instant = |field: &str| {
        payload
            .get(field)
            .and_then(Value::as_str)
            .and_then(|t| crate::ingest::parse_occurred_at(t).map(|(at, _)| (t, at)))
            .ok_or(InterpretError(ERROR))
    };
    let (arrival, arrived) = instant("arrival")?;
    let (departure, departed) = instant("departure")?;
    if departed < arrived {
        return Err(InterpretError(ERROR));
    }
    let accuracy = overland_number(payload, "horizontal_accuracy", ERROR)?;
    Ok(vec![Output::Event(Event {
        details: json!({
            "visit_lat": lat,
            "visit_lon": lon,
            "arrival": arrival,
            "departure": departure,
            "duration_s": (departed - arrived).num_seconds(),
            "horizontal_accuracy": accuracy.cloned().map_or(Value::Null, Value::Number),
        }),
        ..Event::at_envelope(record, "place.visit")
    })])
}

// ─── Riot : League of Legends, Teamfight Tactics, VALORANT ───────────────────
//
// Une partie par enregistrement, à son début, réduite par le connecteur à ma ligne. Chacune donne son
// temps de jeu (`game.play`, des tranches d'une heure au plus depuis le début, comme Steam), la
// déclaration « déjà connu » de son jeu (`game.known` : ces jeux sont joués depuis bien avant le
// suivi, jamais des découvertes) et la partie elle-même (`game.match`, comptée). Le nom d'une file vient
// d'une table figée ici, jamais de l'heure ni d'un appel : une file inconnue s'affiche `File <id>`, et
// une nouvelle version de l'interprète, suivie d'un rebuild, la nomme plus tard.

/// Un jeu de Riot : la clé de son élément `game` et son nom affiché.
#[derive(Clone, Copy)]
enum RiotGame {
    Lol,
    Tft,
    Valorant,
}

impl RiotGame {
    fn code(self) -> &'static str {
        match self {
            Self::Lol => "lol",
            Self::Tft => "tft",
            Self::Valorant => "valorant",
        }
    }

    /// Le préfixe `riot:` sépare ces clés des appids de Steam, des entiers.
    fn item(self) -> Item {
        let (key, name) = match self {
            Self::Lol => ("riot:lol", "League of Legends"),
            Self::Tft => ("riot:tft", "Teamfight Tactics"),
            Self::Valorant => ("riot:valorant", "VALORANT"),
        };
        Item::single("game", key.to_owned(), name)
    }
}

/// Un identifiant de partie : `EUW1_7234567890` chez Riot, un UUID chez HenrikDev.
const MAX_MATCH_ID_BYTES: usize = 64;
/// Un nom court (champion, agent, carte, mode, file, poste) : largement de quoi tenir.
const MAX_RIOT_NAME_BYTES: usize = 64;
/// Au-delà, une durée de partie est absurde : six heures.
const MAX_MATCH_MS: i64 = 6 * 3_600_000;

/// Les files de League of Legends, par `queueId` : celles de mon historique (relevé le 12 nov. 2026 :
/// 420, 440, 450, 400, 900, 1700, 1750) et les plus courantes de la liste de Riot. 1750 n'y figure pas :
/// une variante d'Arena (`gameMode` `CHERRY`).
fn lol_queue(id: i64) -> Option<&'static str> {
    Some(match id {
        400 => "Normale (draft)",
        420 => "Classée solo/duo",
        430 => "Normale (aveugle)",
        440 => "Classée flexible",
        450 => "ARAM",
        480 => "Partie rapide (Swiftplay)",
        490 => "Partie rapide",
        700 => "Clash",
        900 => "URF",
        1700 | 1710 | 1750 => "Arena",
        1900 => "Pick URF",
        830 | 840 | 850 | 870 | 880 | 890 => "Contre l'IA",
        _ => return None,
    })
}

/// Les files de Teamfight Tactics, par `queue_id` : celles de mon historique (1100, 1160, 6130, 1130,
/// 1090) et les modes événement nommés par Data Dragon.
fn tft_queue(id: i64) -> Option<&'static str> {
    Some(match id {
        1090 => "Normale",
        1100 => "Classée",
        1130 => "Hyper Roll",
        1160 => "Double Up",
        1170 => "Fortune's Favor",
        1210 => "Choncc's Classic Treasure",
        1220 => "Tocker's Trials",
        6000 => "Revival: Festival of Beasts",
        6100 => "Choncc's K.O. Coliseum",
        6120 => "Pengu's Party",
        6130 => "Ao Shin's Ascent",
        _ => return None,
    })
}

fn queue_name(id: i64, table: fn(i64) -> Option<&'static str>) -> String {
    table(id).map_or_else(|| format!("File {id}"), str::to_owned)
}

/// Une file de LoL inconnue de la table se nomme par son mode quand il est sans ambiguïté (`CHERRY`
/// est l'Arena, `ARAM` l'ARAM), sinon `File <id>`.
fn lol_queue_name(id: i64, mode: Option<&str>) -> String {
    match (lol_queue(id), mode) {
        (Some(name), _) => name.to_owned(),
        (None, Some("CHERRY")) => "Arena".to_owned(),
        (None, Some("ARAM")) => "ARAM".to_owned(),
        (None, _) => format!("File {id}"),
    }
}

fn match_id<'a>(payload: &'a Value, error: &'static str) -> Result<&'a str, InterpretError> {
    payload
        .get("match_id")
        .and_then(Value::as_str)
        .filter(|id| {
            (1..=MAX_MATCH_ID_BYTES).contains(&id.len())
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        })
        .ok_or(InterpretError(error))
}

/// Un nom facultatif : absent, `null` ou vide, rien ; autre chose qu'un texte court, une erreur.
fn riot_name<'a>(
    payload: &'a Value,
    field: &str,
    error: &'static str,
) -> Result<Option<&'a str>, InterpretError> {
    match payload.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text))
            if text.trim().len() <= MAX_RIOT_NAME_BYTES && !text.chars().any(char::is_control) =>
        {
            Ok(Some(text.trim()).filter(|t| !t.is_empty()))
        }
        Some(_) => Err(InterpretError(error)),
    }
}

/// Un entier facultatif dans des bornes.
fn riot_integer(
    payload: &Value,
    field: &str,
    range: std::ops::RangeInclusive<i64>,
    error: &'static str,
) -> Result<Option<i64>, InterpretError> {
    match payload.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_i64()
            .filter(|n| range.contains(n))
            .map(Some)
            .ok_or(InterpretError(error)),
    }
}

/// Un booléen exigé.
fn riot_flag(payload: &Value, field: &str, error: &'static str) -> Result<bool, InterpretError> {
    payload
        .get(field)
        .and_then(Value::as_bool)
        .ok_or(InterpretError(error))
}

/// Les sorties communes d'une partie : son temps de jeu en tranches d'une heure au plus depuis son
/// début, la déclaration « déjà connu » de son jeu, et la partie, avec son personnage s'il y en a un.
fn match_outputs(
    record: &Record,
    game: RiotGame,
    played_ms: i64,
    character: Option<Item>,
    details: Value,
) -> Vec<Output> {
    let mut outputs = Vec::new();
    let mut start = record.occurred_at;
    let mut remaining = played_ms;
    while remaining > 0 {
        let played = remaining.min(MAX_MEASURED_MS);
        outputs.push(Output::Event(Event {
            details: json!({ "played_ms": played }),
            items: vec![game.item()],
            occurred_at: start,
            ..Event::at_envelope(record, "game.play")
        }));
        start += chrono::TimeDelta::milliseconds(played);
        remaining -= played;
    }
    outputs.push(Output::Event(Event {
        items: vec![game.item()],
        ..Event::at_envelope(record, "game.known")
    }));
    let mut items = vec![game.item()];
    items.extend(character);
    outputs.push(Output::Event(Event {
        details,
        items,
        ..Event::at_envelope(record, "game.match")
    }));
    outputs
}

/// `{match_id, started_at, ended_at, duration_s, queue_id, game_mode, end_of_game_result, champion,
/// win, position, remake}` : une partie de League of Legends, à son début (l'enveloppe). Temps de jeu :
/// `duration_s`, toute la partie (un remake compte aussi). Résultat : `remake` si la partie a fini en
/// reddition anticipée, sinon `win` ou `loss`. Élément `character` : le champion, sous `lol:<nom>`.
fn riot_lol_match(record: &Record) -> Result<Vec<Output>, InterpretError> {
    const ERROR: &str = "riot.lol_match: invalid match";
    let payload = &record.payload;
    let id = match_id(payload, "riot.lol_match: invalid match_id")?;
    let duration = riot_integer(payload, "duration_s", 1..=MAX_MATCH_MS / 1000, ERROR)?
        .ok_or(InterpretError(ERROR))?;
    let queue_id = riot_integer(payload, "queue_id", 0..=i64::from(i32::MAX), ERROR)?
        .ok_or(InterpretError(ERROR))?;
    let mode = riot_name(payload, "game_mode", ERROR)?;
    let champion = riot_name(payload, "champion", ERROR)?;
    let position = riot_name(payload, "position", ERROR)?;
    let win = riot_flag(payload, "win", ERROR)?;
    let remake = riot_flag(payload, "remake", ERROR)?;
    let result = if remake {
        "remake"
    } else if win {
        "win"
    } else {
        "loss"
    };
    let played_ms = duration * 1000;
    let character = champion.map(|name| Item::single("character", format!("lol:{name}"), name));
    Ok(match_outputs(
        record,
        RiotGame::Lol,
        played_ms,
        character,
        json!({
            "game": RiotGame::Lol.code(),
            "match_id": id,
            "queue": lol_queue_name(queue_id, mode),
            "queue_id": queue_id,
            "mode": mode,
            "result": result,
            "character": champion,
            "position": position,
            "played_ms": played_ms,
        }),
    ))
}

/// `{match_id, started_at, ended_at, game_length_s, eliminated_s, queue_id, game_type, set_number,
/// placement, partner_group_id}` : une partie de Teamfight Tactics, à son début (`gameCreation`, dans
/// l'enveloppe ; `game_datetime` en est la fin). Mon temps de jeu est `eliminated_s`, jusqu'à mon
/// élimination, jamais la durée de la partie. Pas de résultat : la place (1 à 8) en tient lieu.
fn riot_tft_match(record: &Record) -> Result<Vec<Output>, InterpretError> {
    const ERROR: &str = "riot.tft_match: invalid match";
    let payload = &record.payload;
    let id = match_id(payload, "riot.tft_match: invalid match_id")?;
    let played_ms = payload
        .get("eliminated_s")
        .and_then(Value::as_f64)
        .map(|s| (s * 1000.0).round())
        .filter(|ms| *ms >= 1.0 && *ms <= MAX_MATCH_MS as f64)
        .ok_or(InterpretError(ERROR))? as i64;
    let queue_id = riot_integer(payload, "queue_id", 0..=i64::from(i32::MAX), ERROR)?
        .ok_or(InterpretError(ERROR))?;
    let mode = riot_name(payload, "game_type", ERROR)?;
    let placement =
        riot_integer(payload, "placement", 1..=8, ERROR)?.ok_or(InterpretError(ERROR))?;
    Ok(match_outputs(
        record,
        RiotGame::Tft,
        played_ms,
        None,
        json!({
            "game": RiotGame::Tft.code(),
            "match_id": id,
            "queue": queue_name(queue_id, tft_queue),
            "queue_id": queue_id,
            "mode": mode,
            "result": null,
            "placement": placement,
            "played_ms": played_ms,
        }),
    ))
}

/// `{match_id, started_at, duration_ms, queue_id, queue_name, map, agent, team, won, rounds_won,
/// rounds_lost}` : une partie terminée de VALORANT, à son début. Temps de jeu : `duration_ms`. File :
/// le nom donné par HenrikDev, tel quel. Résultat : `win` si mon équipe a gagné ; sinon `draw` à rounds
/// égaux, `loss` autrement ; rien sans équipe (deathmatch). Élément `character` : l'agent, sous
/// `valorant:<nom en minuscules>`.
fn valorant_match(record: &Record) -> Result<Vec<Output>, InterpretError> {
    const ERROR: &str = "valorant.match: invalid match";
    let payload = &record.payload;
    let id = match_id(payload, "valorant.match: invalid match_id")?;
    let played_ms = riot_integer(payload, "duration_ms", 1..=MAX_MATCH_MS, ERROR)?
        .ok_or(InterpretError(ERROR))?;
    let queue = riot_name(payload, "queue_name", ERROR)?.or(riot_name(payload, "queue_id", ERROR)?);
    let map = riot_name(payload, "map", ERROR)?;
    let agent = riot_name(payload, "agent", ERROR)?;
    let won = match payload.get("won") {
        None | Some(Value::Null) => None,
        Some(Value::Bool(won)) => Some(*won),
        Some(_) => return Err(InterpretError(ERROR)),
    };
    let rounds_won = riot_integer(payload, "rounds_won", 0..=1000, ERROR)?;
    let rounds_lost = riot_integer(payload, "rounds_lost", 0..=1000, ERROR)?;
    let result = match won {
        None => None,
        Some(true) => Some("win"),
        Some(false) if rounds_won.is_some() && rounds_won == rounds_lost => Some("draw"),
        Some(false) => Some("loss"),
    };
    let character = agent.map(|name| {
        Item::single(
            "character",
            format!("valorant:{}", name.to_lowercase()),
            name,
        )
    });
    Ok(match_outputs(
        record,
        RiotGame::Valorant,
        played_ms,
        character,
        json!({
            "game": RiotGame::Valorant.code(),
            "match_id": id,
            "queue": queue,
            "mode": null,
            "result": result,
            "character": agent,
            "map": map,
            "rounds_won": rounds_won,
            "rounds_lost": rounds_lost,
            "played_ms": played_ms,
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(payload: Value) -> Record {
        Record {
            seq: 1,
            source_id: 1,
            schema_version: 1,
            occurred_at: "2026-10-13T07:42:10Z".parse().unwrap(),
            utc_offset_minutes: Some(120),
            payload,
        }
    }

    #[test]
    fn a_coffee_ignores_its_payload() {
        for payload in [json!({}), json!({"note": "x"}), json!(null), json!([1])] {
            let record = record(payload);
            assert_eq!(
                coffee_cup(&record).unwrap(),
                vec![Output::Event(Event::at_envelope(&record, "coffee"))]
            );
        }
    }

    #[test]
    fn a_cancellation_requires_source_and_key() {
        let ok = record_cancel(&record(json!({"source": "webapp", "key": "k", "extra": 1})));
        assert_eq!(
            ok.unwrap(),
            vec![Output::Cancellation {
                source: "webapp".to_owned(),
                key: "k".to_owned()
            }]
        );
        for payload in [
            json!({}),
            json!(null),
            json!("webapp"),
            json!({"source": "webapp"}),
            json!({"key": "k"}),
            json!({"source": 3, "key": "k"}),
            json!({"source": "webapp", "key": 3}),
            json!({"source": "", "key": "k"}),
            json!({"source": "webapp", "key": ""}),
            json!({"source": "webapp", "key": "x".repeat(513)}),
        ] {
            assert!(record_cancel(&record(payload)).is_err());
        }
    }

    fn single_event(outputs: Vec<Output>) -> Event {
        let mut outputs = outputs.into_iter();
        let (Some(Output::Event(event)), None) = (outputs.next(), outputs.next()) else {
            panic!("un seul événement attendu");
        };
        event
    }

    #[test]
    fn a_scrobble_becomes_a_listen_with_its_items() {
        let record = record(json!({
            "artist": {"mbid": "", "#text": "Daft Punk feat. Someone"},
            "name": " Veridis Quo - Remastered ",
            "album": {"mbid": "x", "#text": "Discovery"},
            "date": {"uts": "1", "#text": "ignorée : la date est celle de l'enveloppe"},
        }));
        let event = single_event(lastfm_scrobble(&record).unwrap());
        assert_eq!(event.metric, "music.listen");
        assert_eq!(event.occurred_at, record.occurred_at);
        assert_eq!(event.utc_offset_minutes, Some(120));
        assert_eq!(event.value, None);
        assert_eq!(event.details["track"], "Veridis Quo - Remastered");
        assert_eq!(event.details["artist_key"], "daft punk");
        assert_eq!(event.details["track_key"], "daft punk\tveridis quo");
        assert_eq!(event.details["album_key"], "daft punk\tdiscovery");
        let levels: Vec<_> = event
            .items
            .iter()
            .map(|i| (i.level, i.item_key.as_str()))
            .collect();
        assert_eq!(
            levels,
            [
                ("artist", "daft punk"),
                ("artist", "someone"),
                ("album", "daft punk\tdiscovery"),
                ("track", "daft punk\tveridis quo"),
            ]
        );
        assert_eq!(event.details["artist"], "Daft Punk feat. Someone");
        assert_eq!(
            event.details["artists"],
            json!([{"key": "daft punk", "name": "Daft Punk"}, {"key": "someone", "name": "Someone"}])
        );
    }

    #[test]
    fn a_listen_credits_each_artist_once_and_counts_as_one_event() {
        let record = record(json!({
            "artist": {"#text": "Zek, Jungle Jack & Caballero"},
            "name": "T (feat. Jungle Jack & Invité)",
            "album": {"#text": "GRAND LUXXXE Vol.1"},
        }));
        let event = single_event(lastfm_scrobble(&record).unwrap());
        let items: Vec<_> = event
            .items
            .iter()
            .map(|i| (i.level, i.position, i.item_key.as_str(), i.display.as_str()))
            .collect();
        assert_eq!(
            items,
            [
                ("artist", 0, "zek", "Zek"),
                ("artist", 1, "jungle jack", "Jungle Jack"),
                ("artist", 2, "caballero", "Caballero"),
                ("artist", 3, "invité", "Invité"),
                ("album", 0, "zek\tgrand luxxxe vol.1", "GRAND LUXXXE Vol.1"),
                ("track", 0, "zek\tt", "T (feat. Jungle Jack & Invité)"),
            ]
        );
        assert_eq!(event.details["artist_key"], "zek");
        assert_eq!(event.details["album_key"], "zek\tgrand luxxxe vol.1");
        assert_eq!(event.details["track_key"], "zek\tt");
    }

    #[test]
    fn a_scrobble_without_album_has_no_album_item() {
        for album in [
            json!({"mbid": "", "#text": ""}),
            json!({"#text": "  "}),
            json!(null),
        ] {
            let record = record(json!({"artist": {"#text": "A"}, "name": "T", "album": album}));
            let event = single_event(lastfm_scrobble(&record).unwrap());
            assert_eq!(event.details["album"], Value::Null);
            assert_eq!(event.details["album_key"], Value::Null);
            assert_eq!(event.items.len(), 2);
        }
        let record = record(json!({"artist": {"name": "Étendu"}, "name": "T"}));
        assert_eq!(
            single_event(lastfm_scrobble(&record).unwrap()).details["artist"],
            "Étendu"
        );
    }

    #[test]
    fn the_track_playing_now_is_not_a_listen() {
        let record = record(json!({"artist": {"#text": "A"}, "name": "T",
                                   "@attr": {"nowplaying": "true"}}));
        assert_eq!(lastfm_scrobble(&record).unwrap(), vec![]);
    }

    #[test]
    fn an_unusable_scrobble_is_an_error_that_cites_nothing() {
        for payload in [
            json!({}),
            json!(null),
            json!({"artist": {"#text": ""}, "name": "T"}),
            json!({"artist": {"#text": "A"}}),
            json!({"artist": {"#text": "A"}, "name": 3}),
        ] {
            assert!(lastfm_scrobble(&record(payload)).is_err());
        }
    }

    #[test]
    fn a_track_info_gives_a_duration_in_seconds_under_the_key_of_the_scrobble() {
        let record = record(
            json!({"artist": "DAFT punk", "track": "Veridis Quo (Live)", "info": {
                "track": {"name": "Corrigé par Last.fm", "duration": "345500",
                          "artist": {"name": "Autre"}},
            }}),
        );
        let event = single_event(lastfm_track_info(&record).unwrap());
        assert_eq!(event.metric, "music.track_duration");
        assert_eq!(event.value.as_deref(), Some("345.500"));
        assert_eq!(event.items[0].level, "track");
        assert_eq!(event.items[0].item_key, "daft punk\tveridis quo");
        assert_eq!(event.occurred_at, record.occurred_at);
    }

    #[test]
    fn a_duration_may_be_a_number_and_may_be_missing() {
        let with = |duration: Value| {
            let payload =
                json!({"artist": "A", "track": "T", "info": {"track": {"duration": duration}}});
            lastfm_track_info(&record(payload)).unwrap()
        };
        assert_eq!(with(json!(180000)).len(), 1);
        assert_eq!(with(json!("0")), vec![]);
        assert_eq!(with(json!(0)), vec![]);
        assert_eq!(with(json!("")), vec![]);
        assert_eq!(with(json!(null)), vec![]);
        assert_eq!(with(json!(-1)), vec![]);
        let absent = json!({"artist": "A", "track": "T", "info": {"track": {}}});
        assert_eq!(lastfm_track_info(&record(absent)).unwrap(), vec![]);
        for bad in [
            json!({}),
            json!({"artist": "A", "track": "T"}),
            json!({"artist": "A", "info": {"track": {}}}),
        ] {
            assert!(lastfm_track_info(&record(bad)).is_err());
        }
    }

    #[test]
    fn a_mark_gives_items_at_its_own_level_under_the_key_of_the_listens() {
        let items = |payload: Value| {
            let event = single_event(music_mark_known(&record(payload)).unwrap());
            assert_eq!(event.metric, "music.known");
            event
                .items
                .iter()
                .map(|i| (i.level, i.position, i.item_key.clone(), i.display.clone()))
                .collect::<Vec<_>>()
        };
        let item = |payload: Value| {
            let mut found = items(payload);
            assert_eq!(found.len(), 1);
            let (level, _, key, display) = found.remove(0);
            (level, key, display)
        };
        let artist = "DAFT PUNK feat. X";
        assert_eq!(
            items(json!({"level": "artist", "artist": artist, "album": "Discovery", "track": "T"})),
            [
                (
                    "artist",
                    0,
                    music::artist_key("Daft Punk"),
                    "DAFT PUNK".to_owned()
                ),
                ("artist", 1, "x".to_owned(), "X".to_owned()),
            ]
        );
        assert_eq!(
            item(json!({"level": "artist", "artist": "Simon & Garfunkel"})),
            (
                "artist",
                "simon & garfunkel".to_owned(),
                "Simon & Garfunkel".to_owned()
            )
        );
        assert_eq!(
            item(json!({"level": "album", "artist": artist, "album": "Discovery"})),
            (
                "album",
                music::album_key("Daft Punk", "discovery"),
                "Discovery".to_owned()
            )
        );
        assert_eq!(
            item(json!({"level": "track", "artist": artist, "track": "Voyager (Live)"})),
            (
                "track",
                music::track_key("Daft Punk", "Voyager"),
                "Voyager (Live)".to_owned()
            )
        );
    }

    #[test]
    fn an_unusable_mark_is_an_error_that_cites_nothing() {
        for payload in [
            json!({}),
            json!(null),
            json!({"level": "artist"}),
            json!({"level": "artist", "artist": "  "}),
            json!({"level": "artist", "artist": 3}),
            json!({"level": "artist", "artist": "x".repeat(1025)}),
            json!({"level": "genre", "artist": "A"}),
            json!({"artist": "A"}),
            json!({"level": "album", "artist": "A"}),
            json!({"level": "album", "artist": "A", "track": "T"}),
            json!({"level": "track", "artist": "A", "album": "L"}),
            json!({"level": "track", "track": "T"}),
        ] {
            let failure = music_mark_known(&record(payload)).unwrap_err();
            assert!(failure.0.starts_with("music.mark_known: invalid "));
        }
    }

    fn apple_row(start: i64, end: i64, media: i64, played: i64) -> Value {
        json!({"song": "Titre fictif", "album": "Album fictif", "artist": "Artiste Fictif",
               "artist_source": "library_album", "play_duration_ms": played,
               "media_duration_ms": media, "start_position_ms": start, "end_position_ms": end})
    }

    #[test]
    fn a_listen_crosses_half_the_track_or_four_minutes_once() {
        let listen = |start, end, media, played| {
            let outputs = apple_play_history_row(&record(apple_row(start, end, media, played)));
            !outputs.unwrap().is_empty()
        };
        for (case, expected, start, end, media) in [
            ("en entier", true, 0, 200_000, 200_000),
            ("jusqu'à la moitié", true, 0, 100_000, 200_000),
            ("juste avant la moitié", false, 0, 99_999, 200_000),
            ("4 min d'un titre de 20", true, 0, 240_000, 1_200_000),
            ("3 min d'un titre de 20", false, 0, 180_000, 1_200_000),
            ("reprise qui franchit", true, 80_000, 150_000, 200_000),
            ("reprise après le seuil", false, 100_000, 200_000, 200_000),
            ("saut par-dessus le seuil", false, 150_000, 200_000, 200_000),
            ("titre de 30 s", false, 0, 30_000, 30_000),
            ("titre de 31 s", true, 0, 31_000, 31_000),
        ] {
            let played = end - start;
            assert_eq!(listen(start, end, media, played), expected, "{case}");
        }
        assert!(!listen(0, 200_000, 200_000, 0), "aucune lecture");
        assert!(!listen(0, 200_000, 200_000, -5), "durée négative");
    }

    #[test]
    fn pause_and_resume_count_once_and_a_replay_counts_again() {
        let segments = [(0, 60_000), (60_000, 120_000), (120_000, 200_000)];
        let listens = segments
            .iter()
            .filter(|(s, e)| {
                !apple_play_history_row(&record(apple_row(*s, *e, 200_000, e - s)))
                    .unwrap()
                    .is_empty()
            })
            .count();
        assert_eq!(listens, 1);
    }

    #[test]
    fn an_apple_listen_has_the_keys_of_a_scrobble_and_a_bounded_duration() {
        let event = single_event(
            apple_play_history_row(&record(apple_row(0, 200_000, 200_000, 170_000_000))).unwrap(),
        );
        assert_eq!(event.metric, "music.listen");
        assert_eq!(event.details["played_ms"], 200_000);
        let scrobble = record(json!({"artist": {"#text": "ARTISTE FICTIF"},
            "name": "Titre fictif (Live)", "album": {"#text": "Album fictif"}}));
        let from_lastfm = single_event(lastfm_scrobble(&scrobble).unwrap());
        assert_eq!(event.details["track_key"], from_lastfm.details["track_key"]);
        assert_eq!(event.details["album_key"], from_lastfm.details["album_key"]);
        assert_eq!(from_lastfm.details.get("played_ms"), None);
    }

    #[test]
    fn a_listen_without_artist_is_kept_with_no_item_to_invent() {
        for artist in [json!(null), json!(""), json!("  ")] {
            let mut payload = apple_row(0, 200_000, 200_000, 200_000);
            payload["artist"] = artist;
            let event = single_event(apple_play_history_row(&record(payload)).unwrap());
            assert_eq!(event.details["artist"], Value::Null);
            assert_eq!(event.details["track"], "Titre fictif");
            assert_eq!(event.details["track_key"], Value::Null);
            assert_eq!(event.details["played_ms"], 200_000);
            assert!(event.items.is_empty());
        }
    }

    #[test]
    fn an_unusable_apple_row_is_an_error_that_cites_nothing() {
        for payload in [
            json!({}),
            json!(null),
            json!({"song": "T"}),
            json!({"play_duration_ms": 3}),
        ] {
            assert!(apple_play_history_row(&record(payload)).is_err());
        }
        let without_positions = json!({"song": "T", "play_duration_ms": 3});
        assert_eq!(
            apple_play_history_row(&record(without_positions)).unwrap(),
            vec![]
        );
    }

    // ─── Garmin ──────────────────────────────────────────────────────────────

    fn garmin_record(payload: Value) -> Record {
        Record {
            occurred_at: "2026-11-03T11:00:00Z".parse().unwrap(),
            utc_offset_minutes: Some(60),
            ..record(payload)
        }
    }

    #[test]
    fn a_daily_summary_gives_the_steps_and_the_calories_of_its_day() {
        let record = garmin_record(json!({
            "calendar_date": "2026-11-03", "steps": 8412, "distance_m": 6123.5,
            "calories_total": 2350, "calories_active": 512, "calories_resting": 1838,
        }));
        let outputs = garmin_daily_summary(&record).unwrap();
        let events: Vec<&Event> = outputs
            .iter()
            .map(|o| match o {
                Output::Event(e) => e,
                Output::Cancellation { .. } => panic!("pas d'annulation"),
            })
            .collect();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].metric, "steps");
        assert_eq!(events[0].value.as_deref(), Some("8412"));
        assert_eq!(events[0].occurred_at, record.occurred_at);
        assert_eq!(events[0].utc_offset_minutes, Some(60));
        assert_eq!(events[0].details["distance_m"], json!(6123.5));
        assert!(events[0].items.is_empty());
        assert_eq!(events[1].metric, "calories.burned");
        assert_eq!(events[1].value.as_deref(), Some("2350"));
        assert_eq!(events[1].details["active"], 512);
        assert_eq!(events[1].details["resting"], 1838);
    }

    #[test]
    fn unknown_calories_are_no_event_and_zero_steps_are_a_value() {
        let record = garmin_record(json!({"calendar_date": "2026-11-03", "steps": 0,
                                          "calories_total": null}));
        let event = single_event(garmin_daily_summary(&record).unwrap());
        assert_eq!(event.metric, "steps");
        assert_eq!(event.value.as_deref(), Some("0"));
        assert_eq!(event.details["distance_m"], Value::Null);
    }

    #[test]
    fn an_unusable_daily_summary_is_an_error_that_cites_nothing() {
        for payload in [
            json!({}),
            json!(null),
            json!({"calendar_date": "2026-11-03"}),
            json!({"calendar_date": "2026-11-03", "steps": "8412"}),
            json!({"calendar_date": "2026-11-03", "steps": -3}),
            json!({"calendar_date": "3 nov.", "steps": 3}),
            json!({"calendar_date": "2026-11-03", "steps": 3, "calories_total": "x"}),
        ] {
            let failure = garmin_daily_summary(&garmin_record(payload)).unwrap_err();
            assert!(failure.0.starts_with("garmin"), "{failure:?}");
        }
    }

    #[test]
    fn a_night_is_a_measure_in_minutes_with_its_stages() {
        let record = garmin_record(json!({
            "calendar_date": "2026-11-03", "start": "2026-11-02T22:41:00Z",
            "end": "2026-11-03T06:05:00Z", "sleep_seconds": 25_980, "deep_seconds": 4_920,
            "light_seconds": 14_400, "rem_seconds": 6_660, "awake_seconds": 660,
        }));
        let event = single_event(garmin_sleep(&record).unwrap());
        assert_eq!(event.metric, "sleep");
        assert_eq!(event.value.as_deref(), Some("433.000"));
        assert_eq!(event.occurred_at, record.occurred_at);
        assert_eq!(event.details["start"], "2026-11-02T22:41:00+00:00");
        assert_eq!(event.details["end"], "2026-11-03T06:05:00+00:00");
        assert_eq!(event.details["rem_seconds"], 6_660);
        assert!(event.items.is_empty());
        assert_eq!(
            event.details.get("lowest_hr"),
            None,
            "une nuit sans fréquence cardiaque"
        );
        assert_eq!(event.details.get("avg_hr"), None);
        for payload in [
            json!({"calendar_date": "2026-11-03"}),
            json!({"calendar_date": "2026-11-03", "sleep_seconds": 0}),
            json!({"calendar_date": "2026-11-03", "sleep_seconds": 60, "start": "hier"}),
            json!({"sleep_seconds": 60}),
        ] {
            assert!(garmin_sleep(&garmin_record(payload)).is_err());
        }
    }

    #[test]
    fn a_night_with_heart_rate_gives_its_lowest_and_its_rounded_average() {
        let night = |heart_rate: Value| {
            garmin_record(json!({
                "calendar_date": "2026-11-03", "start": "2026-11-02T22:41:00Z",
                "end": "2026-11-03T06:05:00Z", "sleep_seconds": 25_980, "heart_rate": heart_rate,
            }))
        };
        let record = night(json!([[60, 58], [180, 48], [300, 55], [420, 60]]));
        let event = single_event(garmin_sleep(&record).unwrap());
        assert_eq!(event.details["lowest_hr"], 48);
        assert_eq!(event.details["avg_hr"], 55, "55,25 arrondi");
        assert_eq!(
            event.details.get("heart_rate"),
            None,
            "la série reste au journal"
        );
        assert_eq!(event.value.as_deref(), Some("433.000"));
        // Toujours le même résultat : un rejeu n'y voit aucune différence.
        assert_eq!(single_event(garmin_sleep(&record).unwrap()), event);
        let event = single_event(garmin_sleep(&night(json!([[0, 50.5], [120, 51]]))).unwrap());
        assert_eq!(event.details["lowest_hr"], json!(50.5));
        assert_eq!(event.details["avg_hr"], 51);
        for empty in [json!([]), json!(null)] {
            let event = single_event(garmin_sleep(&night(empty)).unwrap());
            assert_eq!(event.details.get("lowest_hr"), None);
        }
        for invalid in [
            json!("48"),
            json!([[60]]),
            json!([[60, 0]]),
            json!([[60, -3]]),
            json!([[60, "48"]]),
            json!([["x", 48]]),
            json!([[-60, 48]]),
            json!([[60, 48, 1]]),
        ] {
            assert_eq!(
                garmin_sleep(&night(invalid)).unwrap_err(),
                InterpretError("garmin.sleep: invalid heart_rate")
            );
        }
    }

    #[test]
    fn a_workout_has_its_type_as_item_and_its_distance_in_km() {
        let record = garmin_record(json!({
            "activity_id": 98_765_432_101_u64, "type": "running", "start": "2026-11-03T06:30:00Z",
            "duration_s": 1_802.4, "moving_s": 1_780, "distance_m": 5_012.3,
            "avg_speed_mps": 2.78, "avg_hr": 151, "max_hr": 176, "calories": 389,
        }));
        let event = single_event(garmin_activity(&record).unwrap());
        assert_eq!(event.metric, "workout");
        assert_eq!(event.value.as_deref(), Some("5.012"));
        assert_eq!(event.occurred_at, record.occurred_at);
        assert_eq!(
            event.items,
            vec![Item::single(
                "activity_type",
                "running".to_owned(),
                "running"
            )]
        );
        assert_eq!(event.details["activity_id"], 98_765_432_101_u64);
        assert_eq!(event.details["avg_hr"], 151);
        assert_eq!(event.details["duration_s"], json!(1_802.4));
        let keys: Vec<&str> = event
            .details
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            [
                "activity_id",
                "avg_hr",
                "avg_speed_mps",
                "calories",
                "distance_m",
                "duration_s",
                "end_lat",
                "end_lon",
                "location_name",
                "max_hr",
                "moving_s",
                "start_lat",
                "start_lon",
                "type"
            ]
        );
    }

    #[test]
    fn a_workout_without_distance_has_no_value_and_an_unusable_one_is_an_error() {
        for distance in [json!(null), json!(0)] {
            let record = garmin_record(json!({"activity_id": 1, "type": "strength_training",
                                              "distance_m": distance}));
            assert_eq!(single_event(garmin_activity(&record).unwrap()).value, None);
        }
        // Un champ hors liste blanche ne passe jamais dans les détails.
        let record = garmin_record(json!({"activity_id": 1, "type": "walking",
                                          "startLatitude": 48.85, "ownerFullName": "X"}));
        let event = single_event(garmin_activity(&record).unwrap());
        assert_eq!(event.details.get("startLatitude"), None);
        assert_eq!(event.details.get("ownerFullName"), None);
        for payload in [
            json!({}),
            json!({"type": "running"}),
            json!({"activity_id": 1}),
            json!({"activity_id": 1, "type": " "}),
            json!({"activity_id": 1, "type": "x".repeat(65)}),
            json!({"activity_id": 1, "type": "running", "avg_hr": "151"}),
        ] {
            assert!(garmin_activity(&garmin_record(payload)).is_err());
        }
    }

    #[test]
    fn a_workout_has_its_place_as_item_and_keeps_its_coordinates_in_details() {
        let record = garmin_record(json!({
            "activity_id": 7, "type": "running", "start_lat": 45.123_456, "start_lon": -1.5,
            "end_lat": 45.2, "end_lon": -1.4, "location_name": "  Ville   Fictive ",
        }));
        let event = single_event(garmin_activity(&record).unwrap());
        assert_eq!(
            event.items[1],
            Item::single("place", "ville fictive".to_owned(), "Ville   Fictive")
        );
        assert_eq!(event.details["start_lat"], json!(45.123_456));
        assert_eq!(event.details["end_lon"], json!(-1.4));
        for payload in [
            json!({"activity_id": 7, "type": "running", "start_lat": 91}),
            json!({"activity_id": 7, "type": "running", "end_lon": "x"}),
            json!({"activity_id": 7, "type": "running", "location_name": 3}),
        ] {
            assert!(garmin_activity(&garmin_record(payload)).is_err());
        }
    }

    #[test]
    fn a_track_is_a_reference_of_its_workout() {
        let record = garmin_record(json!({"activity_id": 7,
            "points": [[45.1, -1.5, 0], [45.2, -1.4, 600]]}));
        let event = single_event(garmin_activity_track(&record).unwrap());
        assert_eq!(event.metric, "workout.track");
        assert_eq!(event.value, None);
        assert_eq!(event.details["activity_id"], 7);
        assert_eq!(event.details["points"][1], json!([45.2, -1.4, 600]));
        for payload in [
            json!({"activity_id": 7, "points": [[45.1, -1.5, 0]]}),
            json!({"activity_id": 7, "points": [[45.1, -1.5, 0], [95.0, 0, 1]]}),
            json!({"activity_id": 7, "points": [[45.1, -1.5], [45.2, -1.4]]}),
            json!({"points": [[45.1, -1.5, 0], [45.2, -1.4, 600]]}),
        ] {
            assert!(garmin_activity_track(&garmin_record(payload)).is_err());
        }
    }

    #[test]
    fn a_watch_has_its_channel_and_its_video_as_items() {
        let record = record(json!({"video_id": "abcdefghijk", "title": " Un titre ",
            "channel_id": "UC1", "channel_name": "Une chaîne", "extra": 1}));
        let event = single_event(youtube_watch(&record).unwrap());
        assert_eq!(event.metric, "video.watch");
        assert_eq!(event.value, None);
        assert_eq!(event.occurred_at, record.occurred_at);
        assert_eq!(
            event.items,
            vec![
                Item::single("channel", "UC1".to_owned(), "Une chaîne"),
                Item::single("video", "abcdefghijk".to_owned(), "Un titre"),
            ]
        );
        assert_eq!(
            event.details,
            json!({"video_id": "abcdefghijk", "title": "Un titre", "channel_id": "UC1",
                   "channel_name": "Une chaîne"})
        );
    }

    #[test]
    fn a_deleted_video_has_no_channel_and_its_id_as_name() {
        let event = single_event(
            youtube_watch(&record(json!({"video_id": "abcdefghijk", "title": null,
                "channel_id": null, "channel_name": null})))
            .unwrap(),
        );
        assert_eq!(
            event.items,
            vec![Item::single(
                "video",
                "abcdefghijk".to_owned(),
                "abcdefghijk"
            )]
        );
        // Un nom de chaîne sans id ne se rattache à rien.
        let event = single_event(
            youtube_watch(&record(
                json!({"video_id": "abcdefghijk", "channel_name": "A"}),
            ))
            .unwrap(),
        );
        assert_eq!(event.details["channel_name"], Value::Null);
        assert_eq!(event.items.len(), 1);
    }

    #[test]
    fn an_unusable_watch_is_an_error_that_cites_nothing() {
        for payload in [
            json!({}),
            json!(null),
            json!({"video_id": "court"}),
            json!({"video_id": "abcdefghij!"}),
            json!({"video_id": 3}),
            json!({"video_id": "abcdefghijk", "channel_id": "a b"}),
            json!({"video_id": "abcdefghijk", "channel_id": 3}),
            json!({"video_id": "abcdefghijk", "title": 3}),
        ] {
            let err = youtube_watch(&record(payload.clone())).unwrap_err();
            assert!(err.0.starts_with("youtube"), "{payload}");
        }
    }

    #[test]
    fn a_video_info_gives_a_duration_in_seconds_when_known() {
        let event = single_event(
            youtube_video_info(&record(
                json!({"video_id": "abcdefghijk", "duration_s": 549,
                "title": "Un titre", "channel_id": "UC1", "channel_name": "Une chaîne"}),
            ))
            .unwrap(),
        );
        assert_eq!(event.metric, "video.duration");
        assert_eq!(event.value.as_deref(), Some("549"));
        assert_eq!(
            event.items,
            vec![Item::single("video", "abcdefghijk".to_owned(), "Un titre")]
        );
        for unknown in [json!(null), json!(0)] {
            let payload = json!({"video_id": "abcdefghijk", "duration_s": unknown});
            assert_eq!(youtube_video_info(&record(payload)).unwrap(), vec![]);
        }
        for payload in [
            json!({"duration_s": 3}),
            json!({"video_id": "abcdefghijk", "duration_s": -3}),
            json!({"video_id": "abcdefghijk", "duration_s": "3"}),
        ] {
            assert!(youtube_video_info(&record(payload)).is_err());
        }
    }

    #[test]
    fn a_twitch_segment_has_its_channel_and_its_played_time() {
        let record = record(
            json!({"channel": "streamer_1", "channel_name": " Streamer_1 ",
            "content": "live", "duration_ms": 60_000, "category": "Just Chatting", "extra": 1}),
        );
        let event = single_event(twitch_watch(&record).unwrap());
        assert_eq!(event.metric, "stream.watch");
        assert_eq!(event.value, None);
        assert_eq!(event.occurred_at, record.occurred_at);
        assert_eq!(
            event.items,
            vec![Item::single(
                "channel",
                "streamer_1".to_owned(),
                "Streamer_1"
            )]
        );
        assert_eq!(
            event.details,
            json!({"channel": "streamer_1", "channel_name": "Streamer_1", "content": "live",
                   "category": "Just Chatting", "played_ms": 60_000})
        );
        // Sans nom, le login ; à la limite d'une heure, accepté.
        let event = single_event(
            twitch_watch(&self::record(json!({"channel": "abc", "content": "clip",
                "duration_ms": MAX_MEASURED_MS})))
            .unwrap(),
        );
        assert_eq!(
            event.items,
            vec![Item::single("channel", "abc".to_owned(), "abc")]
        );
        assert_eq!(event.details["category"], Value::Null);
    }

    #[test]
    fn an_unknown_twitch_channel_has_no_item_but_keeps_its_time() {
        let event = single_event(
            twitch_watch(&record(json!({"channel": null, "channel_name": "Nom seul",
                "content": "vod", "duration_ms": 1, "category": null})))
            .unwrap(),
        );
        assert_eq!(event.items, vec![]);
        assert_eq!(event.details["channel_name"], Value::Null);
        assert_eq!(event.details["played_ms"], 1);
    }

    #[test]
    fn an_unusable_twitch_segment_is_an_error_that_cites_nothing() {
        let ok = json!({"channel": "abc", "content": "live", "duration_ms": 1000});
        let with = |field: &str, value: Value| {
            let mut payload = ok.clone();
            payload[field] = value;
            payload
        };
        for payload in [
            json!({}),
            json!(null),
            with("channel", json!("Majuscules")),
            with("channel", json!("a-b")),
            with("channel", json!("")),
            with("channel", json!("a".repeat(26))),
            with("channel", json!(3)),
            with("channel_name", json!(3)),
            with("channel_name", json!("x".repeat(201))),
            with("content", json!("short")),
            with("content", Value::Null),
            with("duration_ms", json!(0)),
            with("duration_ms", json!(-5)),
            with("duration_ms", json!(MAX_MEASURED_MS + 1)),
            with("duration_ms", json!(1.5)),
            with("duration_ms", json!("1000")),
            with("duration_ms", Value::Null),
            with("category", json!(["x"])),
        ] {
            let err = twitch_watch(&record(payload.clone())).unwrap_err();
            assert!(err.0.starts_with("twitch.watch: invalid"), "{payload}");
        }
    }

    /// Un relevé Steam observé le 13 oct. 2026 à 07:42:10 UTC (l'enveloppe de `record`).
    fn playtime(total: i64, last_played: Value, previous: Value) -> Record {
        record(
            json!({"appid": 620, "name": " Jeu Fictif ", "playtime_forever": total,
            "playtime_disconnected": 0, "rtime_last_played": last_played, "previous": previous}),
        )
    }

    fn plays(outputs: &[Output]) -> Vec<(DateTime<Utc>, i64)> {
        outputs
            .iter()
            .filter_map(|o| match o {
                Output::Event(e) if e.metric == "game.play" => {
                    Some((e.occurred_at, e.details["played_ms"].as_i64().unwrap()))
                }
                _ => None,
            })
            .collect()
    }

    fn at(text: &str) -> DateTime<Utc> {
        text.parse().unwrap()
    }

    fn metrics(outputs: &[Output]) -> Vec<&str> {
        outputs
            .iter()
            .map(|o| match o {
                Output::Event(e) => e.metric,
                _ => panic!("un événement attendu"),
            })
            .collect()
    }

    #[test]
    fn a_first_steam_reading_gives_the_total_and_marks_a_played_game_known() {
        let outputs = steam_playtime(&playtime(125, json!(1_700_000_000), Value::Null)).unwrap();
        assert_eq!(metrics(&outputs), ["game.total_playtime", "game.known"]);
        for output in &outputs {
            let Output::Event(event) = output else {
                panic!()
            };
            assert_eq!(event.occurred_at, at("2026-10-13T07:42:10Z"));
            assert_eq!(
                event.items,
                vec![Item::single("game", "620".to_owned(), "Jeu Fictif")]
            );
            assert_eq!(event.details, json!({}));
        }
        let Output::Event(total) = &outputs[0] else {
            panic!()
        };
        assert_eq!(total.value.as_deref(), Some("125"));
        let Output::Event(known) = &outputs[1] else {
            panic!()
        };
        assert_eq!(known.value, None);
        // Un jeu jamais lancé au premier relevé n'est pas connu : sa première partie sera une découverte.
        let outputs = steam_playtime(&playtime(0, Value::Null, Value::Null)).unwrap();
        assert_eq!(metrics(&outputs), ["game.total_playtime"]);
    }

    #[test]
    fn steam_play_time_ends_at_the_last_played_time_within_the_window() {
        // Relevé précédent à 07:00, dernière partie à 07:30 : 40 min de 06:50 à 07:30.
        let last = at("2026-10-13T07:30:00Z").timestamp();
        let previous = json!({"playtime_forever": 85, "observed_at": "2026-10-13T07:00:00Z"});
        let outputs = steam_playtime(&playtime(125, json!(last), previous.clone())).unwrap();
        assert_eq!(outputs.len(), 2);
        assert_eq!(plays(&outputs), [(at("2026-10-13T06:50:00Z"), 40 * 60_000)]);
        let Output::Event(event) = &outputs[1] else {
            panic!()
        };
        assert_eq!(event.details, json!({"played_ms": 2_400_000}));
        assert_eq!(event.items[0].item_key, "620");
        // Une dernière partie hors de la fenêtre (avant le relevé précédent, ou jamais) : l'heure du relevé.
        for last in [
            json!(at("2026-10-13T06:00:00Z").timestamp()),
            json!(0),
            Value::Null,
        ] {
            let outputs = steam_playtime(&playtime(125, last, previous.clone())).unwrap();
            assert_eq!(plays(&outputs), [(at("2026-10-13T07:02:10Z"), 40 * 60_000)]);
        }
    }

    #[test]
    fn a_long_steam_gap_is_split_into_chunks_back_to_back() {
        let previous = json!({"playtime_forever": 0, "observed_at": "2026-10-13T01:00:00Z"});
        let outputs = steam_playtime(&playtime(150, Value::Null, previous)).unwrap();
        assert_eq!(
            plays(&outputs),
            [
                (at("2026-10-13T05:12:10Z"), MAX_MEASURED_MS),
                (at("2026-10-13T06:12:10Z"), MAX_MEASURED_MS),
                (at("2026-10-13T07:12:10Z"), 30 * 60_000),
            ]
        );
    }

    #[test]
    fn no_steam_play_without_a_positive_plausible_gap() {
        for (total, before) in [(100, 100), (90, 100)] {
            let previous =
                json!({"playtime_forever": before, "observed_at": "2026-10-01T00:00:00Z"});
            let outputs = steam_playtime(&playtime(total, Value::Null, previous)).unwrap();
            assert_eq!(
                metrics(&outputs),
                ["game.total_playtime"],
                "{total} après {before}"
            );
        }
        // Plus de deux semaines d'un coup : joué hors de notre vue, un point de départ déjà connu.
        let previous = json!({"playtime_forever": 0, "observed_at": "2026-10-01T00:00:00Z"});
        let outputs = steam_playtime(&playtime(
            MAX_STEAM_DELTA_MINUTES + 1,
            Value::Null,
            previous,
        ))
        .unwrap();
        assert_eq!(metrics(&outputs), ["game.total_playtime", "game.known"]);
        let previous = json!({"playtime_forever": 0, "observed_at": "2026-10-01T00:00:00Z"});
        let outputs =
            steam_playtime(&playtime(MAX_STEAM_DELTA_MINUTES, Value::Null, previous)).unwrap();
        assert_eq!(plays(&outputs).len(), 14 * 24);
    }

    #[test]
    fn an_unusable_steam_reading_is_an_error_that_cites_nothing() {
        let ok = json!({"appid": 620, "name": "Jeu", "playtime_forever": 10,
            "playtime_disconnected": 0, "rtime_last_played": 0,
            "previous": {"playtime_forever": 5, "observed_at": "2026-10-13T07:00:00Z"}});
        assert!(steam_playtime(&record(ok.clone())).is_ok());
        let with = |field: &str, value: Value| {
            let mut payload = ok.clone();
            payload[field] = value;
            payload
        };
        for payload in [
            json!({}),
            json!(null),
            with("appid", json!(0)),
            with("appid", json!("620")),
            with("name", json!(3)),
            with("name", json!("x".repeat(201))),
            with("playtime_forever", json!(-1)),
            with("playtime_forever", json!(1.5)),
            with("playtime_forever", Value::Null),
            with("playtime_disconnected", json!(-1)),
            with("rtime_last_played", json!(-5)),
            with("rtime_last_played", json!("1700000000")),
            with("previous", json!(5)),
            with("previous", json!({"playtime_forever": 5})),
            with(
                "previous",
                json!({"playtime_forever": 5, "observed_at": "hier"}),
            ),
            // Un relevé précédent qui n'est pas avant celui-ci.
            with(
                "previous",
                json!({"playtime_forever": 5, "observed_at": "2026-10-13T07:42:10Z"}),
            ),
        ] {
            let err = steam_playtime(&record(payload.clone())).unwrap_err();
            assert!(err.0.starts_with("steam.playtime: invalid"), "{payload}");
        }
    }

    #[test]
    fn a_steam_achievement_has_its_game_and_its_key_within_the_game() {
        let event = single_event(
            steam_achievement(&record(json!({"appid": 620, "game_name": "Jeu Fictif",
                "apiname": "ACH_FICTIF", "name": " Succès fictif ", "description": "écartée"})))
            .unwrap(),
        );
        assert_eq!(event.metric, "game.achievement");
        assert_eq!(event.occurred_at, at("2026-10-13T07:42:10Z"));
        assert_eq!(
            event.items,
            vec![
                Item::single("game", "620".to_owned(), "Jeu Fictif"),
                Item::single("achievement", "620:ACH_FICTIF".to_owned(), "Succès fictif"),
            ]
        );
        assert_eq!(event.details, json!({}));
        // Sans noms : l'appid et l'identifiant.
        let event = single_event(
            steam_achievement(&record(
                json!({"appid": 7, "game_name": null, "apiname": "1",
                "name": ""}),
            ))
            .unwrap(),
        );
        assert_eq!(event.items[0].display, "7");
        assert_eq!(event.items[1].display, "1");
        for payload in [
            json!({}),
            json!({"appid": 7, "apiname": ""}),
            json!({"appid": 7, "apiname": 3}),
            json!({"appid": 7, "apiname": "a".repeat(129)}),
            json!({"appid": 7, "apiname": "a\nb"}),
            json!({"appid": -7, "apiname": "a"}),
            json!({"appid": 7, "apiname": "a", "name": ["x"]}),
        ] {
            let err = steam_achievement(&record(payload.clone())).unwrap_err();
            assert!(err.0.starts_with("steam.achievement: invalid"), "{payload}");
        }
    }

    #[test]
    fn an_overland_point_keeps_its_whitelist_and_drops_what_ios_calls_invalid() {
        let event = single_event(
            overland_location(&record(json!({
                "lat": 43.3, "lon": 5.3, "horizontal_accuracy": 8, "altitude": 12,
                "speed": -1, "motion": ["walking", "secret", 3],
            })))
            .unwrap(),
        );
        assert_eq!(event.metric, "location.point");
        assert_eq!(event.value, None);
        assert_eq!(
            event.details,
            json!({"point_lat": 43.3, "point_lon": 5.3, "horizontal_accuracy": 8,
                   "altitude": 12, "speed": null, "motion": ["walking"]})
        );
        let invalid = overland_location(&record(
            json!({"lat": 1, "lon": 2, "horizontal_accuracy": -1}),
        ));
        assert_eq!(invalid, Ok(Vec::new()));
        for payload in [
            json!({"lon": 5.3}),
            json!({"lat": 91, "lon": 5.3}),
            json!({"lat": 43.3, "lon": "5.3"}),
            json!({"lat": 43.3, "lon": 5.3, "speed": "fast"}),
            json!({"lat": 43.3, "lon": 5.3, "motion": "walking"}),
        ] {
            assert_eq!(
                overland_location(&record(payload)),
                Err(InterpretError("overland.location: invalid point"))
            );
        }
    }

    #[test]
    fn an_overland_visit_is_a_place_with_its_duration() {
        let event = single_event(
            overland_visit(&record(json!({
                "lat": 43.3, "lon": 5.3, "arrival": "2026-10-13T07:42:10Z",
                "departure": "2026-10-13T09:12:10Z", "horizontal_accuracy": 35,
            })))
            .unwrap(),
        );
        assert_eq!(event.metric, "place.visit");
        assert_eq!(event.value, None);
        assert_eq!(event.details["visit_lat"], json!(43.3));
        assert_eq!(event.details["duration_s"], json!(5400));
        assert_eq!(event.details["departure"], json!("2026-10-13T09:12:10Z"));
        for payload in [
            json!({"lat": 43.3, "lon": 5.3, "arrival": "2026-10-13T07:42:10Z"}),
            json!({"lat": 43.3, "lon": 5.3, "arrival": "2026-10-13T07:42:10Z",
                   "departure": "2026-10-13T07:00:00Z"}),
            json!({"lon": 5.3, "arrival": "2026-10-13T07:42:10Z",
                   "departure": "2026-10-13T09:12:10Z"}),
        ] {
            assert_eq!(
                overland_visit(&record(payload)),
                Err(InterpretError("overland.visit: invalid visit"))
            );
        }
    }

    fn github_repo_payload() -> Value {
        json!({"id": "R_kgDOFICTIF", "name": "moi/depot-fictif"})
    }

    fn item_keys(event: &Event) -> Vec<(&str, &str, &str)> {
        event
            .items
            .iter()
            .map(|i| (i.level, i.item_key.as_str(), i.display.as_str()))
            .collect()
    }

    #[test]
    fn a_github_commit_keeps_its_whitelist_under_its_repo() {
        let event = single_event(
            github_commit(&record(json!({
                "repo": github_repo_payload(), "oid": "ABCDEF0123456789abcdef0123456789abcdef01",
                "authored_at": "2026-10-13T07:42:10Z", "committed_at": "2026-10-14T07:00:00Z",
                "title": "  Corrige le compteur\u{0007}  ", "additions": 12, "deletions": 3,
                "url": "https://github.com/moi/depot-fictif/commit/abc", "body": "jamais recopié",
            })))
            .unwrap(),
        );
        assert_eq!(event.metric, "code.commit");
        assert_eq!(event.occurred_at, at("2026-10-13T07:42:10Z"), "l'enveloppe");
        assert_eq!(event.value, None);
        assert_eq!(
            event.details,
            json!({"sha": "abcdef0123456789abcdef0123456789abcdef01",
                   "title": "Corrige le compteur", "additions": 12, "deletions": 3,
                   "url": "https://github.com/moi/depot-fictif/commit/abc"})
        );
        assert_eq!(
            item_keys(&event),
            [("repo", "R_kgDOFICTIF", "moi/depot-fictif")]
        );
        // Un très gros commit sans lignes, un titre trop long coupé.
        let event = single_event(
            github_commit(&record(json!({
                "repo": github_repo_payload(), "oid": "abcdef0",
                "title": "é".repeat(400), "additions": null,
            })))
            .unwrap(),
        );
        assert_eq!(event.details["additions"], Value::Null);
        assert_eq!(event.details["url"], Value::Null);
        assert_eq!(
            event.details["title"].as_str().unwrap().chars().count(),
            MAX_GITHUB_TITLE_CHARS
        );
        for payload in [
            json!({"oid": "abcdef0"}),
            json!({"repo": {"id": "R_1"}, "oid": "abcdef0"}),
            json!({"repo": {"id": "R 1", "name": "a/b"}, "oid": "abcdef0"}),
            json!({"repo": github_repo_payload(), "oid": "xyz"}),
            json!({"repo": github_repo_payload(), "oid": "abcdef0", "additions": -1}),
            json!({"repo": github_repo_payload(), "oid": "abcdef0", "deletions": "3"}),
            json!({"repo": github_repo_payload(), "oid": "abcdef0", "title": 3}),
            json!({"repo": github_repo_payload(), "oid": "abcdef0", "url": "javascript:alert(1)"}),
        ] {
            let err = github_commit(&record(payload.clone())).unwrap_err();
            assert!(err.0.starts_with("github.commit: invalid"), "{payload}");
        }
    }

    #[test]
    fn an_opened_and_a_merged_pull_request_are_two_metrics() {
        let pr = |action: &str| {
            json!({"id": "PR_kwFICTIF", "action": action, "repo": github_repo_payload(),
                   "number": 42, "title": "Ajoute le tracker", "additions": 120, "deletions": 8,
                   "url": "https://github.com/moi/depot-fictif/pull/42", "body": "jamais"})
        };
        let opened = single_event(github_pull_request(&record(pr("opened"))).unwrap());
        assert_eq!(opened.metric, "code.pull_request");
        assert_eq!(
            opened.details,
            json!({"number": 42, "title": "Ajoute le tracker",
                   "url": "https://github.com/moi/depot-fictif/pull/42"})
        );
        let merged = single_event(github_pull_request(&record(pr("merged"))).unwrap());
        assert_eq!(merged.metric, "code.pull_request_merged");
        assert_eq!(merged.details["additions"], 120);
        assert_eq!(merged.details["deletions"], 8);
        assert_eq!(item_keys(&merged), item_keys(&opened));
        for payload in [
            pr("closed"),
            json!({"action": "opened", "repo": github_repo_payload(), "number": 1}),
            json!({"id": "PR_1", "action": "opened", "repo": github_repo_payload(), "number": 0}),
            json!({"id": "PR_1", "action": "opened", "number": 1}),
        ] {
            let err = github_pull_request(&record(payload.clone())).unwrap_err();
            assert!(
                err.0.starts_with("github.pull_request: invalid"),
                "{payload}"
            );
        }
    }

    #[test]
    fn a_review_an_issue_and_a_comment_keep_their_whitelists() {
        let review = single_event(
            github_review(&record(json!({
                "id": "PRR_1", "state": "CHANGES_REQUESTED", "repo": github_repo_payload(),
                "number": 7, "title": "La PR d'un autre", "body": "jamais",
                "url": "https://github.com/moi/depot-fictif/pull/7#pullrequestreview-1",
            })))
            .unwrap(),
        );
        assert_eq!(review.metric, "code.review");
        assert_eq!(review.details["state"], "changes_requested");
        assert_eq!(
            review
                .details
                .as_object()
                .unwrap()
                .keys()
                .collect::<Vec<_>>(),
            ["number", "state", "title", "url"]
        );
        assert!(
            github_review(&record(json!({"id": "PRR_1", "state": "PENDING",
                "repo": github_repo_payload(), "number": 7})))
            .is_err()
        );

        let issue = single_event(
            github_issue(&record(json!({
                "id": "I_1", "repo": github_repo_payload(), "number": 3, "title": "Un bogue",
                "url": "https://github.com/moi/depot-fictif/issues/3", "body": "jamais",
            })))
            .unwrap(),
        );
        assert_eq!(issue.metric, "code.issue");
        assert_eq!(
            issue.details,
            json!({"number": 3, "title": "Un bogue",
                   "url": "https://github.com/moi/depot-fictif/issues/3"})
        );

        let comment = single_event(
            github_comment(&record(json!({
                "id": "IC_1", "repo": github_repo_payload(), "target": "pull_request",
                "number": 7, "title": "La PR d'un autre", "body": "jamais",
                "url": "https://github.com/moi/depot-fictif/pull/7#issuecomment-1",
            })))
            .unwrap(),
        );
        assert_eq!(comment.metric, "code.comment");
        assert_eq!(comment.details["target"], "pull_request");
        assert_eq!(comment.details.get("body"), None);
        let on_commit = single_event(
            github_comment(&record(json!({
                "id": "CC_1", "repo": github_repo_payload(), "target": "commit",
                "number": null, "title": "Un commit",
            })))
            .unwrap(),
        );
        assert_eq!(on_commit.details["number"], Value::Null);
        for payload in [
            json!({"id": "IC_1", "repo": github_repo_payload(), "target": "issue"}),
            json!({"id": "IC_1", "repo": github_repo_payload(), "target": "discussion", "number": 1}),
            json!({"repo": github_repo_payload(), "target": "commit"}),
        ] {
            let err = github_comment(&record(payload.clone())).unwrap_err();
            assert!(err.0.starts_with("github.comment: invalid"), "{payload}");
        }
    }

    // ── Riot ────────────────────────────────────────────────────────────────

    fn events_of(outputs: Vec<Output>) -> Vec<Event> {
        outputs
            .into_iter()
            .map(|o| match o {
                Output::Event(e) => e,
                other => panic!("{other:?}"),
            })
            .collect()
    }

    fn lol_payload() -> Value {
        json!({
            "match_id": "EUW1_1000000001", "started_at": "2026-10-13T07:42:10.000Z",
            "ended_at": "2026-10-13T08:12:10.000Z", "duration_s": 1800, "queue_id": 420,
            "game_mode": "CLASSIC", "end_of_game_result": "GameComplete", "champion": "Ahri",
            "win": true, "position": "MIDDLE", "remake": false,
        })
    }

    #[test]
    fn a_lol_match_gives_its_time_its_game_known_and_the_match() {
        let events = events_of(riot_lol_match(&record(lol_payload())).unwrap());
        let metrics: Vec<&str> = events.iter().map(|e| e.metric).collect();
        assert_eq!(metrics, ["game.play", "game.known", "game.match"]);
        assert_eq!(events[0].details, json!({"played_ms": 1_800_000}));
        assert_eq!(events[0].items[0].item_key, "riot:lol");
        assert_eq!(events[0].items[0].display, "League of Legends");
        let played = &events[2];
        assert_eq!(played.value, None);
        assert_eq!(
            played.details,
            json!({"game": "lol", "match_id": "EUW1_1000000001", "queue": "Classée solo/duo",
                   "queue_id": 420, "mode": "CLASSIC", "result": "win", "character": "Ahri",
                   "position": "MIDDLE", "played_ms": 1_800_000})
        );
        assert_eq!(played.items[1].level, "character");
        assert_eq!(played.items[1].item_key, "lol:Ahri");

        // Un remake, une défaite, une file inconnue, une partie de plus d'une heure.
        let mut payload = lol_payload();
        payload["remake"] = json!(true);
        payload["win"] = json!(false);
        payload["queue_id"] = json!(3333);
        payload["duration_s"] = json!(5400);
        let events = events_of(riot_lol_match(&record(payload)).unwrap());
        let plays: Vec<_> = events
            .iter()
            .filter(|e| e.metric == "game.play")
            .map(|e| (e.occurred_at, e.details["played_ms"].as_i64().unwrap()))
            .collect();
        let start: DateTime<Utc> = "2026-10-13T07:42:10Z".parse().unwrap();
        assert_eq!(
            plays,
            [
                (start, 3_600_000),
                (start + chrono::TimeDelta::hours(1), 1_800_000)
            ]
        );
        let played = events.last().unwrap();
        assert_eq!(played.details["result"], "remake");
        assert_eq!(played.details["queue"], "File 3333");
        let mut payload = lol_payload();
        payload["win"] = json!(false);
        let events = events_of(riot_lol_match(&record(payload)).unwrap());
        assert_eq!(events.last().unwrap().details["result"], "loss");
        for (queue, mode, name) in [
            (1750, "CHERRY", "Arena"),
            (4444, "CHERRY", "Arena"),
            (4444, "ARAM", "ARAM"),
            (900, "URF", "URF"),
        ] {
            let mut payload = lol_payload();
            payload["queue_id"] = json!(queue);
            payload["game_mode"] = json!(mode);
            let events = events_of(riot_lol_match(&record(payload)).unwrap());
            assert_eq!(events.last().unwrap().details["queue"], name, "{queue}");
        }
    }

    #[test]
    fn a_tft_match_counts_my_time_until_my_elimination() {
        let events = events_of(
            riot_tft_match(&record(json!({
                "match_id": "EUW1_2000000001", "started_at": "2026-10-13T07:42:10.000Z",
                "ended_at": "2026-10-13T08:20:00.000Z", "game_length_s": 2270.4,
                "eliminated_s": 1500.6, "queue_id": 1160, "game_type": "pairs",
                "set_number": 15, "placement": 3, "partner_group_id": 2,
            })))
            .unwrap(),
        );
        assert_eq!(events[0].details["played_ms"], 1_500_600);
        let played = events.last().unwrap();
        assert_eq!(played.items.len(), 1, "pas de personnage en TFT");
        assert_eq!(played.details["queue"], "Double Up");
        assert_eq!(played.details["placement"], 3);
        assert_eq!(played.details["result"], Value::Null);
        assert_eq!(played.details["mode"], "pairs");
    }

    #[test]
    fn a_valorant_match_has_its_agent_and_a_result_from_its_rounds() {
        let payload = |won: Value, rounds_won: i64, rounds_lost: i64| {
            json!({
                "match_id": "0d3c6a4e-0000-4000-8000-000000000001",
                "started_at": "2026-10-13T07:42:10.000Z", "duration_ms": 2_100_000,
                "queue_id": "competitive", "queue_name": "Competitive", "map": "Ascent",
                "agent": "Jett", "team": "Red", "won": won,
                "rounds_won": rounds_won, "rounds_lost": rounds_lost,
            })
        };
        let result = |p: Value| {
            events_of(valorant_match(&record(p)).unwrap())
                .last()
                .unwrap()
                .details["result"]
                .clone()
        };
        assert_eq!(result(payload(json!(true), 13, 7)), "win");
        assert_eq!(result(payload(json!(false), 7, 13)), "loss");
        assert_eq!(result(payload(json!(false), 12, 12)), "draw");
        assert_eq!(result(payload(Value::Null, 0, 0)), Value::Null);
        let events = events_of(valorant_match(&record(payload(json!(true), 13, 7))).unwrap());
        let played = events.last().unwrap();
        assert_eq!(played.details["queue"], "Competitive");
        assert_eq!(played.details["map"], "Ascent");
        assert_eq!(played.items[1].item_key, "valorant:jett");
        assert_eq!(played.items[0].item_key, "riot:valorant");
    }

    #[test]
    fn an_unusable_riot_match_is_an_error_that_cites_nothing() {
        for (field, value) in [
            ("match_id", json!("EUW1 1")),
            ("match_id", Value::Null),
            ("duration_s", json!(0)),
            ("duration_s", json!(100_000)),
            ("queue_id", Value::Null),
            ("win", Value::Null),
            ("remake", json!("non")),
            ("champion", json!("x".repeat(100))),
        ] {
            let mut payload = lol_payload();
            payload[field] = value;
            let err = riot_lol_match(&record(payload)).unwrap_err();
            assert!(err.0.starts_with("riot.lol_match: invalid"), "{field}");
        }
        let tft =
            json!({"match_id": "EUW1_1", "eliminated_s": 0, "queue_id": 1100, "placement": 1});
        assert!(riot_tft_match(&record(tft)).is_err());
        let tft =
            json!({"match_id": "EUW1_1", "eliminated_s": 60, "queue_id": 1100, "placement": 9});
        assert!(riot_tft_match(&record(tft)).is_err());
        let valorant = json!({"match_id": "x", "duration_ms": 1000, "won": "oui"});
        assert!(valorant_match(&record(valorant)).is_err());
    }
}
