//! L'historique YouTube, tel que le rendent Google Data Portability (`MyActivity.json`) et Takeout
//! (`watch-history.json`) : une liste d'entrées au même format (voir le journal des décisions de
//! l'architecture, 5 nov. 2026). Le fichier mêle vidéos vues, publicités, recherches, posts et
//! abonnements, avec des libellés traduits selon la langue du compte : rien ne se reconnaît au texte.
//!
//! - Une vidéo vue : `titleUrl` en `/watch` avec `v=` (11 caractères, seul paramètre), sans `details`.
//! - Une publicité : `details` présent (« From Google Ads »).
//! - Le reste (recherches `/results`, posts `/post`, chaînes, liens google.com, entrées sans lien) est
//!   écarté et compté par motif, comme YouTube Music, déjà couvert par la musique.
//!
//! La charge utile est construite champ par champ, en liste blanche : jamais `activityControls`,
//! `description`, `details` ni les liens eux-mêmes. Lecture en flux : seules les vidéos vues restent en
//! mémoire, le collecteur n'a que quelques dizaines de Mo.

use std::fmt;
use std::io::Read;

use chrono::{DateTime, SecondsFormat, Utc};
use serde::de::{IgnoredAny, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::{Value, json};

pub const WATCH_KIND: &str = "youtube.watch";
pub const VIDEO_INFO_KIND: &str = "youtube.video_info";

/// Les préfixes connus d'un titre de vidéo vue, par langue. Inconnu, le titre reste entier : il ne
/// sert qu'à l'affichage, jamais à reconnaître une entrée.
const WATCHED_PREFIXES: [&str; 2] = ["Watched ", "Vous avez regardé "];
/// YouTube borne un titre à 100 caractères, un nom de chaîne bien en dessous : au-delà, ce n'en est pas un.
const MAX_NAME_CHARS: usize = 500;

/// Une vidéo vue, réduite à sa liste blanche.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Watch {
    pub video_id: String,
    /// `None` pour une vidéo supprimée ou privée, dont le titre n'est plus que son lien.
    pub title: Option<String>,
    pub channel_id: Option<String>,
    pub channel_name: Option<String>,
    pub watched_at: DateTime<Utc>,
}

impl Watch {
    /// `<id de la vidéo>:<instant en ms>` : aucune paire (vidéo, instant) ne se répète dans l'historique.
    pub fn key(&self) -> String {
        format!("{}:{}", self.video_id, self.watched_at.timestamp_millis())
    }

    pub fn record(&self) -> Value {
        json!({
            "key": self.key(),
            "kind": WATCH_KIND,
            "schema_version": 1,
            "occurred_at": self.watched_at.to_rfc3339_opts(SecondsFormat::Millis, true),
            "payload": {
                "video_id": self.video_id,
                "title": self.title,
                "channel_id": self.channel_id,
                "channel_name": self.channel_name,
            },
        })
    }
}

/// La fiche d'une vidéo, tirée de l'API YouTube Data v3 : un enrichissement, comme la fiche d'un titre
/// de Last.fm. La clé est l'id seul : une vidéo n'a qu'une fiche.
pub fn video_info_record(
    video_id: &str,
    duration_s: Option<u64>,
    title: Option<&str>,
    channel_id: Option<&str>,
    channel_name: Option<&str>,
    now: DateTime<Utc>,
) -> Value {
    json!({
        "key": video_id,
        "kind": VIDEO_INFO_KIND,
        "schema_version": 1,
        "occurred_at": now.to_rfc3339_opts(SecondsFormat::Secs, true),
        "payload": {
            "video_id": video_id,
            "duration_s": duration_s,
            "title": title,
            "channel_id": channel_id,
            "channel_name": channel_name,
        },
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkipReason {
    Ad,
    Search,
    Post,
    Subscription,
    Music,
    Other,
}

/// Les entrées écartées, par motif : seuls ces nombres sortent dans les logs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Skipped {
    pub ad: u32,
    pub search: u32,
    pub post: u32,
    pub subscription: u32,
    pub music: u32,
    pub other: u32,
}

impl Skipped {
    pub fn count(&mut self, reason: SkipReason) {
        let counter = match reason {
            SkipReason::Ad => &mut self.ad,
            SkipReason::Search => &mut self.search,
            SkipReason::Post => &mut self.post,
            SkipReason::Subscription => &mut self.subscription,
            SkipReason::Music => &mut self.music,
            SkipReason::Other => &mut self.other,
        };
        *counter += 1;
    }

    pub fn add(&mut self, other: &Self) {
        self.ad += other.ad;
        self.search += other.search;
        self.post += other.post;
        self.subscription += other.subscription;
        self.music += other.music;
        self.other += other.other;
    }

    pub fn total(&self) -> u32 {
        self.ad + self.search + self.post + self.subscription + self.music + self.other
    }
}

#[derive(Deserialize)]
struct RawSubtitle {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    url: Option<String>,
}

/// Une entrée, réduite à ce que la classification lit. Tout autre champ est ignoré à la lecture.
#[derive(Deserialize)]
struct RawEntry {
    #[serde(default)]
    header: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default, rename = "titleUrl")]
    title_url: Option<String>,
    #[serde(default)]
    subtitles: Option<Vec<RawSubtitle>>,
    #[serde(default)]
    time: Option<String>,
    #[serde(default)]
    products: Option<Vec<String>>,
    /// Seule sa présence compte : une publicité.
    #[serde(default)]
    details: Option<Vec<IgnoredAny>>,
}

