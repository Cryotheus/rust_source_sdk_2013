//! Shortstop's native delayed shove scheduling. The private deadline is not
//! networked or datamapped; accepting its layout requires a native-binary witness.
use super::Weapon;

use crate::raw::{
	tf2::virtuals::ITEM_POST_FRAME_SLOT,
	util::{Image, SignaturePattern},
};

use crate::{Server, entities::Entity, sys};

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ShortstopError {
	#[error("could not snapshot the current native Shortstop module")]
	Snapshot,
	#[error("native Shortstop class RTTI is absent or ambiguous")]
	Class,
	#[error("native Shortstop shove witness is absent or ambiguous in executable code")]
	Witness,
	#[error("the supplied native shove deadline offset does not match the generated layout")]
	Layout,
	#[error("the entity is not a live Shortstop")]
	NotShortstop,
	#[error("the native shove deadline is not finite")]
	Deadline,
}

/// A checked layout witness for one platform's current game binary.
#[derive(Debug, Clone, Copy)]
pub struct ShortstopLayout {
	offset: usize,
}

impl ShortstopLayout {
	/// The generated header's offset, for comparison with native disassembly.
	pub const fn generated_offset() -> usize {
		std::mem::offset_of!(sys::CTFPistol_ScoutPrimary, m_flPushTime)
	}

	/// Validates the class RTTI and a unique native method witness before
	/// accepting its private field. A previously installed virtual hook may
	/// remain in the slot; the original code must still match exactly.
	/// Unknown builds fail closed. Call once while loading.
	pub fn load(server: Server<'_>) -> Result<Self, ShortstopError> {
		// SAFETY: The callback keeps the module containing the server factory
		// loaded while its executable/readable sections are copied.
		let image = unsafe { Image::load(server.game_server_factory().as_raw() as usize) }
			.map_err(|_| ShortstopError::Snapshot)?;
		image
			.primary_vtable("CTFPistol_ScoutPrimary", ITEM_POST_FRAME_SLOT)
			.ok_or(ShortstopError::Class)?;
		let (prefix, native_offset) = if cfg!(target_os = "windows") {
			(
				"40534883ec20f30f1081e0090000488bd90f2f0578321800761c488b059fb167000f2f400c730fe894000000c783e0090000000080bf",
				0x9e0,
			)
		} else {
			(
				"f30f1087080a00000f2f05f9403f007610488d05084fb500488b000f2f400c720fe95a540200662e0f1f840000000000554889e54883ec1048897df8e86ff6ffff488b7df8c787080a0000000080bf",
				0xa08,
			)
		};
		Self::witness(&image, prefix, native_offset)
	}

	fn witness(image: &Image, prefix: &str, native_offset: usize) -> Result<Self, ShortstopError> {
		let signature: Vec<_> = prefix
			.as_bytes()
			.chunks_exact(2)
			.map(|pair| {
				// EXPECT: Both native witness literals contain ASCII hexadecimal pairs.
				SignaturePattern::Exact(
					u8::from_str_radix(std::str::from_utf8(pair).expect("ASCII witness"), 16)
						.expect("hex witness"),
				)
			})
			.collect();
		// KHook retains virtual dispatchers after removing their callbacks.
		// Inspect the unique original executable code, not the mutable slot.
		// Class RTTI validation above still requires an executable slot entry.
		if image.unique(&signature, 1).is_none() {
			return Err(ShortstopError::Witness);
		}
		if native_offset != Self::generated_offset() {
			return Err(ShortstopError::Layout);
		}
		Ok(Self {
			offset: native_offset,
		})
	}

	/// Reads the native scheduled deadline, including -1 when no shove is due.
	pub fn deadline(self, server: Server<'_>, entity: Entity<'_>) -> Result<f32, ShortstopError> {
		if entity.class_name() != c"tf_weapon_handgun_scout_primary"
			|| Weapon::new(server, entity).is_err()
		{
			return Err(ShortstopError::NotShortstop);
		}
		// SAFETY: The witness independently validated this field in the running
		// binary. The callback-scoped entity is live, and the class and weapon
		// ancestry above reject other entities. No Rust reference is formed.
		let deadline = unsafe { entity.as_ptr().byte_add(self.offset).cast::<f32>().read() };
		if !deadline.is_finite() {
			return Err(ShortstopError::Deadline);
		}
		Ok(deadline)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::raw::util::Section;

	#[test]
	fn interposed_slot_preserves_native_layout_proof_but_unknown_or_ambiguous_code_does_not() {
		let native = [0xf3, 0x0f, 0x10, 0x87, 0x08, 0x0a, 0, 0];
		let mut image = Image {
			base: 0x1000,
			sections: vec![
				Section {
					address: 0x1000,
					bytes: native.to_vec(),
					executable: true,
					writable: false,
				},
				// A virtual slot now points at an executable hook dispatcher.
				Section {
					address: 0x2000,
					bytes: 0x3000usize.to_ne_bytes().to_vec(),
					executable: false,
					writable: false,
				},
				Section {
					address: 0x3000,
					bytes: vec![0xcc; 16],
					executable: true,
					writable: false,
				},
			],
		};
		assert_ne!(image.read(0x3000, native.len()), Some(native.as_slice()));
		let offset = ShortstopLayout::generated_offset();
		assert!(ShortstopLayout::witness(&image, "f30f1087080a0000", offset).is_ok());
		assert!(ShortstopLayout::witness(&image, "f30f1087080a0000", offset + 4).is_err());
		image.sections[0].bytes[0] = 0x90;
		assert!(ShortstopLayout::witness(&image, "f30f1087080a0000", offset).is_err());
		image.sections[0].bytes = native.repeat(2);
		assert!(ShortstopLayout::witness(&image, "f30f1087080a0000", offset).is_err());
	}
}
