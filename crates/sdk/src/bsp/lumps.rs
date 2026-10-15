//! Shared bounded directory and Source LZMA normalization.

use super::{Compression, Error, Invalid, Limits, Resource, Work, bound, raw, reserve};
use std::borrow::Cow;
use std::io::{self, Cursor, Write};

pub(super) struct Budget {
	limits: Limits,
	expanded: usize,
	codec: usize,
}

impl Budget {
	pub(super) fn new(limits: Limits) -> Self {
		Self {
			limits,
			expanded: 0,
			codec: 0,
		}
	}

	fn codec(&mut self, input: usize, output: usize, lookahead: usize) -> Result<(), Error> {
		// Reserve both passes before any allocation or codec work.
		for amount in [input, output, lookahead, input, output, lookahead] {
			add(
				&mut self.codec,
				amount,
				self.limits.codec_bytes,
				Resource::CodecBytes,
			)?;
		}
		Ok(())
	}

	fn expanded(&mut self, bytes: usize) -> Result<(), Error> {
		add(
			&mut self.expanded,
			bytes,
			self.limits.expanded_bytes,
			Resource::ExpandedBytes,
		)
	}
}

pub(super) struct Directory<'a> {
	bytes: &'a [u8],
	entries: [Entry; raw::LUMP_COUNT],
	pub(super) version: i32,
	pub(super) revision: i32,
}

impl<'a> Directory<'a> {
	pub(super) fn read(bytes: &'a [u8], limits: Limits, work: &mut Work) -> Result<Self, Error> {
		bound(bytes.len(), limits.input_bytes, Resource::InputBytes)?;
		if bytes.len() < raw::HEADER_BYTES {
			return Err(Invalid::Header.into());
		}
		if bytes[..4] != raw::IDENT {
			return Err(Invalid::Identifier.into());
		}
		let version = integer(bytes, 4);
		if !matches!(version, raw::VERSION_19 | raw::VERSION_20) {
			return Err(Error::UnsupportedVersion(version));
		}
		let mut entries = [Entry::default(); raw::LUMP_COUNT];
		for (id, entry) in entries.iter_mut().enumerate() {
			work.spend(1)?;
			let at = 8 + id * raw::LUMP_ENTRY_BYTES;
			let invalid = || Invalid::LumpRange { lump: id };
			let start = usize::try_from(integer(bytes, at)).map_err(|_| invalid())?;
			let length = usize::try_from(integer(bytes, at + 4)).map_err(|_| invalid())?;
			let end = start.checked_add(length).ok_or_else(invalid)?;
			if end > bytes.len() || (length != 0 && start < raw::HEADER_BYTES) {
				return Err(invalid().into());
			}
			*entry = Entry {
				start,
				end,
				version: integer(bytes, at + 8),
				expanded: integer(bytes, at + 12),
			};
		}
		Ok(Self {
			bytes,
			entries,
			version,
			revision: integer(bytes, raw::HEADER_BYTES - 4),
		})
	}

	/// The whole input, which game lump records address by absolute offset.
	pub(super) fn file(&self) -> &'a [u8] {
		self.bytes
	}

	pub(super) fn prepare(
		&self,
		id: usize,
		versions: &[i32],
		budget: &mut Budget,
		work: &mut Work,
	) -> Result<Lump<'a>, Error> {
		work.spend(1)?;
		let entry = self.entries[id];
		if !versions.contains(&entry.version) {
			return Err(Error::UnsupportedLumpVersion {
				lump: id,
				version: entry.version,
			});
		}
		let stored = &self.bytes[entry.start..entry.end];
		let expanded =
			usize::try_from(entry.expanded).map_err(|_| invalid(id, Compression::ExpandedSize))?;
		Lump::framed(id, stored, expanded, entry.version, budget)
	}
}

#[derive(Clone, Copy, Default)]
struct Entry {
	start: usize,
	end: usize,
	version: i32,
	expanded: i32,
}

struct ExactOutput {
	bytes: Vec<u8>,
	written: usize,
	overflow: bool,
}

impl ExactOutput {
	fn new(length: usize) -> Result<Self, Error> {
		let mut bytes = Vec::new();
		reserve(&mut bytes, length, Resource::ExpandedBytes)?;
		bytes.resize(length, 0);
		Ok(Self {
			bytes,
			written: 0,
			overflow: false,
		})
	}
}

impl Write for ExactOutput {
	fn flush(&mut self) -> io::Result<()> {
		Ok(())
	}

	fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
		let Some(end) = self
			.written
			.checked_add(bytes.len())
			.filter(|&end| end <= self.bytes.len())
		else {
			self.overflow = true;
			return Err(io::ErrorKind::WriteZero.into());
		};
		self.bytes[self.written..end].copy_from_slice(bytes);
		self.written = end;
		Ok(bytes.len())
	}
}

#[derive(Clone, Copy)]
pub(super) struct Lump<'a> {
	id: usize,
	stored: &'a [u8],
	pub(super) length: usize,
	pub(super) version: i32,
	memory_limit: Option<usize>,
}

