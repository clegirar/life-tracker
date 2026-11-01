//! L'outil d'import de l'export Apple : lit les fichiers copiés à la main, envoie par l'API d'ingestion
//! comme n'importe quelle source. Il ne connaît pas la base et ne parle qu'à l'API.
//!
//! Le journal est gardé à vie : le payload est construit champ par champ depuis `Row`, qui ne lit que
//! les colonnes de `WHITELIST`. Une colonne d'Apple n'y entre donc jamais par défaut, quelle qu'elle soit.
//! Rien de ce que l'outil affiche ne vient des lignes : des nombres, des motifs fixes, des empreintes.
//!
//! Le choix de ces règles et les mesures qui les justifient sont au journal de l'architecture.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, FixedOffset, SecondsFormat, Timelike, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::ingest_client::{IngestClient, IngestFailure, http_client};
use crate::music::fold;

pub const KIND: &str = "apple.play_history_row";
pub const PLAY_ACTIVITY: &str = "Apple Music Play Activity.csv";
pub const DAILY_TRACKS: &str = "Apple Music - Play History Daily Tracks.csv";
pub const LIBRARY_TRACKS: &str = "Apple Music Library Tracks.json";

/// Les seules colonnes lues dans Play Activity. Tout le reste (IP, ville, coordonnées, Apple ID,
/// appareil, abonnement…) est ignoré dès la lecture, et aucune colonne n'est jamais recopiée telle quelle.
const WHITELIST: [&str; 14] = [
    "Event Type",
    "Song Name",
    "Album Name",
    "Event Start Timestamp",
    "Event End Timestamp",
    "Play Duration Milliseconds",
    "Media Duration In Milliseconds",
    "Start Position In Milliseconds",
    "End Position In Milliseconds",
    "End Reason Type",
    "Media Type",
    "Item Type",
    "UTC Offset In Seconds",
    "Offline",
];

/// Marge sous la limite de l'API (1 Mo, 500 enregistrements).
const MAX_BATCH_BYTES: usize = 900_000;
const MAX_BATCH_RECORDS: usize = 500;

/// Lit un enregistrement CSV (RFC 4180 : guillemets doublés, retours à la ligne entre guillemets).
/// Pas de bibliothèque : le format est simple, et une dépendance de plus pour cela n'en vaut pas la peine.
pub struct CsvReader<R: Read> {
    input: BufReader<R>,
}

impl<R: Read> CsvReader<R> {
    pub fn new(input: R) -> Result<Self> {
        let mut input = BufReader::with_capacity(1 << 16, input);
        // Le BOM qu'Apple met au début, s'il y est.
        if input.fill_buf()?.starts_with(b"\xEF\xBB\xBF") {
            input.consume(3);
        }
        Ok(Self { input })
    }

    pub fn next_record(&mut self) -> Result<Option<Vec<String>>> {
        let mut fields = Vec::new();
        let mut field = Vec::new();
        let mut quoted = false;
        let mut seen = false;
        let mut chunk = Vec::new();
        loop {
            chunk.clear();
            let read = self.input.read_until(b'\n', &mut chunk)?;
            if read == 0 {
                if !seen {
                    return Ok(None);
                }
                if quoted {
                    bail!("CSV tronqué : guillemet non fermé");
                }
                fields.push(String::from_utf8_lossy(&field).into_owned());
                return Ok(Some(fields));
            }
            seen = true;
            let mut bytes = chunk.iter().copied().peekable();
            while let Some(byte) = bytes.next() {
                match (quoted, byte) {
                    (true, b'"') if bytes.peek() == Some(&b'"') => {
                        bytes.next();
                        field.push(b'"');
                    }
                    (true, b'"') => quoted = false,
                    (false, b'"') if field.is_empty() => quoted = true,
                    (false, b',') => {
                        fields.push(String::from_utf8_lossy(&field).into_owned());
                        field.clear();
                    }
                    (false, b'\n') => {
                        if field.last() == Some(&b'\r') {
                            field.pop();
                        }
                        fields.push(String::from_utf8_lossy(&field).into_owned());
                        return Ok(Some(fields));
                    }
                    (_, other) => field.push(other),
                }
            }
        }
    }
}

