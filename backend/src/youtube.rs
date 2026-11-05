//! L'historique YouTube, tel que le rendent Google Data Portability (`MyActivity.json`) et Takeout
//! (`watch-history.json`) : une liste d'entrées au même format (voir le journal des décisions de
//! l'architecture, 5 nov. 2026). Le fichier mêle vidéos vues, réactions (« Liked », « Disliked »…),
//! publicités, recherches, posts et abonnements, avec des libellés traduits selon la langue du compte.
//!
//! - Une vidéo vue : `titleUrl` en `/watch` avec `v=` (11 caractères, seul paramètre), sans `details`,
//!   **et** un titre qui commence par un préfixe de vue connu (`WATCHED_PREFIXES`).
//! - Une réaction : la même entrée `/watch`, sans ce préfixe. Aucun champ neutre ne la distingue d'une
//!   vue (`activityControls` est traduit lui aussi) : c'est la seule règle qui lit le texte. Elle est
//!   écartée, et sa clé gardée pour annuler la fausse vue que la version d'avant en avait tirée.
//! - Une publicité : `details` présent (« From Google Ads »).
//! - Le reste (recherches `/results`, posts `/post`, chaînes, liens google.com, entrées sans lien) est
//!   écarté et compté par motif, comme YouTube Music, déjà couvert par la musique.
//!
//! Seules les vidéos classiques entrent au journal : ni Short, ni vidéo supprimée ou privée. Le fichier
//! ne le dit pas ; le connecteur le demande à l'API Data v3 (`is_short`).
//!
//! Un export d'une langue inconnue n'aurait que des réactions : `Activity::unrecognized` le signale, et
//! le connecteur n'en tire rien plutôt que de tout écarter en silence.
//!
//! La charge utile est construite champ par champ, en liste blanche : jamais `activityControls`,
//! `description`, `details` ni les liens eux-mêmes. Lecture en flux : seules les vidéos vues restent en
//! mémoire, le collecteur n'a que quelques dizaines de Mo.

use std::fmt;
use std::io::Read;

use chrono::{DateTime, SecondsFormat, Utc};
use serde::de::{IgnoredAny, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};

pub const WATCH_KIND: &str = "youtube.watch";
pub const VIDEO_INFO_KIND: &str = "youtube.video_info";

/// Les préfixes d'un titre de vidéo vue, par langue : ce qui distingue une vue d'une réaction. Une
/// langue de plus s'ajoute ici.
const WATCHED_PREFIXES: [&str; 2] = ["Watched ", "Vous avez regardé "];
/// YouTube borne un titre à 100 caractères, un nom de chaîne bien en dessous : au-delà, ce n'en est pas un.
const MAX_NAME_CHARS: usize = 500;

/// Une vidéo vue, réduite à sa liste blanche. Sérialisable : le connecteur garde dans son état les vues
/// dont la vidéo attend son classement.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Watch {
    pub video_id: String,
    /// `None` pour une vidéo supprimée ou privée, dont le titre n'est plus que son lien.
    pub title: Option<String>,
    pub channel_id: Option<String>,
    pub channel_name: Option<String>,
    pub watched_at: DateTime<Utc>,
}

/// `<id de la vidéo>:<instant en ms>` : aucune paire (vidéo, instant) ne se répète dans l'historique.
fn watch_key(video_id: &str, at: DateTime<Utc>) -> String {
    format!("{video_id}:{}", at.timestamp_millis())
}

impl Watch {
    pub fn key(&self) -> String {
        watch_key(&self.video_id, self.watched_at)
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

/// Une réaction (« Liked », « Disliked »…) à une vidéo : rien n'en entre au journal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reaction {
    pub video_id: String,
    pub at: DateTime<Utc>,
}

impl Reaction {
    /// La clé de la vue que la version d'avant le correctif du 5 nov. 2026 en a tirée, à annuler.
    pub fn watch_key(&self) -> String {
        watch_key(&self.video_id, self.at)
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
    Reaction,
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
    pub reaction: u32,
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
            SkipReason::Reaction => &mut self.reaction,
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
        self.reaction += other.reaction;
        self.other += other.other;
    }

