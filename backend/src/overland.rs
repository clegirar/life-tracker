//! La route d'adaptation d'Overland (iOS) : un lot de Features GeoJSON traduit en enregistrements de
//! l'API d'ingestion, qui passent ensuite par les mêmes vérifications et le même ajout que
//! `POST /v1/records` (voir `api::post_overland`). Sans état et sans accès à la base.
//!
//! Ce qu'envoie Overland (vérifié dans son code, `GLManager.m`) : `{"locations": [Feature…]}`, plus
//! parfois `current` (le dernier point, renvoyé plus tard dans la file) et `trip` (le trajet en cours),
//! tous deux ignorés. Chaque Feature a une géométrie `Point` (`[lon, lat]`) et des propriétés :
//! - un point : `timestamp` (ISO 8601 à la seconde, en UTC), `altitude`, `speed`, `course`,
//!   `horizontal_accuracy`, `vertical_accuracy`, `motion` (liste), et des métadonnées (`device_id`,
//!   `wifi`, `wifi_bssid`, `battery_state`, `battery_level`, `unique_id`, statistiques de suivi) ;
//! - une visite iOS : `action: "visit"`, `arrival_date` et `departure_date` (`null` quand iOS ne les
//!   connaît pas : départ pas encore arrivé, arrivée d'avant le suivi), `horizontal_accuracy`, et son
//!   `timestamp` est l'heure de l'envoi, pas de l'arrivée ;
//! - une autre `action` (statistiques de suivi) ou `type: "trip"` (un trajet terminé) : ignorées.
//!
//! **Liste blanche.** Un point garde `{lat, lon, horizontal_accuracy, altitude, speed, motion}`, une
//! visite `{lat, lon, arrival, departure, horizontal_accuracy}`. Le reste (Wi-Fi, batterie, identifiants
//! d'appareil, trajets) n'entre jamais au journal.
//!
//! **Visites.** iOS signale une visite deux fois : à l'arrivée, départ inconnu, puis au départ, complète.
//! Seule la version complète entre, sous la clé `visit:<arrivée en ms>` : il n'y a jamais deux versions
//! à annuler l'une par l'autre, et un renvoi est un doublon.

use serde_json::{Value, json};

use crate::ingest::parse_occurred_at;

/// Overland est réglé sur 200 points par lot (voir `docs/deploiement.md`).
pub const MAX_FEATURES: usize = 500;

/// Les Features à traduire, ou `None` si le document n'est pas un lot d'Overland.
pub fn features(document: &Value) -> Option<&Vec<Value>> {
    document.get("locations").and_then(Value::as_array)
}

/// L'instant d'un champ des propriétés, tel qu'envoyé (texte RFC 3339) et en millisecondes.
fn instant<'a>(properties: &'a Value, field: &str) -> Option<(&'a str, i64)> {
    let text = properties.get(field).and_then(Value::as_str)?;
    let (at, _) = parse_occurred_at(text)?;
    Some((text, at.timestamp_millis()))
}

/// `[lon, lat]` de la géométrie, deux nombres.
fn position(feature: &Value) -> Option<(Value, Value)> {
    let coordinates = feature.get("geometry")?.get("coordinates")?.as_array()?;
    let (lon, lat) = (coordinates.first()?, coordinates.get(1)?);
    (lon.is_number() && lat.is_number()).then(|| (lat.clone(), lon.clone()))
}

/// Un champ des propriétés, ou `null`.
fn field(properties: &Value, name: &str) -> Value {
    properties.get(name).cloned().unwrap_or(Value::Null)
}

/// Un enregistrement par Feature utile, dans l'ordre du lot. Une Feature illisible (sans instant ou
/// sans position) devient `null`, que `parse_envelope` rejette et que le compteur de rejets de la
/// source compte ; une Feature ignorée ne donne rien.
pub fn translate(features: &[Value]) -> Vec<Value> {
    features.iter().filter_map(record).collect()
}

fn record(feature: &Value) -> Option<Value> {
    let Some(properties) = feature.get("properties").filter(|p| p.is_object()) else {
        return Some(Value::Null);
    };
    if properties.get("type").and_then(Value::as_str) == Some("trip") {
        return None;
    }
    match properties.get("action") {
        None | Some(Value::Null) => Some(location(feature, properties).unwrap_or(Value::Null)),
        Some(action) if action == "visit" => visit(feature, properties),
        Some(_) => None,
    }
}

