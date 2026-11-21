use triton_design::material::{veil, veil_hi};
use triton_design::shape::{connected_corners, list_corners};
use triton_design::{Material, RADII, Scheme, Variant};

#[test]
fn mica_sees_the_wallpaper_and_glass_sees_windows() {
    for v in [Variant::Dark, Variant::Light] {
        assert_eq!(Material::Mica.spec(v).unwrap().xray, Some(true));
        assert_eq!(Material::Glass.spec(v).unwrap().xray, Some(false));
        assert_eq!(Material::Thick.spec(v).unwrap().xray, Some(false));
        assert!(Material::Solid.spec(v).is_none());
    }
}

#[test]
fn fills_are_tints_of_the_scheme() {
    let s = Scheme::DARK;
    assert_eq!(
        Material::Solid.fill(&s, Variant::Dark),
        s.surface_container_low
    );
    let glass = Material::Glass.fill(&s, Variant::Dark);
    assert!(
        (glass.a - 0.70).abs() < 1e-6,
        "Glass .70 is the contrast-measured value"
    );
    assert_eq!((glass.r, glass.g, glass.b), {
        let c = s.surface_container_high;
        (c.r, c.g, c.b)
    });
    // Thick is more opaque than Glass, in both schemes.
    for v in [Variant::Dark, Variant::Light] {
        let s = Scheme::reference(v);
        assert!(Material::Thick.fill(&s, v).a > Material::Glass.fill(&s, v).a);
    }
}

#[test]
fn adaptive_mica_adds_tint_only_on_bright_backdrops() {
    let s = Scheme::DARK;
    let base = Material::Mica.fill(&s, Variant::Dark);
    let bright = Material::Mica.fill_adaptive(&s, Variant::Dark, true);
    assert!((bright.a - base.a - 0.08).abs() < 1e-6);
    assert_eq!(Material::Mica.fill_adaptive(&s, Variant::Dark, false), base);
    assert_eq!(
        Material::Glass.fill_adaptive(&s, Variant::Dark, true),
        Material::Glass.fill(&s, Variant::Dark)
    );
}

#[test]
fn only_floating_materials_cast_shadows() {
    assert!(Material::Glass.floats() && Material::Glass.shadow().is_some());
    assert!(!Material::Mica.floats() && Material::Mica.shadow().is_none());
}

#[test]
fn veil_is_stronger_when_hovered() {
    for v in [Variant::Dark, Variant::Light] {
        assert!(veil_hi(v).a > veil(v).a);
    }
}

#[test]
fn list_rows_round_only_the_group_ends() {
    let (o, i) = (RADII.list_outer, RADII.list_inner);
    assert_eq!(list_corners(0, 1), [o, o, o, o]);
    assert_eq!(list_corners(0, 3), [o, o, i, i]);
    assert_eq!(list_corners(1, 3), [i, i, i, i]);
    assert_eq!(list_corners(2, 3), [i, i, o, o]);
}

#[test]
fn connected_groups_round_only_the_outer_ends() {
    let (e, i) = (RADII.connected_end, RADII.connected_inner);
    assert_eq!(connected_corners(0, 3, e), [e, i, i, e]);
    assert_eq!(connected_corners(1, 3, e), [i, i, i, i]);
    assert_eq!(connected_corners(2, 3, e), [i, e, e, i]);
}

#[test]
fn concentric_corners_share_a_center() {
    use triton_design::shape::{concentric, nested};
    // The large segmented toolbar: 24 px ends, 8 px inset.
    assert_eq!(concentric(24.0, 8.0), 32.0);
    assert_eq!(nested(concentric(24.0, 8.0), 8.0), 24.0);
    // An island already obeys it: 40 px controls, 8 px inset, 56 px pill.
    assert_eq!(concentric(20.0, 8.0), triton_design::SIZE.bar_height / 2.0);
    assert_eq!(nested(6.0, 8.0), 0.0);
}
