/// One byte of a signature, either unconstrained or matched exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SignaturePattern {
	Any,
	Exact(u8),
}

/// Build a signature from byte literals and single-byte `?` wildcards.
///
/// ```
/// use source_sdk_2013_raw::util::{pattern, sig};
/// assert!(pattern(&[0x48, 0x89, 0xff], &sig![0x48 ? 0xff]));
/// ```
#[macro_export]
macro_rules! sig {
	(macro $Exact:literal) => { $crate::util::SignaturePattern::Exact($Exact) };
	(macro ?) => { $crate::util::SignaturePattern::Any };
	(macro $($Other:tt)*) => { compile_error!("Unknown signature token") };
	($($Token:tt)*) => {{
		let sig: [$crate::util::SignaturePattern; _] = [$($crate::sig!(macro $Token)),*];
		sig
	}};
}

/// Match a signature against a byte prefix, rejecting inputs that are too short.
pub fn pattern(bytes: &[u8], expected: &[SignaturePattern]) -> bool {
	bytes.len() >= expected.len()
		&& bytes
			.iter()
			.zip(expected)
			.all(|(actual, expected)| match expected {
				SignaturePattern::Any => true,
				SignaturePattern::Exact(byte) => actual == byte,
			})
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn signature_tokens_preserve_literal_bytes_and_prefix_matching() {
		for byte in 0..=u8::MAX {
			assert!(pattern(&[byte], &sig![?]));
			assert!(pattern(&[byte], &[SignaturePattern::Exact(byte)]));
			assert!(!pattern(
				&[byte],
				&[SignaturePattern::Exact(byte.wrapping_add(1))]
			));
		}
		assert!(pattern(&[0, 0x89, 0xff, 0x90], &sig![0 ? 0xff]));
		assert!(!pattern(&[0, 0x89], &sig![0 ? 0xff]));
		assert!(!pattern(&[1, 0x89, 0xff], &sig![0 ? 0xff]));
		assert!(!pattern(&[0, 0x89, 0xfe], &sig![0 ? 0xff]));
	}
}
