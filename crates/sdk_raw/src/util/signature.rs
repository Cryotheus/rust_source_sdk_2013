//! Byte signatures with single-byte wildcards, and matching them against
//! code.

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

/// Finds every starting offset at which `signature` matches `bytes`.
///
/// Offsets are relative to `bytes`, in ascending order, including overlapping
/// matches. An empty signature or a signature longer than `bytes` has no
/// matches. The iterator borrows its inputs and does not allocate.
///
/// Scan owned snapshots when searching engine memory, and validate what a
/// candidate refers to before using it as an instruction or engine address.
///
/// ```
/// use source_sdk_2013_raw::util::{find_all, sig};
///
/// let bytes = [0x48, 0x8b, 0x48, 0x8b, 0x48];
/// assert_eq!(find_all(&bytes, &sig![0x48 ? 0x48]).collect::<Vec<_>>(), [0, 2]);
/// ```
pub fn find_all<'a>(
	bytes: &'a [u8],
	signature: &'a [SignaturePattern],
) -> impl Iterator<Item = usize> + 'a {
	// An empty signature yields no windows; windows itself needs a nonzero width.
	let bytes = if signature.is_empty() { &[][..] } else { bytes };

	bytes
		.windows(signature.len().max(1))
		.enumerate()
		.filter_map(move |(offset, candidate)| pattern(candidate, signature).then_some(offset))
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
	fn empty_or_short_inputs_have_no_matches() {
		assert!(find_all(&[], &[]).next().is_none());
		assert!(find_all(&[0x48], &[]).next().is_none());
		assert!(find_all(&[], &sig![?]).next().is_none());
		assert!(find_all(&[0x48], &sig![0x48 0x8b]).next().is_none());
	}

	#[test]
	fn matches_at_zero_and_the_last_possible_offset() {
		let signature = sig![0x48 0x8b 0x0d];
		assert_eq!(
			find_all(&[0x48, 0x8b, 0x0d], &signature).collect::<Vec<_>>(),
			[0]
		);
		assert_eq!(
			find_all(&[0x48, 0x8b, 0x0d, 0x90, 0x48, 0x8b, 0x0d], &signature).collect::<Vec<_>>(),
			[0, 4]
		);
	}

	#[test]
	fn overlapping_matches_are_all_returned() {
		assert_eq!(
			find_all(b"AAA", &sig![b'A' b'A']).collect::<Vec<_>>(),
			[0, 1]
		);
		// A partial match must not hide a later match that starts inside it.
		assert_eq!(
			find_all(b"AAAB", &sig![b'A' b'A' b'B']).collect::<Vec<_>>(),
			[1]
		);
	}

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

	#[test]
	fn wildcards_each_consume_one_byte() {
		assert_eq!(
			find_all(&[0xff, 0, 0xaa, 0xbb, 0, 0xcc], &sig![? 0 ?]).collect::<Vec<_>>(),
			[0, 3]
		);
		assert_eq!(find_all(&[0, 1, 2], &sig![? ?]).collect::<Vec<_>>(), [0, 1]);
		assert!(find_all(&[0], &sig![0?]).next().is_none());
	}

	#[test]
	fn zero_and_ff_are_literal_bytes() {
		assert_eq!(
			find_all(&[0xff, 0, 0xff], &sig![0 0xff]).collect::<Vec<_>>(),
			[1]
		);
		assert!(find_all(&[1, 0xff], &sig![0 0xff]).next().is_none());
	}
}
