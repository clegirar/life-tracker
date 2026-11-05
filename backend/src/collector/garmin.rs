//! Le connecteur Garmin Connect, par l'API personnelle de Garmin (voir le journal des décisions de
//! l'architecture, 4 nov. 2026) : une clé que je crée dans Garmin Connect, échangée contre un jeton de
//! 24 h, sans jeton de rafraîchissement. Lecture seule, en tirage.
//!
//! Quatre types de données, chacun réduit à une liste blanche champ par champ : jamais le nom de la
//! séance, le propriétaire, `userProfilePk` ni `uuid`. Le GPS et le lieu des séances, eux, sont voulus
//! (journal du 4 nov. 2026) : coordonnées de début et de fin, nom du lieu, et la trace simplifiée,
//! tirée du fichier FIT d'origine, qui n'est pas gardé. Les totaux de Garmin sont des états,
//! révisés dans la journée et après une synchro tardive de la montre : chaque version est un fait
//! (règle 9), sous une clé qui dépend de son contenu, et une nouvelle version part avec l'annulation
//! de la précédente, dans le même lot. Toute lecture ne voit donc qu'une version vivante.
//!
//! La fréquence cardiaque d'une nuit vient de la série du jour (`dailyHeartRate`, un échantillon toutes
//! les deux minutes, du minuit au minuit de l'heure locale) : une nuit en chevauche deux, celle de la
//! veille de son réveil et celle du jour. Seuls les échantillons entre le début et la fin de la nuit
//! entrent dans la charge utile, pour que sa clé ne change pas quand la journée continue.
//!
//! Ses logs ne donnent que des nombres, des statuts et des motifs fixes, jamais une donnée ni un secret.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::Duration;

