//! The reconciler: the difference between wanted and existing surfaces as a list of
//! operations. Pure, so lifecycle and hotplug are tested without Wayland.

use std::collections::BTreeMap;

use crate::shell::{Anchors, Keyboard, Layer, SurfaceKey, SurfaceSpec};

/// One property to change on a live surface: the narrowest layer-shell request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Layer(Layer),
    AnchorSize(Anchors, (u32, u32)),
    Margin((i32, i32, i32, i32)),
    ExclusiveZone(i32),
    Keyboard(Keyboard),
    InputHeight(Option<u32>),
    BlurRadius(Option<u32>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SurfaceOp {
    Create(SurfaceKey, SurfaceSpec),
    Reconfigure(SurfaceKey, Vec<Change>),
    Destroy(SurfaceKey),
}

/// The changes that turn `from` into `to`, or `None` if only a new surface can
/// (layer shell cannot change a namespace).
fn changes(from: &SurfaceSpec, to: &SurfaceSpec) -> Option<Vec<Change>> {
    if from.namespace != to.namespace {
        return None;
    }
    let mut out = Vec::new();
    if from.layer != to.layer {
        out.push(Change::Layer(to.layer));
    }
    if from.anchors != to.anchors || from.size != to.size {
        out.push(Change::AnchorSize(to.anchors, to.size));
    }
    if from.margin != to.margin {
        out.push(Change::Margin(to.margin));
    }
    if from.exclusive_zone != to.exclusive_zone {
        out.push(Change::ExclusiveZone(to.exclusive_zone));
    }
    if from.keyboard != to.keyboard {
        out.push(Change::Keyboard(to.keyboard));
    }
    if from.input_height != to.input_height {
        out.push(Change::InputHeight(to.input_height));
    }
    if from.blur_radius != to.blur_radius {
        out.push(Change::BlurRadius(to.blur_radius));
    }
    Some(out)
}

/// Operations from `actual` to `desired`: destroys first (an unplugged output
/// frees its surfaces before anything new is made), then reconfigures, then
/// creates, each in key order.
pub fn plan(
    desired: &BTreeMap<SurfaceKey, SurfaceSpec>,
    actual: &BTreeMap<SurfaceKey, SurfaceSpec>,
) -> Vec<SurfaceOp> {
    let mut destroy = Vec::new();
    let mut reconfigure = Vec::new();
    let mut create = Vec::new();
    for (key, have) in actual {
        match desired.get(key) {
            None => destroy.push(SurfaceOp::Destroy(key.clone())),
            Some(want) => match changes(have, want) {
                Some(c) if c.is_empty() => {}
                Some(c) => reconfigure.push(SurfaceOp::Reconfigure(key.clone(), c)),
                None => {
                    destroy.push(SurfaceOp::Destroy(key.clone()));
                    create.push(SurfaceOp::Create(key.clone(), want.clone()));
                }
            },
        }
    }
    for (key, want) in desired {
        if !actual.contains_key(key) {
            create.push(SurfaceOp::Create(key.clone(), want.clone()));
        }
    }
    destroy
        .into_iter()
        .chain(reconfigure)
        .chain(create)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::{BAR_SIDE, BarMode, Modals, Role, Sizes, desired};

    fn key(role: Role, output: &str) -> SurfaceKey {
        SurfaceKey {
            role,
            output: output.to_owned(),
        }
    }

    fn launcher_on(output: &str) -> Modals<'_> {
        Modals {
            launcher: Some(output),
            ..Modals::default()
        }
    }

    fn want(outputs: &[&str]) -> BTreeMap<SurfaceKey, SurfaceSpec> {
        desired(
            outputs.iter().copied(),
            &Sizes::default(),
            &Modals::default(),
            BarMode::AutoHide,
            |_| true,
        )
    }

    fn creates(ops: &[SurfaceOp]) -> Vec<&SurfaceKey> {
        ops.iter()
            .filter_map(|op| match op {
                SurfaceOp::Create(k, _) => Some(k),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_bar_per_output_from_nothing() {
        let ops = plan(&want(&["DP-1", "eDP-1"]), &BTreeMap::new());
        assert_eq!(ops.len(), 8);
        assert!(creates(&ops).contains(&&key(Role::IslandStart, "DP-1")));
        assert!(creates(&ops).contains(&&key(Role::RevealStart, "eDP-1")));
        assert!(creates(&ops).contains(&&key(Role::RevealEnd, "eDP-1")));
    }

    #[test]
    fn nothing_to_do_when_settled() {
        let d = want(&["eDP-1"]);
        assert_eq!(plan(&d, &d), Vec::new());
    }

    #[test]
    fn unplug_destroys_before_plug_creates() {
        let ops = plan(&want(&["HDMI-A-1"]), &want(&["DP-1"]));
        let first_create = ops
            .iter()
            .position(|op| matches!(op, SurfaceOp::Create(..)))
            .expect("creates");
        assert!(
            ops[..first_create]
                .iter()
                .all(|op| matches!(op, SurfaceOp::Destroy(k) if k.output == "DP-1"))
        );
        assert_eq!(first_create, 4);
        assert_eq!(ops.len(), 8);
    }

    #[test]
    fn a_measured_island_is_resized_not_recreated() {
        let k = key(Role::IslandStart, "eDP-1");
        let widths = BTreeMap::from([(k.clone(), 300)]);
        let ops = plan(
            &desired(
                ["eDP-1"],
                &Sizes {
                    widths: widths.clone(),
                    ..Sizes::default()
                },
                &Modals::default(),
                BarMode::AutoHide,
                |_| true,
            ),
            &want(&["eDP-1"]),
        );
        let spec = &desired(
            ["eDP-1"],
            &Sizes {
                widths: widths.clone(),
                ..Sizes::default()
            },
            &Modals::default(),
            BarMode::AutoHide,
            |_| true,
        )[&k];
        assert_eq!(
            ops,
            vec![
                SurfaceOp::Reconfigure(
                    k,
                    vec![Change::AnchorSize(spec.anchors, (300, spec.size.1))]
                ),
                // The island's reveal edge tracks its width.
                SurfaceOp::Reconfigure(
                    key(Role::RevealStart, "eDP-1"),
                    vec![Change::AnchorSize(
                        Anchors {
                            top: true,
                            bottom: false,
                            left: true,
                            right: false
                        },
                        (300 + BAR_SIDE as u32, 2)
                    )]
                )
            ]
        );
    }

    #[test]
    fn a_changed_spec_uses_the_narrowest_request() {
        let mut w = want(&["eDP-1"]);
        let spec = w.get_mut(&key(Role::RevealStart, "eDP-1")).expect("reveal");
        spec.exclusive_zone = 1;
        spec.layer = Layer::Overlay;
        let ops = plan(&w, &want(&["eDP-1"]));
        assert_eq!(
            ops,
            vec![SurfaceOp::Reconfigure(
                key(Role::RevealStart, "eDP-1"),
                vec![Change::Layer(Layer::Overlay), Change::ExclusiveZone(1)]
            )]
        );
    }

    #[test]
    fn the_launcher_maps_scrim_first() {
        // niri stacks same-layer surfaces in map order: the scrim must be
        // created before the panel so it sits below it.
        let ops = plan(
            &desired(
                ["eDP-1"],
                &Sizes::default(),
                &launcher_on("eDP-1"),
                BarMode::AutoHide,
                |_| true,
            ),
            &want(&["eDP-1"]),
        );
        let scrim = ops
            .iter()
            .position(|op| matches!(op, SurfaceOp::Create(k, _) if k.role == Role::Scrim))
            .expect("scrim");
        let panel = ops
            .iter()
            .position(|op| matches!(op, SurfaceOp::Create(k, _) if k.role == Role::Launcher))
            .expect("launcher");
        assert!(scrim < panel);
        let spec = &desired(
            ["eDP-1"],
            &Sizes::default(),
            &launcher_on("eDP-1"),
            BarMode::AutoHide,
            |_| true,
        )[&key(Role::Launcher, "eDP-1")];
        assert_eq!(spec.keyboard, Keyboard::Exclusive);
        assert_eq!(spec.blur_radius, Some(28));
    }

    #[test]
    fn the_launcher_only_lives_on_its_output() {
        let d = desired(
            ["eDP-1", "DP-1"],
            &Sizes::default(),
            &launcher_on("DP-1"),
            BarMode::AutoHide,
            |_| true,
        );
        assert!(!d.contains_key(&key(Role::Launcher, "eDP-1")));
        assert!(d.contains_key(&key(Role::Launcher, "DP-1")));
        // Its output vanishing takes the modal with it.
        let ops = plan(&want(&["eDP-1"]), &d);
        for role in [Role::Scrim, Role::Launcher] {
            assert!(ops.iter().any(|op| matches!(
                op,
                SurfaceOp::Destroy(k) if k.role == role && k.output == "DP-1"
            )));
        }
    }

    #[test]
    fn autohide_hidden_maps_only_the_reveal_edges() {
        let d = desired(
            ["eDP-1"],
            &Sizes::default(),
            &Modals::default(),
            BarMode::AutoHide,
            |_| false,
        );
        assert_eq!(d.len(), 2);
        for role in [Role::RevealStart, Role::RevealEnd] {
            let spec = &d[&key(role, "eDP-1")];
            assert_eq!(spec.size.1, 2);
            assert_eq!(spec.exclusive_zone, 0);
        }
    }

    #[test]
    fn the_islands_come_and_go_independently() {
        let d = desired(
            ["eDP-1"],
            &Sizes::default(),
            &Modals::default(),
            BarMode::AutoHide,
            |k| k.role == Role::IslandStart,
        );
        assert!(d.contains_key(&key(Role::IslandStart, "eDP-1")));
        assert!(!d.contains_key(&key(Role::IslandEnd, "eDP-1")));
        // Both edges stay mapped either way.
        assert!(d.contains_key(&key(Role::RevealEnd, "eDP-1")));
    }

    #[test]
    fn pinned_maps_one_exclusive_strip() {
        let d = desired(
            ["eDP-1"],
            &Sizes::default(),
            &Modals::default(),
            BarMode::Pinned,
            |_| false,
        );
        assert_eq!(d.len(), 1);
        let spec = &d[&key(Role::Strip, "eDP-1")];
        assert_eq!(spec.exclusive_zone, i32::try_from(spec.size.1).unwrap());
        assert!(spec.anchors.left && spec.anchors.right);
    }

    #[test]
    fn the_notifications_margin_tracks_the_bar() {
        let modals = Modals {
            notifications: Some("eDP-1"),
            ..Modals::default()
        };
        let margin = |shown| {
            desired(
                ["eDP-1"],
                &Sizes::default(),
                &modals,
                BarMode::AutoHide,
                |_| shown,
            )[&key(Role::Notifications, "eDP-1")]
                .margin
                .0
        };
        assert_eq!(margin(true), 76);
        assert_eq!(margin(false), 20);
    }

    #[test]
    fn a_new_namespace_means_a_new_surface() {
        let mut w = want(&["eDP-1"]);
        let k = key(Role::RevealStart, "eDP-1");
        w.get_mut(&k).expect("reveal").namespace = "other";
        let ops = plan(&w, &want(&["eDP-1"]));
        assert_eq!(
            ops,
            vec![
                SurfaceOp::Destroy(k.clone()),
                SurfaceOp::Create(k, w[&key(Role::RevealStart, "eDP-1")].clone())
            ]
        );
    }
}
