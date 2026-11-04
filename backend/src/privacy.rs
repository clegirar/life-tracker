//! Les zones de confidentialité, comme celles de Strava : autour de chez moi, le MCP ne voit ni le
//! début ou la fin d'une séance, ni les points de sa trace (voir le journal des décisions, 4 nov. 2026).
//!
//! Configurées sur le NAS seulement (`PRIVACY_ZONES` de `serve`, dans un fichier de secrets), jamais
//! dans le dépôt, le journal ni les logs : ce module ne dérive pas `Debug` et rien n'affiche une zone.
//! Appliquées à la lecture, aux seules réponses du jeton de service ; la session voit tout.
//!
//! **Sûr par défaut.** Sans zone, ou avec une valeur illisible, le jeton de service ne reçoit aucune
//! coordonnée ni aucune trace.

use std::sync::Arc;

use serde_json::{Map, Value};

const EARTH_RADIUS_M: f64 = 6_371_008.8;
/// Au-delà, ce n'est plus une zone autour d'un lieu mais une région : sans doute une erreur de saisie.
const MAX_RADIUS_M: f64 = 50_000.0;

struct Zone {
    lat: f64,
    lon: f64,
    radius_m: f64,
}

impl Zone {
    /// Les coordonnées d'un point en mètres sur un plan centré sur la zone : à l'échelle d'une zone,
    /// l'écart à la sphère est négligeable.
    fn local(&self, lat: f64, lon: f64) -> (f64, f64) {
        let meters_per_degree = EARTH_RADIUS_M * std::f64::consts::PI / 180.0;
        (
            (lon - self.lon) * meters_per_degree * self.lat.to_radians().cos(),
            (lat - self.lat) * meters_per_degree,
        )
    }

    fn contains(&self, lat: f64, lon: f64) -> bool {
        let (x, y) = self.local(lat, lon);
        x.hypot(y) <= self.radius_m
    }

    /// Le segment `a`–`b` passe dans la zone, même si ses deux bouts sont dehors.
    fn crossed_by(&self, a: (f64, f64), b: (f64, f64)) -> bool {
        let (ax, ay) = self.local(a.0, a.1);
        let (bx, by) = self.local(b.0, b.1);
        let (dx, dy) = (bx - ax, by - ay);
        let length = dx * dx + dy * dy;
        let t = if length == 0.0 {
            0.0
        } else {
            (-(ax * dx + ay * dy) / length).clamp(0.0, 1.0)
        };
        (ax + t * dx).hypot(ay + t * dy) <= self.radius_m
    }
}

/// Les zones lues au démarrage. `None` : absentes ou illisibles, donc rien de localisé pour le MCP.
#[derive(Clone, Default)]
pub struct Zones(Option<Arc<Vec<Zone>>>);

/// Ce que vaut la valeur de `PRIVACY_ZONES`, pour le seul log du démarrage, qui ne la cite jamais.
pub enum Status {
    Zones(usize),
    Absent,
    Invalid,
}

fn coordinate(text: &str, bound: f64) -> Option<f64> {
    text.trim()
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && v.abs() <= bound)
}

impl Zones {
    /// `lat,lon,rayon_m` en degrés et en mètres, plusieurs séparées par `;`. Une seule zone illisible
    /// rend tout illisible : mieux vaut ne rien montrer qu'oublier une zone.
    pub fn parse(value: Option<&str>) -> (Self, Status) {
        let Some(value) = value.map(str::trim).filter(|v| !v.is_empty()) else {
            return (Self(None), Status::Absent);
        };
        let zones: Option<Vec<Zone>> = value
            .split(';')
            .map(|zone| {
                let parts: Vec<&str> = zone.split(',').collect();
                let [lat, lon, radius] = parts.as_slice() else {
                    return None;
                };
                Some(Zone {
                    lat: coordinate(lat, 90.0)?,
                    lon: coordinate(lon, 180.0)?,
                    radius_m: coordinate(radius, MAX_RADIUS_M).filter(|r| *r > 0.0)?,
                })
            })
            .collect();
        match zones {
            Some(zones) => {
                let count = zones.len();
                (Self(Some(Arc::new(zones))), Status::Zones(count))
            }
            None => (Self(None), Status::Invalid),
        }
    }

    pub fn configured(&self) -> bool {
        self.0.is_some()
    }

    fn hides(&self, lat: f64, lon: f64) -> bool {
        match &self.0 {
            None => true,
            Some(zones) => zones.iter().any(|z| z.contains(lat, lon)),
        }
    }

    fn crossed_by(&self, a: (f64, f64), b: (f64, f64)) -> bool {
        match &self.0 {
            None => true,
            Some(zones) => zones.iter().any(|z| z.crossed_by(a, b)),
        }
    }

