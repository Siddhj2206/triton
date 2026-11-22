//! Typed timer tags for `TritonShell`'s `AppCommand::ScheduleTimer` / `on_timer`.
//!
//! Single source of truth for encoding and decoding to `triton-engine`'s `u32`
//! timer tag space so fixed and per-output tags can never collide.

use crate::state::OutputKey;

/// Base value for per-output Bar Autohide reveal/hide timers.
const BAR_OUTPUT_BASE: u32 = 0x1000;

/// Typed timer identifier owned by `TritonShell`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TimerTag {
    /// 1.2 s OSD hold deadline (re-armed on every OSD event).
    OsdHide,
    /// Minute-aligned live Bar clock tick.
    BarClock,
    /// Exit-morph settle poll before unmounting the OSD surface.
    OsdReap,
    /// Release outgoing wallpaper textures once a crossfade settles.
    WallpaperSettle,
    /// Autohide Bar hover-reveal timer for `output`.
    BarReveal(OutputKey),
    /// Autohide Bar pointer-leave hide timer for `output`.
    BarHide(OutputKey),
}

impl TimerTag {
    /// Encode this tag as the `u32` accepted by `AppCommand::ScheduleTimer`.
    pub const fn to_u32(self) -> u32 {
        match self {
            Self::OsdHide => 1,
            Self::BarClock => 2,
            Self::OsdReap => 3,
            Self::WallpaperSettle => 4,
            Self::BarReveal(output) => BAR_OUTPUT_BASE + output.0 * 2,
            Self::BarHide(output) => BAR_OUTPUT_BASE + output.0 * 2 + 1,
        }
    }

    /// Decode a raw `u32` timer tag back into its typed [`TimerTag`].
    pub const fn from_u32(tag: u32) -> Option<Self> {
        match tag {
            1 => Some(Self::OsdHide),
            2 => Some(Self::BarClock),
            3 => Some(Self::OsdReap),
            4 => Some(Self::WallpaperSettle),
            t if t >= BAR_OUTPUT_BASE => {
                let offset = t - BAR_OUTPUT_BASE;
                let key = OutputKey(offset / 2);
                if key.0 == 0 {
                    return None;
                }
                if offset.is_multiple_of(2) {
                    Some(Self::BarReveal(key))
                } else {
                    Some(Self::BarHide(key))
                }
            }
            _ => None,
        }
    }
}

impl From<TimerTag> for u32 {
    fn from(tag: TimerTag) -> Self {
        tag.to_u32()
    }
}
