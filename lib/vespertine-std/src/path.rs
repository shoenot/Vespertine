extern crate alloc;
use core::fmt::Display;

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use vespertine_common::path::{Component, Components};

pub const PATH_MAX: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathError {
    Empty,
    NoFileName,
    ContainsNull,
    NameTooLong,
}

impl Display for PathError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            PathError::Empty => f.write_str("path is empty"),
            PathError::NoFileName => f.write_str("path has no valid file name"),
            PathError::ContainsNull => f.write_str("path contains null byte"),
            PathError::NameTooLong => f.write_str("path is too long"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Path<'a> {
    raw: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathBuf {
    inner: String,
}

impl<'a> Path<'a> {
    pub fn new(raw: &'a str) -> Self {
        Self { raw }
    }

    pub fn as_str(&self) -> &'a str {
        self.raw
    }

    pub fn components(&self) -> Components<'a> {
        Components::new(self.raw)
    }

    pub fn is_absolute(&self) -> bool {
        matches!(self.components().next(), Some(Component::Root))
    }

    pub fn parent(&self) -> Option<PathBuf> {
        let norm = self.normalize_lexical();
        norm.parent()
    }

    pub fn file_name(&self) -> Option<&'a str> {
        let mut last = None;
        for component in self.components() {
            match component {
                Component::Root | Component::Current => {},
                Component::Parent => { last = None; },
                Component::Normal(name) => { last = Some(name); },
            }
        }
        last
    }

    pub fn to_path_buf(&self) -> PathBuf {
        PathBuf { inner: self.raw.to_string() }
    }

    pub fn normalize_lexical(&self) -> PathBuf {
        PathBuf::normalize_lexical(self)
    }

    pub fn join(&self, child: &Path<'_>) -> PathBuf {
        self.to_path_buf().join(child)
    }

    pub fn validate(&self) -> Result<(), PathError> {
        if self.raw.is_empty() {
            return Err(PathError::Empty);
        }
        if self.raw.as_bytes().contains(&0) {
            return Err(PathError::ContainsNull);
        }
        if self.raw.len() > PATH_MAX {
            return Err(PathError::NameTooLong);
        }
        Ok(())
    }

    pub fn starts_with(&self, base: &Path<'_>) -> bool {
        let (self_abs, self_parts) = normalized_parts(self);
        let (base_abs, base_parts) = normalized_parts(base);

        if self_abs != base_abs { return false; }

        if base_parts.len() > self_parts.len() { return false; }

        self_parts.iter().zip(base_parts.iter()).all(|(a, b)| a == b)
    }

    pub fn strip_prefix(&self, base: &Path<'_>) -> Option<PathBuf> {
        let (self_abs, self_parts) = normalized_parts(self);
        let (base_abs, base_parts) = normalized_parts(base);

        if self_abs != base_abs {
            return None;
        }

        if base_parts.len() > self_parts.len() {
            return None;
        }

        if !self_parts
            .iter()
            .zip(base_parts.iter())
            .all(|(a, b)| a == b)
        {
            return None;
        }

        let rest = &self_parts[base_parts.len()..];

        if rest.is_empty() {
            return Some(PathBuf::from_str("."));
        }

        Some(PathBuf::from_str(&rest.join("/")))
    }
}

impl<'a> AsRef<Path<'a>> for Path<'a> {
    fn as_ref(&self) -> &Path<'a> {
        self
    }
}

impl Display for Path<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl PathBuf {
    pub fn new() -> Self {
        Self::current()
    }

    pub fn empty() -> Self {
        Self { inner: String::new() }
    }

    pub fn root() -> Self {
        Self { inner: "/".to_string() }
    }

    pub fn current() -> Self {
        Self { inner: ".".to_string() }
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn is_absolute(&self) -> bool {
        self.as_path().is_absolute()
    }

    pub fn file_name(&self) -> Option<&str> {
        self.as_path().file_name()
    }

    pub fn clear(&mut self) {
        self.inner.clear()
    }

    pub fn into_string(self) -> String {
        self.inner
    }

    pub fn as_str(&self) -> &str {
        &self.inner
    }

    pub fn from_str(s: &str) -> Self {
        Self { inner: s.to_string() }
    }

    pub fn as_path(&self) -> Path<'_> {
        Path::new(&self.inner)
    }

    pub fn from_path(path: &Path<'_>) -> Self {
        path.to_path_buf()
    }

    pub fn starts_with(&self, base: &Path<'_>) -> bool {
        self.as_path().starts_with(base)
    }

    pub fn strip_prefix(&self, base: &Path<'_>) -> Option<PathBuf> {
        self.as_path().strip_prefix(base)
    }

    pub fn join(&self, child: &Path<'_>) -> PathBuf {
        if child.is_absolute() {
            return PathBuf::normalize_lexical(child);
        }

        let mut combined = self.inner.clone();

        if !combined.ends_with('/') {
            combined.push('/');
        }

        combined.push_str(child.as_str());

        PathBuf::normalize_lexical(&Path::new(&combined))
    }

    pub fn normalize_lexical(path: &Path<'_>) -> Self {
        let mut absolute = false;
        let mut stack = Vec::new();

        for component in path.components() {
            match component {
                Component::Root => {
                    absolute = true;
                    stack.clear();
                },
                Component::Current => {},
                Component::Parent => {
                    if !stack.is_empty() {
                        stack.pop();
                    } else if !absolute {
                        stack.push("..".to_string());
                    }
                },
                Component::Normal(name) => {
                    stack.push(name.to_string());
                },
            }
        }

        let mut inner = String::new();

        if absolute { inner.push('/'); }

        inner.push_str(&stack.join("/"));

        if inner.is_empty() { 
            if absolute {
                inner.push('/'); 
            } else {
                inner.push('.');
            }
        }

        Self { inner }
    }

