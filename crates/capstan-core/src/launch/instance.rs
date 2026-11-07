//! Bounded framing for raw arguments forwarded by a second desktop process.
use super::{MAX_ARGUMENTS, MAX_ARGUMENT_LENGTH};

pub const MAX_PAYLOAD_BYTES: usize = 1 + MAX_ARGUMENTS * (4 + MAX_ARGUMENT_LENGTH * 4);

/// Invalid or excessive input is forwarded as no arguments, without interpreting it.
#[must_use]
pub fn encode_arguments(arguments: &[String]) -> Vec<u8> {
    if arguments.len() > MAX_ARGUMENTS
        || arguments.iter().any(|argument| {
            argument.contains('\0') || argument.chars().count() > MAX_ARGUMENT_LENGTH
        })
    {
        return vec![0];
    }
    let mut payload = vec![arguments.len() as u8];
    for argument in arguments {
        payload.extend_from_slice(&(argument.len() as u32).to_le_bytes());
        payload.extend_from_slice(argument.as_bytes());
    }
    payload
}

#[must_use]
pub fn decode_arguments(payload: &[u8]) -> Option<Vec<String>> {
    if payload.len() > MAX_PAYLOAD_BYTES {
        return None;
    }
    let (&count, mut remaining) = payload.split_first()?;
    if usize::from(count) > MAX_ARGUMENTS {
        return None;
    }
    let mut arguments = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        let length = u32::from_le_bytes(remaining.get(..4)?.try_into().ok()?) as usize;
        remaining = remaining.get(4..)?;
        let value = std::str::from_utf8(remaining.get(..length)?).ok()?;
        if value.contains('\0') || value.chars().count() > MAX_ARGUMENT_LENGTH {
            return None;
        }
        arguments.push(value.to_owned());
        remaining = remaining.get(length..)?;
    }
    remaining.is_empty().then_some(arguments)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_arguments_round_trip_without_separator_or_unicode_loss() {
        let arguments = vec![
            "ssh://user@[::1]:2222/?ignored=one|two".into(),
            "C:\\项目 资料\\directory".into(),
            "".into(),
        ];
        assert_eq!(
            decode_arguments(&encode_arguments(&arguments)),
            Some(arguments)
        );
        let boundary = vec!["字".repeat(MAX_ARGUMENT_LENGTH); MAX_ARGUMENTS];
        assert_eq!(
            decode_arguments(&encode_arguments(&boundary)),
            Some(boundary)
        );
    }

    #[test]
    fn malformed_and_oversized_frames_cannot_allocate_unbounded_strings() {
        for payload in [
            &[][..],
            &[9],
            &[1, 255, 255, 255, 255],
            &[0, 1],
            &[1, 1, 0, 0, 0, 255],
        ] {
            assert!(decode_arguments(payload).is_none());
        }
        assert!(decode_arguments(&vec![0; MAX_PAYLOAD_BYTES + 1]).is_none());
        assert_eq!(encode_arguments(&vec![String::new(); 9]), vec![0]);
        assert_eq!(
            encode_arguments(&["x".repeat(MAX_ARGUMENT_LENGTH + 1)]),
            vec![0]
        );
        assert_eq!(encode_arguments(&["a\0b".into()]), vec![0]);
        let valid = encode_arguments(&["ssh://host".into()]);
        for length in 0..valid.len() {
            assert!(decode_arguments(&valid[..length]).is_none());
        }
    }
}
