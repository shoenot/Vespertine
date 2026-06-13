
pub enum Component<'a> {
    Root,
    Current,
    Parent,
    Normal(&'a str),
}

pub struct Components<'a> {
    path: &'a str,
    emitted_root: bool,
}

impl<'a> Components<'a> {
    pub fn new(path: &'a str) -> Self {
        Self { path, emitted_root: false }
    }
}

impl<'a> Iterator for Components<'a> {
    type Item = Component<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if !self.emitted_root && self.path.starts_with('/') {
            self.emitted_root = true;
            self.path = self.path.trim_start_matches('/');
            return Some(Component::Root);
        }

        self.emitted_root = true;

        while self.path.starts_with('/') {
            self.path = &self.path[1..];
        }

        if self.path.is_empty() {
            return None
        }

        let end = self.path.find('/').unwrap_or(self.path.len());
        let segment = &self.path[..end];
        self.path = &self.path[end..];

        Some(match segment {
            "." => Component::Current,
            ".." => Component::Parent,
            name => Component::Normal(name),
        })
    }
}