    /// Chaque paire `<x>_lat` et `<x>_lon` des détails est une coordonnée : dans une zone, ou sans
    /// zone, les deux passent à `null` et `<x>_hidden` vaut `true`. Le reste des détails ne change pas.
    pub fn hide_coordinates(&self, details: &mut Map<String, Value>) {
        let prefixes: Vec<String> = details
            .keys()
            .filter_map(|k| k.strip_suffix("_lat").map(str::to_owned))
            .filter(|p| details.contains_key(&format!("{p}_lon")))
            .collect();
        for prefix in prefixes {
            let (lat_key, lon_key) = (format!("{prefix}_lat"), format!("{prefix}_lon"));
            let point = (
                details.get(&lat_key).and_then(Value::as_f64),
                details.get(&lon_key).and_then(Value::as_f64),
            );
            let (Some(lat), Some(lon)) = point else {
                continue;
            };
            if self.hides(lat, lon) {
                details.insert(lat_key, Value::Null);
                details.insert(lon_key, Value::Null);
                details.insert(format!("{prefix}_hidden"), Value::Bool(true));
            }
        }
    }

    /// Les morceaux d'une trace hors des zones : un point dedans disparaît, et un segment qui traverse
    /// une zone coupe la trace, pour ne jamais tracer de ligne à travers. Sans zone : rien.
    pub fn segments(&self, points: &[(f64, f64)]) -> Vec<Vec<(f64, f64)>> {
        if !self.configured() {
            return Vec::new();
        }
        let mut segments: Vec<Vec<(f64, f64)>> = Vec::new();
        let mut current: Vec<(f64, f64)> = Vec::new();
        for &point in points {
            if self.hides(point.0, point.1) {
                segments.push(std::mem::take(&mut current));
                continue;
            }
            if let Some(&last) = current.last()
                && self.crossed_by(last, point)
            {
                segments.push(std::mem::take(&mut current));
            }
            current.push(point);
        }
        segments.push(current);
        // Un point isolé ne trace rien et situerait la zone à lui seul.
        segments.retain(|s| s.len() >= 2);
        segments
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn zones(value: &str) -> Zones {
        Zones::parse(Some(value)).0
    }

    #[test]
    fn a_value_is_read_strictly_and_anything_unreadable_hides_everything() {
        assert!(matches!(Zones::parse(None).1, Status::Absent));
        assert!(matches!(Zones::parse(Some("  ")).1, Status::Absent));
        assert!(matches!(
            Zones::parse(Some("45.1, -1.2, 600 ; 44,2,300")).1,
            Status::Zones(2)
        ));
        for invalid in [
            "45.1,-1.2",
            "45.1,-1.2,600,1",
            "91,0,600",
            "45,181,600",
            "45,1,0",
            "45,1,-5",
            "45,1,60000",
            "45,1,nan",
            "45,1,600;",
            "x,1,600",
        ] {
            let (parsed, status) = Zones::parse(Some(invalid));
            assert!(matches!(status, Status::Invalid), "{invalid}");
            assert!(!parsed.configured());
        }
    }

    #[test]
    fn coordinates_inside_a_zone_or_without_zone_are_hidden() {
        let home = zones("45.0,-1.0,500");
        let mut inside = json!({"start_lat": 45.002, "start_lon": -1.001, "end_lat": 45.1,
                                "end_lon": -1.0, "type": "running"});
        home.hide_coordinates(inside.as_object_mut().unwrap());
        assert_eq!(
            inside,
            json!({"start_lat": null, "start_lon": null, "start_hidden": true,
                   "end_lat": 45.1, "end_lon": -1.0, "type": "running"})
        );
        let mut any = json!({"start_lat": 10.0, "start_lon": 10.0});
        Zones::default().hide_coordinates(any.as_object_mut().unwrap());
        assert_eq!(
            any,
            json!({"start_lat": null, "start_lon": null, "start_hidden": true})
        );
    }

    #[test]
    fn a_track_is_cut_where_it_enters_a_zone_and_never_drawn_across() {
        let home = zones("45.0,-1.0,500");
        // Vers l'est en passant par le centre : avant, dedans, après.
        let points: Vec<(f64, f64)> = (-20..=20)
            .map(|i| (45.0, -1.0 + f64::from(i) * 0.001))
            .collect();
        let cut = home.segments(&points);
        assert_eq!(cut.len(), 2);
        for segment in &cut {
            assert!(segment.iter().all(|&(lat, lon)| !home.hides(lat, lon)));
        }
        // Deux points dehors dont le segment traverse la zone : jamais reliés.
        let across = home.segments(&[(45.0, -1.02), (45.0, -1.03), (45.0, -0.98), (45.0, -0.97)]);
        assert_eq!(
            across,
            [
                vec![(45.0, -1.02), (45.0, -1.03)],
                vec![(45.0, -0.98), (45.0, -0.97)]
            ]
        );
        assert!(Zones::default().segments(&points).is_empty());
        // Plusieurs zones : chacune coupe.
        let two = zones("45.0,-1.0,500;45.0,-0.99,300");
        assert!(
            two.segments(&points)
                .iter()
                .flatten()
                .all(|&(lat, lon)| !two.hides(lat, lon))
        );
    }
}
