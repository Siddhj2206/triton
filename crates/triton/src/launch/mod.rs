//! Placeholder until the launcher engine module lands.
#![allow(dead_code)]

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Apps,
    Windows,
    Commands,
}

#[derive(Debug, Clone)]
pub struct Hit {
    pub title: String,
    pub supporting: Option<String>,
    pub tag: Option<String>,
    pub kind: HitKind,
}

#[derive(Debug, Clone)]
pub enum HitKind {
    App { app: usize, action: Option<usize> },
    Window { id: u64 },
    Command(usize),
    Math { expr: String, result: String },
}

#[derive(Debug, Default)]
pub struct Results {
    pub groups: Vec<(Group, Vec<Hit>)>,
}