impl<'a> Lump<'a> {
	/// Checks the framing of stored bytes: raw when `expanded` is zero, and
	/// Source LZMA declaring `expanded` bytes otherwise. `id` names the lump in
	/// errors; game lumps use their containing lump's id.
	pub(super) fn framed(
		id: usize,
		stored: &'a [u8],
		expanded: usize,
		version: i32,
		budget: &mut Budget,
	) -> Result<Self, Error> {
		let length = if expanded == 0 {
			stored.len()
		} else {
			expanded
		};
		budget.expanded(length)?;
		let memory_limit = if expanded == 0 {
			None
		} else {
			if stored.len() < raw::LZMA_HEADER_BYTES {
				return Err(invalid(id, Compression::Header));
			}
			if stored[..4] != raw::LZMA_IDENT {
				return Err(Error::UnsupportedCompression { lump: id });
			}
			if word(stored, 4) as usize != length {
				return Err(invalid(id, Compression::SizeMismatch));
			}
			let encoded = word(stored, 8) as usize;
			if encoded.checked_add(raw::LZMA_HEADER_BYTES) != Some(stored.len()) {
				return Err(invalid(id, Compression::PayloadLength));
			}
			let properties = usize::from(stored[12]);
			if properties >= 225 || stored.get(raw::LZMA_HEADER_BYTES) != Some(&0) {
				return Err(invalid(id, Compression::Properties));
			}
			let lc = properties % 9;
			let lp = (properties / 9) % 5;
			let probability = 1536usize << (lc + lp);
			bound(
				probability,
				budget.limits.probability_bytes,
				Resource::ProbabilityBytes,
			)?;
			let dictionary = (word(stored, 13) as usize).max(4096);
			bound(
				dictionary,
				budget.limits.dictionary_bytes,
				Resource::DictionaryBytes,
			)?;
			// The codec flushes one dictionary chunk at a time. Restrict growth
			// to declared output when smaller, and account for the one possible
			// extra chunk before the exact output sink can reject excess data.
			let memory = dictionary.min(length);
			budget.codec(encoded, length, memory)?;
			Some(memory)
		};
		Ok(Self {
			id,
			stored,
			length,
			version,
			memory_limit,
		})
	}

	pub(super) fn decode(self) -> Result<Cow<'a, [u8]>, Error> {
		let Some(memory_limit) = self.memory_limit else {
			return Ok(Cow::Borrowed(self.stored));
		};
		let mut output = ExactOutput::new(self.length)?;
		// Source's five property bytes are followed directly by the payload.
		let encoded = &self.stored[12..];
		let mut input = Cursor::new(encoded);
		let mut options = lzma_rs::decompress::Options {
			// Pinned lzma-rs 0.3.0 Finish/None requires EOF and zero arithmetic
			// code, including for Valve's markerless streams. Some(length)
			// alone stops at that length without validating this tail state.
			unpacked_size: lzma_rs::decompress::UnpackedSize::UseProvided(None),
			memlimit: Some(memory_limit),
			allow_incomplete: false,
		};
		decode_pass(self.id, encoded.len(), &mut input, &mut output, &options)?;
		if output.written < self.length {
			// A markerless stream can reach zero-code EOF before its declared
			// final zero literals (e.g. 32 zeros first stop at 31). Retry only
			// after a successful, fully consumed first pass. The deterministic
			// path reaches the same zero-code state; successful continuation
			// without further input preserves it. An EOS cannot be extended.
			input.set_position(0);
			output.written = 0;
			options.unpacked_size =
				lzma_rs::decompress::UnpackedSize::UseProvided(Some(self.length as u64));
			decode_pass(self.id, encoded.len(), &mut input, &mut output, &options)?;
		}
		if output.written != self.length {
			return Err(invalid(self.id, Compression::OutputLength));
		}
		Ok(Cow::Owned(output.bytes))
	}

	pub(super) fn owned(self) -> Result<Vec<u8>, Error> {
		match self.decode()? {
			Cow::Owned(bytes) => Ok(bytes),

			Cow::Borrowed(bytes) => {
				let mut output = Vec::new();
				reserve(&mut output, bytes.len(), Resource::ExpandedBytes)?;
				output.extend_from_slice(bytes);
				Ok(output)
			}
		}
	}
}

fn add(total: &mut usize, amount: usize, limit: usize, resource: Resource) -> Result<(), Error> {
	*total = total
		.checked_add(amount)
		.ok_or(Error::Budget { resource, limit })?;
	bound(*total, limit, resource)
}

fn decode_pass(
	id: usize,
	encoded_length: usize,
	input: &mut Cursor<&[u8]>,
	output: &mut ExactOutput,
	options: &lzma_rs::decompress::Options,
) -> Result<(), Error> {
	if lzma_rs::lzma_decompress_with_options(input, output, options).is_err() {
		return Err(invalid(
			id,
			if output.overflow {
				Compression::OutputLength
			} else {
				Compression::Stream
			},
		));
	}
	if input.position() != encoded_length as u64 {
		return Err(invalid(id, Compression::TrailingData));
	}
	Ok(())
}

fn integer(bytes: &[u8], at: usize) -> i32 {
	i32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn invalid(lump: usize, reason: Compression) -> Error {
	Invalid::Compression { lump, reason }.into()
}

fn word(bytes: &[u8], at: usize) -> u32 {
	u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}
