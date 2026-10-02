//! Signature matching over byte slices on both Windows and Linux.
//!
//! Scan owned snapshots when searching engine memory. Callers are responsible
//! for obtaining readable bytes and validating what a candidate refers to
//! before using it as an instruction or engine address.

/// Finds every starting offset at which `pattern` matches `bytes`.
///
/// `Some(byte)` matches that exact byte; `None` matches any single byte.
/// Offsets are relative to `bytes`, in ascending order, including overlapping
/// matches. An empty pattern or a pattern longer than `bytes` has no matches.
/// The iterator borrows its inputs and does not allocate.
///
/// ```
/// use source_sdk_2013::sigscan::find_all;
///
/// let bytes = [0x48, 0x8b, 0x48, 0x8b, 0x48];
/// let pattern = [Some(0x48), None, Some(0x48)];
/// assert_eq!(find_all(&bytes, &pattern).collect::<Vec<_>>(), [0, 2]);
/// ```
pub fn find_all<'a>(
	bytes: &'a [u8],
	pattern: &'a [Option<u8>],
) -> impl Iterator<Item = usize> + 'a {
	// An empty pattern yields no windows; windows itself needs a nonzero width.
	let bytes = if pattern.is_empty() { &[][..] } else { bytes };
	bytes
		.windows(pattern.len().max(1))
		.enumerate()
		.filter_map(move |(offset, candidate)| {
			pattern
				.iter()
				.zip(candidate)
				.all(|(expected, actual)| expected.is_none_or(|expected| expected == *actual))
				.then_some(offset)
		})
}

#[cfg(test)]
mod tests {
	use super::find_all;

	#[test]
	fn empty_or_short_inputs_have_no_matches() {
		assert!(find_all(&[], &[]).next().is_none());
		assert!(find_all(&[0x48], &[]).next().is_none());
		assert!(find_all(&[], &[None]).next().is_none());
		assert!(
			find_all(&[0x48], &[Some(0x48), Some(0x8b)])
				.next()
				.is_none()
		);
	}

	#[test]
	fn matches_at_zero_and_the_last_possible_offset() {
		let pattern = [Some(0x48), Some(0x8b), Some(0x0d)];
		assert_eq!(
			find_all(&[0x48, 0x8b, 0x0d], &pattern).collect::<Vec<_>>(),
			[0]
		);
		assert_eq!(
			find_all(&[0x48, 0x8b, 0x0d, 0x90, 0x48, 0x8b, 0x0d], &pattern).collect::<Vec<_>>(),
			[0, 4]
		);
	}

	#[test]
	fn overlapping_matches_are_all_returned() {
		assert_eq!(
			find_all(b"AAA", &[Some(b'A'), Some(b'A')]).collect::<Vec<_>>(),
			[0, 1]
		);
		// A partial match must not hide a later match that starts inside it.
		assert_eq!(
			find_all(b"AAAB", &[Some(b'A'), Some(b'A'), Some(b'B')]).collect::<Vec<_>>(),
			[1]
		);
	}

	#[test]
	fn wildcards_each_consume_one_byte() {
		assert_eq!(
			find_all(&[0xff, 0, 0xaa, 0xbb, 0, 0xcc], &[None, Some(0), None]).collect::<Vec<_>>(),
			[0, 3]
		);
		assert_eq!(
			find_all(&[0, 1, 2], &[None, None]).collect::<Vec<_>>(),
			[0, 1]
		);
		assert!(find_all(&[0], &[Some(0), None]).next().is_none());
	}

	#[test]
	fn zero_and_ff_are_literal_bytes() {
		assert_eq!(
			find_all(&[0xff, 0, 0xff], &[Some(0), Some(0xff)]).collect::<Vec<_>>(),
			[1]
		);
		assert!(
			find_all(&[1, 0xff], &[Some(0), Some(0xff)])
				.next()
				.is_none()
		);
	}
}
