#![forbid(unsafe_code)]

//! Central deadline-based timer scheduler for shell behaviors.
//!
//! Supports scheduling, refreshing, cancelling, and hover-pausing/resuming timers
//! keyed by any `Copy + Eq + Hash` identifier (for example bar autohide dwell,
//! toast TTLs, refreshable OSD timeouts, or clipboard auto-wipe). The event loop
//! can query [`TimerScheduler::next_deadline`] to arm a single OS timer and drain
//! due keys via [`TimerScheduler::poll_expired`].

use std::collections::HashMap;
use std::hash::Hash;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TimerEntry {
    Running { deadline: Instant },
    Paused { remaining: Duration },
}

/// Manages named one-shot timers with pause/resume support and deterministic
/// deadline polling.
#[derive(Debug, Clone)]
pub struct TimerScheduler<K> {
    entries: HashMap<K, TimerEntry>,
}

impl<K> Default for TimerScheduler<K> {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }
}

impl<K: Copy + Eq + Hash> TimerScheduler<K> {
    /// Create an empty timer scheduler.
    pub fn new() -> Self {
        Self::default()
    }

    /// Schedule or replace/refresh a timer for `key` to fire at `now + delay`.
    pub fn schedule(&mut self, key: K, delay: Duration, now: Instant) {
        self.entries.insert(
            key,
            TimerEntry::Running {
                deadline: now + delay,
            },
        );
    }

    /// Cancel the timer for `key` if present. Returns `true` if a timer was removed.
    pub fn cancel(&mut self, key: K) -> bool {
        self.entries.remove(&key).is_some()
    }

    /// Pause a running timer, freezing its remaining duration.
    ///
    /// Returns `true` if the timer was running and is now paused, or `false` if
    /// it was already paused or absent.
    pub fn pause(&mut self, key: K, now: Instant) -> bool {
        if let Some(entry) = self.entries.get_mut(&key)
            && let TimerEntry::Running { deadline } = *entry
        {
            *entry = TimerEntry::Paused {
                remaining: deadline.saturating_duration_since(now),
            };
            return true;
        }
        false
    }

    /// Resume a paused timer so it fires at `now + remaining`.
    ///
    /// Returns `true` if the timer was paused and is now running, or `false` if
    /// it was not paused or absent.
    pub fn resume(&mut self, key: K, now: Instant) -> bool {
        if let Some(entry) = self.entries.get_mut(&key)
            && let TimerEntry::Paused { remaining } = *entry
        {
            *entry = TimerEntry::Running {
                deadline: now + remaining,
            };
            return true;
        }
        false
    }

    /// Returns `true` if a timer for `key` is scheduled (running or paused).
    pub fn is_active(&self, key: K) -> bool {
        self.entries.contains_key(&key)
    }

    /// Returns `true` if a timer for `key` is scheduled and currently paused.
    pub fn is_paused(&self, key: K) -> bool {
        matches!(self.entries.get(&key), Some(TimerEntry::Paused { .. }))
    }

    /// Returns the remaining duration for `key` (whether running or paused), or
    /// `None` if no timer is scheduled for `key`.
    pub fn remaining(&self, key: K, now: Instant) -> Option<Duration> {
        self.entries.get(&key).map(|entry| match *entry {
            TimerEntry::Running { deadline } => deadline.saturating_duration_since(now),
            TimerEntry::Paused { remaining } => remaining,
        })
    }

