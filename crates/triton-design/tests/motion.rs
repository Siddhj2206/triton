use std::time::{Duration, Instant};

use triton_design::{Animated, Motion, Spring};

fn at(start: Instant, ms: u64) -> Instant {
    start + Duration::from_millis(ms)
}

/// Peak overshoot past the target, as a fraction of the distance, sampled each ms.
fn overshoot(spring: Spring) -> f32 {
    let t0 = Instant::now();
    let mut a = Animated::new(0.0, Motion::Spring(spring), 0.001, t0);
    a.set_target(1.0, t0);
    (0..3000)
        .map(|ms| a.value(at(t0, ms)) - 1.0)
        .fold(0.0, f32::max)
}

#[test]
fn effects_springs_never_overshoot() {
    for s in [Spring::EFFECTS_FAST, Spring::EFFECTS, Spring::EFFECTS_SLOW] {
        assert!(overshoot(s) <= 1e-4, "{s:?} overshot by {}", overshoot(s));
    }
}

#[test]
fn spatial_springs_overshoot_only_a_little() {
    for s in [Spring::SPATIAL_FAST, Spring::SPATIAL, Spring::SPATIAL_SLOW] {
        let o = overshoot(s);
        assert!(o < 0.10, "{s:?} overshot by {o}");
    }
    // The fast spatial spring is the bouncy one.
    assert!(overshoot(Spring::SPATIAL_FAST) > 0.01);
}

#[test]
fn every_token_spring_settles_within_a_second() {
    let t0 = Instant::now();
    for s in [
        Spring::SPATIAL_FAST,
        Spring::SPATIAL,
        Spring::SPATIAL_SLOW,
        Spring::EFFECTS_FAST,
        Spring::EFFECTS,
        Spring::EFFECTS_SLOW,
    ] {
        // 100 px move, settled at half a pixel.
        let mut a = Animated::new(0.0, Motion::Spring(s), 0.5, t0);
        a.set_target(100.0, t0);
        assert!(!a.is_settled(t0));
        assert!(a.is_settled(at(t0, 1000)), "{s:?} still moving after 1 s");
    }
}

#[test]
fn retargeting_keeps_position_and_velocity() {
    let t0 = Instant::now();
    let mut a = Animated::new(0.0, Motion::Spring(Spring::SPATIAL), 0.5, t0);
    a.set_target(100.0, t0);
    let mid = at(t0, 80);
    let (x, v) = (a.value(mid), a.velocity(mid));
    a.set_target(-50.0, mid);
    assert!((a.value(mid) - x).abs() < 1e-3, "no jump in position");
    assert!((a.velocity(mid) - v).abs() < 1e-2, "no jump in velocity");
    assert!(a.is_settled(at(mid, 1500)));
    assert!((a.value(at(mid, 1500)) + 50.0).abs() < 0.5);
}

#[test]
fn reduced_motion_is_a_120ms_linear_fade() {
    let t0 = Instant::now();
    let mut a = Animated::new(0.0, Motion::pick(Spring::SPATIAL, true), 0.001, t0);
    a.set_target(1.0, t0);
    assert!((a.value(at(t0, 60)) - 0.5).abs() < 1e-3);
    assert_eq!(a.value(at(t0, 120)), 1.0);
    assert!(a.is_settled(at(t0, 120)));
}

#[test]
fn switching_motion_mid_flight_does_not_jump() {
    let t0 = Instant::now();
    let mut a = Animated::new(0.0, Motion::Spring(Spring::SPATIAL), 0.5, t0);
    a.set_target(100.0, t0);
    let mid = at(t0, 50);
    let x = a.value(mid);
    a.set_motion(Motion::REDUCED, mid);
    assert!((a.value(mid) - x).abs() < 1e-3);
    assert_eq!(a.value(at(mid, 120)), 100.0);
}
