//! The layer-shell side of ADR-0003: a table from surface keys to iced window ids,
//! and the glue that turns planned operations into `iced_layershell` messages. Every
//! layer-shell type stays in this module.

mod plan;

use std::collections::{BTreeMap, HashMap};

use iced_core::window::Id;
use iced_layershell::actions::ActionCallback;
use iced_layershell::reexport::{
    Anchor, KeyboardInteractivity, Layer as LsLayer, NewLayerShellSettings, OutputOption, Task,
};

use triton_design::shape::rounded_rect_bands;

use crate::app::Message;
use crate::shell::{Anchors, Keyboard, Layer, SurfaceKey, SurfaceSpec};
pub use plan::{Change, SurfaceOp, plan};

fn layer(l: Layer) -> LsLayer {
    match l {
        Layer::Background => LsLayer::Background,
        Layer::Bottom => LsLayer::Bottom,
        Layer::Top => LsLayer::Top,
        Layer::Overlay => LsLayer::Overlay,
    }
}

fn anchor(a: Anchors) -> Anchor {
    let mut out = Anchor::empty();
    for (on, edge) in [
        (a.top, Anchor::Top),
        (a.bottom, Anchor::Bottom),
        (a.left, Anchor::Left),
        (a.right, Anchor::Right),
    ] {
        if on {
            out |= edge;
        }
    }
    out
}

fn keyboard(k: Keyboard) -> KeyboardInteractivity {
    match k {
        Keyboard::None => KeyboardInteractivity::None,
        Keyboard::OnDemand => KeyboardInteractivity::OnDemand,
        Keyboard::Exclusive => KeyboardInteractivity::Exclusive,
    }
}

fn settings(key: &SurfaceKey, spec: &SurfaceSpec) -> NewLayerShellSettings {
    NewLayerShellSettings {
        size: Some(spec.size),
        layer: layer(spec.layer),
        anchor: anchor(spec.anchors),
        exclusive_zone: Some(spec.exclusive_zone),
        margin: Some(spec.margin),
        keyboard_interactivity: keyboard(spec.keyboard),
        output_option: OutputOption::OutputName(key.output.clone()),
        events_transparent: false,
        namespace: Some(spec.namespace.to_owned()),
    }
}

/// Asks the compositor to blur behind the surface's drawn shape
/// (ext-background-effect), banded into rectangles to follow its corners.
/// Re-sent whenever the size or radius changes: the region is surface-local.
fn blur_region(id: Id, spec: &SurfaceSpec) -> Task<Message> {
    let Some(radius) = spec.blur_radius else {
        return Task::none();
    };
    let (width, height) = spec.size;
    Task::done(Message::SetBlurRegion {
        id,
        callback: ActionCallback::new(move |region| {
            for (x, y, w, h) in rounded_rect_bands(width, height, radius as f32) {
                region.add(x, y, w, h);
            }
        }),
    })
}

/// The surfaces that exist, by key and by iced window id.
#[derive(Debug, Default)]
pub struct SurfaceTable {
    surfaces: BTreeMap<SurfaceKey, (Id, SurfaceSpec)>,
    ids: HashMap<Id, SurfaceKey>,
}

impl SurfaceTable {
    pub fn key(&self, id: Id) -> Option<&SurfaceKey> {
        self.ids.get(&id)
    }

    /// The compositor closed a surface (output gone, or it refused it).
    pub fn forget(&mut self, id: Id) {
        if let Some(key) = self.ids.remove(&id) {
            self.surfaces.remove(&key);
        }
    }

    fn actual(&self) -> BTreeMap<SurfaceKey, SurfaceSpec> {
        self.surfaces
            .iter()
            .map(|(k, (_, spec))| (k.clone(), spec.clone()))
            .collect()
    }

    /// Brings the surfaces in line with `desired`.
    pub fn reconcile(&mut self, desired: &BTreeMap<SurfaceKey, SurfaceSpec>) -> Task<Message> {
        let ops = plan(desired, &self.actual());
        let tasks: Vec<_> = ops.into_iter().map(|op| self.apply(op)).collect();
        Task::batch(tasks)
    }

    fn apply(&mut self, op: SurfaceOp) -> Task<Message> {
        match op {
            SurfaceOp::Create(key, spec) => {
                tracing::info!(?key, "create surface");
                let id = Id::unique();
                let task = Task::done(Message::NewLayerShell {
                    settings: settings(&key, &spec),
                    id,
                });
                let blur = blur_region(id, &spec);
                self.ids.insert(id, key.clone());
                self.surfaces.insert(key, (id, spec));
                task.chain(blur)
            }
            SurfaceOp::Destroy(key) => {
                tracing::info!(?key, "destroy surface");
                let Some((id, _)) = self.surfaces.remove(&key) else {
                    return Task::none();
                };
                self.ids.remove(&id);
                Task::done(Message::RemoveWindow(id))
            }
            SurfaceOp::Reconfigure(key, changes) => {
                let Some((id, spec)) = self.surfaces.get_mut(&key) else {
                    return Task::none();
                };
                let id = *id;
                let mut resend_blur = false;
                let task = Task::batch(changes.into_iter().map(|change| {
                    let message = match change {
                        Change::BlurRadius(r) => {
                            spec.blur_radius = r;
                            resend_blur = true;
                            return Task::none();
                        }
                        Change::Layer(l) => {
                            spec.layer = l;
                            Message::LayerChange {
                                id,
                                layer: layer(l),
                            }
                        }
                        Change::AnchorSize(a, size) => {
                            spec.anchors = a;
                            spec.size = size;
                            resend_blur = true;
                            Message::AnchorSizeChange {
                                id,
                                anchor: anchor(a),
                                size,
                            }
                        }
                        Change::Margin(m) => {
                            spec.margin = m;
                            Message::MarginChange { id, margin: m }
                        }
                        Change::ExclusiveZone(z) => {
                            spec.exclusive_zone = z;
                            Message::ExclusiveZoneChange { id, zone_size: z }
                        }
                        Change::Keyboard(k) => {
                            spec.keyboard = k;
                            Message::KeyboardInteractivityChange {
                                id,
                                keyboard_interactivity: keyboard(k),
                            }
                        }
                    };
                    Task::done(message)
                }));
                if resend_blur {
                    task.chain(blur_region(id, spec))
                } else {
                    task
                }
            }
        }
    }
}
