//! Resolves the actual Short Circuit zap method from witnessed native code.
use crate::{
	Server,
	raw::util::{Image, SignaturePattern},
};

use std::{ffi::c_void, ptr::NonNull};

/// A native `void CheckForPlayers(int)` address in the current game module.
#[derive(Debug, Clone, Copy)]
pub struct MechanicalOrbZap {
	address: NonNull<c_void>,
}

impl MechanicalOrbZap {
	/// Validates independently inspected reader/call/writer code once at load.
	/// Unknown or patched builds fail closed; no private field is guessed.
	pub fn load(server: Server<'_>) -> Result<Self, OrbError> {
		// SAFETY: The callback keeps the factory's game module loaded.
		let image = unsafe { Image::load(server.game_server_factory().as_raw() as usize) }
			.map_err(|_| OrbError::Snapshot)?;
		let (reader, writer, call_offset, writer_offset) = if cfg!(target_os = "windows") {
			(
				"40534883ec20488b0593746800488bd9f30f1081780600000f2f400c770aba02000000e848f5ffff",
				"488b05a17a68004c8bb424880100004c8bac2490010000488bb42498010000f30f10400cf30f5805acbe1b00f30f118778060000",
				35,
				0x488,
			)
		} else {
			(
				"554889e541544989fc53488d1d9fb0b000488b03f30f10400c0f2f87a8060000733e",
				"488d055fbab000f30f10052fe53c00488b00f30f58400cf30f1183a8060000",
				0x65,
				0x52a,
			)
		};
		let address = resolve(&image, reader, writer, call_offset, writer_offset)?;
		Ok(Self {
			address: NonNull::new(address as *mut c_void).ok_or(OrbError::Call)?,
		})
	}

	/// Address of the validated native method. Its ABI is
	/// `unsafe extern "C" fn(*mut CBaseEntity, c_int)` on supported x64 targets.
	/// It is valid only while this callback's game module remains loaded.
	pub const fn address(self) -> NonNull<c_void> {
		self.address
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OrbError {
	#[error("could not snapshot the current native orb module")]
	Snapshot,
	#[error("native orb reader witness is absent or ambiguous in executable code")]
	Reader,
	#[error("native orb writer witness is absent or ambiguous in executable code")]
	Writer,
	#[error("native orb reader does not call an executable zap method")]
	Call,
	#[error("native orb writer does not belong to the witnessed zap method")]
	Relation,
}

fn resolve(
	image: &Image,
	reader: &str,
	writer: &str,
	call_offset: usize,
	writer_offset: usize,
) -> Result<usize, OrbError> {
	let unique = |text: &str| {
		let signature: Vec<_> = text
			.as_bytes()
			.as_chunks::<2>()
			.0
			.iter()
			.map(|pair| {
				// EXPECT: The independently verified native literals are ASCII hex.
				SignaturePattern::Exact(
					u8::from_str_radix(std::str::from_utf8(pair).expect("ASCII witness"), 16)
						.expect("hex witness"),
				)
			})
			.collect();
		// The image's exact-byte matches() searches DATA for RTTI strings.
		// unique() is the existing executable-region scanner required here.
		image.unique(&signature, 1)
	};
	let reader = unique(reader).ok_or(OrbError::Reader)?;
	let writer = unique(writer).ok_or(OrbError::Writer)?;
	let address = image
		.call(reader.checked_add(call_offset).ok_or(OrbError::Call)?)
		.ok_or(OrbError::Call)?;
	if address.checked_add(writer_offset) != Some(writer) {
		return Err(OrbError::Relation);
	}
	Ok(address)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::raw::util::Section;

	#[test]
	fn native_call_and_writer_must_agree_in_code_even_when_data_contains_the_same_witness() {
		let reader = [0x90, 0xe8, 0x1a, 0, 0, 0];
		let mut code = vec![0xcc; 0x80];
		code[..reader.len()].copy_from_slice(&reader);
		code[0x40..0x42].copy_from_slice(&[0x12, 0x34]);
		let mut image = Image {
			base: 0x1000,
			sections: vec![
				Section {
					address: 0x1000,
					bytes: code,
					executable: true,
					writable: false,
				},
				Section {
					address: 0x2000,
					bytes: reader.to_vec(),
					executable: false,
					writable: false,
				},
			],
		};
		// Reproduces the incorrect scanner choice: it returns only the data copy.
		assert_eq!(image.matches(&reader, 1), [0x2000]);
		assert_eq!(resolve(&image, "90e81a000000", "1234", 1, 0x20), Ok(0x1020));
		assert_eq!(
			resolve(&image, "90e81a000000", "1234", 1, 0x21),
			Err(OrbError::Relation)
		);
		image.sections[0].bytes[0x50..0x56].copy_from_slice(&reader);
		assert_eq!(
			resolve(&image, "90e81a000000", "1234", 1, 0x20),
			Err(OrbError::Reader)
		);
	}
}
