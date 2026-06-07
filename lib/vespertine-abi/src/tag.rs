// tag definitions for HandleGrant

// tags for standard posix-like execution
pub const TAG_ARG_FILE_0: usize = 0x1000;
pub const TAG_ARG_FILE_1: usize = 0x1001;

// tags for internal sys services
pub const TAG_SYS_LOGGER: usize = 0x2000;
pub const TAG_SYS_CONFIG: usize = 0x2001;
pub const TAG_SYS_PROCMAN: usize = 0x2002;
pub const TAG_SYS_SOCKFAC: usize = 0x2003;
pub const TAG_SYS_RES_MAN: usize = 0x2004;
pub const TAG_SYS_CLOCK: usize = 0x2005;
pub const TAG_SYS_MEMMAN: usize = 0x2006;

// tags for applications
pub const TAG_APP_TERM: usize = 0x3000;
