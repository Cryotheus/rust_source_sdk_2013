//! `ITempEntsSystem`, the game's temporary entities: effects sent to clients
//! once, which no entity on the server keeps, such as particle effects.
//!
//! [`TempEntities::dispatch_particle_effect`] plays a particle system on an
//! entity, as the game's `DispatchParticleEffect` does.

pub mod sparks;

#[cfg(test)]
#[path = "../../tests/interfaces/temp_entities.rs"]
mod tests;

use crate::NotThreadSafe;
use crate::entities::Entity;
use crate::user_messages::Recipients;

use sdk_raw::interfaces::temp_entities::{
	EffectData, MAX_PARTICLE_SYSTEMS, PARTICLE_DISPATCH_FROM_ENTITY,
	PARTICLE_DISPATCH_RESET_PARTICLES, PARTICLE_EFFECT, PATTACH_ABSORIGIN,
	PATTACH_ABSORIGIN_FOLLOW, PATTACH_POINT, PATTACH_POINT_FOLLOW, PATTACH_ROOTBONE_FOLLOW,
};

use sdk_raw::vcall;
use std::ffi::c_int;
use std::marker::PhantomData;
use std::ops::RangeInclusive;
use std::ptr::NonNull;

/// The model attachments clients receive a particle effect's attachment as:
/// the index is sent in five signed bits, and 0 names no attachment.
const ATTACHMENTS: RangeInclusive<u8> = 1..=15;

/// Where a particle effect played on an entity starts, and whether it moves
/// with the entity (`ParticleAttachment_t`).
#[doc(alias("ParticleAttachment_t"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParticleAttachment {
	/// At the entity's origin, where it stays (`PATTACH_ABSORIGIN`).
	#[doc(alias("PATTACH_ABSORIGIN"))]
	Origin,

	/// At the entity's origin, following it (`PATTACH_ABSORIGIN_FOLLOW`).
	#[doc(alias("PATTACH_ABSORIGIN_FOLLOW"))]
	OriginFollow,

	/// At the model attachment with the index, from 1 to 15, where it stays
	/// (`PATTACH_POINT`). Attachments are numbered from 1 in the order the
	/// model declares them.
	#[doc(alias("PATTACH_POINT"))]
	Point(u8),

	/// At the model attachment with the index, from 1 to 15, following it
	/// (`PATTACH_POINT_FOLLOW`).
	#[doc(alias("PATTACH_POINT_FOLLOW"))]
	PointFollow(u8),

	/// At the root bone of the entity's model, following it
	/// (`PATTACH_ROOTBONE_FOLLOW`).
	#[doc(alias("PATTACH_ROOTBONE_FOLLOW"))]
	RootBoneFollow,
}

impl ParticleAttachment {
	/// The `PATTACH_*` value clients receive, and the attachment, if any.
	const fn to_raw(self) -> (c_int, Option<u8>) {
		match self {
			Self::Origin => (PATTACH_ABSORIGIN, None),
			Self::OriginFollow => (PATTACH_ABSORIGIN_FOLLOW, None),
			Self::Point(point) => (PATTACH_POINT, Some(point)),
			Self::PointFollow(point) => (PATTACH_POINT_FOLLOW, Some(point)),
			Self::RootBoneFollow => (PATTACH_ROOTBONE_FOLLOW, None),
		}
	}
}

/// A particle system played on an entity, which
/// [`TempEntities::dispatch_particle_effect`] sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ParticleEffect<'a> {
	/// The particle system, by its index in the
	/// [`PARTICLE_EFFECT_NAMES`](crate::interfaces::network_string_tables::PARTICLE_EFFECT_NAMES)
	/// table, which
	/// [`NetworkStringTables::particle_system_index`](crate::interfaces::NetworkStringTables::particle_system_index)
	/// finds.
	pub system: usize,

	/// The entity it plays on, which must be networked.
	pub entity: Entity<'a>,

	/// Where on the entity it starts.
	pub attachment: ParticleAttachment,

	/// Whether the entity's other particle effects stop emitting first
	/// (`PARTICLE_DISPATCH_RESET_PARTICLES`).
	pub reset: bool,
}

impl<'a> ParticleEffect<'a> {
	/// The particle system on `entity`, starting at its origin and following
	/// it, as the game plays effects that cover a body, such as TF2's ash.
	pub const fn new(system: usize, entity: Entity<'a>) -> Self {
		Self {
			system,
			entity,
			attachment: ParticleAttachment::OriginFollow,
			reset: false,
		}
	}
}

/// Why a particle effect could not be dispatched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
pub enum ParticleEffectError {
	/// The attachment's index is outside 1 to 15, the indices clients
	/// receive.
	#[error("model attachment {0} is outside the 1 to 15 clients receive")]
	Attachment(u8),

	/// The entity has no edict, so no client knows it.
	#[error("the entity is not networked, so no client knows it")]
	NotNetworked,

	/// The particle system's index is beyond the table's 8192 systems, the
	/// indices clients receive.
	#[error("particle system {0} is beyond the 8192 clients receive")]
	SystemOutOfRange(usize),
}

