//! Resolves the actual Short Circuit zap method from witnessed native code.
use crate::{Server, raw::util::Image};
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
			.map_err(|_| OrbError::Layout)?;
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
		let unique = |text: &str| {
			let bytes: Vec<u8> = text
				.as_bytes()
				.chunks_exact(2)
				.map(|pair| {
					// EXPECT: The independently verified native literals are ASCII hex.
					u8::from_str_radix(std::str::from_utf8(pair).expect("ASCII witness"), 16)
						.expect("hex witness")
				})
				.collect();
			let addresses: Vec<_> = image
				.matches(&bytes, 1)
				.into_iter()
				.filter(|address| image.executable(*address))
				.collect();
			(addresses.len() == 1).then(|| addresses[0])
		};
		let reader = unique(reader).ok_or(OrbError::Layout)?;
		let writer = unique(writer).ok_or(OrbError::Layout)?;
		let address = image.call(reader + call_offset).ok_or(OrbError::Layout)?;
		if writer != address + writer_offset {
			return Err(OrbError::Layout);
		}
		Ok(Self {
			address: NonNull::new(address as *mut c_void).ok_or(OrbError::Layout)?,
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
	#[error("the current native orb code does not match a verified zap method")]
	Layout,
}
