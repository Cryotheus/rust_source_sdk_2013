//! Entity factories: the objects the game creates entities with, one per class
//! name it links to an entity class (`LINK_ENTITY_TO_CLASS`).
//!
//! [`ServerTools::entity_factory`] finds a class name's factory in the game's
//! entity factory dictionary. The game creates every entity it creates by name
//! through its class name's factory: map entities, `CreateEntityByName`,
//! `ent_create`, scripts' `SpawnEntityFromTable` and `CreateByClassname`, and
//! the precaches of the classes it registers for precaching as each level
//! loads. Hooks on a factory, such as Metamod's entity factory hooks, see each
//! of those creations, and can refuse them.
//!
//! Factories are statics of the game DLL, as [`sdk_raw::entities::factory`]
//! describes, so they live as long as the server.

#[cfg(test)]
#[path = "../tests/entities/factory.rs"]
mod tests;

use crate::NotThreadSafe;
use crate::interfaces::ServerTools;
use sdk_raw::vcall;
use std::ffi::CStr;
use std::marker::PhantomData;
use std::ptr::NonNull;

/// The factory a class name's entities are created with (`IEntityFactory`),
/// from [`ServerTools::entity_factory`].
#[doc(alias("IEntityFactory"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EntityFactory<'s> {
	raw: NonNull<sys::IEntityFactory>,
	_scope: PhantomData<&'s ()>,
	_not_thread_safe: NotThreadSafe,
}

impl EntityFactory<'_> {
	/// Returns the factory pointer, for calls and hooks this crate does not
	/// wrap.
	pub const fn as_ptr(self) -> *mut sys::IEntityFactory {
		self.raw.as_ptr()
	}
}

impl<'s> ServerTools<'s> {
	/// The factory the game creates entities of `class_name` with, or `None` if
	/// the game links no entity class to the name. The game compares names
	/// without regard to case.
	#[doc(alias("GetEntityFactoryDictionary", "FindFactory"))]
	pub fn entity_factory(self, class_name: &CStr) -> Option<EntityFactory<'s>> {
		// SAFETY: `Server::new` guarantees the interface is live. The game's
		// dictionary is a static of the game DLL.
		let dictionary = NonNull::new(unsafe {
			vcall!(self.as_ptr() => IServerTools_GetEntityFactoryDictionary())
		})?;

		// SAFETY: The dictionary is live, and reads the NUL-terminated name only
		// during the call.
		let factory = unsafe {
			vcall!(dictionary.as_ptr() => IEntityFactoryDictionary_FindFactory(class_name.as_ptr()))
		};

		// Factories are statics of the game DLL, which outlives `'s`.
		NonNull::new(factory).map(|raw| EntityFactory {
			raw,
			_scope: PhantomData,
			_not_thread_safe: PhantomData,
		})
	}
}
