use alloc::{format, string::{String, ToString}, vec::Vec};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellPath {
    pub abs: bool,
    pub components: Vec<String>,
}

impl ShellPath {
    pub fn new(raw: &str) -> Self {
        let components = raw
            .split('/')
            .filter(|s| !s.is_empty() && *s != ".")
            .map(|s| s.to_string())
            .collect();

        if raw.starts_with('/') {
            Self {
                abs: true,
                components,
            }
        } else {
            Self {
                abs: false,
                components,
            }
        }
    }

    pub fn normalize(&mut self) {
        let mut stack = Vec::new();

        for component in &self.components {
            match component.as_str() {
                "." | "" => {}
                ".." => {
                    if stack.last().is_some_and(|last: &String| last != "..") {
                        stack.pop();
                    } else if !self.abs {
                        stack.push(component.clone());
                    }
                }
                other => stack.push(other.to_string()),
            }
        }
        self.components = stack;
    }

    pub fn join(&self, rel: &ShellPath) -> Self {
        if rel.abs {
            let mut path = rel.clone();
            path.normalize();
            return path;
        }

        let mut new = self.components.clone();
        new.extend(rel.components.clone());

        let mut new_path = Self {
            abs: self.abs,
            components: new,
        };
        new_path.normalize();
        new_path
    }
}

impl ToString for ShellPath {
    fn to_string(&self) -> String {
        match (self.abs, self.components.is_empty()) {
            (true, true) => String::from("/"),
            (true, false) => format!("/{}", self.components.join("/")),
            (false, true) => String::from("."),
            (false, false) => self.components.join("/"),
        }
    }
}