fn location(feature: &Value, properties: &Value) -> Option<Value> {
    let (at, ms) = instant(properties, "timestamp")?;
    let (lat, lon) = position(feature)?;
    Some(json!({
        "key": format!("point:{ms}"),
        "kind": "overland.location",
        "schema_version": 1,
        "occurred_at": at,
        "payload": {
            "lat": lat,
            "lon": lon,
            "horizontal_accuracy": field(properties, "horizontal_accuracy"),
            "altitude": field(properties, "altitude"),
            "speed": field(properties, "speed"),
            "motion": field(properties, "motion"),
        },
    }))
}

/// Une visite n'entre que complète : sans arrivée ou sans départ, elle est ignorée (la version
/// complète suivra, ou l'arrivée est d'avant le suivi). Sans position, elle est illisible.
fn visit(feature: &Value, properties: &Value) -> Option<Value> {
    let (arrival, ms) = instant(properties, "arrival_date")?;
    let (departure, _) = instant(properties, "departure_date")?;
    let Some((lat, lon)) = position(feature) else {
        return Some(Value::Null);
    };
    Some(json!({
        "key": format!("visit:{ms}"),
        "kind": "overland.visit",
        "schema_version": 1,
        "occurred_at": arrival,
        "payload": {
            "lat": lat,
            "lon": lon,
            "arrival": arrival,
            "departure": departure,
            "horizontal_accuracy": field(properties, "horizontal_accuracy"),
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(timestamp: Value) -> Value {
        json!({
            "type": "Feature",
            "geometry": {"type": "Point", "coordinates": [5.3, 43.2]},
            "properties": {
                "timestamp": timestamp, "altitude": 12, "speed": 1.25, "course": 90,
                "horizontal_accuracy": 8, "vertical_accuracy": 4, "motion": ["walking"],
                "device_id": "mon-iphone", "wifi": "Réseau fictif", "wifi_bssid": "00:00:00:00:00:00",
                "battery_state": "unplugged", "battery_level": 0.5, "unique_id": "X",
            },
        })
    }

    fn visit_feature(arrival: Value, departure: Value) -> Value {
        json!({
            "type": "Feature",
            "geometry": {"type": "Point", "coordinates": [5.3, 43.2]},
            "properties": {
                "timestamp": "2026-10-20T10:00:00Z", "action": "visit",
                "arrival_date": arrival, "departure_date": departure,
                "horizontal_accuracy": 35, "wifi": "Réseau fictif", "battery_level": 0.5,
            },
        })
    }

    #[test]
    fn a_point_keeps_its_whitelist_only() {
        let records = translate(&[point(json!("2026-10-20T08:00:05Z"))]);
        assert_eq!(
            records,
            [json!({
                "key": "point:1792483205000",
                "kind": "overland.location",
                "schema_version": 1,
                "occurred_at": "2026-10-20T08:00:05Z",
                "payload": {"lat": 43.2, "lon": 5.3, "horizontal_accuracy": 8, "altitude": 12,
                            "speed": 1.25, "motion": ["walking"]},
            })]
        );
    }

    #[test]
    fn only_a_complete_visit_enters_under_its_arrival() {
        let records = translate(&[
            visit_feature(json!("2026-10-20T08:00:00Z"), Value::Null),
            visit_feature(Value::Null, json!("2026-10-20T09:00:00Z")),
            visit_feature(json!("2026-10-20T08:00:00Z"), json!("2026-10-20T09:00:00Z")),
        ]);
        assert_eq!(
            records,
            [json!({
                "key": "visit:1792483200000",
                "kind": "overland.visit",
                "schema_version": 1,
                "occurred_at": "2026-10-20T08:00:00Z",
                "payload": {"lat": 43.2, "lon": 5.3, "arrival": "2026-10-20T08:00:00Z",
                            "departure": "2026-10-20T09:00:00Z", "horizontal_accuracy": 35},
            })]
        );
    }

    #[test]
    fn trips_and_tracking_logs_are_ignored_and_an_unreadable_feature_is_null() {
        let trip = json!({"type": "Feature", "geometry": {"type": "Point", "coordinates": [5.3, 43.2]},
                          "properties": {"type": "trip", "timestamp": "2026-10-20T08:00:00Z"}});
        let log = json!({"type": "Feature", "properties": {"action": "paused_location_updates",
                         "timestamp": "2026-10-20T08:00:00Z"}});
        let mut no_geometry = point(json!("2026-10-20T08:00:05Z"));
        no_geometry.as_object_mut().unwrap().remove("geometry");
        let records = translate(&[
            trip,
            log,
            point(json!("pas une date")),
            no_geometry,
            json!("x"),
        ]);
        assert_eq!(records, [Value::Null, Value::Null, Value::Null]);
    }
}