/// La position de chaque colonne demandée, sans lire les autres. Seul le nom d'une colonne manquante
/// peut être cité : il vient de ce code, pas du fichier.
struct Columns(Vec<usize>);

impl Columns {
    fn locate(header: &[String], wanted: &[&str]) -> Result<Self> {
        let mut positions = Vec::with_capacity(wanted.len());
        for name in wanted {
            let position = header
                .iter()
                .position(|column| column.trim() == *name)
                .ok_or_else(|| anyhow!("colonne absente : {name}"))?;
            positions.push(position);
        }
        Ok(Self(positions))
    }

    fn get<'a>(&self, record: &'a [String], index: usize) -> &'a str {
        record.get(self.0[index]).map_or("", |value| value.trim())
    }
}

struct Row<'a> {
    event_type: &'a str,
    song: &'a str,
    album: &'a str,
    started_at: &'a str,
    ended_at: &'a str,
    play_duration_ms: Option<i64>,
    media_duration_ms: Option<i64>,
    start_position_ms: Option<i64>,
    end_position_ms: Option<i64>,
    end_reason: &'a str,
    media_type: &'a str,
    item_type: &'a str,
    utc_offset_seconds: Option<i64>,
    offline: &'a str,
}

fn integer(text: &str) -> Option<i64> {
    text.parse().ok().or_else(|| {
        let value = text.parse::<f64>().ok()?;
        value.is_finite().then_some(value as i64)
    })
}

impl<'a> Row<'a> {
    fn read(columns: &Columns, record: &'a [String]) -> Self {
        let text = |index| columns.get(record, index);
        let number = |index| integer(columns.get(record, index));
        Self {
            event_type: text(0),
            song: text(1),
            album: text(2),
            started_at: text(3),
            ended_at: text(4),
            play_duration_ms: number(5),
            media_duration_ms: number(6),
            start_position_ms: number(7),
            end_position_ms: number(8),
            end_reason: text(9),
            media_type: text(10),
            item_type: text(11),
            utc_offset_seconds: number(12),
            offline: text(13),
        }
    }
}

/// Comment un artiste a été attribué à une ligne, écrit au journal : une déduction reste une déduction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ArtistSource {
    /// Le titre et l'album sont dans la bibliothèque, avec un seul artiste.
    LibraryAlbum,
    /// Plusieurs artistes en bibliothèque, un seul écouté ce jour-là.
    LibraryAlbumDay,
    /// Le titre est dans l'historique par jour, ce jour et à cette heure (UTC), avec un seul artiste.
    DailyHour,
    /// Même chose, ce jour-là.
    DailyDay,
    /// Le titre n'a qu'un seul artiste dans tout l'historique par jour.
    DailyTitle,
    /// Le titre n'a qu'un seul artiste dans la bibliothèque, quel que soit l'album.
    LibraryTitle,
    /// Plusieurs artistes possibles : aucun n'est choisi.
    Ambiguous,
    Unknown,
}

impl ArtistSource {
    pub fn name(self) -> &'static str {
        match self {
            Self::LibraryAlbum => "library_album",
            Self::LibraryAlbumDay => "library_album_day",
            Self::DailyHour => "daily_hour",
            Self::DailyDay => "daily_day",
            Self::DailyTitle => "daily_title",
            Self::LibraryTitle => "library_title",
            Self::Ambiguous => "ambiguous",
            Self::Unknown => "unknown",
        }
    }
}

type Artists = BTreeSet<String>;

/// Tout passe par `BTreeSet` : le résultat ne dépend pas de l'ordre des lignes.
#[derive(Default)]
pub struct Attribution {
    by_title_album: HashMap<(String, String), Artists>,
    by_title: HashMap<String, Artists>,
    daily_by_hour: HashMap<(String, u32, String), Artists>,
    daily_by_day: HashMap<(String, String), Artists>,
    daily_by_title: HashMap<String, Artists>,
}

fn only(artists: &Artists) -> Option<&str> {
    let mut iter = artists.iter();
    let first = iter.next()?;
    iter.next().is_none().then_some(first.as_str())
}

