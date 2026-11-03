use crate::{BinaryFrame, Result, TerminalError, MAX_FRAME_SIZE};

const FRAME_MAGIC: [u8; 4] = *b"CSTN";
const FRAME_HEADER_LENGTH: usize = 20;

/// Encodes sequence, payload length and a payload checksum inside one raw IPC
/// message so the frontend can validate the end-to-end frame contract.
pub fn encode_binary_frame(frame: BinaryFrame) -> Result<Vec<u8>> {
    if frame.len() > MAX_FRAME_SIZE {
        return Err(TerminalError::ChunkTooLarge);
    }
    let payload_length = u32::try_from(frame.payload.len()).map_err(|_| {
        TerminalError::OutputSend("frame payload exceeds protocol limit".to_owned())
    })?;
    let mut encoded = Vec::with_capacity(FRAME_HEADER_LENGTH + frame.payload.len());
    encoded.extend_from_slice(&FRAME_MAGIC);
    encoded.extend_from_slice(&frame.sequence.to_le_bytes());
    encoded.extend_from_slice(&payload_length.to_le_bytes());
    encoded.extend_from_slice(&checksum(&frame.payload).to_le_bytes());
    encoded.extend_from_slice(&frame.payload);
    Ok(encoded)
}

fn checksum(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811c_9dc5_u32, |hash, byte| {
        (hash ^ u32::from(*byte)).wrapping_mul(0x0100_0193)
    })
}

#[cfg(test)]
mod tests {
    use crate::BinaryFrame;

    use super::{checksum, encode_binary_frame, FRAME_HEADER_LENGTH, FRAME_MAGIC};

    #[test]
    fn raw_frame_retains_sequence_length_checksum_and_binary_payload() {
        let frame = BinaryFrame {
            sequence: 0x0102_0304_0506_0708,
            payload: vec![0, 255, 1, 0],
        };
        let encoded = encode_binary_frame(frame.clone()).expect("encode frame");
        assert_eq!(&encoded[..4], &FRAME_MAGIC);
        assert_eq!(
            u64::from_le_bytes(encoded[4..12].try_into().expect("sequence")),
            frame.sequence
        );
        assert_eq!(
            u32::from_le_bytes(encoded[12..16].try_into().expect("length")),
            4
        );
        assert_eq!(
            u32::from_le_bytes(encoded[16..20].try_into().expect("checksum")),
            checksum(&frame.payload)
        );
        assert_eq!(&encoded[FRAME_HEADER_LENGTH..], frame.payload);
    }
}
