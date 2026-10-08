//! Voice in Steam's codec, as TF2 clients send it (`clc_VoiceData`) and play
//! it from [`VoiceData`]: the codec `sv_voicecodec steam` names, which TF2 has
//! used by default since the Jungle Inferno update, with Opus audio.
//!
//! Steam documents its compressed voice as opaque, to be passed to
//! `ISteamUser::DecompressVoice` as it is. This module follows the packets
//! clients send, as community decoders of TF2's demos read them:
//!
//! - the speaker's 64-bit Steam ID,
//! - sections, each a type byte and a 16-bit value, followed by as many bytes
//!   as the value says for [`Section::Opus`],
//! - a CRC-32 of everything before it.
//!
//! Every number is little-endian.
//!
//! [`VoiceData`]: super::messages::VoiceData

#[cfg(test)]
#[path = "../tests/net/steam_voice.rs"]
mod tests;

/// The bytes of a packet's Steam ID.
const STEAM_ID_BYTES: usize = 8;

/// The bytes of a packet's CRC-32.
const CRC_BYTES: usize = 4;

/// The type byte of a [`Section::Silence`].
const SILENCE: u8 = 0;

/// The type byte of a [`Section::Opus`].
const OPUS_PLC: u8 = 6;

/// The type byte of a [`Section::SampleRate`].
const SAMPLE_RATE: u8 = 11;

/// The length of an [`OpusFrame::Reset`] in place of a frame's.
const RESET: u16 = u16::MAX;

/// Why a packet could not be read or written.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SteamVoiceError {
	/// The packet is too short for a Steam ID and a CRC.
	#[error("the packet is {len} bytes long, too short for a Steam ID and a CRC")]
	TooShort {
		/// The packet's length.
		len: usize,
	},

	/// The packet's CRC does not match its contents.
	#[error("the packet's CRC is {stored:#010x}, but its contents' is {computed:#010x}")]
	Crc {
		/// The CRC the packet ends with.
		stored: u32,

		/// The CRC of the packet's contents.
		computed: u32,
	},

	/// A section of a type this module cannot measure, so nothing after it
	/// can be read.
	#[error("a section has the unknown type {kind}")]
	UnknownSection {
		/// The section's type byte.
		kind: u8,
	},

	/// A section or frame runs past the end of what holds it.
	#[error("a section or frame is cut short")]
	Truncated,

	/// An Opus section or frame to encode is longer than its 16-bit length
	/// can say.
	#[error("an Opus section or frame of {len} bytes is too long to encode")]
	TooLong {
		/// Its length in bytes.
		len: usize,
	},
}

/// A part of a packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section<'a> {
	/// Silence, as a number of samples.
	Silence(u16),

	/// The sample rate of the Opus frames, such as 24000.
	SampleRate(u16),

	/// Opus frames, which [`opus_frames`] reads.
	Opus(&'a [u8]),
}

/// One of the frames of a [`Section::Opus`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpusFrame<'a> {
	/// An Opus packet, numbered in the order the speaker encoded them.
	Audio {
		/// The frame's number, which wraps around.
		sequence: u16,

		/// The Opus packet.
		data: &'a [u8],
	},

	/// Resets the listener's decoder, as a frame whose length is `0xFFFF`.
	Reset {
		/// The frame's number.
		sequence: u16,
	},
}

/// A packet of voice, checked against its CRC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SteamVoice<'a> {
	steam_id: u64,
	sections: &'a [u8],
}

impl<'a> SteamVoice<'a> {
	/// Reads a packet's Steam ID, after checking its CRC.
	pub fn parse(packet: &'a [u8]) -> Result<Self, SteamVoiceError> {
		let len = packet.len();
		let Some(contents_len) = len
			.checked_sub(CRC_BYTES)
			.filter(|&contents| contents >= STEAM_ID_BYTES)
		else {
			return Err(SteamVoiceError::TooShort { len });
		};

		let (contents, stored) = packet.split_at(contents_len);
		let stored = u32::from_le_bytes(stored.try_into().unwrap());
		let computed = crc32(contents);

		if stored != computed {
			return Err(SteamVoiceError::Crc { stored, computed });
		}

		let (steam_id, sections) = contents.split_at(STEAM_ID_BYTES);

		Ok(Self {
			steam_id: u64::from_le_bytes(steam_id.try_into().unwrap()),
			sections,
		})
	}

	/// The packet's sections, in order. Reading stops at the first that cannot
	/// be read.
	pub fn sections(self) -> impl Iterator<Item = Result<Section<'a>, SteamVoiceError>> {
		let mut rest = self.sections;

