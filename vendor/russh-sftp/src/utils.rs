use bytes::Bytes;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncRead, AsyncReadExt};

use crate::error::Error;

/// Whole seconds since the Unix epoch, rounded down as upstream's chrono
/// conversion did and truncated to the protocol's 32 bits.
pub(crate) fn unix(time: SystemTime) -> u32 {
    let seconds = match time.duration_since(UNIX_EPOCH) {
        Ok(after) => after.as_secs() as i64,
        Err(before) => {
            let before = before.duration();
            -(before.as_secs() as i64) - i64::from(before.subsec_nanos() > 0)
        }
    };
    seconds as u32
}

/// `%b %d %Y %H:%M` in UTC for a 32-bit protocol time, as `ls -l` prints it.
pub(crate) fn long_time(seconds: u32) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let days = i64::from(seconds / 86_400);
    let minutes = seconds % 86_400 / 60;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let shifted = days + 719_468;
    let era = shifted / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{} {day:02} {year:04} {:02}:{:02}",
        MONTHS[(month - 1) as usize],
        minutes / 60,
        minutes % 60
    )
}

pub(crate) async fn read_packet<S: AsyncRead + Unpin>(
    stream: &mut S,
    max_length: u32,
) -> Result<Bytes, Error> {
    let length = stream.read_u32().await?;
    if length > max_length {
        return Err(Error::BadMessage("packet length limit exceeded".to_owned()));
    }

    let mut buf = vec![0; length as usize];
    stream.read_exact(&mut buf).await?;

    Ok(Bytes::from(buf))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn protocol_times_match_the_former_chrono_conversion() {
        assert_eq!(unix(UNIX_EPOCH + Duration::from_millis(1500)), 1);
        assert_eq!(unix(UNIX_EPOCH - Duration::from_millis(500)), u32::MAX);
        assert_eq!(unix(UNIX_EPOCH - Duration::from_secs(2)), u32::MAX - 1);
        assert_eq!(long_time(0), "Jan 01 1970 00:00");
        assert_eq!(long_time(951_782_400), "Feb 29 2000 00:00");
        assert_eq!(long_time(1_798_761_599), "Dec 31 2026 23:59");
        assert_eq!(long_time(u32::MAX), "Feb 07 2106 06:28");
    }
}
