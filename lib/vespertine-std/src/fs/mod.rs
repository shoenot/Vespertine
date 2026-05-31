mod dir;
mod file;

pub use dir::*;
pub use file::*;

pub fn parse_parent_and_name(path: &str) -> (&str, &str) {
    if let Some(idx) = path.rfind('/') {
        let parent = &path[..idx];
        let name = &path[idx + 1..];
        let parent = if parent.is_empty() && path.starts_with('/') {
            "/"
        } else {
            parent
        };
        (parent, name)
    } else {
        ("", path)
    }
}