		std::iter::from_fn(move || {
			let (&kind, after) = rest.split_first()?;
			let section = read_u16(after).and_then(|(value, after)| {
				let (section, after) = match kind {
					SILENCE => (Section::Silence(value), after),
					SAMPLE_RATE => (Section::SampleRate(value), after),

					OPUS_PLC => {
						let frames = after
							.get(..value as usize)
							.ok_or(SteamVoiceError::Truncated)?;

						(Section::Opus(frames), &after[frames.len()..])
					}

					kind => return Err(SteamVoiceError::UnknownSection { kind }),
				};

				rest = after;
				Ok(section)
			});

			if section.is_err() {
				rest = &[];
			}

			Some(section)
		})
	}

	/// The Steam ID of the speaker the packet says it is from.
	pub const fn steam_id(self) -> u64 {
		self.steam_id
	}

	/// The packet as `steam_id` would have sent it: the same sections, with
	/// that Steam ID and their CRC.
	pub fn with_steam_id(self, steam_id: u64) -> Vec<u8> {
		packet(steam_id, self.sections)
	}
}

/// The standard CRC-32 (IEEE 802.3, reflected), as a packet ends with.
pub fn crc32(bytes: &[u8]) -> u32 {
	const POLYNOMIAL: u32 = 0xEDB8_8320;

	let crc = bytes.iter().fold(u32::MAX, |crc, &byte| {
		(0..8).fold(crc ^ u32::from(byte), |crc, _| match crc & 1 {
			0 => crc >> 1,
			_ => (crc >> 1) ^ POLYNOMIAL,
		})
	});

	!crc
}

/// Encodes a packet of `sections` from the speaker with `steam_id`.
///
/// Fails if an Opus section holds more than 65535 bytes, which its length
/// cannot say.
pub fn encode(steam_id: u64, sections: &[Section<'_>]) -> Result<Vec<u8>, SteamVoiceError> {
	let mut body = Vec::new();

	for section in sections {
		let (kind, value, data) = match *section {
			Section::Silence(samples) => (SILENCE, samples, &[][..]),
			Section::SampleRate(rate) => (SAMPLE_RATE, rate, &[][..]),

			Section::Opus(frames) => {
				let len = u16::try_from(frames.len())
					.map_err(|_| SteamVoiceError::TooLong { len: frames.len() })?;

				(OPUS_PLC, len, frames)
			}
		};

		body.push(kind);
		body.extend_from_slice(&value.to_le_bytes());
		body.extend_from_slice(data);
	}

	Ok(packet(steam_id, &body))
}

/// Encodes Opus frames as a [`Section::Opus`] holds them.
///
/// Fails if a frame holds 65535 bytes or more, which its length cannot say.
pub fn encode_opus_frames(frames: &[OpusFrame<'_>]) -> Result<Vec<u8>, SteamVoiceError> {
	let mut out = Vec::new();

	for frame in frames {
		let (len, sequence, data) = match *frame {
			OpusFrame::Reset { sequence } => (RESET, sequence, &[][..]),

			OpusFrame::Audio { sequence, data } => {
				let len = u16::try_from(data.len())
					.ok()
					.filter(|&len| len != RESET)
					.ok_or(SteamVoiceError::TooLong { len: data.len() })?;

				(len, sequence, data)
			}
		};

		out.extend_from_slice(&len.to_le_bytes());
		out.extend_from_slice(&sequence.to_le_bytes());
		out.extend_from_slice(data);
	}

	Ok(out)
}

/// The frames of a [`Section::Opus`], in order. Reading stops at the first
/// that is cut short.
pub fn opus_frames(
	mut frames: &[u8],
) -> impl Iterator<Item = Result<OpusFrame<'_>, SteamVoiceError>> {
	std::iter::from_fn(move || {
		if frames.is_empty() {
			return None;
		}

		let frame = read_u16(frames).and_then(|(len, after)| {
			let (sequence, after) = read_u16(after)?;

			if len == RESET {
				frames = after;
				return Ok(OpusFrame::Reset { sequence });
			}

			let data = after
				.get(..len as usize)
				.ok_or(SteamVoiceError::Truncated)?;

			frames = &after[data.len()..];
			Ok(OpusFrame::Audio { sequence, data })
		});

		if frame.is_err() {
			frames = &[];
		}

		Some(frame)
	})
}

/// A packet of the Steam ID, `body`, and their CRC.
fn packet(steam_id: u64, body: &[u8]) -> Vec<u8> {
	let mut packet = Vec::with_capacity(STEAM_ID_BYTES + body.len() + CRC_BYTES);

	packet.extend_from_slice(&steam_id.to_le_bytes());
	packet.extend_from_slice(body);
	packet.extend_from_slice(&crc32(&packet).to_le_bytes());
	packet
}

/// A little-endian `u16`, and the bytes after it.
fn read_u16(bytes: &[u8]) -> Result<(u16, &[u8]), SteamVoiceError> {
	let (value, rest) = bytes
		.split_first_chunk::<2>()
		.ok_or(SteamVoiceError::Truncated)?;

	Ok((u16::from_le_bytes(*value), rest))
}
