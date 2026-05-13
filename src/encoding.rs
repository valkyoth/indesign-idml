//! Base64 helpers for IDML resources.

use crate::error::{IdmlError, Result};

/// Base64 decoding profile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Base64Mode {
    /// Strict RFC 4648 standard alphabet with canonical padding.
    Strict,
    /// Legacy compatibility profile that ignores ASCII transport whitespace.
    LegacyWhitespace,
}

/// Decodes standard-alphabet Base64 with an explicit decoded-size limit.
pub fn decode_standard(input: &[u8], mode: Base64Mode, max_decoded_len: usize) -> Result<Vec<u8>> {
    let decoded_len = decoded_standard_len(input, mode)?;
    if decoded_len > max_decoded_len {
        return Err(IdmlError::LimitExceeded {
            what: "base64 decoded length",
            limit: max_decoded_len as u64,
            actual: decoded_len as u64,
        });
    }

    let mut output = vec![0u8; decoded_len];
    let written = match mode {
        Base64Mode::Strict => base64_ng::STANDARD.decode_slice(input, &mut output)?,
        Base64Mode::LegacyWhitespace => {
            base64_ng::STANDARD.decode_slice_legacy(input, &mut output)?
        }
    };
    output.truncate(written);
    Ok(output)
}

/// Encodes bytes using canonical padded standard-alphabet Base64.
pub fn encode_standard(input: &[u8], max_input_len: usize) -> Result<Vec<u8>> {
    if input.len() > max_input_len {
        return Err(IdmlError::LimitExceeded {
            what: "base64 input length",
            limit: max_input_len as u64,
            actual: input.len() as u64,
        });
    }

    let encoded_len = base64_ng::checked_encoded_len(input.len(), true)
        .ok_or(base64_ng::EncodeError::LengthOverflow)?;
    let mut output = vec![0u8; encoded_len];
    let written = base64_ng::STANDARD.encode_slice(input, &mut output)?;
    output.truncate(written);
    Ok(output)
}

/// Returns the exact decoded length for the selected standard-alphabet mode.
pub fn decoded_standard_len(input: &[u8], mode: Base64Mode) -> Result<usize> {
    match mode {
        Base64Mode::Strict => Ok(base64_ng::STANDARD.decoded_len(input)?),
        Base64Mode::LegacyWhitespace => Ok(base64_ng::STANDARD.decoded_len_legacy(input)?),
    }
}

#[cfg(test)]
mod tests {
    use super::{Base64Mode, decode_standard, encode_standard};
    use crate::IdmlError;

    #[test]
    fn strict_base64_round_trips() {
        let encoded = encode_standard(b"hello", 32).unwrap();
        assert_eq!(encoded, b"aGVsbG8=");
        assert_eq!(
            decode_standard(&encoded, Base64Mode::Strict, 32).unwrap(),
            b"hello"
        );
    }

    #[test]
    fn strict_mode_rejects_whitespace() {
        let err = decode_standard(b"aG Vs", Base64Mode::Strict, 32).unwrap_err();
        assert!(matches!(err, IdmlError::Base64Decode(_)));
    }

    #[test]
    fn legacy_mode_allows_only_transport_whitespace() {
        assert_eq!(
            decode_standard(b" aG\r\nVs\tbG8= ", Base64Mode::LegacyWhitespace, 32).unwrap(),
            b"hello"
        );
    }

    #[test]
    fn base64_decode_limit_is_enforced_before_allocation() {
        let err = decode_standard(b"aGVsbG8=", Base64Mode::Strict, 4).unwrap_err();
        assert!(
            matches!(err, IdmlError::LimitExceeded { what, .. } if what == "base64 decoded length")
        );
    }
}
