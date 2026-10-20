//! Shortstop's native delayed shove scheduling. The private deadline is not
//! networked or datamapped; accepting its layout requires a native-binary witness.
use super::Weapon;
use crate::raw::{tf2::virtuals::ITEM_POST_FRAME_SLOT, util::Image};
use crate::{Server, entities::Entity, sys};

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ShortstopError {
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

	/// Inspects the current module's RTTI-selected native method before
	/// accepting its private field. Unknown builds and replaced methods fail
	/// closed. Call once while loading, before installing weapon hooks.
	pub fn load(server: Server<'_>) -> Result<Self, ShortstopError> {
		// SAFETY: The callback keeps the module containing the server factory
		// loaded while its executable/readable sections are copied.
		let image = unsafe { Image::load(server.game_server_factory().as_raw() as usize) }
			.map_err(|_| ShortstopError::Layout)?;
		let table = image
			.primary_vtable("CTFPistol_ScoutPrimary", ITEM_POST_FRAME_SLOT)
			.ok_or(ShortstopError::Layout)?;
		let entry = image
			.read(
				table + ITEM_POST_FRAME_SLOT * size_of::<usize>(),
				size_of::<usize>(),
			)
			.ok_or(ShortstopError::Layout)?;
		let method = usize::from_ne_bytes(entry.try_into().map_err(|_| ShortstopError::Layout)?);
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
		let expected: Vec<u8> = prefix
			.as_bytes()
			.chunks_exact(2)
			.map(|pair| {
				// EXPECT: Both native witness literals contain ASCII hexadecimal pairs.
				u8::from_str_radix(std::str::from_utf8(pair).expect("ASCII witness"), 16)
					.expect("hex witness")
			})
			.collect();
		if image.read(method, expected.len()) != Some(expected.as_slice())
			|| native_offset != Self::generated_offset()
		{
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
