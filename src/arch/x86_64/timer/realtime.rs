use core::hint::spin_loop;

use crate::{arch::{disable_interrupts, enable_interrupts}, kernel::acpi::fadt::get_century_register};

use super::super::io::*;

#[inline(always)]
fn get_update_flag() -> bool {
    unsafe {
        (read_cmos(0x0A) & 0x80) != 0
    }
}

pub fn read_rtc() -> (u8, u8, u8, u8, u8, u16) {
    let century_reg = get_century_register();
    disable_interrupts();
    let (mut second, mut minute, mut hour,
         mut day, mut month, mut year, 
         mut century): (u8, u8, u8, u8, u8, u8, u8);

    unsafe {
        loop {
            while get_update_flag() { spin_loop() };
            second = read_cmos(0x00);
            minute = read_cmos(0x02);
            hour = read_cmos(0x04);
            day = read_cmos(0x07);
            month = read_cmos(0x08);
            year = read_cmos(0x09);
            century = if century_reg != 0 {
                read_cmos(century_reg)
            } else { 20 };

            let new_second = read_cmos(0x00);
            let new_minute = read_cmos(0x02);
            let new_hour = read_cmos(0x04);
            let new_day = read_cmos(0x07);
            let new_month = read_cmos(0x08);
            let new_year = read_cmos(0x09);
            let new_century = if century_reg != 0 {
                read_cmos(century_reg)
            } else { 20 };

            if (second == new_second) && (minute == new_minute) && (hour == new_hour) &&
               (day == new_day) && (month == new_month) && (year == new_year) && 
               (century == new_century) {
                break;
            }
        }


        // convert BCD to binary values if necessary 
        if (read_cmos(0x0B) & 0x04) == 0 {
            second = (second & 0x0F) + ((second / 16) * 10);
            minute = (minute & 0x0F) + ((minute / 16) * 10);
            hour = ( (hour & 0x0F) + (((hour & 0x70) / 16) * 10) ) | (hour & 0x80);
            day = (day & 0x0F) + ((day / 16) * 10);
            month = (month & 0x0F) + ((month / 16) * 10);
            year = (year & 0x0F) + ((year / 16) * 10);
            if century_reg != 0 {
                  century = (century & 0x0F) + ((century / 16) * 10);
            }
        }
        
        // convert 12 hour to 24 hour
        if (read_cmos(0x0B) & 0x02) == 0 && (hour & 0x80) != 0 {
            hour = ((hour & 0x7F) + 12) % 24;
        }
    }

    enable_interrupts();

    let full_year = year as u16 + (century as u16 * 100);
    
    (second, minute, hour, day, month, full_year)
}