use base64::Engine;
use chrono::{DateTime, Datelike, Days, FixedOffset, NaiveDate, NaiveDateTime, SecondsFormat, Utc};
use reqwest::header::{ACCEPT, AUTHORIZATION, RETRY_AFTER};
use reqwest::{Client, Response, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::watch;
use tracing::{error, info, warn};

use super::lastfm::network_reason;
use super::{backoff, load_state, rate_limit_wait, report_status, save_state};
use crate::ingest_client::{IngestClient, IngestFailure, RecordOutcome, http_client};
use crate::source_status::{Reason, SourceStatus, State as Health};

pub const DEFAULT_GARMIN_URL: &str = "https://personal.connectapi.garmin.com";
const STATE_FILE: &str = "garmin-state.json";

const TOKEN_PATH: &str = "/personalapitoken-service/personal-api/personal-api-key/token";
const SUMMARIES_PATH: &str = "/usersummary-service/v2/dailysummaries";
const SLEEP_PATH: &str = "/sleep-service/sleep/dailySleepSummariesByDate";
const HEART_RATE_PATH: &str = "/wellness-service/wellness/dailyHeartRate";
const ACTIVITIES_PATH: &str = "/activitylist-service/activities/search/activities";
const DOWNLOAD_PATH: &str = "/download-service/files/activity";
/// Un fichier d'une séance de plusieurs heures pèse quelques Mo : au-delà, ce n'est pas une séance.
const MAX_DOWNLOAD_BYTES: usize = 32 * 1024 * 1024;
/// La trace garde ses virages à 5 m près, et 1 000 points au plus : quelques dizaines de ko.
pub const TRACK_TOLERANCE_M: f64 = 5.0;
pub const MAX_TRACK_POINTS: usize = 1000;

/// Le premier passage remonte aussi loin : tout l'historique (juin 2026) y tient.
pub const BACKFILL_DAYS: u64 = 400;
/// Chaque passage relit ces jours-là, aujourd'hui compris : une synchro tardive révise le passé.
pub const REREAD_DAYS: u64 = 7;
/// Les plages que l'API accepte : 90 jours passent pour les résumés et 400 sont refusés (400),
/// 32 pour le sommeil. 31 pour les deux, avec de la marge.
const SUMMARY_CHUNK_DAYS: u64 = 31;
pub const SLEEP_CHUNK_DAYS: u64 = 31;
pub const ACTIVITY_PAGE_SIZE: usize = 100;
/// Au-delà de la fenêtre relue, un sujet n'est plus jamais redemandé : l'état l'oublie.
const KEEP_SUBJECT_DAYS: u64 = 14;
/// Le jeton vaut 24 h ; on en redemande un une heure avant.
const TOKEN_MARGIN: chrono::TimeDelta = chrono::TimeDelta::hours(1);
const DEFAULT_RATE_LIMIT_DELAY: Duration = Duration::from_secs(300);
/// Un lot reste loin des 500 enregistrements du contrat, une paire n'y est jamais coupée.
const UNITS_PER_BATCH: usize = 200;

pub struct Config {
    pub api_url: String,
    pub api_key: String,
    pub ingest_url: String,
    pub token: String,
    /// Le nom de la source dans le registre : une annulation désigne sa cible par source et clé.
    pub source: String,
    pub state_dir: PathBuf,
    pub interval: Duration,
    /// Garmin n'annonce aucune limite de débit : une pause entre deux appels, comme pour Last.fm.
    pub pause: Duration,
    /// L'échéance de la clé (`GARMIN_KEY_EXPIRES_ON`, voir `key_expiry`) : Garmin ne la donne pas.
    /// Rapportée avec chaque état, pour que la page d'état prévienne avant.
    pub key_expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ServiceError {
    /// La clé est refusée à l'échange (401 ou 403) : expirée ou révoquée, il faut en créer une.
    KeyRejected {
        status: u16,
    },
    RateLimited {
        retry_after: Duration,
    },
    /// Réponse d'erreur : son statut et l'appel refusé, jamais son corps.
    Failed {
        status: u16,
        call: &'static str,
    },
    /// Rien d'exploitable : coupure, délai dépassé, réponse illisible.
    Unavailable {
        reason: &'static str,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub enum TickError {
    Service(ServiceError),
    Ingest {
        reason: &'static str,
        status: Option<u16>,
    },
    State,
}

impl From<ServiceError> for TickError {
    fn from(err: ServiceError) -> Self {
        Self::Service(err)
    }
}

impl From<IngestFailure> for TickError {
    fn from(failure: IngestFailure) -> Self {
        Self::Ingest {
            reason: failure.reason,
            status: failure.status,
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct TickReport {
    /// Appels réussis à Garmin, échange du jeton compris.
    pub calls: u32,
    pub accepted: u32,
    pub duplicate: u32,
    pub rejected: u32,
    /// Versions précédentes annulées : autant de révisions de Garmin.
    pub cancelled: u32,
    /// Éléments de Garmin sans enregistrement : jour sans données, nuit sans sommeil, champ illisible.
    pub skipped: u32,
}

// ─── Le temps de Paris, sans base de fuseaux ─────────────────────────────────

/// Le dernier dimanche d'un mois de 31 jours.
fn last_sunday(year: i32, month: u32) -> NaiveDate {
    let last = NaiveDate::from_ymd_opt(year, month, 31).expect("mois de 31 jours");
    last - Days::new(u64::from(last.weekday().num_days_from_sunday()))
}

/// Le décalage de Paris à un instant : heure d'été du dernier dimanche de mars au dernier dimanche
/// d'octobre, à 01:00 UTC (la règle de l'Union européenne depuis 1996). Si elle change un jour, ici.
pub fn paris_offset(at: DateTime<Utc>) -> FixedOffset {
    let year = at.year();
    let switch = |month| {
        last_sunday(year, month)
            .and_hms_opt(1, 0, 0)
            .expect("heure valide")
            .and_utc()
    };
    let summer = at >= switch(3) && at < switch(10);
    FixedOffset::east_opt(if summer { 7200 } else { 3600 }).expect("décalage valide")
}

/// Le jour qu'il est à Paris.
pub fn paris_today(now: DateTime<Utc>) -> NaiveDate {
    now.with_timezone(&paris_offset(now)).date_naive()
}

/// Midi heure de Paris, avec son décalage : la date d'un total du jour. Un total appartient à son jour
/// calendaire, et midi le garde dans son jour quel que soit le changement d'heure.
pub fn paris_noon(date: NaiveDate) -> String {
    let around = date.and_hms_opt(10, 0, 0).expect("heure valide").and_utc();
    date.and_hms_opt(12, 0, 0)
        .expect("heure valide")
        .and_local_timezone(paris_offset(around))
        .single()
        .expect("décalage fixe")
        .to_rfc3339_opts(SecondsFormat::Secs, false)
}

/// La fin du jour de Paris où la clé expire, en UTC : la date affichée par Garmin à la création de la
/// clé, `AAAA-MM-JJ`. La clé vaut tout ce jour-là ; l'heure exacte n'est pas connue. `None` : illisible.
pub fn key_expiry(text: &str) -> Option<DateTime<Utc>> {
    let date = (text.len() == 10)
        .then(|| NaiveDate::parse_from_str(text, "%Y-%m-%d").ok())
        .flatten()?;
    let next = date.checked_add_days(Days::new(1))?;
    // Minuit à Paris tombe la veille à 22 h ou 23 h UTC, jamais près d'un changement d'heure (01:00 UTC).
    let around = next.and_hms_opt(0, 0, 0)?.and_utc();
    next.and_hms_opt(0, 0, 0)?
        .and_local_timezone(paris_offset(around))
        .single()
        .map(|at| at.with_timezone(&Utc))
}

fn utc(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Secs, true)
}

// ─── Les listes blanches ─────────────────────────────────────────────────────

/// Une observation de Garmin, réduite à sa liste blanche, prête à devenir un enregistrement.
#[derive(Debug, Clone, PartialEq)]
pub struct Observation {
    pub kind: &'static str,
    /// Ce que décrit l'observation, une version à la fois : `summary:<date>`, `sleep:<date>`,
    /// `activity:<id>`.
    pub subject: String,
    /// Le jour du sujet, pour oublier les sujets sortis de la fenêtre relue.
    pub date: NaiveDate,
    pub occurred_at: String,
    pub payload: Value,
}

impl Observation {
    /// `<sujet>:<16 premiers hexadécimaux du SHA-256 de la charge utile>` : un contenu identique
    /// retombe sur la même clé (`duplicate`), un contenu révisé en donne une nouvelle. La charge utile
    /// est sérialisée clés triées, donc toujours de la même façon.
    pub fn key(&self) -> String {
        key_of(&self.subject, &self.payload)
    }

    /// La clé qu'aurait eue une nuit avant la fréquence cardiaque (version 0.3 du connecteur) : la même
    /// charge utile, sans `heart_rate`. `None` si la charge utile n'en a pas : la clé est la même.
    pub fn key_without_heart_rate(&self) -> Option<String> {
        let mut payload = self.payload.clone();
        payload.as_object_mut()?.remove("heart_rate")?;
        Some(key_of(&self.subject, &payload))
    }

    pub fn record(&self, key: &str) -> Value {
        json!({
            "key": key,
            "kind": self.kind,
            "schema_version": 1,
            "occurred_at": self.occurred_at,
            "payload": self.payload,
        })
    }
}

fn key_of(subject: &str, payload: &Value) -> String {
    let canonical = serde_json::to_string(payload).expect("JSON sérialisable");
    let digest = hex::encode(Sha256::digest(canonical.as_bytes()));
    format!("{subject}:{}", &digest[..16])
}

/// Un nombre positif, normalisé : entier s'il l'est, sinon arrondi au millième. La même valeur donne
/// donc toujours la même charge utile, et la même clé. Négatif, absent ou illisible : `null`.
fn number(value: Option<&Value>) -> Value {
    let Some(f) = value
        .and_then(Value::as_f64)
        .filter(|f| f.is_finite() && *f >= 0.0)
    else {
        return Value::Null;
    };
    if f.fract() == 0.0 && f < 9.0e15 {
        json!(f as i64)
    } else {
        json!((f * 1000.0).round() / 1000.0)
    }
}

fn calendar_date(raw: &Value, field: &str) -> Option<NaiveDate> {
    let text = raw.get(field)?.as_str()?;
    (text.len() == 10)
        .then(|| NaiveDate::parse_from_str(text, "%Y-%m-%d").ok())
        .flatten()
}

/// Un jour de `dailysummaries` : `{calendar_date, steps, distance_m, calories_total, calories_active,
/// calories_resting}`. Rien pour un jour sans données de bien-être ou sans pas : une journée sans
/// données n'est jamais une journée sans activité.
pub fn daily_summary(raw: &Value) -> Option<Observation> {
    let date = calendar_date(raw, "calendarDate")?;
    if raw.get("includesWellnessData").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    let steps = number(raw.pointer("/movement/steps/value"));
    if steps.is_null() {
        return None;
    }
    Some(Observation {
        kind: "garmin.daily_summary",
        subject: format!("summary:{date}"),
        date,
        occurred_at: paris_noon(date),
        payload: json!({
            "calendar_date": date.to_string(),
            "steps": steps,
            "distance_m": number(raw.pointer("/movement/steps/distanceInMeters")),
            "calories_total": number(raw.pointer("/calories/burnedTotal")),
            "calories_active": number(raw.pointer("/calories/burnedActive")),
            "calories_resting": number(raw.pointer("/calories/burnedResting")),
        }),
    })
}

/// Un instant en millisecondes depuis l'époque, en RFC 3339 UTC.
fn epoch_ms(value: Option<&Value>) -> Value {
    value
        .and_then(Value::as_i64)
        .and_then(DateTime::from_timestamp_millis)
        .map_or(Value::Null, |at| Value::String(utc(at)))
}

/// Une nuit de `dailySleepSummariesByDate` : `{calendar_date, start, end, sleep_seconds,
/// deep_seconds, light_seconds, rem_seconds, awake_seconds}`, au jour du réveil (celui de Garmin).
/// Rien pour une nuit sans sommeil mesuré : la montre n'était pas portée.
pub fn sleep(raw: &Value) -> Option<Observation> {
    let date = calendar_date(raw, "calendarDate")?;
    let seconds = number(raw.get("sleepTimeSeconds"));
    if seconds.as_f64().is_none_or(|s| s <= 0.0) {
        return None;
    }
    Some(Observation {
        kind: "garmin.sleep",
        subject: format!("sleep:{date}"),
        date,
        occurred_at: paris_noon(date),
        payload: json!({
            "calendar_date": date.to_string(),
            "start": epoch_ms(raw.get("sleepStartTimestampGMT")),
            "end": epoch_ms(raw.get("sleepEndTimestampGMT")),
            "sleep_seconds": seconds,
            "deep_seconds": number(raw.get("deepSleepSeconds")),
            "light_seconds": number(raw.get("lightSleepSeconds")),
            "rem_seconds": number(raw.get("remSleepSeconds")),
            "awake_seconds": number(raw.get("awakeSleepSeconds")),
        }),
    })
}

/// Les jours dont la série de fréquence cardiaque couvre une nuit : la veille du réveil et le jour du
/// réveil. Aucun pour une nuit sans début ni fin.
pub fn heart_rate_dates(night: &Observation) -> Vec<NaiveDate> {
    if night.kind != "garmin.sleep"
        || night.payload["start"].is_null()
        || night.payload["end"].is_null()
    {
        return Vec::new();
    }
    vec![night.date - Days::new(1), night.date]
}

fn instant_ms(value: &Value) -> Option<i64> {
    DateTime::parse_from_rfc3339(value.as_str()?)
        .ok()
        .map(|at| at.timestamp_millis())
}

/// Les échantillons d'une série `dailyHeartRate` : `[instant en ms, bpm]`, sans les vides (`null`,
/// zéro). Une série sans `heartRateValues` n'en a aucun ; une réponse qui n'est pas un objet est illisible.
pub fn heart_rate_samples(series: &Value) -> Option<Vec<(i64, Value)>> {
    let object = series.as_object()?;
    let Some(values) = object.get("heartRateValues").and_then(Value::as_array) else {
        return Some(Vec::new());
    };
    Some(
        values
            .iter()
            .filter_map(|sample| {
                let pair = sample.as_array()?;
                let at = pair.first()?.as_i64()?;
                let bpm = number(pair.get(1));
                bpm.as_f64().is_some_and(|b| b > 0.0).then_some((at, bpm))
            })
            .collect(),
    )
}

/// Ajoute à une nuit `heart_rate` : `[[secondes depuis le début, bpm], …]`, les échantillons des séries
/// compris entre son début et sa fin, bornes comprises, triés, un par instant. Rien sans échantillon :
/// la charge utile reste celle d'une nuit sans fréquence cardiaque.
pub fn with_heart_rate(mut night: Observation, series: &[&[(i64, Value)]]) -> Observation {
    let (Some(start), Some(end)) = (
        instant_ms(&night.payload["start"]),
        instant_ms(&night.payload["end"]),
    ) else {
        return night;
    };
    let mut samples = BTreeMap::new();
    for (at, bpm) in series.iter().flat_map(|s| s.iter()) {
        if (start..=end).contains(at) {
            samples
                .entry((at - start + 500).div_euclid(1000))
                .or_insert_with(|| bpm.clone());
        }
    }
    if !samples.is_empty() {
        night.payload["heart_rate"] = samples
            .into_iter()
            .map(|(seconds, bpm)| json!([seconds, bpm]))
            .collect();
    }
    night
}

fn garmin_time(raw: &Value, field: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(raw.get(field)?.as_str()?, "%Y-%m-%d %H:%M:%S").ok()
}

/// Une coordonnée en degrés, au millionième (10 cm) ; hors bornes ou absente : `null`.
fn coordinate(value: Option<&Value>, bound: f64) -> Value {
    value
        .and_then(Value::as_f64)
        .filter(|c| c.is_finite() && c.abs() <= bound)
        .map_or(Value::Null, |c| json!((c * 1e6).round() / 1e6))
}

/// Le nom du lieu de Garmin, en général la ville ; borné, jamais vide.
fn place(value: Option<&Value>) -> Value {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|p| !p.is_empty() && p.chars().count() <= 200)
        .map_or(Value::Null, |p| Value::String(p.to_owned()))
}

/// Une séance de la liste des activités : `{activity_id, type, start, duration_s, moving_s,
/// distance_m, avg_speed_mps, avg_hr, max_hr, calories, start_lat, start_lon, end_lat, end_lon,
/// location_name}`, à son début. Le décalage de l'enveloppe est celui de l'heure locale de la séance,
/// déduit de ses deux heures. Jamais le nom de la séance ni son propriétaire.
pub fn activity(raw: &Value) -> Option<Observation> {
    let id = raw.get("activityId")?.as_u64()?;
    let kind = raw
        .pointer("/activityType/typeKey")?
        .as_str()
        .map(str::trim)
        .filter(|k| !k.is_empty())?;
    let start = garmin_time(raw, "startTimeGMT")?.and_utc();
    let occurred_at = garmin_time(raw, "startTimeLocal")
        .map(|local| (local - start.naive_utc()).num_seconds())
        .filter(|offset| offset.abs() <= 15 * 3600 && offset % 60 == 0)
        .and_then(|offset| FixedOffset::east_opt(i32::try_from(offset).ok()?))
        .map_or_else(
            || utc(start),
            |offset| {
                start
                    .with_timezone(&offset)
                    .to_rfc3339_opts(SecondsFormat::Secs, true)
            },
        );
    Some(Observation {
        kind: "garmin.activity",
        subject: format!("activity:{id}"),
        date: start.date_naive(),
        occurred_at,
        payload: json!({
            "activity_id": id,
            "type": kind,
            "start": utc(start),
            "duration_s": number(raw.get("duration")),
            "moving_s": number(raw.get("movingDuration")),
            "distance_m": number(raw.get("distance")),
            "avg_speed_mps": number(raw.get("averageSpeed")),
            "avg_hr": number(raw.get("averageHR")),
            "max_hr": number(raw.get("maxHR")),
            "calories": number(raw.get("calories")),
            "start_lat": coordinate(raw.get("startLatitude"), 90.0),
            "start_lon": coordinate(raw.get("startLongitude"), 180.0),
            "end_lat": coordinate(raw.get("endLatitude"), 90.0),
            "end_lon": coordinate(raw.get("endLongitude"), 180.0),
            "location_name": place(raw.get("locationName")),
        }),
    })
}

/// Une séance a une trace à demander si Garmin lui connaît un point de départ : un tapis de course
/// n'en a pas.
pub fn has_gps(observation: &Observation) -> bool {
    observation.kind == "garmin.activity" && !observation.payload["start_lat"].is_null()
}

// ─── La trace d'une séance ───────────────────────────────────────────────────

/// Le premier `.fit` du ZIP de Garmin.
pub fn fit_from_zip(bytes: &[u8]) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).ok()?;
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).ok()?;
        if file.is_file()
            && file
                .name()
                .is_ok_and(|n| n.to_ascii_lowercase().ends_with(".fit"))
        {
            let mut fit = Vec::new();
            file.by_ref()
                .take(MAX_DOWNLOAD_BYTES as u64)
                .read_to_end(&mut fit)
                .ok()?;
            return Some(fit);
        }
    }
    None
}