impl Attribution {
    fn add_library(&mut self, title: &str, album: Option<&str>, artist: &str) {
        let title = fold(title);
        if let Some(album) = album.filter(|a| !a.trim().is_empty()) {
            self.by_title_album
                .entry((title.clone(), fold(album)))
                .or_default()
                .insert(artist.to_owned());
        }
        self.by_title
            .entry(title)
            .or_default()
            .insert(artist.to_owned());
    }

    /// `Artiste - Titre` : un nom peut contenir ` - `, donc la coupure retenue est celle dont le titre
    /// existe dans Play Activity. Si plusieurs conviennent, l'artiste le plus court : la première.
    fn add_daily(&mut self, day: &str, hours: &str, description: &str, songs: &HashSet<String>) {
        let found = description.match_indices(" - ").find_map(|(position, _)| {
            let title = fold(&description[position + 3..]);
            songs
                .contains(&title)
                .then(|| (description[..position].trim(), title))
        });
        let Some((artist, title)) = found.filter(|(artist, _)| !artist.is_empty()) else {
            return;
        };
        for hour in hours
            .split(|c: char| !c.is_ascii_digit())
            .filter_map(|hour| hour.parse::<u32>().ok())
        {
            self.daily_by_hour
                .entry((day.to_owned(), hour, title.clone()))
                .or_default()
                .insert(artist.to_owned());
        }
        self.daily_by_day
            .entry((day.to_owned(), title.clone()))
            .or_default()
            .insert(artist.to_owned());
        self.daily_by_title
            .entry(title)
            .or_default()
            .insert(artist.to_owned());
    }

    /// De la preuve la plus forte à la plus faible. Jamais de choix entre plusieurs artistes.
    pub fn resolve(
        &self,
        song: &str,
        album: &str,
        at: DateTime<Utc>,
    ) -> (Option<&str>, ArtistSource) {
        let title = fold(song);
        // `Date Played` et `Hours` sont en UTC : mesuré, voir le journal de l'architecture.
        let day = at.format("%Y%m%d").to_string();
        let daily_day = self.daily_by_day.get(&(day.clone(), title.clone()));
        if let Some(artists) = self.by_title_album.get(&(title.clone(), fold(album))) {
            if let Some(artist) = only(artists) {
                return (Some(artist), ArtistSource::LibraryAlbum);
            }
            let common: Vec<&String> = daily_day
                .map(|day| artists.intersection(day).collect())
                .unwrap_or_default();
            return match common.as_slice() {
                [artist] => (Some(artist.as_str()), ArtistSource::LibraryAlbumDay),
                _ => (None, ArtistSource::Ambiguous),
            };
        }
        if let Some(artists) = self
            .daily_by_hour
            .get(&(day, at.hour(), title.clone()))
            .and_then(only)
        {
            return (Some(artists), ArtistSource::DailyHour);
        }
        if let Some(artists) = daily_day {
            return match only(artists) {
                Some(artist) => (Some(artist), ArtistSource::DailyDay),
                None => (None, ArtistSource::Ambiguous),
            };
        }
        for (artists, source) in [
            (self.daily_by_title.get(&title), ArtistSource::DailyTitle),
            (self.by_title.get(&title), ArtistSource::LibraryTitle),
        ] {
            if let Some(artists) = artists {
                return match only(artists) {
                    Some(artist) => (Some(artist), source),
                    None => (None, ArtistSource::Ambiguous),
                };
            }
        }
        (None, ArtistSource::Unknown)
    }
}

/// Pourquoi une ligne n'est pas envoyée. Des motifs fixes, seuls à être affichés.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Ignored {
    /// Pas un fait à part : le `PLAY_END` qui suit donne les mêmes heures, et plus. N'existe pas avant
    /// octobre 2022, et seuls ces événements portent des `Event ID` répétés.
    PlayStart,
    LyricDisplay,
    OtherEvent,
    Video,
    /// Sans heure de début, ces lignes ont toutes une durée négative.
    NoStartTimestamp,
    NoSongName,
    /// Durée nulle ou négative : rien n'a été joué (échecs de chargement, sauts).
    NoPlayback,
    UnreadableDate,
    TooLarge,
}