    /// Returns the earliest deadline among all running (non-paused) timers,
    /// allowing the event loop to arm a single OS timer.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.entries
            .values()
            .filter_map(|entry| match *entry {
                TimerEntry::Running { deadline } => Some(deadline),
                TimerEntry::Paused { .. } => None,
            })
            .min()
    }

    /// Remove and return all running timers whose deadline is at or before `now`,
    /// ordered from earliest deadline to latest.
    pub fn poll_expired(&mut self, now: Instant) -> Vec<K> {
        let mut expired: Vec<(K, Instant)> = self
            .entries
            .iter()
            .filter_map(|(&key, entry)| match *entry {
                TimerEntry::Running { deadline } if deadline <= now => Some((key, deadline)),
                _ => None,
            })
            .collect();

        expired.sort_by_key(|&(_, deadline)| deadline);

        for &(key, _) in &expired {
            self.entries.remove(&key);
        }

        expired.into_iter().map(|(key, _)| key).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    enum ShellTimer {
        BarDwell,
        Toast(u32),
        Osd,
        ClipboardWipe,
    }

    #[test]
    fn schedule_refresh_and_cancel() {
        let mut sched = TimerScheduler::new();
        let t0 = Instant::now();

        assert!(!sched.is_active(ShellTimer::Osd));
        assert_eq!(sched.remaining(ShellTimer::Osd, t0), None);
        assert!(!sched.cancel(ShellTimer::Osd));

        // Schedule OSD for 1200ms.
        sched.schedule(ShellTimer::Osd, Duration::from_millis(1200), t0);
        assert!(sched.is_active(ShellTimer::Osd));
        assert!(!sched.is_paused(ShellTimer::Osd));
        assert_eq!(
            sched.remaining(ShellTimer::Osd, t0 + Duration::from_millis(500)),
            Some(Duration::from_millis(700))
        );

        // Refresh OSD at t0 + 800ms for another 1200ms -> fires at t0 + 2000ms.
        let t1 = t0 + Duration::from_millis(800);
        sched.schedule(ShellTimer::Osd, Duration::from_millis(1200), t1);
        assert_eq!(
            sched.next_deadline(),
            Some(t0 + Duration::from_millis(2000))
        );
        assert_eq!(
            sched.poll_expired(t0 + Duration::from_millis(1500)),
            Vec::<ShellTimer>::new()
        );

        // Cancel removes it.
        assert!(sched.cancel(ShellTimer::Osd));
        assert!(!sched.is_active(ShellTimer::Osd));
        assert!(!sched.cancel(ShellTimer::Osd));
        assert_eq!(sched.next_deadline(), None);
    }

    #[test]
    fn hover_pause_and_resume_preserves_exact_remaining_duration() {
        let mut sched = TimerScheduler::new();
        let t0 = Instant::now();
        let toast = ShellTimer::Toast(1);

        // Toast 6s TTL.
        sched.schedule(toast, Duration::from_secs(6), t0);

        // Pause after 2s -> 4s remaining.
        let t_pause = t0 + Duration::from_secs(2);
        assert!(sched.pause(toast, t_pause));
        assert!(sched.is_active(toast));
        assert!(sched.is_paused(toast));
        assert_eq!(
            sched.remaining(toast, t_pause),
            Some(Duration::from_secs(4))
        );

        // Pausing again is a no-op.
        assert!(!sched.pause(toast, t_pause + Duration::from_secs(1)));

        // 10s pass while hovered: timer does not expire and remaining stays 4s.
        let t_resume = t_pause + Duration::from_secs(10);
        assert_eq!(sched.poll_expired(t_resume), Vec::<ShellTimer>::new());
        assert_eq!(
            sched.remaining(toast, t_resume),
            Some(Duration::from_secs(4))
        );

        // Resume at t_resume -> should fire 4s later.
        assert!(sched.resume(toast, t_resume));
        assert!(!sched.is_paused(toast));
        assert!(!sched.resume(toast, t_resume));

        assert_eq!(
            sched.poll_expired(t_resume + Duration::from_millis(3999)),
            Vec::<ShellTimer>::new()
        );
        assert_eq!(
            sched.poll_expired(t_resume + Duration::from_secs(4)),
            vec![toast]
        );
        assert!(!sched.is_active(toast));
    }

    #[test]
    fn pause_and_resume_absent_are_noops() {
        let mut sched = TimerScheduler::<ShellTimer>::new();
        let t0 = Instant::now();
        assert!(!sched.pause(ShellTimer::BarDwell, t0));
        assert!(!sched.resume(ShellTimer::BarDwell, t0));
    }

    #[test]
    fn next_deadline_ignores_paused_and_returns_earliest_running() {
        let mut sched = TimerScheduler::new();
        let t0 = Instant::now();

        assert_eq!(sched.next_deadline(), None);

        sched.schedule(ShellTimer::BarDwell, Duration::from_millis(120), t0);
        sched.schedule(ShellTimer::Osd, Duration::from_millis(1200), t0);
        sched.schedule(ShellTimer::ClipboardWipe, Duration::from_secs(30), t0);

        assert_eq!(sched.next_deadline(), Some(t0 + Duration::from_millis(120)));

        // Pausing the earliest timer makes Osd the next deadline.
        assert!(sched.pause(ShellTimer::BarDwell, t0 + Duration::from_millis(20)));
        assert_eq!(
            sched.next_deadline(),
            Some(t0 + Duration::from_millis(1200))
        );

        // Pausing all timers yields None.
        assert!(sched.pause(ShellTimer::Osd, t0 + Duration::from_millis(20)));
        assert!(sched.pause(ShellTimer::ClipboardWipe, t0 + Duration::from_millis(20)));
        assert_eq!(sched.next_deadline(), None);
    }

    #[test]
    fn poll_expired_orders_by_deadline_and_keeps_future_and_paused() {
        let mut sched = TimerScheduler::new();
        let t0 = Instant::now();

        // Schedule out of chronological order.
        sched.schedule(ShellTimer::ClipboardWipe, Duration::from_secs(30), t0);
        sched.schedule(ShellTimer::Osd, Duration::from_millis(1200), t0);
        sched.schedule(ShellTimer::BarDwell, Duration::from_millis(300), t0);
        sched.schedule(ShellTimer::Toast(1), Duration::from_millis(600), t0);
        sched.schedule(ShellTimer::Toast(2), Duration::from_millis(100), t0);

        // Pause Toast(2) before it expires.
        assert!(sched.pause(ShellTimer::Toast(2), t0 + Duration::from_millis(50)));

        // Poll at t0 + 1200ms: BarDwell (300ms), Toast(1) (600ms), Osd (1200ms) expire in order.
        let expired = sched.poll_expired(t0 + Duration::from_millis(1200));
        assert_eq!(
            expired,
            vec![ShellTimer::BarDwell, ShellTimer::Toast(1), ShellTimer::Osd]
        );

        // Expired timers were removed.
        assert!(!sched.is_active(ShellTimer::BarDwell));
        assert!(!sched.is_active(ShellTimer::Toast(1)));
        assert!(!sched.is_active(ShellTimer::Osd));

        // Future and paused timers remain intact.
        assert!(sched.is_active(ShellTimer::ClipboardWipe));
        assert!(sched.is_active(ShellTimer::Toast(2)));
        assert!(sched.is_paused(ShellTimer::Toast(2)));
        assert_eq!(sched.next_deadline(), Some(t0 + Duration::from_secs(30)));
    }
}