    pub fn push(&mut self, child: &Path<'_>) {
        if child.is_absolute() {
            *self = child.normalize_lexical();
            return;
        }

        if self.inner.is_empty() || self.inner == "." {
            self.inner = child.as_str().to_string();
        } else {
            if !self.inner.ends_with('/') {
                self.inner.push('/');
            }

            self.inner.push_str(child.as_str());
        }

        *self = Self::normalize_lexical(&self.as_path());
    }

    pub fn pop(&mut self) -> bool {
        let Some(parent) = self.parent() else {
            return false;
        };

        if parent.inner == self.inner {
            return false;
        }

        self.inner = parent.inner;
        true
    }

    pub fn parent(&self) -> Option<PathBuf> {
        let path = self.as_path();
        let absolute = path.is_absolute();

        let mut components: Vec<String> = Vec::new();

        for component in path.components() {
            match component {
                Component::Root | Component::Current => {},
                Component::Parent => {
                    if !components.is_empty() {
                        components.pop();
                    } else if !absolute {
                        components.push("..".to_string());
                    }
                },
                Component::Normal(name) => {
                    components.push(name.to_string());
                },
            }
        }

        if components.is_empty() {
            if absolute {
                // "/" has no parent 
                return None;
            } else {
                // "foo" -> "."
                return Some(PathBuf::from_str("."));
            }
        }

        components.pop();

        if components.is_empty() {
            if absolute {
                Some(PathBuf::from_str("/"))
            } else {
                Some(PathBuf::from_str("."))
            }
        } else {
            let mut inner = String::new();
            if absolute { inner.push('/'); }
            inner .push_str(&components.join("/"));
            Some(PathBuf { inner })
        }
    }

    pub fn set_file_name(&mut self, name: &str) -> Result<(), PathError> {
        validate_file_name(name)?;
        self.pop();

        if self.inner == "." { self.inner.clear(); }

        if !self.inner.is_empty() && !self.inner.ends_with('/') {
            self.inner.push('/');
        }

        self.inner.push_str(name);
        Ok(())
    }
}

impl Default for PathBuf {
    fn default() -> Self {
        Self::new()
    }
}

impl From<&str> for PathBuf {
    fn from(value: &str) -> Self {
        Self::from_str(value)
    }
}

impl From<String> for PathBuf {
    fn from(value: String) -> Self {
        Self { inner: value }
    }
}

impl Display for PathBuf {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.inner)
    }
}

pub fn split_parent_name<'a>(path: &'a Path<'a>) -> Result<(PathBuf, &'a str), PathError> {
    let mut absolute = false;
    let mut components: Vec<&'a str> = Vec::new();

    for component in path.components() {
        match component {
            Component::Root => {
                absolute = true;
                components.clear();
            }

            Component::Current => {}

            Component::Parent => {
                if !components.is_empty() {
                    components.pop();
                } else if !absolute {
                    components.push("..");
                }
            }

            Component::Normal(name) => {
                components.push(name);
            }
        }
    }

    let Some(name) = components.pop() else {
        return Err(PathError::NoFileName);
    };

    if name == "." || name == ".." {
        return Err(PathError::NoFileName);
    }

    let parent = if components.is_empty() {
        if absolute {
            PathBuf::from_str("/")
        } else {
            PathBuf::from_str(".")
        }
    } else {
        let mut inner = String::new();

        if absolute {
            inner.push('/');
        }

        inner.push_str(&components.join("/"));

        PathBuf { inner }
    };

    validate_file_name(name)?;

    Ok((parent, name))
}


pub fn validate_file_name(name: &str) -> Result<(), PathError> {
    if name.is_empty() || name == "." || name == ".." {
        return Err(PathError::NoFileName);
    }
    if name.as_bytes().contains(&0) {
        return Err(PathError::ContainsNull);
    }
    if name.as_bytes().contains(&b'/') {
        return Err(PathError::NoFileName);
    }
    Ok(())
}

fn normalized_parts(path: &Path<'_>) -> (bool, Vec<String>) {
    let mut absolute = false;
    let mut components = Vec::new();

    for component in path.components() {
        match component { 
            Component::Root => {
                absolute = true;
                components.clear();
            },
            Component::Current => {},
            Component::Parent => {
                if !components.is_empty() {
                    components.pop();
                } else if !absolute {
                    components.push("..".to_string());
                }
            },
            Component::Normal(name) => {
                components.push(name.to_string());
            }
        }
    }

    (absolute, components)
}

#[test]
fn normalize_root() {
    assert_eq!(Path::new("/").normalize_lexical().as_str(), "/");
    assert_eq!(Path::new("/..").normalize_lexical().as_str(), "/");
    assert_eq!(Path::new("/a/../b").normalize_lexical().as_str(), "/b");
}

#[test]
fn join_paths() {
    assert_eq!(PathBuf::from_str("/a/b").join(&Path::new("c")).as_str(), "/a/b/c");
    assert_eq!(PathBuf::from_str("/a/b").join(&Path::new("../c")).as_str(), "/a/c");
    assert_eq!(PathBuf::from_str("/a/b").join(&Path::new("/x")).as_str(), "/x");
}

#[test]
fn split_parent_and_name() {
    let (parent, name) = split_parent_name(&Path::new("/a/b/c")).unwrap();
    assert_eq!(parent.as_str(), "/a/b");
    assert_eq!(name, "c");

    let (parent, name) = split_parent_name(&Path::new("c")).unwrap();
    assert_eq!(parent.as_str(), ".");
    assert_eq!(name, "c");

    assert!(split_parent_name(&Path::new("/")).is_err());
    assert!(split_parent_name(&Path::new(".")).is_err());
    assert!(split_parent_name(&Path::new("..")).is_err());
}
