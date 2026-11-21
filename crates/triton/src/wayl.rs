//! Placeholder until the wayland side-channel module lands.
#![allow(dead_code)]

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleEvent {
    Idle(usize),
    Resumed(usize),
}