/// L'historique d'un fichier : les vidéos vues, dans l'ordre du fichier, et les entrées écartées.
#[derive(Debug, Default)]
pub struct Activity {
    pub watches: Vec<Watch>,
    pub skipped: Skipped,
}

impl<'de> Deserialize<'de> for Activity {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Entries;
        impl<'de> Visitor<'de> for Entries {
            type Value = Activity;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("une liste d'entrées d'activité")
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Activity, A::Error> {
                let mut activity = Activity::default();
                while let Some(entry) = seq.next_element::<RawEntry>()? {
                    match classify(&entry) {
                        Ok(watch) => activity.watches.push(watch),
                        Err(reason) => activity.skipped.count(reason),
                    }
                }
                Ok(activity)
            }
        }
        deserializer.deserialize_seq(Entries)
    }
}

/// Lit un `MyActivity.json` ou un `watch-history.json` en flux. Une erreur ne cite rien du fichier.
pub fn parse_activity(reader: impl Read) -> Result<Activity, serde_json::Error> {
    serde_json::from_reader(std::io::BufReader::new(reader))
}

fn is_youtube_host(url: &reqwest::Url) -> bool {
    matches!(
        url.host_str(),
        Some("www.youtube.com" | "youtube.com" | "m.youtube.com")
    )
}

