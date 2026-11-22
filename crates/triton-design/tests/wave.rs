use triton_design::shape::Point;
use triton_design::wave::{self, LINEAR_AMPLITUDE, LOBES};

#[test]
fn outline_has_nine_lobes_inside_its_box() {
    let pts = wave::outline(Point::ORIGIN, 100.0);
    assert!(pts.len() > 100);
    assert!(
        pts.iter()
            .all(|p| (0.0..=100.0).contains(&p.x) && (0.0..=100.0).contains(&p.y))
    );

    // Lobes are the local maxima of distance from the centroid.
    let n = pts.len() as f32;
    let c = Point::new(
        pts.iter().map(|p| p.x).sum::<f32>() / n,
        pts.iter().map(|p| p.y).sum::<f32>() / n,
    );
    let r: Vec<f32> = pts.iter().map(|p| p.distance(c)).collect();
    let peaks = (0..r.len())
        .filter(|&i| {
            let prev = r[(i + r.len() - 1) % r.len()];
            let next = r[(i + 1) % r.len()];
            r[i] > prev && r[i] >= next
        })
        .count();
    assert_eq!(peaks as u32, LOBES);
}

#[test]
fn outline_scales_and_moves() {
    let unit = wave::outline(Point::ORIGIN, 100.0);
    let small = wave::outline(Point::new(10.0, 20.0), 50.0);
    assert_eq!(unit.len(), small.len());
    let (a, b) = (unit[3], small[3]);
    assert!((b.x - (10.0 + a.x / 2.0)).abs() < 1e-4);
    assert!((b.y - (20.0 + a.y / 2.0)).abs() < 1e-4);
}

#[test]
fn wavy_line_covers_the_played_part_within_its_amplitude() {
    let pts = wave::wavy_line(Point::new(0.0, 50.0), 200.0, 0.25, 0.0);
    assert!((pts.last().unwrap().x - 50.0).abs() < 1e-3);
    assert!(
        pts.iter()
            .all(|p| (p.y - 50.0).abs() <= LINEAR_AMPLITUDE + 1e-4)
    );
    assert!(
        pts.iter()
            .any(|p| (p.y - 50.0).abs() > LINEAR_AMPLITUDE * 0.9)
    );
}

#[test]
fn wavy_ring_starts_at_twelve_and_stays_near_its_radius() {
    let c = Point::new(17.0, 17.0);
    let pts = wave::wavy_ring(c, 34.0, 1.0, 0.0);
    let first = pts[0];
    assert!((first.x - 17.0).abs() < 1e-3 && first.y < 17.0);
    assert!(
        pts.iter()
            .all(|p| (p.distance(c) - 17.0).abs() <= 1.1 + 1e-3)
    );
}