const SEMICIRCLE_TO_DEGREES: f64 = 180.0 / 2_147_483_648.0;

/// Une position du FIT en degrés : des demi-cercles d'ordinaire (× 180 / 2³¹).
fn degrees(field: &fitparser::FitDataField) -> Option<f64> {
    let raw = match field.value() {
        fitparser::Value::SInt32(v) => f64::from(*v),
        fitparser::Value::Float64(v) => *v,
        fitparser::Value::Float32(v) => f64::from(*v),
        _ => return None,
    };
    let value = if field.units() == "semicircles" {
        raw * SEMICIRCLE_TO_DEGREES
    } else {
        raw
    };
    value.is_finite().then_some(value)
}

/// Les points `record` du FIT qui ont une position : `[lat, lon, secondes depuis le premier]`.
pub fn fit_points(fit: &[u8]) -> Option<Vec<[f64; 3]>> {
    let records = fitparser::from_bytes(fit).ok()?;
    let mut points = Vec::new();
    let mut first = None;
    for record in records
        .iter()
        .filter(|r| r.kind() == fitparser::profile::MesgNum::Record)
    {
        let field = |name: &str| record.fields().iter().find(|f| f.name() == name);
        let (Some(lat), Some(lon)) = (
            field("position_lat").and_then(degrees),
            field("position_long").and_then(degrees),
        ) else {
            continue;
        };
        if lat.abs() > 90.0 || lon.abs() > 180.0 {
            continue;
        }
        let Some(fitparser::Value::Timestamp(at)) = field("timestamp").map(|f| f.value()) else {
            continue;
        };
        let at = at.timestamp();
        let start = *first.get_or_insert(at);
        points.push([lat, lon, (at - start) as f64]);
    }
    Some(points)
}

/// La distance en mètres d'un point au segment `a`–`b`, sur un plan local (équirectangulaire) : à
/// l'échelle d'une séance, l'écart à la sphère est négligeable.
fn segment_distance(p: &[f64; 3], a: &[f64; 3], b: &[f64; 3]) -> f64 {
    const METERS_PER_DEGREE: f64 = 111_320.0;
    let scale = a[0].to_radians().cos();
    let xy = |q: &[f64; 3]| {
        (
            (q[1] - a[1]) * METERS_PER_DEGREE * scale,
            (q[0] - a[0]) * METERS_PER_DEGREE,
        )
    };
    let ((px, py), (bx, by)) = (xy(p), xy(b));
    let length = bx * bx + by * by;
    let t = if length == 0.0 {
        0.0
    } else {
        ((px * bx + py * by) / length).clamp(0.0, 1.0)
    };
    ((px - t * bx).powi(2) + (py - t * by).powi(2)).sqrt()
}

/// Douglas–Peucker, sans récursion : les points qui s'écartent de plus de `tolerance` mètres.
fn douglas_peucker(points: &[[f64; 3]], tolerance: f64) -> Vec<[f64; 3]> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let mut keep = vec![false; points.len()];
    keep[0] = true;
    keep[points.len() - 1] = true;
    let mut stack = vec![(0, points.len() - 1)];
    while let Some((first, last)) = stack.pop() {
        let mut farthest = (0.0, first);
        for i in first + 1..last {
            let distance = segment_distance(&points[i], &points[first], &points[last]);
            if distance > farthest.0 {
                farthest = (distance, i);
            }
        }
        if farthest.0 > tolerance {
            keep[farthest.1] = true;
            stack.push((first, farthest.1));
            stack.push((farthest.1, last));
        }
    }
    points
        .iter()
        .zip(keep)
        .filter_map(|(p, kept)| kept.then_some(*p))
        .collect()
}

