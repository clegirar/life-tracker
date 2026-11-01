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
    pub item_key: String,
    pub display: String,
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
        version: 1,
        schema_version: 1,
        interpret: lastfm_scrobble,
    },
    Interpreter {
        kind: "lastfm.track_info",
        version: 1,
        schema_version: 1,
        interpret: lastfm_track_info,
    },
    Interpreter {
        kind: "music.mark_known",
        version: 1,
        schema_version: 1,
        interpret: music_mark_known,
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

    let artist_key = music::artist_key(artist);
    let track_key = music::track_key(artist, track);
    let album_key = album.map(|album| music::album_key(artist, album));
    let mut items = vec![Item {
        level: "artist",
        item_key: artist_key.clone(),
        display: artist.to_owned(),
    }];
    if let (Some(album), Some(key)) = (album, &album_key) {
        items.push(Item {
            level: "album",
            item_key: key.clone(),
            display: album.to_owned(),
        });
    }
    items.push(Item {
        level: "track",
        item_key: track_key.clone(),
        display: track.to_owned(),
    });
    Ok(vec![Output::Event(Event {
        details: json!({
            "artist": artist,
            "album": album,
            "track": track,
            "artist_key": artist_key,
            "album_key": album_key,
            "track_key": track_key,
        }),
        items,
        ..Event::at_envelope(record, "music.listen")
    })])
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
        items: vec![Item {
            level: "track",
            item_key: track_key,
            display: track.to_owned(),
        }],
        ..Event::at_envelope(record, "music.track_duration")
    })])
}

/// Assez pour tout vrai nom, et assez court pour tenir dans l'index des éléments : une clé trop longue
/// serait une panne pour la base, pas un refus, et bloquerait le worker.
const MAX_MARKED_NAME_BYTES: usize = 1024;

/// `{"level", "artist", "album", "track"}` : la déclaration « déjà connu », avec les noms tels
/// qu'affichés. Un seul élément, à son niveau : c'est ce qui fait qu'un marquage n'agit qu'à son niveau.
/// Les clés se recalculent comme celles des écoutes, donc suivent la normalisation à chaque rebuild.
fn music_mark_known(record: &Record) -> Result<Vec<Output>, InterpretError> {
    let name = |field: &str| {
        let text = record.payload.get(field)?.as_str()?.trim();
        Some(text).filter(|t| !t.is_empty() && t.len() <= MAX_MARKED_NAME_BYTES)
    };
    let artist = name("artist").ok_or(InterpretError("music.mark_known: invalid artist"))?;
    let (album, track) = (name("album"), name("track"));
    let level = record.payload.get("level").and_then(Value::as_str);
    let (level, item_key, display) = match level {
        Some("artist") => ("artist", music::artist_key(artist), artist),
        Some("album") => {
            let album = album.ok_or(InterpretError("music.mark_known: invalid album"))?;
            ("album", music::album_key(artist, album), album)
        }
        Some("track") => {
            let track = track.ok_or(InterpretError("music.mark_known: invalid track"))?;
            ("track", music::track_key(artist, track), track)
        }
        _ => return Err(InterpretError("music.mark_known: invalid level")),
    };
    Ok(vec![Output::Event(Event {
        details: json!({ "level": level, "artist": artist, "album": album, "track": track }),
        items: vec![Item {
            level,
            item_key,
            display: display.to_owned(),
        }],
        ..Event::at_envelope(record, "music.known")
    })])
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
                ("album", "daft punk\tdiscovery"),
                ("track", "daft punk\tveridis quo"),
            ]
        );
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
    fn a_mark_gives_one_item_at_its_own_level_under_the_key_of_the_listens() {
        let item = |payload: Value| {
            let event = single_event(music_mark_known(&record(payload)).unwrap());
            assert_eq!(event.metric, "music.known");
            assert_eq!(event.items.len(), 1);
            let item = &event.items[0];
            (item.level, item.item_key.clone(), item.display.clone())
        };
        let artist = "DAFT PUNK feat. X";
        assert_eq!(
            item(json!({"level": "artist", "artist": artist, "album": "Discovery", "track": "T"})),
            ("artist", music::artist_key("Daft Punk"), artist.to_owned())
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
}
