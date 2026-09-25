//! `IServerGameEnts`, the game's conversions between entities and edicts.

use crate::edicts::Edict;
use crate::entities::Entity;
use crate::ffi::vcall;
use std::ptr::NonNull;

interface! {
	/// The game's conversions between entities and edicts (`IServerGameEnts`).
	#[doc(alias = "IServerGameEnts")]
	pub struct ServerGameEnts(sys::IServerGameEnts) = GameServer c"ServerGameEnts001";
}

impl<'s> ServerGameEnts<'s> {
	/// The edict of a networked entity.
	#[doc(alias = "BaseEntityToEdict")]
	pub fn edict_of_entity(self, entity: Entity<'_>) -> Option<Edict<'s>> {
		// SAFETY: `Server::new` guarantees the interface is live, and the entity is live.
		let edict =
			unsafe { vcall!(self.as_ptr() => IServerGameEnts_BaseEntityToEdict(entity.as_ptr())) };

		// SAFETY: The edict table outlives `'s`.
		NonNull::new(edict).map(|edict| unsafe { Edict::from_raw(edict) })
	}

	/// The entity occupying an edict.
	#[doc(alias = "EdictToBaseEntity")]
	pub fn entity_of_edict(self, edict: Edict<'_>) -> Option<Entity<'s>> {
		if edict.is_free() {
			return None;
		}

		// SAFETY: As for `edict_of_entity`, and the edict is occupied.
		let entity =
			unsafe { vcall!(self.as_ptr() => IServerGameEnts_EdictToBaseEntity(edict.as_ptr())) };

		// SAFETY: Entities are not freed immediately during `'s`.
		NonNull::new(entity).map(|entity| unsafe { Entity::from_raw(entity) })
	}
}
