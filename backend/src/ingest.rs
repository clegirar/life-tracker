//! Vérification de l'enveloppe d'un enregistrement, sans accès à la base.

use std::collections::HashSet;

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectReason {
    KindNotDeclared,
    MissingField,
    InvalidField,
    OccurredAtTooFarInFuture,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rejection {
    pub reason: RejectReason,
    pub field: &'static str,
}

impl Rejection {
    pub const fn invalid(field: &'static str) -> Self {
        Self {
            reason: RejectReason::InvalidField,
            field,
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct Envelope {
    pub key: String,
    pub kind: String,
    pub schema_version: i16,
    pub occurred_at: DateTime<Utc>,
    pub utc_offset_minutes: Option<i16>,
    /// Charge utile sérialisée : sa conversion en jsonb revient à la base.
    pub payload: String,
}

fn required<'a>(record: &'a Value, field: &'static str) -> Result<&'a Value, Rejection> {
    match record.get(field) {
        Some(value) => Ok(value),
        None => Err(Rejection {
            reason: RejectReason::MissingField,
            field,
        }),
    }
}

pub fn parse_envelope(
    record: &Value,
    granted_kinds: &HashSet<String>,
    now: DateTime<Utc>,
) -> Result<Envelope, Rejection> {
    if !record.is_object() {
        return Err(Rejection::invalid("record"));
    }

    let key = required(record, "key")?
        .as_str()
        // Mêmes bornes que la contrainte de core.records ; PostgreSQL refuse NUL dans un texte.
        .filter(|k| (1..=512).contains(&k.chars().count()) && !k.contains('\0'))
        .ok_or(Rejection::invalid("key"))?;

    let kind = required(record, "kind")?
        .as_str()
        .ok_or(Rejection::invalid("kind"))?;
    // Le registre seul décide : un type inconnu ou révoqué est absent de cet ensemble.
    if !granted_kinds.contains(kind) {
        return Err(Rejection {
            reason: RejectReason::KindNotDeclared,
            field: "kind",
        });
    }

    let schema_version = required(record, "schema_version")?
        .as_i64()
        .and_then(|v| i16::try_from(v).ok())
        .filter(|v| *v > 0)
        .ok_or(Rejection::invalid("schema_version"))?;

    let occurred_at = required(record, "occurred_at")?
        .as_str()
        .ok_or(Rejection::invalid("occurred_at"))?;
    let (occurred_at, utc_offset_minutes) =
        parse_occurred_at(occurred_at).ok_or(Rejection::invalid("occurred_at"))?;
    if occurred_at > now + Duration::days(1) {
        return Err(Rejection {
            reason: RejectReason::OccurredAtTooFarInFuture,
            field: "occurred_at",
        });
    }

    // La charge utile doit être présente ; son contenu n'est jamais examiné ici.
    let payload = required(record, "payload")?.to_string();

    Ok(Envelope {
        key: key.to_owned(),
        kind: kind.to_owned(),
        schema_version,
        occurred_at,
        utc_offset_minutes,
        payload,
    })
}

/// RFC 3339 strict. `Z` et `-00:00` (le sens que leur donne la RFC) signifient « décalage inconnu » ;
/// `+00:00` est un vrai décalage nul.
pub fn parse_occurred_at(value: &str) -> Option<(DateTime<Utc>, Option<i16>)> {
    // chrono accepte aussi une espace entre la date et l'heure, pas le contrat.
    if !matches!(value.as_bytes().get(10), Some(b'T' | b't')) {
        return None;
    }
    let parsed = DateTime::parse_from_rfc3339(value).ok()?;
    let unknown_offset = value.ends_with(['Z', 'z']) || value.ends_with("-00:00");
    let offset = if unknown_offset {
        None
    } else {
        // Mêmes bornes que la contrainte de core.records.
        let minutes = i16::try_from(parsed.offset().local_minus_utc() / 60).ok()?;
        if !(-900..=900).contains(&minutes) {
            return None;
        }
        Some(minutes)
    };
    Some((parsed.with_timezone(&Utc), offset))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn offset_of(value: &str) -> Option<Option<i16>> {
        parse_occurred_at(value).map(|(_, offset)| offset)
    }

    #[test]
    fn offsets() {
        assert_eq!(offset_of("2026-10-13T09:42:10Z"), Some(None));
        assert_eq!(offset_of("2026-10-13T09:42:10z"), Some(None));
        assert_eq!(offset_of("2026-10-13T09:42:10-00:00"), Some(None));
        assert_eq!(offset_of("2026-10-13T09:42:10+00:00"), Some(Some(0)));
        assert_eq!(offset_of("2026-10-13T09:42:10+02:00"), Some(Some(120)));
        assert_eq!(offset_of("2026-10-13T09:42:10-09:30"), Some(Some(-570)));
        assert_eq!(offset_of("2026-10-13T09:42:10+15:00"), Some(Some(900)));
        assert_eq!(
            offset_of("2026-10-13T09:42:10.123456+02:00"),
            Some(Some(120))
        );
    }

    #[test]
    fn invalid_dates() {
        for value in [
            "2026-10-13T09:42:10+15:01",
            "2026-10-13T09:42:10",
            "2026-10-13 09:42:10Z",
            "2026-13-13T09:42:10Z",
            "13/10/2026",
            "",
        ] {
            assert_eq!(offset_of(value), None, "{value}");
        }
    }

    #[test]
    fn stored_in_utc() {
        let (at, _) = parse_occurred_at("2026-10-13T09:42:10+02:00").unwrap();
        assert_eq!(at.to_rfc3339(), "2026-10-13T07:42:10+00:00");
    }

    #[test]
    fn checks_every_field() {
        let granted = HashSet::from(["coffee.cup".to_owned()]);
        let now = Utc::now();
        let full = json!({"key": "k", "kind": "coffee.cup", "schema_version": 1,
                          "occurred_at": "2026-10-13T09:42:10Z", "payload": {}});
        assert!(parse_envelope(&full, &granted, now).is_ok());
        for field in ["key", "kind", "schema_version", "occurred_at", "payload"] {
            let mut record = full.clone();
            record.as_object_mut().unwrap().remove(field);
            assert_eq!(
                parse_envelope(&record, &granted, now),
                Err(Rejection {
                    reason: RejectReason::MissingField,
                    field
                })
            );
        }
    }
}
