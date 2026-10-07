//! An entity's solid flags (`m_usSolidFlags`), which its collision property,
//! `m_Collision`, holds, and which tell the engine's traces and the game's
//! movement how the entity collides.
//!
//! The flags are found through the entity's data description maps:
//! `CBaseEntity`'s embeds the collision property, whose own map declares the
//! flags.
//!
//! [`Entity::solid_flags`] reads them all, but only two can be changed here:
//! the custom ray test, by [`Entity::set_custom_ray_test`], which only decides
//! what the engine's ray traces test against the entity, and a trigger's
//! touches of debris, by [`Entity::set_trigger_touch_debris`], which only
//! decides whether the engine pairs it with debris. When most other flags
//! change, the game also updates the entity's collision rules, bounds, or
//! touches (`CCollisionProperty::SetSolidFlags`), which writing the member
//! would skip.

#[cfg(test)]
#[path = "../tests/entities/solid.rs"]
mod tests;

use crate::entities::Entity;
use crate::server::{InterfaceError, Server};

use sdk_raw::entities::{
	FSOLID_CUSTOMBOXTEST, FSOLID_CUSTOMRAYTEST, FSOLID_FORCE_WORLD_ALIGNED, FSOLID_NOT_SOLID,
	FSOLID_NOT_STANDABLE, FSOLID_ROOT_PARENT_ALIGNED, FSOLID_TRIGGER, FSOLID_TRIGGER_TOUCH_DEBRIS,
	FSOLID_USE_TRIGGER_BOUNDS, FSOLID_VOLUME_CONTENTS, find_solid_flags_field,
};

use std::sync::OnceLock;

/// The offset of `m_usSolidFlags` in every entity, once found.
static SOLID_FLAGS_OFFSET: OnceLock<usize> = OnceLock::new();

bitflags::bitflags! {
	/// An entity's solid flags (`SolidFlags_t`), the `FSOLID_*` values from
	/// `public/const.h`.
	///
	/// Flags read from an entity keep every bit, including those without a
	/// constant here.
	#[doc(alias("SolidFlags_t", "m_usSolidFlags"))]
	#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
	pub struct SolidFlags: u16 {
		/// `FSOLID_CUSTOMBOXTEST`: the engine's swept box traces against the
		/// entity ask its `TestCollision`, whatever its solid type.
		#[doc(alias("FSOLID_CUSTOMBOXTEST"))]
		const CUSTOM_BOX_TEST = FSOLID_CUSTOMBOXTEST;

		/// `FSOLID_CUSTOMRAYTEST`: the engine's ray traces, lines and points,
		/// against the entity ask its `TestCollision`, whatever its solid
		/// type.
		#[doc(alias("FSOLID_CUSTOMRAYTEST"))]
		const CUSTOM_RAY_TEST = FSOLID_CUSTOMRAYTEST;

		/// `FSOLID_FORCE_WORLD_ALIGNED`: the entity collides as a
		/// world-aligned box, even with a `SOLID_BSP` or `SOLID_VPHYSICS`
		/// model.
		#[doc(alias("FSOLID_FORCE_WORLD_ALIGNED"))]
		const FORCE_WORLD_ALIGNED = FSOLID_FORCE_WORLD_ALIGNED;

		/// `FSOLID_NOT_SOLID`: the entity is not solid.
		#[doc(alias("FSOLID_NOT_SOLID"))]
		const NOT_SOLID = FSOLID_NOT_SOLID;

		/// `FSOLID_NOT_STANDABLE`: nothing can stand on the entity.
		#[doc(alias("FSOLID_NOT_STANDABLE"))]
		const NOT_STANDABLE = FSOLID_NOT_STANDABLE;

		/// `FSOLID_ROOT_PARENT_ALIGNED`: the entity's collisions are in its
		/// root parent's local space.
		#[doc(alias("FSOLID_ROOT_PARENT_ALIGNED"))]
		const ROOT_PARENT_ALIGNED = FSOLID_ROOT_PARENT_ALIGNED;

		/// `FSOLID_TRIGGER`: the entity runs touch functions, as triggers do.
		#[doc(alias("FSOLID_TRIGGER"))]
		const TRIGGER = FSOLID_TRIGGER;

		/// `FSOLID_TRIGGER_TOUCH_DEBRIS`: the trigger touches debris.
		#[doc(alias("FSOLID_TRIGGER_TOUCH_DEBRIS"))]
		const TRIGGER_TOUCH_DEBRIS = FSOLID_TRIGGER_TOUCH_DEBRIS;

		/// `FSOLID_USE_TRIGGER_BOUNDS`: the entity has trigger bounds of its
		/// own, apart from its box.
		#[doc(alias("FSOLID_USE_TRIGGER_BOUNDS"))]
		const USE_TRIGGER_BOUNDS = FSOLID_USE_TRIGGER_BOUNDS;

		/// `FSOLID_VOLUME_CONTENTS`: the entity has contents throughout its
		/// volume, as water does.
		#[doc(alias("FSOLID_VOLUME_CONTENTS"))]
		const VOLUME_CONTENTS = FSOLID_VOLUME_CONTENTS;

		// Bits without a constant here, which the game may set.
		const _ = !0;
	}
}

