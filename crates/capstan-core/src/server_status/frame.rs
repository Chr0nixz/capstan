use super::contracts::{
    Details, Disk, Interface, Memory, NetworkRate, ServerStatusState, StatusError,
    StatusObservation,
};
use crate::sftp::ExactU64;
use capstan_ssh::exec::ExecChannel;
use std::{collections::BTreeMap, time::Duration};

pub(super) const MAX_FRAME_BYTES: usize = 64 * 1024;
pub(super) const MAX_INTERFACES: usize = 64;
pub(super) const MAX_MOUNTS: usize = 64;
pub(super) const MAX_CPU_LINES: usize = 1024;

pub(super) const SCRIPT: &str = r#"{
LC_ALL=C
export LC_ALL
os=$(uname -s) || exit 1
printf '%s\n' "$os"
[ "$os" = Linux ] || exit 0
while IFS= read -r tick; do
    [ "$tick" = sample ] || exit 1
    printf 'CAPSTAN-STATUS/1\n'
    cat /proc/stat || exit 1
    printf 'CAPSTAN-MEMORY\n'
    cat /proc/meminfo || exit 1
    printf 'CAPSTAN-LOAD\n'
    cat /proc/loadavg || exit 1
    printf 'CAPSTAN-NETWORK\n'
    cat /proc/net/dev || exit 1
    printf 'CAPSTAN-DISK\n'
    df -P -k || exit 1
    printf 'CAPSTAN-END\n'
done
}
"#;

const START: &str = "CAPSTAN-STATUS/1\n";
const END: &str = "CAPSTAN-END\n";

#[derive(Debug, PartialEq)]
pub(super) struct Sample {
    cpu: [u64; 8],
    cpu_lines: usize,
    memory: Memory,
    load: [f32; 3],
    network: BTreeMap<String, (u64, u64)>,
    disks: Vec<Disk>,
}

pub(super) async fn sample(channel: &mut ExecChannel) -> Result<Sample, StatusError> {
    channel.write_stdin(b"sample\n").await?;
    let mut frame = Vec::new();
    loop {
        let line = channel
            .read_line(MAX_FRAME_BYTES - frame.len())
            .await?
            .ok_or(StatusError::Closed)?;
        if frame.is_empty() && line != START.as_bytes() {
            return Err(StatusError::Parse);
        }
        let complete = line == END.as_bytes();
        frame.extend_from_slice(&line);
        if complete {
            return parse(&frame);
        }
    }
}

pub(super) fn parse(frame: &[u8]) -> Result<Sample, StatusError> {
    if frame.len() > MAX_FRAME_BYTES {
        return Err(StatusError::Limit);
    }
    // Linux paths may contain non-UTF-8 bytes. The displayed labels are only descriptive.
    let frame = String::from_utf8_lossy(frame);
    let body = frame
        .strip_prefix(START)
        .and_then(|body| body.strip_suffix(END))
        .ok_or(StatusError::Parse)?;
    let (stat, body) = body
        .split_once("\nCAPSTAN-MEMORY\n")
        .ok_or(StatusError::Parse)?;
    let (memory, body) = body
        .split_once("\nCAPSTAN-LOAD\n")
        .ok_or(StatusError::Parse)?;
    let (load, body) = body
        .split_once("\nCAPSTAN-NETWORK\n")
        .ok_or(StatusError::Parse)?;
    let (network, disk) = body
        .split_once("\nCAPSTAN-DISK\n")
        .ok_or(StatusError::Parse)?;
    let (cpu, cpu_lines) = parse_cpu(stat)?;
    Ok(Sample {
        cpu,
        cpu_lines,
        memory: parse_memory(memory)?,
        load: parse_load(load)?,
        network: parse_network(network)?,
        disks: parse_disks(disk)?,
    })
}

fn number(value: Option<&str>) -> Result<u64, StatusError> {
    value
        .and_then(|value| value.parse().ok())
        .ok_or(StatusError::Parse)
}

