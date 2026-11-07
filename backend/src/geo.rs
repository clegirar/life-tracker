//! La simplification d'une trace, commune aux traces des séances Garmin (au connecteur) et à la trace
//! du jour de l'iPhone (à la lecture, `location`).

/// La distance en mètres d'un point au segment `a`–`b`, sur un plan local (équirectangulaire) : à
/// l'échelle d'une séance ou d'un trajet, l'écart à la sphère est négligeable.
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

/// La distance en mètres entre deux points `[lat, lon]`, par la formule de haversine.
pub fn distance_m(a: [f64; 2], b: [f64; 2]) -> f64 {
    const EARTH_RADIUS_M: f64 = 6_371_008.8;
    let (lat1, lat2) = (a[0].to_radians(), b[0].to_radians());
    let half_lat = (lat2 - lat1) / 2.0;
    let half_lon = (b[1] - a[1]).to_radians() / 2.0;
    let h = half_lat.sin().powi(2) + lat1.cos() * lat2.cos() * half_lon.sin().powi(2);
    2.0 * EARTH_RADIUS_M * h.sqrt().min(1.0).asin()
}

/// Douglas–Peucker, sans récursion : les points `[lat, lon, t]` qui s'écartent de plus de `tolerance`
/// mètres, le premier et le dernier toujours gardés. Le troisième nombre suit son point sans compter.
pub fn douglas_peucker(points: &[[f64; 3]], tolerance: f64) -> Vec<[f64; 3]> {
    kept(points, &douglas_peucker_mask(points, tolerance))
}

/// Douglas–Peucker, puis au moins un point toutes les `step` secondes (le troisième nombre) : sur une
/// longue ligne droite, l'allure garde ses arrêts et ses accélérations. Un point est gardé dès que
/// `step` secondes ou plus le séparent du précédent gardé ; un trou plus long dans les points reste un
/// trou.
pub fn douglas_peucker_timed(points: &[[f64; 3]], tolerance: f64, step: f64) -> Vec<[f64; 3]> {
    let mut keep = douglas_peucker_mask(points, tolerance);
    let mut last = f64::NEG_INFINITY;
    for (point, kept) in points.iter().zip(keep.iter_mut()) {
        if point[2] - last >= step {
            *kept = true;
        }
        if *kept {
            last = point[2];
        }
    }
    kept(points, &keep)
}

fn kept(points: &[[f64; 3]], keep: &[bool]) -> Vec<[f64; 3]> {
    points
        .iter()
        .zip(keep)
        .filter_map(|(p, kept)| kept.then_some(*p))
        .collect()
}

fn douglas_peucker_mask(points: &[[f64; 3]], tolerance: f64) -> Vec<bool> {
    let mut keep = vec![points.len() < 3; points.len()];
    if points.len() < 3 {
        return keep;
    }
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
    keep
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_straight_line_keeps_its_ends_and_a_corner_is_kept() {
        let line: Vec<[f64; 3]> = (0..10)
            .map(|i| [45.0, 1.0 + f64::from(i) * 1e-4, 0.0])
            .collect();
        assert_eq!(douglas_peucker(&line, 5.0), vec![line[0], line[9]]);
        // Vers l'est, puis vers le nord à partir du sixième point.
        let corner: Vec<[f64; 3]> = (0..10)
            .map(|i| {
                let (east, north) = (f64::from(i.min(5)), f64::from((i - 5).max(0)));
                [45.0 + north * 1e-4, 1.0 + east * 1e-4, 0.0]
            })
            .collect();
        assert_eq!(
            douglas_peucker(&corner, 5.0),
            vec![corner[0], corner[5], corner[9]]
        );
        assert_eq!(douglas_peucker(&line[..2], 5.0), line[..2].to_vec());
    }

    #[test]
    fn a_straight_line_keeps_a_point_every_step() {
        // Une ligne droite d'un point par seconde pendant 100 s : les deux bouts, et un point toutes
        // les 15 s entre eux.
        let line: Vec<[f64; 3]> = (0..=100)
            .map(|i| [45.0, 1.0 + f64::from(i) * 1e-5, f64::from(i)])
            .collect();
        let seconds: Vec<f64> = douglas_peucker_timed(&line, 5.0, 15.0)
            .iter()
            .map(|p| p[2])
            .collect();
        assert_eq!(seconds, [0.0, 15.0, 30.0, 45.0, 60.0, 75.0, 90.0, 100.0]);
        // Un virage gardé par Douglas–Peucker remet le compte à zéro.
        let mut corner = line[..=20].to_vec();
        corner.extend((21..=40).map(|i| [45.0 + f64::from(i - 20) * 1e-4, 1.0002, f64::from(i)]));
        let seconds: Vec<f64> = douglas_peucker_timed(&corner, 5.0, 15.0)
            .iter()
            .map(|p| p[2])
            .collect();
        assert_eq!(seconds, [0.0, 15.0, 20.0, 35.0, 40.0]);
        // Un trou de plus de 15 s dans les points reste un trou ; un tracé court reste entier.
        let sparse = [[45.0, 1.0, 0.0], [45.0, 1.001, 60.0], [45.0, 1.002, 61.0]];
        assert_eq!(
            douglas_peucker_timed(&sparse, 5.0, 15.0),
            vec![sparse[0], sparse[1], sparse[2]]
        );
        assert_eq!(
            douglas_peucker_timed(&sparse[..2], 5.0, 15.0),
            sparse[..2].to_vec()
        );
    }

    #[test]
    fn the_distance_is_haversine() {
        assert_eq!(distance_m([45.0, 1.0], [45.0, 1.0]), 0.0);
        let degree = distance_m([0.0, 0.0], [1.0, 0.0]);
        assert!((degree - 111_195.0).abs() < 1.0, "{degree}");
        let east = distance_m([45.0, 1.0], [45.0, 1.001]);
        assert!((east - 78.6).abs() < 0.5, "{east}");
    }
}