/// La trace simplifiée : 5 m de tolérance, doublée tant qu'il reste plus de `cap` points. Les
/// coordonnées au millionième, les secondes entières : la même trace donne la même charge utile.
pub fn simplify(points: &[[f64; 3]], tolerance: f64, cap: usize) -> Vec<Value> {
    let mut tolerance = tolerance;
    let mut kept = douglas_peucker(points, tolerance);
    while kept.len() > cap {
        tolerance *= 2.0;
        kept = douglas_peucker(points, tolerance);
    }
    kept.iter()
        .map(|[lat, lon, seconds]| {
            json!([
                (lat * 1e6).round() / 1e6,
                (lon * 1e6).round() / 1e6,
                seconds.round() as i64
            ])
        })
        .collect()
}

/// La trace d'une séance : `{activity_id, points}`, à la date de la séance. Rien sans au moins deux
/// points : une séance en intérieur n'a pas de trace.
pub fn activity_track(workout: &Observation, fit: &[u8]) -> Option<Observation> {
    let id = workout.payload["activity_id"].as_u64()?;
    let points = fit_points(fit)?;
    if points.len() < 2 {
        return None;
    }
    Some(Observation {
        kind: "garmin.activity_track",
        subject: format!("track:{id}"),
        date: workout.date,
        occurred_at: workout.occurred_at.clone(),
        payload: json!({
            "activity_id": id,
            "points": simplify(&points, TRACK_TOLERANCE_M, MAX_TRACK_POINTS),
        }),
    })
}

// ─── Le client de Garmin ─────────────────────────────────────────────────────

struct Bearer {
    jwt: String,
    refresh_after: DateTime<Utc>,
}

/// L'expiration d'un JWT (`exp`, en secondes), sans en vérifier la signature : seul Garmin la vérifie.
fn jwt_expiry(jwt: &str) -> Option<DateTime<Utc>> {
    let claims = jwt.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(claims.trim_end_matches('='))
        .ok()?;
    let claims: Value = serde_json::from_slice(&bytes).ok()?;
    DateTime::from_timestamp(claims.get("exp")?.as_i64()?, 0)
}

fn retry_after(response: &Response) -> Duration {
    response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse().ok())
        .map_or(DEFAULT_RATE_LIMIT_DELAY, Duration::from_secs)
}

fn unavailable(err: &reqwest::Error) -> ServiceError {
    ServiceError::Unavailable {
        reason: network_reason(err),
    }
}

pub struct Garmin {
    http: Client,
    url: String,
    api_key: String,
    /// Gardé en mémoire seulement : un jeton ne touche jamais le disque.
    bearer: Option<Bearer>,
}

impl Garmin {
    pub fn new(http: Client, url: &str, api_key: &str) -> Self {
        Self {
            http,
            url: url.trim_end_matches('/').to_owned(),
            api_key: api_key.to_owned(),
            bearer: None,
        }
    }

    /// Échange la clé contre un jeton de 24 h. Un refus de la clé est permanent jusqu'à ce que j'en crée
    /// une autre : il se distingue de toute autre erreur.
    async fn mint(&self) -> Result<Bearer, ServiceError> {
        let response = self
            .http
            .post(format!("{}{TOKEN_PATH}", self.url))
            .header(AUTHORIZATION, format!("ApiKey {}", self.api_key))
            .header(ACCEPT, "*/*")
            .send()
            .await
            .map_err(|err| unavailable(&err))?;
        let status = response.status();
        if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
            return Err(ServiceError::KeyRejected {
                status: status.as_u16(),
            });
        }
        if status == StatusCode::TOO_MANY_REQUESTS {
            return Err(ServiceError::RateLimited {
                retry_after: retry_after(&response),
            });
        }
        if !status.is_success() {
            return Err(ServiceError::Failed {
                status: status.as_u16(),
                call: "token",
            });
        }
        let body = response.text().await.map_err(|err| unavailable(&err))?;
        let jwt = body.trim().trim_matches('"').to_owned();
        if jwt.split('.').count() != 3 || jwt.chars().any(char::is_whitespace) {
            return Err(ServiceError::Unavailable { reason: "token" });
        }
        let now = Utc::now();
        let refresh_after = jwt_expiry(&jwt)
            .map(|exp| exp - TOKEN_MARGIN)
            .filter(|at| *at > now)
            .unwrap_or(now + chrono::TimeDelta::hours(23));
        Ok(Bearer { jwt, refresh_after })
    }

    async fn bearer(&mut self) -> Result<String, ServiceError> {
        match &self.bearer {
            Some(bearer) if Utc::now() < bearer.refresh_after => Ok(bearer.jwt.clone()),
            _ => {
                let bearer = self.mint().await?;
                let jwt = bearer.jwt.clone();
                self.bearer = Some(bearer);
                Ok(jwt)
            }
        }
    }

    /// Un appel de lecture, `calls` compte chaque appel réussi. Un 401 jette le jeton et réessaie
    /// une fois avec un neuf : Garmin a pu le révoquer avant son expiration.
    async fn get(
        &mut self,
        path: &str,
        query: &[(&str, String)],
        calls: &mut u32,
    ) -> Result<Value, ServiceError> {
        for attempt in 0..2 {
            let fresh = self.bearer.is_none();
            let jwt = self.bearer().await?;
            if fresh {
                *calls += 1;
            }
            let response = self
                .http
                .get(format!("{}{path}", self.url))
                .query(query)
                .bearer_auth(&jwt)
                .header(ACCEPT, "application/json")
                .send()
                .await
                .map_err(|err| unavailable(&err))?;
            let status = response.status();
            if status == StatusCode::UNAUTHORIZED && attempt == 0 {
                self.bearer = None;
                continue;
            }
            if status == StatusCode::TOO_MANY_REQUESTS {
                return Err(ServiceError::RateLimited {
                    retry_after: retry_after(&response),
                });
            }
            if !status.is_success() {
                return Err(ServiceError::Failed {
                    status: status.as_u16(),
                    call: call_name(path),
                });
            }
            let body = response
                .json()
                .await
                .map_err(|_| ServiceError::Unavailable { reason: "body" })?;
            *calls += 1;
            return Ok(body);
        }
        Err(ServiceError::Failed {
            status: 401,
            call: call_name(path),
        })
    }
}

impl Garmin {
    /// Le fichier d'origine d'une séance, un ZIP. `None` si Garmin n'en a pas (404).
    async fn download(
        &mut self,
        id: u64,
        calls: &mut u32,
    ) -> Result<Option<Vec<u8>>, ServiceError> {
        for attempt in 0..2 {
            let jwt = self.bearer().await?;
            let response = self
                .http
                .get(format!("{}{DOWNLOAD_PATH}/{id}", self.url))
                .bearer_auth(&jwt)
                .header(
                    ACCEPT,
                    "application/zip, application/x-zip-compressed, application/octet-stream",
                )
                .send()
                .await
                .map_err(|err| unavailable(&err))?;
            let status = response.status();
            if status == StatusCode::UNAUTHORIZED && attempt == 0 {
                self.bearer = None;
                continue;
            }
            if status == StatusCode::NOT_FOUND {
                *calls += 1;
                return Ok(None);
            }
            if status == StatusCode::TOO_MANY_REQUESTS {
                return Err(ServiceError::RateLimited {
                    retry_after: retry_after(&response),
                });
            }
            if !status.is_success() {
                return Err(ServiceError::Failed {
                    status: status.as_u16(),
                    call: "download",
                });
            }
            if response
                .content_length()
                .is_some_and(|length| length > MAX_DOWNLOAD_BYTES as u64)
            {
                return Err(ServiceError::Unavailable {
                    reason: "too large",
                });
            }
            let bytes = response.bytes().await.map_err(|err| unavailable(&err))?;
            if bytes.len() > MAX_DOWNLOAD_BYTES {
                return Err(ServiceError::Unavailable {
                    reason: "too large",
                });
            }
            *calls += 1;
            return Ok(Some(bytes.to_vec()));
        }
        Err(ServiceError::Failed {
            status: 401,
            call: "download",
        })
    }
}

/// Le nom d'un appel de lecture, pour les logs.
fn call_name(path: &str) -> &'static str {
    match path {
        SUMMARIES_PATH => "summaries",
        SLEEP_PATH => "sleep",
        HEART_RATE_PATH => "heart_rate",
        ACTIVITIES_PATH => "activities",
        _ => "other",
    }
}

/// `startDate` et `endDate`, bornes comprises.
fn range(from: NaiveDate, to: NaiveDate) -> Vec<(&'static str, String)> {
    vec![("startDate", from.to_string()), ("endDate", to.to_string())]
}

