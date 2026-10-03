//! Byte signatures with single-byte wildcards, and matching them against
//! code.

/// One byte of a signature, either unconstrained or matched exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SignaturePattern {
	/// Matches any byte (`?`).
	Any,

	/// Matches this byte only.
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