impl Ignored {
    pub fn name(self) -> &'static str {
        match self {
            Self::PlayStart => "play_start",
            Self::LyricDisplay => "lyric_display",
            Self::OtherEvent => "other_event",
            Self::Video => "video",
            Self::NoStartTimestamp => "no_start_timestamp",
            Self::NoSongName => "no_song_name",
            Self::NoPlayback => "no_playback",
            Self::UnreadableDate => "unreadable_date",
            Self::TooLarge => "too_large",
        }
    }
}

fn parse_timestamp(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|at| at.with_timezone(&Utc))
}

/// Ce qui ne change pas d'un export à l'autre et distingue deux lignes : les instants, en millisecondes
/// (indépendants de leur écriture), et les positions dans le titre. Ni `Event ID`, dont rien ne garantit
/// la stabilité, ni noms, ni artiste déduit : la bibliothèque évolue d'un export à l'autre.
pub fn record_key(start: DateTime<Utc>, end: DateTime<Utc>, from_ms: i64, to_ms: i64) -> String {
    format!(
        "play_end:{}:{}:{from_ms}:{to_ms}",
        start.timestamp_millis(),
        end.timestamp_millis()
    )
}

/// Le décalage de la ligne. Zéro n'existe pas à Paris : Apple ne le connaissait pas, donc `Z`.
fn occurred_at(start: DateTime<Utc>, offset_seconds: Option<i64>) -> String {
    let offset = offset_seconds
        .and_then(|seconds| i32::try_from(seconds).ok())
        .filter(|seconds| seconds.abs() <= 15 * 3600)
        .and_then(FixedOffset::east_opt);
    match offset {
        Some(offset) => start
            .with_timezone(&offset)
            .to_rfc3339_opts(SecondsFormat::Millis, true),
        None => start.to_rfc3339_opts(SecondsFormat::Millis, true),
    }
}

/// L'enveloppe et la charge utile d'une ligne : la liste blanche, champ par champ.
fn to_record(
    row: &Row,
    attribution: &Attribution,
    archive_sha256: &str,
) -> Result<(Value, &'static str), Ignored> {
    match row.event_type {
        "PLAY_END" => {}
        "PLAY_START" => return Err(Ignored::PlayStart),
        "LYRIC_DISPLAY" => return Err(Ignored::LyricDisplay),
        _ => return Err(Ignored::OtherEvent),
    }
    if row.media_type != "AUDIO" {
        return Err(Ignored::Video);
    }
    if row.started_at.is_empty() {
        return Err(Ignored::NoStartTimestamp);
    }
    if row.song.is_empty() {
        return Err(Ignored::NoSongName);
    }
    let played = row.play_duration_ms.filter(|ms| *ms > 0);
    let Some(played) = played else {
        return Err(Ignored::NoPlayback);
    };
    let (Some(start), Some(end)) = (
        parse_timestamp(row.started_at),
        parse_timestamp(row.ended_at),
    ) else {
        return Err(Ignored::UnreadableDate);
    };
    let (artist, artist_source) = attribution.resolve(row.song, row.album, start);
    let record = json!({
        "key": record_key(
            start,
            end,
            row.start_position_ms.unwrap_or(-1),
            row.end_position_ms.unwrap_or(-1),
        ),
        "kind": KIND,
        "schema_version": 1,
        "occurred_at": occurred_at(start, row.utc_offset_seconds),
        "payload": {
            "song": row.song,
            "album": (!row.album.is_empty()).then_some(row.album),
            "artist": artist,
            "artist_source": artist_source.name(),
            "started_at": row.started_at,
            "ended_at": row.ended_at,
            "play_duration_ms": played,
            "media_duration_ms": row.media_duration_ms,
            "start_position_ms": row.start_position_ms,
            "end_position_ms": row.end_position_ms,
            "end_reason": row.end_reason,
            "media_type": row.media_type,
            "item_type": row.item_type,
            "utc_offset_seconds": row.utc_offset_seconds,
            "offline": row.offline == "true",
            "archive_sha256": archive_sha256,
        },
    });
    Ok((record, artist_source.name()))
}

