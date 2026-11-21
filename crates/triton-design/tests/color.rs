use triton_design::color::{contrast, over};
use triton_design::{Scheme, Variant};

/// Every "on" role must read on its container at WCAG AA (4.5:1). Material 3
/// guarantees this for generated schemes; the canvas's light scheme is hand-picked,
/// so this is what catches a bad pick.
#[test]
fn on_roles_meet_aa_on_their_containers() {
    for variant in [Variant::Dark, Variant::Light] {
        let s = Scheme::reference(variant);
        let pairs = [
            ("on_primary/primary", s.on_primary, s.primary),
            (
                "on_primary_container",
                s.on_primary_container,
                s.primary_container,
            ),
            ("on_secondary/secondary", s.on_secondary, s.secondary),
            (
                "on_secondary_container",
                s.on_secondary_container,
                s.secondary_container,
            ),
            ("on_tertiary/tertiary", s.on_tertiary, s.tertiary),
            (
                "on_tertiary_container",
                s.on_tertiary_container,
                s.tertiary_container,
            ),
            ("on_error/error", s.on_error, s.error),
            (
                "on_error_container",
                s.on_error_container,
                s.error_container,
            ),
            ("on_surface/surface", s.on_surface, s.surface),
            (
                "on_surface/highest",
                s.on_surface,
                s.surface_container_highest,
            ),
            (
                "on_surface_variant/surface",
                s.on_surface_variant,
                s.surface,
            ),
        ];
        for (name, fg, bg) in pairs {
            let ratio = contrast(fg, bg);
            assert!(ratio >= 4.5, "{variant:?} {name}: {ratio:.2}:1");
        }
    }
}

#[test]
fn surface_containers_step_away_from_the_surface() {
    // Dark steps up in lightness, light steps down: each container is one tone
    // further from the surface, which is what "rows sit one tone above" relies on.
    let d = Scheme::DARK;
    let dark = [
        d.surface_container_lowest,
        d.surface_container_low,
        d.surface_container,
        d.surface_container_high,
        d.surface_container_highest,
    ];
    assert!(
        dark.windows(2)
            .all(|w| contrast(w[0], d.on_surface) > contrast(w[1], d.on_surface))
    );
    let l = Scheme::LIGHT;
    let light = [
        l.surface_container_lowest,
        l.surface_container_low,
        l.surface_container,
        l.surface_container_high,
        l.surface_container_highest,
    ];
    assert!(
        light
            .windows(2)
            .all(|w| contrast(w[0], l.on_surface) > contrast(w[1], l.on_surface))
    );
}

#[test]
fn state_layer_composites_toward_the_overlay() {
    let s = Scheme::DARK;
    let hovered = over(s.surface, s.on_surface, 0.08);
    assert!(hovered.r > s.surface.r && hovered.r < s.on_surface.r);
    assert_eq!(over(s.surface, s.on_surface, 0.0), s.surface);
}
