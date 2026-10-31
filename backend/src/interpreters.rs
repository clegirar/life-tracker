//! Les interprètes : un par type de donnée, versionné. Fonctions pures, sans heure courante ni
//! accès extérieur. Ajouter un type, c'est ajouter une entrée à `INTERPRETERS` (principe 12).

use chrono::{DateTime, Utc};
use serde_json::{Value, json};

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
}