/// Des plages de `days` jours au plus, de `from` à `to` compris.
pub fn chunks(from: NaiveDate, to: NaiveDate, days: u64) -> Vec<(NaiveDate, NaiveDate)> {
    let mut out = Vec::new();
    let mut start = from;
    while start <= to {
        let end = (start + Days::new(days - 1)).min(to);
        out.push((start, end));
        start = end + Days::new(1);
    }
    out
}

/// Une liste, que Garmin la donne seule ou sous `data`.
fn list(body: Value) -> Result<Vec<Value>, ServiceError> {
    match body {
        Value::Array(items) => Ok(items),
        Value::Object(mut object) => match object.remove("data") {
            Some(Value::Array(items)) => Ok(items),
            _ => Err(ServiceError::Unavailable { reason: "shape" }),
        },
        _ => Err(ServiceError::Unavailable { reason: "shape" }),
    }
}

// ─── Le connecteur ───────────────────────────────────────────────────────────

/// La dernière version envoyée d'un sujet, et celles qu'elle a remplacées.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
struct Subject {
    key: String,
    date: Option<NaiveDate>,
    /// Les clés déjà annulées : un contenu qui reviendrait à l'une d'elles prend une clé voisine, sinon
    /// il retomberait sur un enregistrement annulé, en `duplicate`, et le sujet n'aurait plus de version.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    cancelled: Vec<String>,
}

/// Ce qui survit à un redémarrage. Le perdre coûte un rattrapage en `duplicate`, et une révision
/// survenue entre-temps ne pourrait pas annuler la version d'avant (voir l'architecture).
#[derive(Debug, Default, Deserialize, Serialize)]
struct State {
    /// L'historique a été collecté une fois : la suite ne relit que la fenêtre.
    backfilled: bool,
    /// Le sommeil de tout l'historique a été relu une fois avec sa fréquence cardiaque. Absent de
    /// l'état d'une version d'avant : le passage suivant relit les nuits de l'historique, et annule
    /// celles d'avant la fréquence cardiaque, que l'état a oubliées, par leur ancienne clé.
    #[serde(default)]
    heart_rate_backfilled: bool,
    subjects: BTreeMap<String, Subject>,
}

/// La clé notée pour une séance dont le fichier n'a pas de trace : jamais une clé du journal.
const NO_TRACK: &str = "-";

/// Une version à envoyer, et l'annulation de la précédente s'il y en a une.
struct Unit {
    subject: String,
    date: NaiveDate,
    key: String,
    record: Value,
    previous: Option<String>,
}

/// L'annulation d'une version de Garmin par la même source, sous une clé qui ne dépend que de sa cible :
/// la renvoyer ne fait qu'un `duplicate`.
pub fn cancel_record(source: &str, key: &str, now: DateTime<Utc>) -> Value {
    json!({
        "key": format!("cancel:{key}"),
        "kind": "record.cancel",
        "schema_version": 1,
        "occurred_at": utc(now),
        "payload": { "source": source, "key": key },
    })
}

pub struct Collector {
    config: Config,
    garmin: Garmin,
    ingest: IngestClient,
    state: State,
    /// Passages en échec d'affilée, pour espacer les essais.
    failures: u32,
}

/// L'état rapporté pour la page d'état après un passage en échec (`ok` sinon). Une clé refusée
/// demande une action ; le reste passe seul. Un état qui ne s'écrit plus sur disque n'a pas de code à
/// lui : `service_unavailable`, le connecteur est en panne.
pub fn failure_status(error: &TickError) -> SourceStatus {
    let (state, reason) = match error {
        TickError::Service(ServiceError::KeyRejected { .. }) => {
            (Health::ActionRequired, Reason::KeyRejected)
        }
        TickError::Service(ServiceError::RateLimited { .. }) => {
            (Health::Error, Reason::RateLimited)
        }
        TickError::Service(_) | TickError::State => (Health::Error, Reason::ServiceUnavailable),
        TickError::Ingest { .. } => (Health::Error, Reason::IngestFailed),
    };
    SourceStatus::failed(state, reason)
}

/// Le motif de log d'un échec : jamais de secret ni de donnée.
fn log_failure(error: &TickError, failures: u32) {
    match error {
        TickError::Service(ServiceError::KeyRejected { status }) => {
            error!(status, failures, "clé Garmin refusée : expirée ou révoquée");
        }
        TickError::Service(ServiceError::RateLimited { retry_after }) => {
            info!(
                retry_after_s = retry_after.as_secs(),
                "garmin rate limit, waiting"
            );
        }
        TickError::Service(ServiceError::Failed { status, call }) => {
            warn!(status, call, failures, "garmin refused");
        }
        TickError::Service(ServiceError::Unavailable { reason }) => {
            warn!(reason, failures, "garmin unavailable");
        }
        TickError::Ingest { reason, status } => {
            error!(reason, status, failures, "garmin ingestion unavailable");
        }
        TickError::State => error!(failures, "garmin state not saved"),
    }
}

pub fn retry_delay(failures: u32, error: &TickError) -> Duration {
    match error {
        TickError::Service(ServiceError::RateLimited { retry_after }) => {
            rate_limit_wait(*retry_after)
        }
        _ => backoff(failures),
    }
}

impl Collector {
    pub fn new(config: Config) -> anyhow::Result<Self> {
        let http = http_client()?;
        let state = load_state(&config.state_dir, STATE_FILE)?;
        let garmin = Garmin::new(http.clone(), &config.api_url, &config.api_key);
        let ingest = IngestClient::new(
            http,
            &config.ingest_url,
            &config.token,
            concat!("collector-", env!("CARGO_PKG_VERSION")),
        );
        Ok(Self {
            config,
            garmin,
            ingest,
            state,
            failures: 0,
        })
    }

    pub fn backfilled(&self) -> bool {
        self.state.backfilled
    }

    /// Le nombre de sujets dont une version a été reçue, encore dans la fenêtre gardée.
    pub fn subjects(&self) -> usize {
        self.state.subjects.len()
    }

    async fn pause(&self) {
        tokio::time::sleep(self.config.pause).await;
    }

    /// La série de fréquence cardiaque de chaque jour demandé, un appel par jour, une fois chacun : la
    /// série d'un jour sert à deux nuits. Un 404 est un jour sans série : ses nuits partent sans ses
    /// échantillons. Toute autre erreur fait échouer le passage, comme les autres appels.
    async fn heart_rates(
        &mut self,
        dates: BTreeSet<NaiveDate>,
        report: &mut TickReport,
    ) -> Result<BTreeMap<NaiveDate, Vec<(i64, Value)>>, ServiceError> {
        let mut series = BTreeMap::new();
        for date in dates {
            let query = [("date", date.to_string())];
            let samples = match self
                .garmin
                .get(HEART_RATE_PATH, &query, &mut report.calls)
                .await
            {
                Ok(body) => heart_rate_samples(&body)
                    .ok_or(ServiceError::Unavailable { reason: "shape" })?,
                Err(ServiceError::Failed { status: 404, .. }) => Vec::new(),
                Err(err) => return Err(err),
            };
            series.insert(date, samples);
            self.pause().await;
        }
        Ok(series)
    }

    /// Tout ce que Garmin sait de `from` à `to`, réduit aux listes blanches, une version par sujet. Le
    /// sommeil remonte à `sleep_from`, plus tôt une fois pour relire l'historique avec la fréquence
    /// cardiaque.
    async fn fetch(
        &mut self,
        from: NaiveDate,
        sleep_from: NaiveDate,
        to: NaiveDate,
        report: &mut TickReport,
    ) -> Result<BTreeMap<String, Observation>, ServiceError> {
        let mut found = BTreeMap::new();
        let mut keep = |observation: Option<Observation>, report: &mut TickReport| match observation
        {
            Some(o) => {
                found.insert(o.subject.clone(), o);
            }
            None => report.skipped += 1,
        };
        for (start, end) in chunks(from, to, SUMMARY_CHUNK_DAYS) {
            let body = self
                .garmin
                .get(SUMMARIES_PATH, &range(start, end), &mut report.calls)
                .await?;
            for raw in list(body)? {
                keep(daily_summary(&raw), report);
            }
            self.pause().await;
        }
        let mut nights = Vec::new();
        for (start, end) in chunks(sleep_from, to, SLEEP_CHUNK_DAYS) {
            let body = self
                .garmin
                .get(SLEEP_PATH, &range(start, end), &mut report.calls)
                .await?;
            for raw in list(body)? {
                match sleep(&raw) {
                    Some(night) => nights.push(night),
                    None => report.skipped += 1,
                }
            }
            self.pause().await;
        }
        let mut offset = 0;
        loop {
            let mut query = range(from, to);
            query.push(("start", offset.to_string()));
            query.push(("limit", ACTIVITY_PAGE_SIZE.to_string()));
            let page = list(
                self.garmin
                    .get(ACTIVITIES_PATH, &query, &mut report.calls)
                    .await?,
            )?;
            let length = page.len();
            for raw in page {
                keep(activity(&raw), report);
            }
            if length < ACTIVITY_PAGE_SIZE {
                break;
            }
            offset += length;
            self.pause().await;
        }
        let dates = nights.iter().flat_map(heart_rate_dates).collect();
        let series = self.heart_rates(dates, report).await?;
        for night in nights {
            let days: Vec<&[(i64, Value)]> = heart_rate_dates(&night)
                .iter()
                .filter_map(|date| series.get(date).map(Vec::as_slice))
                .collect();
            let night = with_heart_rate(night, &days);
            found.insert(night.subject.clone(), night);
        }
        Ok(found)
    }

