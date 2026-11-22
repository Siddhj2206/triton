//! `~/.config/triton/settings.toml` — the small persisted slice of shell
//! state the boards put behind toggles: bar mode, DND, the color variant and
//! night light. Triton owns this file: every mutation writes it back through
//! [`crate::util::write_if_changed`], so a restart restores the user's last
//! choices instead of the defaults (the Noctalia "GUI overrides" layer).

use std::io;

use serde::{Deserialize, Serialize};

use crate::shell::BarMode;

/// What survives a restart. Strings for enums: the file stays stable across
/// renames and a hand edit can't corrupt an unknown variant — unknown values
/// fall back to defaults on load.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// `autohide` | `pinned` | `hidden`.
    pub bar_mode: String,
    /// Do-not-disturb across restarts.
    pub dnd: bool,
    /// `dark` | `light`.
    pub variant: String,
    /// Night light on (4500 K) at boot.
    pub night: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            bar_mode: "autohide".to_owned(),
            dnd: false,
            variant: "dark".to_owned(),
            night: false,
        }
    }
}

impl Settings {
    /// `settings.toml` or defaults — a corrupt or missing file never blocks
    /// boot; it gets rewritten on the next mutation anyway.
    pub fn load() -> Self {
        crate::util::config_root()
            .and_then(|root| {
                std::fs::read_to_string(root.join("triton").join("settings.toml")).ok()
            })
            .and_then(|text| toml::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// Persist. Errors are logged by the caller's context, not fatal — a
    /// settings write failing should never break a toggle.
    pub fn save(&self) -> io::Result<bool> {
        let Some(root) = crate::util::config_root() else {
            return Ok(false);
        };
        let text = toml::to_string(self).unwrap_or_default();
        crate::util::write_if_changed(&root.join("triton"), "settings.toml", &text)
    }

    pub fn bar_mode(&self) -> BarMode {
        match self.bar_mode.as_str() {
            "pinned" => BarMode::Pinned,
            "hidden" => BarMode::Hidden,
            _ => BarMode::AutoHide,
        }
    }

    pub fn variant(&self) -> triton_design::Variant {
        match self.variant.as_str() {
            "light" => triton_design::Variant::Light,
            _ => triton_design::Variant::Dark,
        }
    }

    /// Snapshot the live shell fields into a `Settings` to save.
    pub fn capture(
        bar_mode: BarMode,
        dnd: bool,
        variant: triton_design::Variant,
        night: bool,
    ) -> Self {
        Self {
            bar_mode: match bar_mode {
                BarMode::AutoHide => "autohide",
                BarMode::Pinned => "pinned",
                BarMode::Hidden => "hidden",
            }
            .to_owned(),
            dnd,
            variant: match variant {
                triton_design::Variant::Dark => "dark",
                triton_design::Variant::Light => "light",
            }
            .to_owned(),
            night,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_through_toml() {
        let settings =
            Settings::capture(BarMode::Pinned, true, triton_design::Variant::Light, true);
        let text = toml::to_string(&settings).unwrap();
        let loaded: Settings = toml::from_str(&text).unwrap();
        assert_eq!(loaded.bar_mode(), BarMode::Pinned);
        assert!(loaded.dnd);
        assert!(matches!(loaded.variant(), triton_design::Variant::Light));
        assert!(loaded.night);
    }

    #[test]
    fn corrupt_or_missing_falls_back() {
        let loaded: Settings = toml::from_str("bar_mode = \x00 nonsense").unwrap_or_default();
        assert_eq!(loaded.bar_mode(), BarMode::AutoHide);
    }
}
