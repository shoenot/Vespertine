#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct IoCompletion {
    pub cookie: usize,
    pub bytes: usize,
    pub status: usize,
}