/// Un identifiant de YouTube : lettres, chiffres, `-` et `_`.
fn is_id(text: &str, lengths: std::ops::RangeInclusive<usize>) -> bool {
    lengths.contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// L'id d'une vidéo vue : `https://www.youtube.com/watch?v=<11 caractères>`, sans autre paramètre.
fn watched_video_id(url: &reqwest::Url) -> Option<String> {
    if url.path() != "/watch" || url.fragment().is_some() {
        return None;
    }
    let mut pairs = url.query_pairs();
    let (name, value) = pairs.next()?;
    (name == "v" && is_id(&value, 11..=11) && pairs.next().is_none()).then(|| value.into_owned())
}

/// Un nom affichable : sans blancs autour, non vide, borné.
fn name(text: Option<&str>) -> Option<String> {
    let text = text?.trim();
    (!text.is_empty() && text.chars().count() <= MAX_NAME_CHARS).then(|| text.to_owned())
}

/// Le titre sans son préfixe connu ; `None` si ce qui reste est un lien (vidéo supprimée ou privée).
fn video_title(title: Option<&str>) -> Option<String> {
    let title = title?.trim();
    let title = WATCHED_PREFIXES
        .iter()
        .find_map(|prefix| title.strip_prefix(prefix))
        .unwrap_or(title);
    let lower = title.trim_start().to_ascii_lowercase();
    if lower.starts_with("https://") || lower.starts_with("http://") {
        return None;
    }
    name(Some(title))
}

/// La chaîne d'une vidéo : `subtitles[0]`, avec un lien `https://www.youtube.com/channel/<id>`.
/// Sans id lisible, aucune chaîne : un nom seul ne se rattacherait à rien de stable.
fn channel(subtitles: Option<&[RawSubtitle]>) -> (Option<String>, Option<String>) {
    let Some(first) = subtitles.and_then(<[RawSubtitle]>::first) else {
        return (None, None);
    };
    let id = first
        .url
        .as_deref()
        .and_then(|url| reqwest::Url::parse(url).ok())
        .filter(is_youtube_host)
        .and_then(|url| {
            let id = url.path().strip_prefix("/channel/")?.trim_end_matches('/');
            is_id(id, 1..=64).then(|| id.to_owned())
        });
    match id {
        Some(id) => (Some(id), name(first.name.as_deref())),
        None => (None, None),
    }
}

fn classify(entry: &RawEntry) -> Result<Watch, SkipReason> {
    let music = |text: &str| text.trim() == "YouTube Music";
    if entry.header.as_deref().is_some_and(music)
        || entry
            .products
            .as_deref()
            .is_some_and(|products| products.iter().any(|p| music(p)))
    {
        return Err(SkipReason::Music);
    }
    if entry.details.as_ref().is_some_and(|d| !d.is_empty()) {
        return Err(SkipReason::Ad);
    }
    let url = entry
        .title_url
        .as_deref()
        .and_then(|url| reqwest::Url::parse(url).ok())
        .ok_or(SkipReason::Other)?;
    if !is_youtube_host(&url) {
        return Err(SkipReason::Other);
    }
    let path = url.path();
    let video_id = match watched_video_id(&url) {
        Some(id) => id,
        None if path == "/results" => return Err(SkipReason::Search),
        None if path.starts_with("/post/") => return Err(SkipReason::Post),
        None if path.starts_with("/channel/") || path.starts_with("/@") => {
            return Err(SkipReason::Subscription);
        }
        None => return Err(SkipReason::Other),
    };
    let watched_at = entry
        .time
        .as_deref()
        .and_then(|time| DateTime::parse_from_rfc3339(time).ok())
        .map(|at| at.with_timezone(&Utc))
        .ok_or(SkipReason::Other)?;
    let (channel_id, channel_name) = channel(entry.subtitles.as_deref());
    Ok(Watch {
        video_id,
        title: video_title(entry.title.as_deref()),
        channel_id,
        channel_name,
        watched_at,
    })
}

/// Une durée ISO 8601 de l'API Data v3 en secondes : `PT9M9S`, `PT9S`, `PT1H2M3S`, `P1DT2H`, `P0D`
/// (un direct). Les fractions de seconde sont tronquées ; une forme inconnue ne donne rien.
pub fn iso8601_seconds(text: &str) -> Option<u64> {
    let rest = text.strip_prefix('P')?;
    let (date, time) = match rest.split_once('T') {
        Some((date, time)) => (date, Some(time)),
        None => (rest, None),
    };
    if date.is_empty() && time.is_none_or(str::is_empty) {
        return None;
    }
    fn fields(part: &str, units: &[(char, u64)]) -> Option<u64> {
        let mut total: u64 = 0;
        let mut number = String::new();
        let mut next_unit = 0;
        for c in part.chars() {
            if c.is_ascii_digit() || c == '.' {
                number.push(c);
                continue;
            }
            let index = units[next_unit..].iter().position(|(u, _)| *u == c)? + next_unit;
            next_unit = index + 1;
            let value: f64 = number.parse().ok()?;
            number.clear();
            total = total.checked_add((value * units[index].1 as f64) as u64)?;
        }
        number.is_empty().then_some(total)
    }
    let days = fields(date, &[('W', 604_800), ('D', 86_400)])?;
    let hours = match time {
        Some("") => return None,
        Some(time) => fields(time, &[('H', 3600), ('M', 60), ('S', 1)])?,
        None => 0,
    };
    days.checked_add(hours)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(value: Value) -> Result<Watch, SkipReason> {
        let raw: RawEntry = serde_json::from_value(value).unwrap();
        classify(&raw)
    }

    const WATCH: &str = "https://www.youtube.com/watch?v=abcdefghijk";

    #[test]
    fn a_watched_video_is_recognized_by_its_link_never_by_its_text() {
        let watch = entry(json!({
            "header": "YouTube", "title": "Ceci n'est pas un libellé connu",
            "titleUrl": WATCH, "time": "2026-11-04T19:12:03.456Z", "products": ["YouTube"],
            "subtitles": [{"name": "Chaîne", "url": "https://www.youtube.com/channel/UCabc_DEF-123"}],
        }))
        .unwrap();
        assert_eq!(watch.video_id, "abcdefghijk");
        assert_eq!(
            watch.title.as_deref(),
            Some("Ceci n'est pas un libellé connu")
        );
        assert_eq!(watch.channel_id.as_deref(), Some("UCabc_DEF-123"));
        assert_eq!(watch.channel_name.as_deref(), Some("Chaîne"));
        assert_eq!(watch.key(), "abcdefghijk:1793819523456");
        assert_eq!(
            watch.record()["occurred_at"],
            "2026-11-04T19:12:03.456Z",
            "la milliseconde est gardée"
        );
    }

    #[test]
    fn known_prefixes_are_stripped_and_a_link_is_no_title() {
        let title = |t: &str| video_title(Some(t));
        assert_eq!(title("Watched Un titre").as_deref(), Some("Un titre"));
        assert_eq!(
            title("Vous avez regardé Un titre").as_deref(),
            Some("Un titre")
        );
        assert_eq!(
            title("Watched https://www.youtube.com/watch?v=abcdefghijk"),
            None
        );
        assert_eq!(title("Vous avez regardé http://youtube.com/x"), None);
        assert_eq!(title("   "), None);
        assert_eq!(title(&"x".repeat(501)), None);
    }

    #[test]
    fn everything_else_is_skipped_by_reason() {
        let cases = [
            (
                json!({"titleUrl": WATCH, "time": "2026-11-04T19:12:03Z",
                    "details": [{"name": "From Google Ads"}]}),
                SkipReason::Ad,
            ),
            (
                json!({"titleUrl": "https://www.youtube.com/results?search_query=x",
                    "time": "2026-11-04T19:12:03Z"}),
                SkipReason::Search,
            ),
            (
                json!({"titleUrl": "https://www.youtube.com/post/UgkxABC",
                    "time": "2026-11-04T19:12:03Z"}),
                SkipReason::Post,
            ),
            (
                json!({"titleUrl": "https://www.youtube.com/channel/UCabc",
                    "time": "2026-11-04T19:12:03Z"}),
                SkipReason::Subscription,
            ),
            (
                json!({"header": "YouTube Music", "titleUrl": WATCH,
                    "time": "2026-11-04T19:12:03Z"}),
                SkipReason::Music,
            ),
            (
                json!({"products": ["YouTube Music"], "titleUrl": WATCH,
                    "time": "2026-11-04T19:12:03Z"}),
                SkipReason::Music,
            ),
            (
                json!({"titleUrl": "https://www.google.com/search?q=x",
                    "time": "2026-11-04T19:12:03Z"}),
                SkipReason::Other,
            ),
            (
                json!({"title": "Watched x", "time": "2026-11-04T19:12:03Z"}),
                SkipReason::Other,
            ),
            (
                json!({"titleUrl": format!("{WATCH}&t=10"),
                    "time": "2026-11-04T19:12:03Z"}),
                SkipReason::Other,
            ),
            (
                json!({"titleUrl": "https://www.youtube.com/watch?v=short",
                    "time": "2026-11-04T19:12:03Z"}),
                SkipReason::Other,
            ),
            (
                json!({"titleUrl": WATCH, "time": "hier"}),
                SkipReason::Other,
            ),
            (json!({"titleUrl": WATCH}), SkipReason::Other),
        ];
        for (value, reason) in cases {
            assert_eq!(entry(value.clone()), Err(reason), "{value}");
        }
        // Des `details` vides ne font pas une publicité.
        assert!(
            entry(json!({"titleUrl": WATCH, "time": "2026-11-04T19:12:03Z", "details": []}))
                .is_ok()
        );
    }

    #[test]
    fn a_deleted_video_keeps_its_id_only() {
        let watch = entry(json!({
            "header": "YouTube", "title": format!("Watched {WATCH}"), "titleUrl": WATCH,
            "time": "2026-11-04T19:12:03Z",
        }))
        .unwrap();
        assert_eq!(
            (watch.title, watch.channel_id, watch.channel_name),
            (None, None, None)
        );
    }

    #[test]
    fn a_channel_needs_its_id() {
        let channel_of = |subtitles: Value| {
            let raw: Vec<RawSubtitle> = serde_json::from_value(subtitles).unwrap();
            channel(Some(&raw))
        };
        assert_eq!(
            channel_of(json!([{"name": " A ", "url": "https://www.youtube.com/channel/UC1"}])),
            (Some("UC1".to_owned()), Some("A".to_owned()))
        );
        assert_eq!(
            channel_of(json!([{"name": "A", "url": "https://www.youtube.com/@a"}])),
            (None, None)
        );
        assert_eq!(
            channel_of(json!([{"name": "A", "url": "https://evil.example/channel/UC1"}])),
            (None, None)
        );
        assert_eq!(
            channel_of(json!([{"url": "https://www.youtube.com/channel/UC1"}])),
            (Some("UC1".to_owned()), None)
        );
        assert_eq!(channel_of(json!([])), (None, None));
    }

    #[test]
    fn iso8601_durations() {
        for (text, seconds) in [
            ("PT9M9S", Some(549)),
            ("PT9S", Some(9)),
            ("PT1H2M3S", Some(3723)),
            ("PT1H", Some(3600)),
            ("P1DT2H", Some(93_600)),
            ("P1D", Some(86_400)),
            ("P0D", Some(0)),
            ("PT0S", Some(0)),
            ("P1W", Some(604_800)),
            ("PT1.5S", Some(1)),
            ("PT", None),
            ("P", None),
            ("", None),
            ("9M", None),
            ("PT9S9M", None),
            ("PT9X", None),
            ("PT9", None),
        ] {
            assert_eq!(iso8601_seconds(text), seconds, "{text}");
        }
    }
}
