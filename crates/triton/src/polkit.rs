//! Placeholder until the polkit agent module lands.
#![allow(dead_code)]

#[derive(Debug, Clone)]
pub struct Prompt {
    pub cookie: String,
    pub action_id: String,
    pub message: String,
    pub icon: Option<String>,
    pub user: String,
    pub identity: String,
}

#[derive(Debug, Clone)]
pub enum Event {
    Prompt(Prompt),
    Cancelled { cookie: String },
}
