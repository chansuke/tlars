//! The `SimpleDateFormat` patterns TLC uses, and nothing more. The caller supplies the
//! zone offset, so formatting is a pure function (no process time zone). [`Clock`] is the
//! one place that reads the system time.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pattern {
    /// `yyyy-MM-dd HH:mm:ss` (`MP.now`).
    Timestamp,
    /// `D'd' HH'h'` (`TLC.convertRuntimeToHumanReadable`, more than a day).
    DayHour,
    /// `HH'h' mm'min'`.
    HourMinute,
    /// `mm'min' ss's'`.
    MinuteSecond,
    /// `ss's'`.
    Second,
    /// `yy-MM-dd-HH-mm-ss.SSS` (metadir names).
    MetaDir,
    /// `yy-MM-dd-HH-mm-ss` (metadir names without milliseconds).
    MetaDirSeconds,
}

impl Pattern {
    /// The Java pattern string, for diagnostics and golden files.
    pub fn java_pattern(self) -> &'static str {
        match self {
            Pattern::Timestamp => "yyyy-MM-dd HH:mm:ss",
            Pattern::DayHour => "D'd' HH'h'",
            Pattern::HourMinute => "HH'h' mm'min'",
            Pattern::MinuteSecond => "mm'min' ss's'",
            Pattern::Second => "ss's'",
            Pattern::MetaDir => "yy-MM-dd-HH-mm-ss.SSS",
            Pattern::MetaDirSeconds => "yy-MM-dd-HH-mm-ss",
        }
    }
}

struct Civil {
    year: i64,
    month: u32,
    day: u32,
    day_of_year: u32,
    hour: i64,
    minute: i64,
    second: i64,
    millis: i64,
}

/// Proleptic Gregorian date from days since 1970-01-01 (Howard Hinnant's algorithm).
/// TLC's dates are all after the 1582 Julian cutover `GregorianCalendar` applies.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn civil(epoch_millis: i64, utc_offset_minutes: i32) -> Civil {
    let local = epoch_millis + i64::from(utc_offset_minutes) * 60_000;
    let days = local.div_euclid(86_400_000);
    let ms_of_day = local.rem_euclid(86_400_000);
    let (year, month, day) = civil_from_days(days);
    const CUMULATIVE: [u32; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    let leap = u32::from(month > 2 && is_leap(year));
    Civil {
        year,
        month,
        day,
        day_of_year: CUMULATIVE[month as usize - 1] + day + leap,
        hour: ms_of_day / 3_600_000,
        minute: ms_of_day / 60_000 % 60,
        second: ms_of_day / 1000 % 60,
        millis: ms_of_day % 1000,
    }
}

pub fn format(pattern: Pattern, epoch_millis: i64, utc_offset_minutes: i32) -> String {
    let c = civil(epoch_millis, utc_offset_minutes);
    let yy = c.year.rem_euclid(100);
    match pattern {
        Pattern::Timestamp => format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", c.year, c.month, c.day, c.hour, c.minute, c.second),
        Pattern::DayHour => format!("{}d {:02}h", c.day_of_year, c.hour),
        Pattern::HourMinute => format!("{:02}h {:02}min", c.hour, c.minute),
        Pattern::MinuteSecond => format!("{:02}min {:02}s", c.minute, c.second),
        Pattern::Second => format!("{:02}s", c.second),
        Pattern::MetaDir => format!("{yy:02}-{:02}-{:02}-{:02}-{:02}-{:02}.{:03}", c.month, c.day, c.hour, c.minute, c.second, c.millis),
        Pattern::MetaDirSeconds => {
            format!("{yy:02}-{:02}-{:02}-{:02}-{:02}-{:02}", c.month, c.day, c.hour, c.minute, c.second)
        }
    }
}

/// Where `System.currentTimeMillis()` and timed waits get the time.
#[derive(Clone, Copy, Debug)]
pub enum Clock {
    System,
    Fixed { epoch_millis: i64, utc_offset_minutes: i32 },
}

#[expect(clippy::disallowed_methods, reason = "Clock is the one place that reads the system time")]
impl Clock {
    /// `System.currentTimeMillis()`.
    pub fn now_millis(&self) -> i64 {
        match self {
            Clock::Fixed { epoch_millis, .. } => *epoch_millis,
            Clock::System => std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64),
        }
    }

    /// When a wait of `timeout` that starts now gives up (`Object.wait(ms)`). A fixed clock
    /// does not advance, so its waits end only when notified: a hermetic run prints no
    /// progress report however long it takes.
    pub fn deadline(&self, timeout: std::time::Duration) -> Deadline {
        match self {
            Clock::Fixed { .. } => Deadline::Never,
            Clock::System => Deadline::At(std::time::Instant::now() + timeout),
        }
    }
}

/// The end of a timed wait, from [`Clock::deadline`].
pub enum Deadline {
    /// The wait ends only when it is notified.
    Never,
    At(std::time::Instant),
}

/// What is left of a [`Deadline`].
pub enum Remaining {
    Unbounded,
    For(std::time::Duration),
    Passed,
}

#[expect(clippy::disallowed_methods, reason = "Clock is the one place that reads the system time")]
impl Deadline {
    pub fn remaining(&self) -> Remaining {
        match self {
            Deadline::Never => Remaining::Unbounded,
            Deadline::At(at) => match at.checked_duration_since(std::time::Instant::now()) {
                Some(left) if !left.is_zero() => Remaining::For(left),
                _ => Remaining::Passed,
            },
        }
    }
}