pub struct Config {
    pub archive_dir: PathBuf,
    pub ingest_url: String,
    pub token: String,
    pub max_attempts: u32,
    pub retry_base: Duration,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub activity_sha256: String,
    pub daily_sha256: String,
    pub library_sha256: String,
    pub rows: u64,
    pub accepted: u64,
    pub duplicate: u64,
    /// Par motif de l'API.
    pub rejected: BTreeMap<String, u64>,
    /// Par motif, seulement des lignes jamais envoyées.
    pub ignored: BTreeMap<&'static str, u64>,
    /// Par méthode d'attribution, sur les lignes envoyées.
    pub attribution: BTreeMap<&'static str, u64>,
}

fn open(dir: &Path, name: &str) -> Result<File> {
    File::open(dir.join(name)).map_err(|err| match err.kind() {
        ErrorKind::NotFound => anyhow!("fichier absent de l'archive : {name}"),
        ErrorKind::PermissionDenied => anyhow!("lecture refusée : {name}"),
        _ => anyhow!("lecture impossible : {name}"),
    })
}

fn sha256_of(dir: &Path, name: &str) -> Result<String> {
    let mut input = open(dir, name)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 16];
    loop {
        let read = input
            .read(&mut buffer)
            .with_context(|| format!("lecture de {name}"))?;
        if read == 0 {
            return Ok(hex::encode(hasher.finalize()));
        }
        hasher.update(&buffer[..read]);
    }
}

fn played_songs(dir: &Path) -> Result<HashSet<String>> {
    let mut reader = CsvReader::new(open(dir, PLAY_ACTIVITY)?)?;
    let header = reader.next_record()?.context("Play Activity vide")?;
    let columns = Columns::locate(&header, &["Song Name"])?;
    let mut songs = HashSet::new();
    while let Some(record) = reader.next_record()? {
        let song = columns.get(&record, 0);
        if !song.is_empty() {
            songs.insert(fold(song));
        }
    }
    Ok(songs)
}

/// Seuls `Title`, `Artist` et `Album` sont lus, et ils ne vont nulle part ailleurs que dans l'index.
fn load_library(dir: &Path, attribution: &mut Attribution) -> Result<()> {
    let tracks: Vec<Value> = serde_json::from_reader(BufReader::new(open(dir, LIBRARY_TRACKS)?))
        .map_err(|_| anyhow!("bibliothèque illisible : {LIBRARY_TRACKS}"))?;
    for track in tracks {
        let text = |field| track.get(field).and_then(Value::as_str);
        if let (Some(title), Some(artist)) = (text("Title"), text("Artist")) {
            let artist = artist.trim();
            if !artist.is_empty() {
                attribution.add_library(title, text("Album"), artist);
            }
        }
    }
    Ok(())
}

fn load_daily(dir: &Path, attribution: &mut Attribution, songs: &HashSet<String>) -> Result<()> {
    let mut reader = CsvReader::new(open(dir, DAILY_TRACKS)?)?;
    let header = reader.next_record()?.context("Daily Tracks vide")?;
    let columns = Columns::locate(&header, &["Date Played", "Hours", "Track Description"])?;
    while let Some(record) = reader.next_record()? {
        attribution.add_daily(
            columns.get(&record, 0),
            columns.get(&record, 1),
            columns.get(&record, 2),
            songs,
        );
    }
    Ok(())
}

pub fn load_attribution(dir: &Path) -> Result<Attribution> {
    let songs = played_songs(dir)?;
    let mut attribution = Attribution::default();
    load_library(dir, &mut attribution)?;
    load_daily(dir, &mut attribution, &songs)?;
    Ok(attribution)
}

/// Un refus d'authentification ou un droit manquant ne s'arrangera pas en réessayant.
fn is_final(failure: &IngestFailure) -> bool {
    failure.reason == "kind_not_declared" || matches!(failure.status, Some(400..=499))
}

fn describe(failure: &IngestFailure) -> String {
    match (failure.reason, failure.status) {
        ("kind_not_declared", _) => format!("le droit {KIND} manque à cette source"),
        (_, Some(401)) => "token refusé".to_owned(),
        (_, Some(status)) => format!("réponse {status} de l'API d'ingestion"),
        (reason, None) => format!("API d'ingestion injoignable ({reason})"),
    }
}

struct Uploader {
    client: IngestClient,
    max_attempts: u32,
    retry_base: Duration,
    batch: Vec<Value>,
    bytes: usize,
}

