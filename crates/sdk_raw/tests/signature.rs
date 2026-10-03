//! Tests of byte signatures and matching them.

use source_sdk_2013_raw::sig;
use source_sdk_2013_raw::util::{SignaturePattern, find_all, pattern};

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
