//! Shared JSON size limits for tool arguments and provider/transport messages.
//! Limits count encoded UTF-8 bytes, including escaping and JSON punctuation.

use serde::Serialize;
use std::io::{self, Write};

struct LimitedWriter<W> {
    inner: W,
    remaining: usize,
}

impl<W: Write> Write for LimitedWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.remaining {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "JSON exceeds size limit",
            ));
        }
        self.inner.write_all(bytes)?;
        self.remaining -= bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Count encoded bytes without allocating an encoded copy. Serialization stops
/// as soon as the limit is exceeded; both overflow and serialization errors fail.
pub fn encoded_len<T: Serialize + ?Sized>(value: &T, limit: usize) -> serde_json::Result<usize> {
    let mut writer = LimitedWriter {
        inner: io::sink(),
        remaining: limit,
    };
    serde_json::to_writer(&mut writer, value)?;
    Ok(limit - writer.remaining)
}

/// Encode once into a bounded buffer, suitable for sending the validated bytes
/// directly instead of serializing the same message again after a size check.
pub fn to_vec<T: Serialize + ?Sized>(value: &T, limit: usize) -> serde_json::Result<Vec<u8>> {
    let mut writer = LimitedWriter {
        inner: Vec::new(),
        remaining: limit,
    };
    serde_json::to_writer(&mut writer, value)?;
    Ok(writer.inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::ser::SerializeSeq;
    use serde_json::json;
    use std::cell::Cell;

    #[test]
    fn both_paths_match_exact_json_encoding_and_inclusive_limit() {
        for value in [
            json!(null),
            json!({}),
            json!({"escaped": "\n\t\"\\\u{0000}", "unicode": "音🎹"}),
            json!([0, -12, 0.125, true, ""]),
        ] {
            let expected = serde_json::to_vec(&value).unwrap();
            assert_eq!(encoded_len(&value, expected.len()).unwrap(), expected.len());
            assert_eq!(to_vec(&value, expected.len()).unwrap(), expected);
            assert!(encoded_len(&value, expected.len() - 1).is_err());
            assert!(to_vec(&value, expected.len() - 1).is_err());
            assert!(encoded_len(&value, 0).is_err());
        }
    }

    #[test]
    fn oversized_input_stops_before_visiting_the_remaining_items() {
        struct Counted<'a>(&'a Cell<usize>);
        impl Serialize for Counted<'_> {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                let mut sequence = serializer.serialize_seq(Some(10_000))?;
                for _ in 0..10_000 {
                    self.0.set(self.0.get() + 1);
                    sequence.serialize_element("item")?;
                }
                sequence.end()
            }
        }
        let visited = Cell::new(0);
        assert!(encoded_len(&Counted(&visited), 32).is_err());
        assert!(visited.get() < 10);
        visited.set(0);
        assert!(to_vec(&Counted(&visited), 32).is_err());
        assert!(visited.get() < 10);
    }

    #[test]
    fn serialization_failure_is_not_mistaken_for_a_valid_size() {
        let invalid = std::collections::BTreeMap::from([(vec![1, 2], "invalid JSON key")]);
        assert!(encoded_len(&invalid, usize::MAX).is_err());
        assert!(to_vec(&invalid, usize::MAX).is_err());
    }
}