impl Uploader {
    /// Le lot est renvoyé entier après une coupure : ce qui était écrit revient en `duplicate`.
    async fn flush(&mut self, report: &mut Report) -> Result<()> {
        if self.batch.is_empty() {
            return Ok(());
        }
        let mut attempt = 0;
        let results = loop {
            attempt += 1;
            match self.client.send(&self.batch).await {
                Ok(results) => break results,
                Err(failure) if is_final(&failure) || attempt >= self.max_attempts => {
                    print_progress(report);
                    bail!(
                        "{} ; relancer l'import : ce qui est déjà passé reviendra en duplicate",
                        describe(&failure)
                    );
                }
                Err(_) => {
                    tokio::time::sleep(self.retry_base * 2u32.pow(attempt.min(6) - 1)).await;
                }
            }
        };
        for result in results {
            match result.status.as_str() {
                "accepted" => report.accepted += 1,
                "duplicate" => report.duplicate += 1,
                _ => {
                    let reason = result.reason.unwrap_or_else(|| "unknown".to_owned());
                    *report.rejected.entry(reason).or_default() += 1;
                }
            }
        }
        self.batch.clear();
        self.bytes = 0;
        Ok(())
    }

    async fn push(&mut self, record: Value, size: usize, report: &mut Report) -> Result<()> {
        if self.batch.len() == MAX_BATCH_RECORDS || self.bytes + size > MAX_BATCH_BYTES {
            self.flush(report).await?;
        }
        self.batch.push(record);
        self.bytes += size;
        Ok(())
    }
}

fn print_progress(report: &Report) {
    eprintln!(
        "arrêt : {} accepted, {} duplicate",
        report.accepted, report.duplicate
    );
}