fn parse_cpu(text: &str) -> Result<([u64; 8], usize), StatusError> {
    let mut aggregate = None;
    let mut count = 0;
    for line in text.lines() {
        let mut fields = line.split_ascii_whitespace();
        let Some(name) = fields.next() else {
            continue;
        };
        let Some(index) = name.strip_prefix("cpu") else {
            continue;
        };
        if !index.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(StatusError::Parse);
        }
        count += 1;
        if count > MAX_CPU_LINES {
            return Err(StatusError::Limit);
        }
        let mut times = [0; 8];
        let mut fields_read = 0;
        for (slot, value) in times.iter_mut().zip(fields) {
            *slot = number(Some(value))?;
            fields_read += 1;
        }
        if fields_read < 4 {
            return Err(StatusError::Parse);
        }
        if index.is_empty() && aggregate.replace(times).is_some() {
            return Err(StatusError::Parse);
        }
    }
    Ok((aggregate.ok_or(StatusError::Parse)?, count))
}

fn parse_memory(text: &str) -> Result<Memory, StatusError> {
    let mut total = None;
    let mut available = None;
    let mut free = None;
    for line in text.lines() {
        let mut fields = line.split_ascii_whitespace();
        let slot = match fields.next() {
            Some("MemTotal:") => &mut total,
            Some("MemAvailable:") => &mut available,
            Some("MemFree:") => &mut free,
            _ => continue,
        };
        let value = number(fields.next())?
            .checked_mul(1024)
            .ok_or(StatusError::Parse)?;
        if fields.next() != Some("kB") || fields.next().is_some() || slot.replace(value).is_some() {
            return Err(StatusError::Parse);
        }
    }
    let total = total
        .filter(|&total| total != 0)
        .ok_or(StatusError::Parse)?;
    let available = available
        .or(free)
        .filter(|&available| available <= total)
        .ok_or(StatusError::Parse)?;
    Ok(Memory {
        total: ExactU64(total),
        available: ExactU64(available),
    })
}

fn parse_load(text: &str) -> Result<[f32; 3], StatusError> {
    let mut fields = text.split_ascii_whitespace();
    let mut load = [0.; 3];
    for value in &mut load {
        *value = fields
            .next()
            .and_then(|value| value.parse::<f32>().ok())
            .filter(|value| value.is_finite() && *value >= 0.)
            .ok_or(StatusError::Parse)?;
    }
    Ok(load)
}

fn parse_network(text: &str) -> Result<BTreeMap<String, (u64, u64)>, StatusError> {
    let mut lines = text.lines();
    if !lines
        .next()
        .is_some_and(|line| line.trim_start().starts_with("Inter-|"))
        || !lines
            .next()
            .is_some_and(|line| line.trim_start().starts_with("face |"))
    {
        return Err(StatusError::Parse);
    }
    let mut interfaces = BTreeMap::new();
    for line in lines {
        if interfaces.len() == MAX_INTERFACES {
            return Err(StatusError::Limit);
        }
        let (name, counters) = line.rsplit_once(':').ok_or(StatusError::Parse)?;
        let name = name.trim();
        if name.is_empty()
            || name
                .chars()
                .any(|character| character.is_control() || character.is_whitespace())
        {
            return Err(StatusError::Parse);
        }
        let mut fields = counters.split_ascii_whitespace();
        let rx = number(fields.next())?;
        let tx = number(fields.nth(7))?;
        if fields.count() != 7 || interfaces.insert(name.to_owned(), (rx, tx)).is_some() {
            return Err(StatusError::Parse);
        }
    }
    Ok(interfaces)
}

fn column<'a>(line: &mut &'a str) -> Option<&'a str> {
    *line = line.trim_start_matches(|character: char| character.is_ascii_whitespace());
    if line.is_empty() {
        return None;
    }
    let end = line
        .find(|character: char| character.is_ascii_whitespace())
        .unwrap_or(line.len());
    let (field, rest) = line.split_at(end);
    *line = rest;
    Some(field)
}