    /// Les versions nouvelles : un sujet déjà reçu avec le même contenu ne repart pas. Avec `legacy`,
    /// une nuit que l'état ne connaît pas annule sa version d'avant la fréquence cardiaque, retrouvée
    /// par sa clé : annuler une clé absente du journal est sans effet.
    fn units(&self, observations: BTreeMap<String, Observation>, legacy: bool) -> Vec<Unit> {
        observations
            .into_values()
            .filter_map(|o| {
                let known = self.state.subjects.get(&o.subject);
                let base = o.key();
                let mut key = base.clone();
                let cancelled = known.map_or(&[][..], |s| &s.cancelled[..]);
                let mut n = 2;
                while cancelled.contains(&key) {
                    key = format!("{base}:{n}");
                    n += 1;
                }
                if known.is_some_and(|s| s.key == key) {
                    return None;
                }
                let previous = match known {
                    Some(s) => Some(s.key.clone()).filter(|k| k != NO_TRACK),
                    None if legacy => o.key_without_heart_rate().filter(|k| *k != key),
                    None => None,
                };
                Some(Unit {
                    record: o.record(&key),
                    subject: o.subject,
                    date: o.date,
                    key,
                    previous,
                })
            })
            .collect()
    }

    /// Chaque nouvelle version, puis l'annulation de la précédente, dans le même lot : l'ingestion les
    /// écrit en une transaction. L'état n'avance que pour un sujet dont tout a été reçu.
    async fn send(
        &mut self,
        units: Vec<Unit>,
        now: DateTime<Utc>,
        report: &mut TickReport,
    ) -> Result<(), TickError> {
        for batch in units.chunks(UNITS_PER_BATCH) {
            let mut records = Vec::new();
            for unit in batch {
                records.push(unit.record.clone());
                if let Some(previous) = &unit.previous {
                    records.push(cancel_record(&self.config.source, previous, now));
                }
            }
            let results = self.ingest.send(&records).await?;
            let mut outcomes = results.iter().map(RecordOutcome::of);
            for unit in batch {
                let version = outcomes.next().expect("un résultat par enregistrement");
                let cancel = unit
                    .previous
                    .as_ref()
                    .map(|_| outcomes.next().expect("un résultat par enregistrement"));
                match version {
                    RecordOutcome::Accepted => report.accepted += 1,
                    RecordOutcome::Duplicate => report.duplicate += 1,
                    RecordOutcome::Rejected => report.rejected += 1,
                }
                if version == RecordOutcome::Rejected || cancel == Some(RecordOutcome::Rejected) {
                    warn!("garmin record rejected, set aside");
                    continue;
                }
                let entry = self.state.subjects.entry(unit.subject.clone()).or_default();
                if let Some(previous) = &unit.previous {
                    report.cancelled += 1;
                    entry.cancelled.push(previous.clone());
                }
                entry.key = unit.key.clone();
                entry.date = Some(unit.date);
            }
            self.save()?;
        }
        Ok(())
    }

    /// La trace de chaque séance avec GPS qui n'en a pas encore, une à une, avec une pause entre deux
    /// téléchargements. Une seule fois par séance : le fichier ne change pas. Une séance sans trace
    /// exploitable est notée `NO_TRACK`, pour ne pas la redemander.
    async fn tracks(
        &mut self,
        workouts: &[Observation],
        now: DateTime<Utc>,
        report: &mut TickReport,
    ) -> Result<(), TickError> {
        for workout in workouts {
            let Some(id) = workout.payload["activity_id"].as_u64() else {
                continue;
            };
            let subject = format!("track:{id}");
            if self.state.subjects.contains_key(&subject) {
                continue;
            }
            self.pause().await;
            let file = self.garmin.download(id, &mut report.calls).await?;
            let track = file
                .as_deref()
                .and_then(fit_from_zip)
                .and_then(|fit| activity_track(workout, &fit));
            match track {
                Some(track) => {
                    let mut found = BTreeMap::new();
                    found.insert(subject, track);
                    let units = self.units(found, false);
                    self.send(units, now, report).await?;
                }
                None => {
                    report.skipped += 1;
                    self.state.subjects.insert(
                        subject,
                        Subject {
                            key: NO_TRACK.to_owned(),
                            date: Some(workout.date),
                            cancelled: Vec::new(),
                        },
                    );
                    self.save()?;
                }
            }
        }
        Ok(())
    }

    /// Un passage. Garmin d'abord, l'ingestion ensuite : un appel authentifié compte comme un contact,
    /// et le signal de vie ne part qu'après une interrogation réussie. Une clé refusée n'en envoie
    /// donc aucun, et le seuil de silence de la source alerte.
    pub async fn tick(&mut self) -> Result<TickReport, TickError> {
        let mut report = TickReport::default();
        let now = Utc::now();
        let today = paris_today(now);
        let days = if self.state.backfilled {
            REREAD_DAYS
        } else {
            BACKFILL_DAYS - 1
        };
        let from = today - Days::new(days);
        // Une fois, après la mise à jour qui a ajouté la fréquence cardiaque : tout le sommeil.
        let legacy = !self.state.heart_rate_backfilled;
        let sleep_from = if legacy {
            today - Days::new(BACKFILL_DAYS - 1)
        } else {
            from
        };
        let observations = self.fetch(from, sleep_from, today, &mut report).await?;
        let workouts: Vec<Observation> = observations
            .values()
            .filter(|o| has_gps(o))
            .cloned()
            .collect();
        let units = self.units(observations, legacy);
        self.send(units, now, &mut report).await?;
        self.ingest.heartbeat().await?;
        self.tracks(&workouts, now, &mut report).await?;
        self.state.backfilled = true;
        self.state.heart_rate_backfilled = true;
        let horizon = today - Days::new(KEEP_SUBJECT_DAYS);
        self.state
            .subjects
            .retain(|_, s| s.date.is_none_or(|d| d >= horizon));
        // Toujours réécrit : la date du fichier sert au contrôle de santé du conteneur.
        self.save()?;
        Ok(report)
    }

    fn save(&self) -> Result<(), TickError> {
        save_state(&self.config.state_dir, STATE_FILE, &self.state).map_err(|_| TickError::State)
    }

    /// Un passage, son journal et son état rapporté, avec l'échéance de la clé ; renvoie le délai avant
    /// le suivant. Le rapport d'état vient après le passage et n'en change ni le résultat ni le signal
    /// de vie.
    pub async fn pass(&mut self) -> Duration {
        let (delay, mut status) = match self.tick().await {
            Ok(report) => {
                self.failures = 0;
                info!(
                    calls = report.calls,
                    accepted = report.accepted,
                    duplicate = report.duplicate,
                    rejected = report.rejected,
                    cancelled = report.cancelled,
                    skipped = report.skipped,
                    "garmin pass completed"
                );
                (self.config.interval, SourceStatus::ok())
            }
            Err(err) => {
                self.failures += 1;
                log_failure(&err, self.failures);
                (retry_delay(self.failures, &err), failure_status(&err))
            }
        };
        status.expires_at = self.config.key_expires_at;
        report_status(&self.ingest, status, delay, "garmin").await;
        delay
    }

