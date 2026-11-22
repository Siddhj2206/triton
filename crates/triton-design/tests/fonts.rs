use std::collections::{BTreeSet, HashSet};

use triton_design::typography::Weight;
use triton_design::{Family, Icon, IconStyle, TypeStyle, fonts};

fn family_name(bytes: &[u8]) -> String {
    let face = ttf_parser::Face::parse(bytes, 0).expect("font parses");
    face.names()
        .into_iter()
        .filter(|n| n.name_id == ttf_parser::name_id::TYPOGRAPHIC_FAMILY && n.is_unicode())
        .find_map(|n| n.to_string())
        .expect("family name")
}

#[test]
fn every_icon_has_a_glyph_in_both_styles() {
    for (bytes, style) in [
        (fonts::SYMBOLS, IconStyle::Outlined),
        (fonts::SYMBOLS_FILLED, IconStyle::Filled),
    ] {
        let face = ttf_parser::Face::parse(bytes, 0).unwrap();
        for icon in Icon::ALL {
            assert!(
                face.glyph_index(icon.codepoint()).is_some(),
                "{} missing from {style:?}",
                icon.name()
            );
        }
    }
}

#[test]
fn icons_have_distinct_codepoints() {
    let set: HashSet<char> = Icon::ALL.iter().map(|i| i.codepoint()).collect();
    assert_eq!(set.len(), Icon::ALL.len());
}

/// Sums an outline's points, enough to tell two drawings apart.
#[derive(Default)]
struct Fingerprint(f32, usize);

impl ttf_parser::OutlineBuilder for Fingerprint {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0 += x * 3.0 + y;
        self.1 += 1;
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.move_to(x, y);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.move_to(x1, y1);
        self.move_to(x, y);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.move_to(x1, y1);
        self.move_to(x2, y2);
        self.move_to(x, y);
    }
    fn close(&mut self) {}
}

fn fingerprint(face: &ttf_parser::Face<'_>, c: char) -> (f32, usize) {
    let mut f = Fingerprint::default();
    face.outline_glyph(face.glyph_index(c).expect("icon glyph"), &mut f);
    (f.0, f.1)
}

#[test]
fn filled_and_outlined_are_different_drawings() {
    let outlined = ttf_parser::Face::parse(fonts::SYMBOLS, 0).unwrap();
    let filled = ttf_parser::Face::parse(fonts::SYMBOLS_FILLED, 0).unwrap();
    let differs = Icon::ALL
        .iter()
        .filter(|i| fingerprint(&outlined, i.codepoint()) != fingerprint(&filled, i.codepoint()))
        .count();
    // Some icons (arrows, check) look the same filled; most do not.
    assert!(
        differs > Icon::ALL.len() / 2,
        "only {differs} of {} differ",
        Icon::ALL.len()
    );
}

/// The names code asks for must be the names in the files, or iced silently falls back.
#[test]
fn family_names_match_the_files() {
    for f in fonts::JOST {
        assert_eq!(family_name(f), Family::Display.name());
    }
    for f in fonts::GOOGLE_SANS_FLEX {
        assert_eq!(family_name(f), Family::Ui.name());
    }
    for f in fonts::JETBRAINS_MONO {
        assert_eq!(family_name(f), Family::Mono.name());
    }
    assert_eq!(family_name(fonts::SYMBOLS), IconStyle::Outlined.family());
    assert_eq!(
        family_name(fonts::SYMBOLS_FILLED),
        IconStyle::Filled.family()
    );
}

fn weight_class(w: Weight) -> u16 {
    match w {
        Weight::Thin => 100,
        Weight::ExtraLight => 200,
        Weight::Light => 300,
        Weight::Normal => 400,
        Weight::Medium => 500,
        Weight::Semibold => 600,
        Weight::Bold => 700,
        Weight::ExtraBold => 800,
        Weight::Black => 900,
    }
}

fn weights(files: &[&[u8]]) -> BTreeSet<u16> {
    files
        .iter()
        .map(|f| {
            ttf_parser::Face::parse(f, 0)
                .expect("font parses")
                .weight()
                .to_number()
        })
        .collect()
}

/// cosmic-text picks a family's face only on an exact weight match. Every weight
/// the type scale or a component asks for must exist as a face, or text falls back
/// to another family.
#[test]
fn every_requested_weight_has_an_exact_face() {
    let roles = [
        TypeStyle::DISPLAY,
        TypeStyle::HEADLINE_L,
        TypeStyle::HEADLINE_M,
        TypeStyle::TITLE_L,
        TypeStyle::TITLE_M,
        TypeStyle::BODY_L,
        TypeStyle::BODY_M,
        TypeStyle::BODY_S,
        TypeStyle::LABEL_L,
        TypeStyle::LABEL_S,
        TypeStyle::MONO,
    ];
    for role in roles {
        let files: &[&[u8]] = match role.family {
            Family::Display => &fonts::JOST,
            Family::Ui => &fonts::GOOGLE_SANS_FLEX,
            Family::Mono => &fonts::JETBRAINS_MONO,
        };
        assert!(
            weights(files).contains(&weight_class(role.weight)),
            "{role:?}"
        );
    }
    // components.css uses 400, 500 and 600 for UI text and 600 for key caps.
    assert_eq!(
        weights(&fonts::GOOGLE_SANS_FLEX),
        BTreeSet::from([400, 500, 600])
    );
    assert!(weights(&fonts::JETBRAINS_MONO).contains(&600));
    // Icons are drawn at 500.
    let icon = Weight::Medium.numeric();
    assert_eq!(
        weights(&[fonts::SYMBOLS, fonts::SYMBOLS_FILLED]),
        BTreeSet::from([icon])
    );
}
