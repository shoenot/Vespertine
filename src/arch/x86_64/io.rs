unsafe extern "sysv64" {
    pub fn outb(port: u16, value: u8);
    pub fn inb(port: u16) -> u8;
    pub fn outl(port: u16, value: u32);
    pub fn inl(port: u16) -> u32;
}