    /// Jusqu'à l'arrêt demandé, comme le connecteur Last.fm : aucune erreur ne l'arrête.
    pub async fn run(mut self, mut stop: watch::Receiver<bool>) {
        while !*stop.borrow() {
            let delay = self.pass().await;
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                _ = stop.changed() => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(text: &str) -> NaiveDate {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
    }

    fn keys(value: &Value) -> Vec<&str> {
        value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect()
    }

    /// Des réponses de Garmin d'après la structure relevée sur le vrai compte, valeurs fictives,
    /// avec ce que la liste blanche doit écarter.
    fn raw_summary() -> Value {
        json!({
            "calendarDate": "2026-11-03", "includesWellnessData": true, "includesActivityData": true,
            "userProfilePk": 123_456, "uuid": "u-u-i-d",
            "movement": {"steps": {"value": 8412, "distanceInMeters": 6_123.456_789, "goal": 7000}},
            "calories": {"burnedActive": 512.0, "burnedResting": 1838, "burnedTotal": 2350},
            "heartRate": {"restingValue": 52}, "stress": {"avg": 30},
            "wellnessChronology": {"startTimestampGmt": "2026-11-02T23:00:00.0"},
        })
    }

    fn raw_sleep() -> Value {
        json!({
            "calendarDate": "2026-11-03", "id": 1_793_000_000_000_u64, "userProfilePK": 123_456,
            "sleepTimeSeconds": 25_980, "deepSleepSeconds": 4_920, "lightSleepSeconds": 14_400,
            "remSleepSeconds": 6_660, "awakeSleepSeconds": 660, "napTimeSeconds": 1_200,
            "sleepStartTimestampGMT": 1_793_659_260_000_i64, "sleepEndTimestampGMT": 1_793_685_900_000_i64,
        })
    }

    fn raw_activity() -> Value {
        json!({
            "activityId": 98_765_432_101_u64, "activityName": "Course du matin",
            "activityType": {"typeKey": "running", "typeId": 1},
            "startTimeGMT": "2026-11-03 06:30:00", "startTimeLocal": "2026-11-03 07:30:00",
            "beginTimestamp": 1_793_341_800_000_i64,
            "distance": 5_012.300_048_828_125, "duration": 1802.4, "movingDuration": 1780.0,
            "elapsedDuration": 1900.0, "averageSpeed": 2.778_000_116_348_266_6, "averageHR": 151.0,
            "maxHR": 176.0, "calories": 389.0, "steps": 5100,
            "ownerDisplayName": "fictif", "ownerFullName": "Personne Fictive",
            "startLatitude": 48.85, "startLongitude": 2.35, "endLatitude": 48.86,
            "endLongitude": 2.36, "locationName": "Ville fictive",
        })
    }

    #[test]
    fn the_whitelists_are_frozen() {
        let summary = daily_summary(&raw_summary()).unwrap();
        assert_eq!(
            keys(&summary.payload),
            [
                "calendar_date",
                "calories_active",
                "calories_resting",
                "calories_total",
                "distance_m",
                "steps"
            ]
        );
        let series = raw_heart_rate(1_793_659_200_000, 1_793_686_000_000, |_| json!(52));
        let samples = heart_rate_samples(&series).unwrap();
        let night = with_heart_rate(sleep(&raw_sleep()).unwrap(), &[&samples]);
        assert_eq!(
            keys(&night.payload),
            [
                "awake_seconds",
                "calendar_date",
                "deep_seconds",
                "end",
                "heart_rate",
                "light_seconds",
                "rem_seconds",
                "sleep_seconds",
                "start"
            ]
        );
        // Les échantillons seuls : ni le profil, ni les autres chiffres de la série.
        assert!(
            night.payload["heart_rate"]
                .as_array()
                .unwrap()
                .iter()
                .all(|s| { s.as_array().unwrap().len() == 2 && s[1] == 52 })
        );
        let workout = activity(&raw_activity()).unwrap();
        assert_eq!(
            keys(&workout.payload),
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
                "start",
                "start_lat",
                "start_lon",
                "type"
            ]
        );
        for observation in [&summary, &night, &workout] {
            let text = observation.record(&observation.key()).to_string();
            for secret in ["Fictive", "\"fictif\"", "u-u-i-d", "123456", "Course"] {
                assert!(!text.contains(secret), "{secret} dans {text}");
            }
        }
    }

    /// Une série `dailyHeartRate` fictive d'après la structure relevée sur le vrai compte : un
    /// échantillon toutes les deux minutes de `from` (ms) à `to`, des vides, et ce qui n'entre jamais.
    fn raw_heart_rate(from: i64, to: i64, bpm: impl Fn(i64) -> Value) -> Value {
        let values: Vec<Value> = (from..to)
            .step_by(120_000)
            .map(|at| json!([at, bpm(at)]))
            .collect();
        json!({
            "userProfilePK": 123_456, "calendarDate": "2026-11-03",
            "startTimestampGMT": "2026-11-02T23:00:00.0", "endTimestampGMT": "2026-11-03T22:58:00.0",
            "maxHeartRate": 140, "minHeartRate": 44, "restingHeartRate": 50,
            "lastSevenDaysAvgRestingHeartRate": 51,
            "heartRateValueDescriptors": [{"key": "timestamp", "index": 0}, {"key": "heartrate", "index": 1}],
            "heartRateValues": values,
        })
    }

    #[test]
    fn a_night_takes_the_heart_rate_of_its_two_days_within_its_bounds() {
        let night = sleep(&raw_sleep()).unwrap();
        assert_eq!(heart_rate_dates(&night), [d("2026-11-02"), d("2026-11-03")]);
        // Début 22:41 UTC le 2, fin 06:05 UTC le 3 ; la série du 3 commence à 23:00 UTC le 2.
        let (start, end) = (1_793_659_260_000_i64, 1_793_685_900_000_i64);
        let midnight = 1_793_660_400_000_i64;
        let before = heart_rate_samples(&raw_heart_rate(midnight - 86_400_000, midnight, |at| {
            if at < start - 60_000 {
                json!(90)
            } else {
                json!(58.0)
            }
        }))
        .unwrap();
        let after =
            heart_rate_samples(&raw_heart_rate(
                midnight,
                midnight + 86_400_000,
                |at| match at {
                    at if at == midnight + 240_000 => Value::Null,
                    at if at == midnight + 3_600_000 => json!(47),
                    at if at > end => json!(120),
                    _ => json!(53),
                },
            ))
            .unwrap();
        let night = with_heart_rate(night, &[&before, &after]);
        let samples = night.payload["heart_rate"].as_array().unwrap();
        // De 22:42 (le premier échantillon après 22:41) à 06:04 compris, moins le vide.
        assert_eq!(samples.first().unwrap(), &json!([60, 58]));
        assert_eq!(samples.last().unwrap(), &json!([26_580, 53]));
        assert_eq!(samples.len(), (26_580 - 60) / 120 + 1 - 1);
        assert!(
            samples
                .iter()
                .all(|s| s[1] != 90 && s[1] != 120 && !s[1].is_null())
        );
        assert!(samples.contains(&json!([1_140 + 3_600, 47])));
        assert!(
            samples
                .windows(2)
                .all(|w| w[0][0].as_i64() < w[1][0].as_i64())
        );
        // La clé d'avant la fréquence cardiaque : celle de la même nuit sans échantillons.
        let bare = sleep(&raw_sleep()).unwrap();
        assert_eq!(night.key_without_heart_rate(), Some(bare.key()));
        assert_ne!(night.key(), bare.key());
        assert_eq!(bare.key_without_heart_rate(), None);
    }

    #[test]
    fn a_night_without_samples_keeps_the_payload_of_before() {
        let night = sleep(&raw_sleep()).unwrap();
        let empty = heart_rate_samples(&json!({"heartRateValues": null})).unwrap();
        assert!(empty.is_empty());
        assert_eq!(with_heart_rate(night.clone(), &[&empty]), night);
        assert_eq!(heart_rate_samples(&json!([])), None, "illisible");
        let mut unbounded = raw_sleep();
        unbounded["sleepEndTimestampGMT"] = Value::Null;
        let unbounded = sleep(&unbounded).unwrap();
        assert!(heart_rate_dates(&unbounded).is_empty());
        let series =
            heart_rate_samples(&raw_heart_rate(0, i64::from(i32::MAX), |_| json!(50))).unwrap();
        assert_eq!(with_heart_rate(unbounded.clone(), &[&series]), unbounded);
    }

    #[test]
    fn observations_take_garmin_values_normalized() {
        let summary = daily_summary(&raw_summary()).unwrap();
        assert_eq!(summary.subject, "summary:2026-11-03");
        assert_eq!(summary.occurred_at, "2026-11-03T12:00:00+01:00");
        assert_eq!(summary.payload["steps"], 8412);
        assert_eq!(summary.payload["distance_m"], json!(6123.457));
        assert_eq!(summary.payload["calories_active"], 512);

        let night = sleep(&raw_sleep()).unwrap();
        assert_eq!(night.subject, "sleep:2026-11-03");
        assert_eq!(night.payload["start"], "2026-11-02T22:41:00Z");
        assert_eq!(night.payload["end"], "2026-11-03T06:05:00Z");

        let workout = activity(&raw_activity()).unwrap();
        assert_eq!(workout.subject, "activity:98765432101");
        assert_eq!(workout.occurred_at, "2026-11-03T07:30:00+01:00");
        assert_eq!(workout.payload["start"], "2026-11-03T06:30:00Z");
        assert_eq!(workout.payload["avg_speed_mps"], json!(2.778));
        assert_eq!(workout.payload["avg_hr"], 151);
    }

    #[test]
    fn a_day_without_data_or_a_night_without_sleep_gives_nothing() {
        let mut summary = raw_summary();
        summary["includesWellnessData"] = json!(false);
        assert_eq!(daily_summary(&summary), None);
        let mut summary = raw_summary();
        summary["movement"]["steps"]["value"] = Value::Null;
        assert_eq!(daily_summary(&summary), None);
        for seconds in [json!(null), json!(0)] {
            let mut night = raw_sleep();
            night["sleepTimeSeconds"] = seconds;
            assert_eq!(sleep(&night), None);
        }
        let mut workout = raw_activity();
        workout["startTimeGMT"] = json!("hier");
        assert_eq!(activity(&workout), None);
        // Sans heure locale lisible, le décalage reste inconnu.
        let mut workout = raw_activity();
        workout["startTimeLocal"] = Value::Null;
        assert_eq!(
            activity(&workout).unwrap().occurred_at,
            "2026-11-03T06:30:00Z"
        );
    }

    #[test]
    fn the_key_depends_on_the_subject_and_the_content_only() {
        let a = daily_summary(&raw_summary()).unwrap();
        let mut raw = raw_summary();
        raw["heartRate"] = json!({"restingValue": 60});
        assert_eq!(
            daily_summary(&raw).unwrap().key(),
            a.key(),
            "hors liste blanche"
        );
        raw["movement"]["steps"]["value"] = json!(9000);
        let b = daily_summary(&raw).unwrap();
        assert_ne!(b.key(), a.key());
        assert!(a.key().starts_with("summary:2026-11-03:"));
        assert_eq!(a.key().len(), "summary:2026-11-03:".len() + 16);
    }

    #[test]
    fn paris_noon_follows_the_change_of_time() {
        assert_eq!(paris_noon(d("2026-03-28")), "2026-03-28T12:00:00+01:00");
        assert_eq!(paris_noon(d("2026-03-29")), "2026-03-29T12:00:00+02:00");
        assert_eq!(paris_noon(d("2026-10-24")), "2026-10-24T12:00:00+02:00");
        assert_eq!(paris_noon(d("2026-10-25")), "2026-10-25T12:00:00+01:00");
        let at = |text: &str| text.parse::<DateTime<Utc>>().unwrap();
        assert_eq!(paris_today(at("2026-11-03T23:30:00Z")), d("2026-11-04"));
        assert_eq!(paris_today(at("2026-07-03T21:59:00Z")), d("2026-07-03"));
        assert_eq!(paris_today(at("2026-07-03T22:00:00Z")), d("2026-07-04"));
    }

    #[test]
    fn ranges_are_cut_in_chunks() {
        let cut = chunks(d("2026-01-01"), d("2026-03-01"), 31);
        assert_eq!(
            cut,
            [
                (d("2026-01-01"), d("2026-01-31")),
                (d("2026-02-01"), d("2026-03-01")),
            ]
        );
        assert_eq!(chunks(d("2026-01-01"), d("2026-01-01"), 31).len(), 1);
        let from = d("2026-11-04") - Days::new(BACKFILL_DAYS - 1);
        assert_eq!(chunks(from, d("2026-11-04"), SUMMARY_CHUNK_DAYS).len(), 13);
        assert_eq!(chunks(from, d("2026-11-04"), SLEEP_CHUNK_DAYS).len(), 13);
    }

    #[test]
    fn the_expiry_is_read_from_the_token() {
        let claims =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(r#"{"exp":1793400000}"#);
        let jwt = format!("e30.{claims}.signature");
        assert_eq!(jwt_expiry(&jwt), DateTime::from_timestamp(1_793_400_000, 0));
        assert_eq!(jwt_expiry("pas-un-jeton"), None);
    }

    #[test]
    fn a_track_keeps_its_turns_and_stays_under_the_cap() {
        // Une ligne droite vers le nord, un virage, puis vers l'est : un point par seconde.
        let mut points = Vec::new();
        for i in 0..600 {
            points.push([45.0 + f64::from(i) * 1e-5, -1.0, f64::from(i)]);
        }
        for i in 1..600 {
            points.push([45.006, -1.0 + f64::from(i) * 1e-5, f64::from(600 + i)]);
        }
        let kept = simplify(&points, TRACK_TOLERANCE_M, MAX_TRACK_POINTS);
        assert_eq!(kept.len(), 3, "le départ, le virage, l'arrivée");
        assert_eq!(kept[0], json!([45.0, -1.0, 0]));
        assert!((599..=600).contains(&kept[1][2].as_i64().unwrap()));
        assert_eq!(kept[2], json!([45.006, -0.99401, 1199]));
        // Un zigzag de 20 m tous les points : la tolérance double jusqu'à tenir sous le plafond.
        let zigzag: Vec<[f64; 3]> = (0..5000)
            .map(|i| {
                let offset = if i % 2 == 0 { 0.0 } else { 2e-4 };
                [45.0 + f64::from(i) * 1e-5, -1.0 + offset, f64::from(i)]
            })
            .collect();
        let kept = simplify(&zigzag, TRACK_TOLERANCE_M, MAX_TRACK_POINTS);
        assert!(
            kept.len() <= MAX_TRACK_POINTS && kept.len() >= 2,
            "{}",
            kept.len()
        );
        assert_eq!(kept[0], json!([45.0, -1.0, 0]));
    }

    #[test]
    fn the_key_expires_at_the_end_of_its_paris_day() {
        assert_eq!(
            key_expiry("2027-11-04").unwrap().to_rfc3339(),
            "2027-11-04T23:00:00+00:00"
        );
        assert_eq!(
            key_expiry("2027-06-30").unwrap().to_rfc3339(),
            "2027-06-30T22:00:00+00:00"
        );
        // La veille du passage à l'heure d'hiver : minuit est encore à l'heure d'été.
        assert_eq!(
            key_expiry("2027-10-30").unwrap().to_rfc3339(),
            "2027-10-30T22:00:00+00:00"
        );
        for invalid in [
            "",
            "2027-6-30",
            "2027-02-30",
            "30/06/2027",
            "2027-06-30T00:00",
            " 2027-06-30",
        ] {
            assert_eq!(key_expiry(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn a_rejected_key_asks_for_an_action_and_the_rest_is_an_error() {
        let status = |error| {
            let s = failure_status(&error);
            assert!(s.is_consistent());
            (s.state, s.reason.unwrap())
        };
        assert_eq!(
            status(TickError::Service(ServiceError::KeyRejected {
                status: 401
            })),
            (Health::ActionRequired, Reason::KeyRejected)
        );
        assert_eq!(
            status(TickError::Service(ServiceError::RateLimited {
                retry_after: Duration::from_secs(1)
            })),
            (Health::Error, Reason::RateLimited)
        );
        for error in [
            TickError::Service(ServiceError::Failed {
                status: 500,
                call: "summaries",
            }),
            TickError::Service(ServiceError::Unavailable { reason: "connect" }),
            TickError::State,
        ] {
            assert_eq!(status(error), (Health::Error, Reason::ServiceUnavailable));
        }
        assert_eq!(
            status(TickError::Ingest {
                reason: "status",
                status: Some(503)
            }),
            (Health::Error, Reason::IngestFailed)
        );
    }
}