    pub fn total(&self) -> u32 {
        self.ad
            + self.search
            + self.post
            + self.subscription
            + self.music
            + self.reaction
            + self.other
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

/// L'historique d'un fichier : les vidéos vues, dans l'ordre du fichier, les réactions et les entrées
/// écartées (réactions comprises).
#[derive(Debug, Default)]
pub struct Activity {
    pub watches: Vec<Watch>,
    pub reactions: Vec<Reaction>,
    pub skipped: Skipped,
}

impl Activity {
    pub fn extend(&mut self, other: Self) {
        self.watches.extend(other.watches);
        self.reactions.extend(other.reactions);
        self.skipped.add(&other.skipped);
    }

    /// Des entrées `/watch` mais aucune vue reconnue : la langue de l'export n'est sans doute pas dans
    /// `WATCHED_PREFIXES`, et chaque vue passerait pour une réaction.
    pub fn unrecognized(&self) -> bool {
        self.watches.is_empty() && !self.reactions.is_empty()
    }
}

enum Entry {
    Watch(Watch),
    Reaction(Reaction),
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
                        Ok(Entry::Watch(watch)) => activity.watches.push(watch),
                        Ok(Entry::Reaction(reaction)) => {
                            activity.skipped.count(SkipReason::Reaction);
                            activity.reactions.push(reaction);
                        }
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

/// Ce qui suit un préfixe de vue connu, s'il y en a un.
fn without_watched_prefix(title: Option<&str>) -> Option<&str> {
    let title = title?.trim_start();
    WATCHED_PREFIXES
        .iter()
        .find_map(|prefix| title.strip_prefix(prefix))
}

/// Le titre sans son préfixe de vue ; `None` sans préfixe connu (jamais « Disliked https://… ») ou si
/// ce qui reste est un lien (vidéo supprimée ou privée).
fn video_title(title: Option<&str>) -> Option<String> {
    let title = without_watched_prefix(title)?.trim();
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

fn classify(entry: &RawEntry) -> Result<Entry, SkipReason> {
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
    // La seule règle qui lit le texte : aucun champ neutre ne sépare une vue d'une réaction.
    if without_watched_prefix(entry.title.as_deref()).is_none() {
        return Ok(Entry::Reaction(Reaction {
            video_id,
            at: watched_at,
        }));
    }
    let (channel_id, channel_name) = channel(entry.subtitles.as_deref());
    Ok(Entry::Watch(Watch {
        video_id,
        title: video_title(entry.title.as_deref()),
        channel_id,
        channel_name,
        watched_at,
    }))
}

/// Un Short dure 3 min au plus (60 s avant oct. 2024).
pub const SHORT_MAX_SECONDS: u64 = 180;

/// Un Short, d'après l'API Data v3 : une durée de 1 à 180 s **et** un lecteur au moins aussi haut que
/// large (`player.embedWidth`/`embedHeight`, rendus avec `maxWidth`, au rapport de la vidéo ; carré
/// compris). Sans taille de lecteur ou sans durée, rien ne le dit : une vidéo classique. Un direct
/// (`P0D`) n'est jamais un Short.
pub fn is_short(duration_s: Option<u64>, embed: Option<(u64, u64)>) -> bool {
    duration_s.is_some_and(|d| (1..=SHORT_MAX_SECONDS).contains(&d))
        && embed.is_some_and(|(width, height)| width > 0 && height >= width)
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

    fn classified(value: Value) -> Result<Entry, SkipReason> {
        let raw: RawEntry = serde_json::from_value(value).unwrap();
        classify(&raw)
    }

    fn entry(value: Value) -> Result<Watch, SkipReason> {
        match classified(value)? {
            Entry::Watch(watch) => Ok(watch),
            Entry::Reaction(_) => Err(SkipReason::Reaction),
        }
    }

    const WATCH: &str = "https://www.youtube.com/watch?v=abcdefghijk";

    #[test]
    fn a_watched_video_needs_its_link_and_a_watched_prefix() {
        let watch = entry(json!({
            "header": "YouTube", "title": "Watched Un titre",
            "titleUrl": WATCH, "time": "2026-11-04T19:12:03.456Z", "products": ["YouTube"],
            "subtitles": [{"name": "Chaîne", "url": "https://www.youtube.com/channel/UCabc_DEF-123"}],
        }))
        .unwrap();
        assert_eq!(watch.video_id, "abcdefghijk");
        assert_eq!(watch.title.as_deref(), Some("Un titre"));
        assert_eq!(watch.channel_id.as_deref(), Some("UCabc_DEF-123"));
        assert_eq!(watch.channel_name.as_deref(), Some("Chaîne"));
        assert_eq!(watch.key(), "abcdefghijk:1793819523456");
        assert_eq!(
            watch.record()["occurred_at"],
            "2026-11-04T19:12:03.456Z",
            "la milliseconde est gardée"
        );
    }

    /// Le bug du 5 nov. 2026 : « Disliked https://… » était devenu une vue, titre compris.
    #[test]
    fn a_reaction_is_no_watch_and_keeps_the_key_of_the_false_watch() {
        for title in [
            "Liked https://www.youtube.com/watch?v=abcdefghijk",
            "Disliked https://www.youtube.com/watch?v=abcdefghijk",
            "Liked Un titre",
            "Vous avez aimé https://www.youtube.com/watch?v=abcdefghijk",
            "Vous avez aimé Un titre",
            "Commented on Un titre",
            "Ceci n'est pas un libellé connu",
        ] {
            let found = classified(json!({
                "header": "YouTube", "title": title, "titleUrl": WATCH,
                "time": "2023-05-05T19:12:03.456Z", "products": ["YouTube"],
            }));
            let Ok(Entry::Reaction(reaction)) = found else {
                panic!("{title} : une réaction attendue");
            };
            assert_eq!(reaction.video_id, "abcdefghijk");
            assert_eq!(reaction.watch_key(), "abcdefghijk:1683313923456", "{title}");
        }
        // Sans titre non plus, ce n'est pas une vue.
        assert_eq!(
            entry(json!({"titleUrl": WATCH, "time": "2026-11-04T19:12:03Z"})),
            Err(SkipReason::Reaction)
        );
        // Une réaction sans instant lisible n'a jamais fait de vue : rien à annuler.
        assert_eq!(
            entry(json!({"title": "Liked x", "titleUrl": WATCH, "time": "hier"})),
            Err(SkipReason::Other)
        );
    }

    fn parsed(entries: Value) -> Activity {
        parse_activity(entries.to_string().as_bytes()).unwrap()
    }

    #[test]
    fn an_export_in_an_unknown_language_is_unrecognized() {
        let time = "2026-11-04T19:12:03Z";
        let watched = json!({"title": "Watched x", "titleUrl": WATCH, "time": time});
        let liked = json!({"title": "Liked x", "titleUrl": WATCH, "time": time});
        let german = json!({"title": "Angesehen: x", "titleUrl": WATCH, "time": time});
        let search =
            json!({"titleUrl": "https://www.youtube.com/results?search_query=x", "time": time});
        let ad = json!({"title": "Angesehen: x", "titleUrl": WATCH, "time": time,
            "details": [{"name": "Von Google Ads"}]});

        let unknown = parsed(json!([german, german, search, ad]));
        assert!(unknown.unrecognized());
        assert_eq!(
            (
                unknown.reactions.len(),
                unknown.skipped.reaction,
                unknown.skipped.ad
            ),
            (2, 2, 1)
        );
        assert!(!parsed(json!([watched, liked])).unrecognized());
        assert!(
            !parsed(json!([search, ad])).unrecognized(),
            "rien en /watch"
        );
        assert!(!parsed(json!([])).unrecognized());

        let mut both = parsed(json!([liked]));
        assert!(both.unrecognized());
        both.extend(parsed(json!([watched])));
        assert!(!both.unrecognized(), "sur l'export entier");
        assert_eq!((both.watches.len(), both.reactions.len()), (1, 1));
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
        assert_eq!(title("Watched    "), None);
        assert_eq!(title(&format!("Watched {}", "x".repeat(501))), None);
        assert_eq!(title("Un titre sans préfixe"), None);
        assert_eq!(
            title("Disliked https://www.youtube.com/watch?v=abcdefghijk"),
            None
        );
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
            entry(
                json!({"title": "Watched x", "titleUrl": WATCH, "time": "2026-11-04T19:12:03Z",
                "details": []})
            )
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
    fn a_short_is_short_and_at_least_as_tall_as_wide() {
        let horizontal = Some((1000, 562));
        let vertical = Some((562, 1000));
        let square = Some((1000, 1000));
        assert!(is_short(Some(45), vertical), "un Short d'avant oct. 2024");
        assert!(is_short(Some(120), vertical), "un Short de 2 min");
        assert!(is_short(Some(180), vertical));
        assert!(is_short(Some(90), square), "carré : un Short");
        assert!(
            !is_short(Some(181), vertical),
            "une vidéo verticale de plus de 3 min"
        );
        assert!(!is_short(Some(600), vertical));
        assert!(
            !is_short(Some(120), horizontal),
            "une courte vidéo horizontale"
        );
        assert!(!is_short(Some(549), horizontal));
        assert!(
            !is_short(Some(45), None),
            "sans taille de lecteur : classique"
        );
        assert!(!is_short(None, vertical), "sans durée : classique");
        assert!(!is_short(Some(0), vertical), "un direct");
        assert!(!is_short(Some(45), Some((0, 0))));
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