fn parse_disk(mut line: &str) -> Result<Disk, StatusError> {
    // The filesystem and mount columns can contain spaces. Locate the POSIX
    // numeric columns in between and keep the full mount suffix. If the row
    // has multiple plausible splits, the text does not identify the columns.
    column(&mut line).ok_or(StatusError::Parse)?;
    let mut disk = None;
    while !line.is_empty() {
        let mut remaining = line;
        let total = column(&mut remaining).and_then(|value| value.parse::<u64>().ok());
        let used = column(&mut remaining).and_then(|value| value.parse::<u64>().ok());
        let available = column(&mut remaining).and_then(|value| value.parse::<i128>().ok());
        let capacity = column(&mut remaining)
            .and_then(|value| value.strip_suffix('%'))
            .and_then(|value| value.parse::<u64>().ok());
        if let (Some(total), Some(used), Some(_), Some(_)) = (total, used, available, capacity) {
            let mount =
                remaining.trim_start_matches(|character: char| character.is_ascii_whitespace());
            if !mount.starts_with('/') || mount.chars().any(char::is_control) {
                column(&mut line);
                continue;
            }
            let candidate = Disk {
                mount: mount.to_owned(),
                total: ExactU64(total.checked_mul(1024).ok_or(StatusError::Parse)?),
                used: ExactU64(used.checked_mul(1024).ok_or(StatusError::Parse)?),
            };
            if disk.replace(candidate).is_some() {
                return Err(StatusError::Parse);
            }
        }
        column(&mut line);
    }
    disk.ok_or(StatusError::Parse)
}

fn parse_disks(text: &str) -> Result<Vec<Disk>, StatusError> {
    let mut lines = text.lines();
    if !lines
        .next()
        .is_some_and(|line| line.starts_with("Filesystem"))
    {
        return Err(StatusError::Parse);
    }
    let mut disks = Vec::new();
    for line in lines {
        if disks.len() == MAX_MOUNTS {
            return Err(StatusError::Limit);
        }
        disks.push(parse_disk(line)?);
    }
    Ok(disks)
}

// Only the final percentage is rounded; counter subtraction is exact.
#[allow(clippy::cast_precision_loss)]
fn cpu_percent(previous: &Sample, current: &Sample) -> Option<f32> {
    if previous.cpu_lines != current.cpu_lines {
        return None;
    }
    let mut total = 0_u128;
    let mut idle = 0_u128;
    for (index, (previous, current)) in previous.cpu.iter().zip(current.cpu).enumerate() {
        let delta = u128::from(current.checked_sub(*previous)?);
        total += delta;
        if matches!(index, 3 | 4) {
            idle += delta;
        }
    }
    (total != 0).then(|| (total - idle) as f32 / total as f32 * 100.)
}

fn network_rate((rx, tx): (u128, u128), elapsed: Duration) -> Option<NetworkRate> {
    let per_second = |delta: u128| {
        let rate = (delta * 1_000_000_000).checked_div(elapsed.as_nanos())?;
        u64::try_from(rate).ok().map(ExactU64)
    };
    Some(NetworkRate {
        rx_per_sec: per_second(rx)?,
        tx_per_sec: per_second(tx)?,
    })
}

fn network_delta(previous: (u64, u64), current: (u64, u64)) -> Option<(u128, u128)> {
    Some((
        u128::from(current.0.checked_sub(previous.0)?),
        u128::from(current.1.checked_sub(previous.1)?),
    ))
}

impl Sample {
    pub(super) fn observation(
        &self,
        previous: Option<(&Self, Duration)>,
        sampled_at: u64,
    ) -> StatusObservation {
        let cpu_percent = previous.and_then(|(sample, _)| cpu_percent(sample, self));
        let interfaces: Vec<_> = self
            .network
            .iter()
            .map(|(name, &counters)| {
                let rate = previous.and_then(|(sample, elapsed)| {
                    let delta = network_delta(*sample.network.get(name)?, counters)?;
                    network_rate(delta, elapsed)
                });
                Interface {
                    name: name.clone(),
                    rate,
                }
            })
            .collect();
        let network = previous.and_then(|(sample, elapsed)| {
            let mut data_interfaces = self.network.iter().filter(|(name, _)| *name != "lo");
            if data_interfaces.clone().count()
                != sample.network.keys().filter(|name| *name != "lo").count()
            {
                return None;
            }
            let delta = data_interfaces.try_fold((0, 0), |(rx, tx), (name, &counters)| {
                let (rx_delta, tx_delta) = network_delta(*sample.network.get(name)?, counters)?;
                Some((rx + rx_delta, tx + tx_delta))
            })?;
            network_rate(delta, elapsed)
        });
        StatusObservation {
            state: ServerStatusState::Running,
            error_code: None,
            sampled_at: ExactU64(sampled_at),
            cpu_percent,
            load: Some(self.load),
            memory: Some(self.memory.clone()),
            disk: self.disks.iter().find(|disk| disk.mount == "/").cloned(),
            network,
            details: Some(Details {
                disks: self.disks.clone(),
                interfaces,
            }),
        }
    }
}
