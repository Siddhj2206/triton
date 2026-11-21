//! Placeholder until the notification-server module lands.
#![allow(dead_code)]
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct Notification {
    pub id: u32,
    pub app_name: String,
    pub icon: Option<String>,
    pub summary: String,
    pub body: String,
    pub actions: Vec<(String, String)>,
    pub critical: bool,
    pub timeout: Duration,
}

#[derive(Debug, Default)]
pub struct Store;

#[derive(Debug, Clone)]
pub enum Event {
    Arrived(Notification),
    Closed(u32),
}
