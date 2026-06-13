use alloc::{collections::btree_map::BTreeMap, string::String};

use super::ShellPath;

pub struct ShellContext {
    cwd: ShellPath,
    environment: BTreeMap<String, String>,
}

impl ShellContext {
    pub fn new() -> Self {
        Self {
            cwd: ShellPath::new("/"),
            environment: BTreeMap::new(),
        }
    }

    pub fn cwd(&self) -> &ShellPath {
        &self.cwd
    }

    pub fn update_cwd(&mut self, path: ShellPath) {
        self.cwd = self.cwd.join(&path);
    }
}

