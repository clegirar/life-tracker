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

/// Douglas–Peucker, sans récursion : les points `[lat, lon, t]` qui s'écartent de plus de `tolerance`
/// mètres, le premier et le dernier toujours gardés. Le troisième nombre suit son point sans compter.
pub fn douglas_peucker(points: &[[f64; 3]], tolerance: f64) -> Vec<[f64; 3]> {
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
}
