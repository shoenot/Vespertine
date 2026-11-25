use crate::CapabilityID;

// tags for standard posix-like execution
pub const TAG_ARG_FILE_0: usize = 0x1000;
pub const TAG_ARG_FILE_1: usize = 0x1001;

pub const CAP_LOGGER: CapabilityID = CapabilityID(0x2000);
pub const CAP_CLOCK: CapabilityID = CapabilityID(0x2001);
pub const CAP_PROCMAN: CapabilityID = CapabilityID(0x2002);
pub const CAP_SOCKFAC: CapabilityID = CapabilityID(0x2003);
pub const CAP_PORTAL_FACTORY: CapabilityID = CapabilityID(0x2004);
pub const CAP_FRAMEBUFFER: CapabilityID = CapabilityID(0x2005);
pub const CAP_LOG_READER: CapabilityID = CapabilityID(0x2006);
pub const CAP_SERIAL_WRITE: CapabilityID = CapabilityID(0x2007);
pub const CAP_LIBC: CapabilityID = CapabilityID(0x2008);

pub const CAP_LAUNCHER_CONNECT: CapabilityID = CapabilityID(0x2100);
pub const CAP_VREG_CONNECT: CapabilityID = CapabilityID(0x2101);
pub const CAP_AUTH_CONNECT: CapabilityID = CapabilityID(0x2102);

pub const CAP_APP_TERMCTRL: CapabilityID = CapabilityID(0x3000);
