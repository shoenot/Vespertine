use core::hint::spin_loop;

use super::super::io::*;
use crate::arch::{
    disable_interrupts,
    enable_interrupts,
    interrupts_enabled,
};
use crate::core::acpi::fadt::get_century_register;
use crate::core::time::datetime::DateTime;

#[inline(always)]
fn get_update_flag() -> bool { unsafe { (read_cmos(0x0A) & 0x80) != 0 } }

pub fn read_rtc() -> DateTime {
    let century_reg = get_century_register();
    let interrupts_state = interrupts_enabled();
    disable_interrupts();
    let (mut second, mut minute, mut hour, mut day, mut month, mut year, mut century): (u32, u32, u32, u32, u32, i32, i32);

    unsafe {
        loop {
            while get_update_flag() {
                spin_loop()
            }
            second = read_cmos(0x00) as u32;
            minute = read_cmos(0x02) as u32;
            hour = read_cmos(0x04) as u32;
            day = read_cmos(0x07) as u32;
            month = read_cmos(0x08) as u32;
            year = read_cmos(0x09) as i32;
            century = if century_reg != 0 { read_cmos(century_reg) } else { 20 } as i32;

            let new_second = read_cmos(0x00) as u32;
            let new_minute = read_cmos(0x02) as u32;
            let new_hour = read_cmos(0x04) as u32;
            let new_day = read_cmos(0x07) as u32;
            let new_month = read_cmos(0x08) as u32;
            let new_year = read_cmos(0x09) as i32;
            let new_century = if century_reg != 0 { read_cmos(century_reg) } else { 20 } as i32;

            if (second == new_second) &&
                (minute == new_minute) &&
                (hour == new_hour) &&
                (day == new_day) &&
                (month == new_month) &&
                (year == new_year) &&
                (century == new_century)
            {
                break;
            }
        }

        // convert BCD to binary values if necessary
        if (read_cmos(0x0B) & 0x04) == 0 {
            second = (second & 0x0F) + ((second / 16) * 10);
            minute = (minute & 0x0F) + ((minute / 16) * 10);
            hour = ((hour & 0x0F) + (((hour & 0x70) / 16) * 10)) | (hour & 0x80);
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

    if interrupts_state {
        enable_interrupts();
    }

    let full_year = year + (century * 100);

    DateTime { year: full_year, month, day, hour, minute, second }
}