/// Lit, déduit, envoie. Reprendre après une coupure, c'est relancer : le dédoublonnage du journal
/// remplace tout état local, et l'archive reste montée en lecture seule.
pub async fn run(config: Config) -> Result<Report> {
    let dir = &config.archive_dir;
    let mut report = Report {
        activity_sha256: sha256_of(dir, PLAY_ACTIVITY)?,
        daily_sha256: sha256_of(dir, DAILY_TRACKS)?,
        library_sha256: sha256_of(dir, LIBRARY_TRACKS)?,
        ..Report::default()
    };
    let attribution = load_attribution(dir)?;

    let mut uploader = Uploader {
        client: IngestClient::new(
            http_client()?,
            &config.ingest_url,
            &config.token,
            concat!("apple-import-", env!("CARGO_PKG_VERSION")),
        ),
        max_attempts: config.max_attempts,
        retry_base: config.retry_base,
        batch: Vec::with_capacity(MAX_BATCH_RECORDS),
        bytes: 0,
    };
    let mut reader = CsvReader::new(open(dir, PLAY_ACTIVITY)?)?;
    let header = reader.next_record()?.context("Play Activity vide")?;
    let columns = Columns::locate(&header, &WHITELIST)?;
    while let Some(fields) = reader.next_record()? {
        report.rows += 1;
        let row = Row::read(&columns, &fields);
        let sent =
            to_record(&row, &attribution, &report.activity_sha256).and_then(|(record, source)| {
                let size = record.to_string().len();
                if size > MAX_BATCH_BYTES {
                    Err(Ignored::TooLarge)
                } else {
                    Ok((record, source, size))
                }
            });
        match sent {
            Err(reason) => *report.ignored.entry(reason.name()).or_default() += 1,
            Ok((record, source, size)) => {
                *report.attribution.entry(source).or_default() += 1;
                uploader.push(record, size, &mut report).await?;
            }
        }
    }
    uploader.flush(&mut report).await?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Vec<Vec<String>> {
        let mut reader = CsvReader::new(text.as_bytes()).unwrap();
        let mut out = Vec::new();
        while let Some(record) = reader.next_record().unwrap() {
            out.push(record);
        }
        out
    }

    #[test]
    fn csv_quotes_commas_newlines_bom_and_a_missing_final_newline() {
        assert_eq!(
            parse("\u{feff}a,b\r\n\"x,1\",\"dit \"\"non\"\"\"\n\"sur\ndeux\",\nfin,"),
            [
                vec!["a", "b"],
                vec!["x,1", "dit \"non\""],
                vec!["sur\ndeux", ""],
                vec!["fin", ""],
            ]
        );
        assert!(parse("").is_empty());
        let mut reader = CsvReader::new("\"jamais fermé".as_bytes()).unwrap();
        assert!(reader.next_record().is_err());
    }

    fn at(text: &str) -> DateTime<Utc> {
        parse_timestamp(text).unwrap()
    }

    #[test]
    fn the_key_ignores_how_a_date_is_written() {
        let (a, b) = (at("2023-05-01T10:00:00Z"), at("2023-05-01T10:03:00.000Z"));
        let with_millis = record_key(at("2023-05-01T10:00:00.000Z"), b, 0, 180_000);
        assert_eq!(record_key(a, b, 0, 180_000), with_millis);
        assert_ne!(record_key(a, b, 0, 180_000), record_key(a, b, 10, 180_000));
    }

    #[test]
    fn an_offset_of_zero_is_unknown_and_the_day_stays_right() {
        let start = at("2023-05-01T22:30:00Z");
        assert_eq!(
            occurred_at(start, Some(7200)),
            "2023-05-02T00:30:00.000+02:00"
        );
        assert_eq!(occurred_at(start, Some(0)), "2023-05-01T22:30:00.000Z");
        assert_eq!(occurred_at(start, None), "2023-05-01T22:30:00.000Z");
        assert_eq!(occurred_at(start, Some(99_999)), "2023-05-01T22:30:00.000Z");
    }

    fn attribution() -> Attribution {
        let mut a = Attribution::default();
        let songs: HashSet<String> = ["titre", "seul", "tiret - dans le titre", "double"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        a.add_library("Titre", Some("Album"), "Groupe A");
        a.add_library("Titre", Some("Autre"), "Groupe B");
        a.add_library("Double", Some("Album"), "Groupe A");
        a.add_library("Double", Some("Album"), "Groupe B");
        a.add_daily("20230501", "10, 11", "Groupe B - Double", &songs);
        a.add_daily("20230502", "9", "Groupe C - Seul", &songs);
        a.add_daily("20230503", "9", "Groupe C - Seul", &songs);
        a.add_daily("20230503", "9", "Groupe D - Seul", &songs);
        a.add_daily("20230504", "9", "G - E - Tiret - dans le titre", &songs);
        a
    }

    #[test]
    fn the_artist_comes_from_the_strongest_proof_and_is_never_chosen() {
        let a = attribution();
        let may1 = at("2023-05-01T10:30:00Z");
        assert_eq!(
            a.resolve("TITRE", "album", may1),
            (Some("Groupe A"), ArtistSource::LibraryAlbum)
        );
        assert_eq!(
            a.resolve("Double", "Album", may1),
            (Some("Groupe B"), ArtistSource::LibraryAlbumDay)
        );
        assert_eq!(
            a.resolve("Double", "Album", at("2023-06-01T10:30:00Z")),
            (None, ArtistSource::Ambiguous)
        );
        assert_eq!(
            a.resolve("Seul", "x", at("2023-05-02T09:30:00Z")),
            (Some("Groupe C"), ArtistSource::DailyHour)
        );
        assert_eq!(
            a.resolve("Seul", "x", at("2023-05-02T15:30:00Z")),
            (Some("Groupe C"), ArtistSource::DailyDay)
        );
        // Deux artistes ce jour-là, deux dans tout l'historique : aucun choix.
        assert_eq!(
            a.resolve("Seul", "x", at("2023-05-03T09:30:00Z")),
            (None, ArtistSource::Ambiguous)
        );
        assert_eq!(
            a.resolve("Seul", "x", at("2023-07-01T09:30:00Z")),
            (None, ArtistSource::Ambiguous)
        );
        assert_eq!(
            a.resolve("Tiret - dans le titre", "x", at("2023-07-01T09:30:00Z")),
            (Some("G - E"), ArtistSource::DailyTitle)
        );
        assert_eq!(
            a.resolve("Titre", "x", may1),
            (None, ArtistSource::Ambiguous)
        );
        assert_eq!(
            a.resolve("Inconnu", "x", may1),
            (None, ArtistSource::Unknown)
        );
    }
}
