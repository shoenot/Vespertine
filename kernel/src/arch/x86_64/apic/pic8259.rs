use crate::arch::x86_64::io::outb;

const PIC_COMMAND_MASTER: u16 = 0x20;
const PIC_COMMAND_SLAVE: u16 = 0xA0;
const PIC_DATA_MASTER: u16 = 0x21;
const PIC_DATA_SLAVE: u16 = 0xA1;

const ICW_1: u8 = 0x11;
const ICW_2_M: u8 = 0x20;
const ICW_2_S: u8 = 0x28;
const ICW_3_M: u8 = 0x4;
const ICW_3_S: u8 = 0x2;
const ICW_4: u8 = 0x1;

pub(in crate::arch::x86_64::apic) fn disable() {
    unsafe {
        outb(PIC_COMMAND_MASTER, ICW_1);
        outb(PIC_COMMAND_SLAVE, ICW_1);
        outb(PIC_DATA_MASTER, ICW_2_M);
        outb(PIC_DATA_SLAVE, ICW_2_S);
        outb(PIC_DATA_MASTER, ICW_3_M);
        outb(PIC_DATA_SLAVE, ICW_3_S);
        outb(PIC_DATA_MASTER, ICW_4);
        outb(PIC_DATA_SLAVE, ICW_4);
        outb(PIC_DATA_MASTER, 0xFF);
        outb(PIC_DATA_SLAVE, 0xFF);
    }
}