/// An entity's solid flags could not be accessed.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SolidFlagsError {
	/// `CBaseEntity`'s datamap does not embed a collision property whose map
	/// declares `m_usSolidFlags` as the SDK does, at a plausible offset, so the
	/// game DLL does not match the SDK.
	#[error("the game's datamaps do not declare `m_Collision.m_usSolidFlags` as the SDK does")]
	UnsupportedLayout,

	/// The game server does not export `IVEngineServer`, through which the
	/// change is recorded for networking.
	#[error(transparent)]
	Interface(#[from] InterfaceError),
}

impl Entity<'_> {
	/// Sets or clears the entity's [`SolidFlags::CUSTOM_RAY_TEST`], and
	/// records the change for networking. Returns whether it was set.
	///
	/// While it is set, the engine's ray traces against the entity, such as
	/// a trigger's `PointIsWithin` and `ClipRayToEntity`, ask the entity's
	/// `TestCollision` instead of its model. `CBaseEntity`'s reports no hit,
	/// so for the classes that keep it, which include every brush entity,
	/// those traces miss the entity. Swept box traces, and the game's
	/// movement and touches, are unchanged.
	///
	/// Setting it on an entity that sets it itself, such as a ragdoll or a
	/// prop with hitboxes, and clearing it there, changes how traces hit it.
	#[doc(alias("FSOLID_CUSTOMRAYTEST", "m_usSolidFlags"))]
	pub fn set_custom_ray_test(
		self,
		server: Server<'_>,
		enabled: bool,
	) -> Result<bool, SolidFlagsError> {
		self.set_solid_flag(server, SolidFlags::CUSTOM_RAY_TEST, enabled)
	}

	/// Sets or clears `flag` alone in the entity's solid flags, and records the
	/// change for networking. Returns whether it was set.
	fn set_solid_flag(
		self,
		server: Server<'_>,
		flag: SolidFlags,
		enabled: bool,
	) -> Result<bool, SolidFlagsError> {
		let offset = self.solid_flags_offset()?;
		let engine = server.valve_engine()?;
		let flags = self.solid_flags_at(offset);
		let mut changed = flags;

		changed.set(flag, enabled);

		if changed != flags {
			// SAFETY: The offset was validated against the entity's datamaps,
			// which every entity shares through its `CBaseEntity` base, for an
			// aligned `unsigned short`. The member is written without forming a
			// reference, as the game writes it too, on the main thread.
			unsafe {
				self.as_ptr()
					.byte_add(offset)
					.cast::<u16>()
					.write(changed.bits())
			};

			self.network_state_changed(engine, offset);
		}

		Ok(flags.contains(flag))
	}

	/// Sets or clears the entity's [`SolidFlags::TRIGGER_TOUCH_DEBRIS`], and
	/// records the change for networking. Returns whether it was set.
	///
	/// Entities of
	/// [`CollisionGroup::Debris`](crate::entities::CollisionGroup::Debris)
	/// touch only the triggers that have it
	/// (`CCollisionProperty::ShouldTouchTrigger`). While a trigger, an entity
	/// with [`SolidFlags::TRIGGER`], has it, the engine also pairs the trigger
	/// with the debris it overlaps, as either moves, and runs each one's
	/// `Touch` with the other: whatever the trigger's class does to what
	/// touches it, it does to debris too. Clearing it ends those touches as the
	/// engine next checks them.
	///
	/// Nothing else changes: the trigger collides with nothing more, and the
	/// game updates nothing else for the flag
	/// (`CCollisionProperty::SetSolidFlags`).
	#[doc(alias("FSOLID_TRIGGER_TOUCH_DEBRIS", "m_usSolidFlags"))]
	pub fn set_trigger_touch_debris(
		self,
		server: Server<'_>,
		enabled: bool,
	) -> Result<bool, SolidFlagsError> {
		self.set_solid_flag(server, SolidFlags::TRIGGER_TOUCH_DEBRIS, enabled)
	}

	/// The entity's solid flags (`m_usSolidFlags`), as
	/// `CCollisionProperty::GetSolidFlags` returns them.
	#[doc(alias("GetSolidFlags", "m_usSolidFlags"))]
	pub fn solid_flags(self) -> Result<SolidFlags, SolidFlagsError> {
		Ok(self.solid_flags_at(self.solid_flags_offset()?))
	}

	/// Reads the solid flags at `offset`, which
	/// [`solid_flags_offset`](Self::solid_flags_offset) found.
	fn solid_flags_at(self, offset: usize) -> SolidFlags {
		// SAFETY: As for the write in `set_solid_flag`.
		SolidFlags::from_bits_retain(unsafe { self.as_ptr().byte_add(offset).cast::<u16>().read() })
	}

	/// The offset of `m_usSolidFlags`, found through the entity's datamaps.
	///
	/// Only a found offset is kept, so an entity of a class whose maps lack
	/// `CBaseEntity`'s, which no game has, does not hide it from others.
	fn solid_flags_offset(self) -> Result<usize, SolidFlagsError> {
		if let Some(&offset) = SOLID_FLAGS_OFFSET.get() {
			return Ok(offset);
		}

		let offset =
			find_solid_flags_field(self.data_maps()).ok_or(SolidFlagsError::UnsupportedLayout)?;

		Ok(*SOLID_FLAGS_OFFSET.get_or_init(|| offset))
	}
}