/// The game's temporary entities (`ITempEntsSystem`), from
/// [`ServerTools::temp_entities`](crate::interfaces::ServerTools::temp_entities).
#[doc(alias("ITempEntsSystem"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TempEntities<'s> {
	raw: NonNull<sys::ITempEntsSystem>,
	_scope: PhantomData<&'s ()>,
	_not_thread_safe: NotThreadSafe,
}

impl<'s> TempEntities<'s> {
	/// Wraps the game's temporary entity system.
	///
	/// # Safety
	///
	/// `raw` must be the game's live `ITempEntsSystem`, alive for `'s`, and
	/// every call must happen on the server's main thread.
	pub(crate) const unsafe fn from_raw(raw: NonNull<sys::ITempEntsSystem>) -> Self {
		Self {
			raw,
			_scope: PhantomData,
			_not_thread_safe: PhantomData,
		}
	}

	/// Returns the interface pointer, for calls this crate does not wrap.
	pub const fn as_ptr(self) -> *mut sys::ITempEntsSystem {
		self.raw.as_ptr()
	}

	/// Plays a particle system on an entity for `recipients`, as the game's
	/// `DispatchParticleEffect` does, through the `ParticleEffect` effect.
	///
	/// Each client creates the effect on its own copy of the entity, so an
	/// effect that follows the entity, or spreads over its model's hitboxes,
	/// as TF2's `drg_fiery_death` does, plays on the entity as it would on the
	/// client's own. A client for which the entity is dormant, such as one
	/// outside its PVS, ignores the effect, and a client that connects later
	/// never receives it. The game sends effects that follow their entity to
	/// every client (`CBroadcastRecipientFilter`), and others to the clients
	/// that can hear the entity's origin (`CPASFilter`).
	///
	/// While the game runs a player's command, it leaves that player out of
	/// its effects, since their client predicts them. A plugin's effects are
	/// not predicted, and the game's filtering only works on its own recipient
	/// filters, so the dispatch turns it off for itself, as the game's
	/// `CDisablePredictionFiltering` does, and every recipient receives the
	/// effect.
	///
	/// The particle system must be one the level precached, whose index
	/// [`NetworkStringTables::particle_system_index`] finds; clients play
	/// nothing for an index the table does not hold. Fails if the entity has
	/// no edict, or an index is out of the range clients receive. Once the
	/// effect passes, an empty `recipients` returns `Ok` without dispatching
	/// it.
	///
	/// [`NetworkStringTables::particle_system_index`]: crate::interfaces::NetworkStringTables::particle_system_index
	#[doc(alias("DispatchParticleEffect", "DispatchEffect"))]
	pub fn dispatch_particle_effect(
		self,
		recipients: &Recipients,
		effect: &ParticleEffect<'_>,
	) -> Result<(), ParticleEffectError> {
		let ParticleEffect {
			system,
			entity,
			attachment,
			reset,
		} = *effect;

		let hit_box = c_int::try_from(system)
			.ok()
			.filter(|&system| system < MAX_PARTICLE_SYSTEMS)
			.ok_or(ParticleEffectError::SystemOutOfRange(system))?;

		let edict = entity.edict().ok_or(ParticleEffectError::NotNetworked)?;
		let (attach_type, point) = attachment.to_raw();

		if let Some(point) = point
			&& !ATTACHMENTS.contains(&point)
		{
			return Err(ParticleEffectError::Attachment(point));
		}

		if recipients.is_empty() {
			return Ok(());
		}

		let mut data = EffectData::new();

		data.origin = entity.position().map_or(data.origin, sys::Vector::from);
		data.flags = PARTICLE_DISPATCH_FROM_ENTITY;
		data.entity_index = edict.index();
		data.attachment_index = c_int::from(point.unwrap_or(0));
		data.damage_type = attach_type;
		data.hit_box = hit_box;

		if reset {
			data.flags |= PARTICLE_DISPATCH_RESET_PARTICLES;
		}

		let filter = recipients.filter();
		let this = self.as_ptr();

		// SAFETY: The temporary entity system is the game's live static, its
		// primary vtable laid out as the generated one, and its prediction
		// state is that of its base, `IPredictionSystem`, which only the main
		// thread touches. Pushing that state makes `SuppressTE` skip its cast
		// of the filter to the game's `CRecipientFilter`, which the Rust filter
		// is not, as `CDisablePredictionFiltering` does, and the pop restores
		// it. `Server::new` binds any code the call reaches, such as other
		// plugins' temporary entity hooks, to free entities only through
		// deferred deletion. The game reads the filter, origin, name, and data
		// during the call, and all of them outlive it.
		unsafe {
			let pushed = &raw mut (*this)._base.m_nStatusPushed;

			pushed.write(pushed.read() + 1);

			vcall!(this as sys::ITempEntsSystem__bindgen_vtable => ITempEntsSystem_DispatchEffect(
				filter.as_raw(),
				0.0,
				&raw const data.origin,
				PARTICLE_EFFECT.as_ptr(),
				data.as_raw(),
			));

			pushed.write(pushed.read() - 1);
		}

		Ok(())
	}
}
