use core::fmt;
use core::ops::Deref;

#[derive(Debug, PartialEq, Eq)]
pub struct DateTime {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

pub struct IsoFormat<'a>(&'a DateTime);

impl<'a> Deref for IsoFormat<'a> {
    type Target = DateTime;

    fn deref(&self) -> &Self::Target { &self.0 }
}

impl<'a> fmt::Display for IsoFormat<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{:02}-{:02}T{:02}:{:02}:{:02}Z", self.year, self.month, self.day, self.hour, self.minute, self.second,)
    }
}

impl DateTime {
    pub fn iso(&self) -> IsoFormat<'_> { IsoFormat(self) }
}

impl fmt::Display for DateTime {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Date: {:02}-{:02}-{}, Time: {:02}:{:02}:{:02}", self.day, self.month, self.year, self.hour, self.minute, self.second)
    }
}

pub fn epoch_to_datetime(epoch_secs: i64) -> DateTime {
    // 1. split epoch secs into whole days and remaining seconds
    let mut days = (epoch_secs / 86400) as i32;
    let mut secs_of_day = (epoch_secs % 86400) as i32;

    if secs_of_day < 0 {
        secs_of_day += 86400;
        days -= 1;
    }

    // 2. compute time components
    let hour = (secs_of_day / 3600) as u32;
    let rem_secs = secs_of_day % 3600;
    let minute = (rem_secs / 60) as u32;
    let second = (rem_secs % 60) as u32;

    // 3. compute date components
    // (tried to 1:1 translate howard hinnant's civil_from_days algo to rust)
    let days_shifted = days + 719468;
    let era = (if days_shifted >= 0 { days_shifted } else { days_shifted - 146096 }) / 146097;
    let doe = days_shifted - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    DateTime { year: year + (month <= 2) as i32, month, day, hour, minute, second }
}

pub fn datetime_to_epoch(dt: DateTime) -> i64 {
    // do the date part first using the algo
    let (mut y, m, d) = (dt.year as i64, dt.month as i64, dt.day as i64);
    y = y - (m <= 2) as i64;
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let ts = era * 146097 + doe - 719468;

    // add time
    (ts as i64) * 86400 + (dt.hour * 3600) as i64 + (dt.minute * 60) as i64 + dt.second as i64
}
