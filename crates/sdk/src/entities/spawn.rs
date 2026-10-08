//! Creating entities as maps and `ent_create` do: [`EntitySpawn`] creates an
//! entity of a class, gives it key values, spawns it and activates it.

#[cfg(test)]
#[path = "../tests/entities/spawn.rs"]
mod tests;

use crate::entities::Entity;
use crate::interfaces::ServerTools;
use crate::math::{QAngle, Vector};
use std::ffi::{CStr, CString};

/// An entity to create: its class, and the key values it spawns with, as a
/// map's entity lump or `ent_create` gives them.
///
/// ```no_run
/// # use source_sdk_2013::entities::spawn::EntitySpawn;
/// # use source_sdk_2013::interfaces::ServerTools;
/// # use source_sdk_2013::math::Vector;
/// # fn example(tools: ServerTools<'_>) {
/// let spawn = EntitySpawn::new(c"prop_dynamic")
///     .model(c"models/props_gameplay/resupply_locker.mdl")
///     .origin(Vector::new(0.0, 0.0, 64.0))
///     .key(c"solid", c"6");
///
/// // SAFETY: Neither `CDynamicProp`'s constructor, nor its `Spawn` or
/// // `Activate`, frees entities other than through deferred deletion.
/// let prop = unsafe { spawn.spawn(tools) };
/// # }
/// ```
#[doc(alias("CreateEntityByName", "DispatchSpawn", "ent_create"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntitySpawn {
	activate: bool,
	class: CString,
	keys: Vec<(CString, CString)>,
	non_finite: Option<&'static CStr>,
}

impl EntitySpawn {
	/// An entity of the class `class`, such as `prop_dynamic`, with no key
	/// values, which [`Self::spawn`] activates.
	pub fn new(class: &CStr) -> Self {
		Self {
			activate: true,
			class: class.to_owned(),
			keys: Vec::new(),
			non_finite: None,
		}
	}

	/// Whether [`Self::spawn`] activates the entity once it has spawned, as
	/// the game activates a map's entities, and `ent_create` the entity it
	/// creates. On by default.
	///
	/// Without it, the entity skips what [`Entity::activate`] does, such as
	/// finding the entities it refers to by name.
	pub fn activate(mut self, activate: bool) -> Self {
		self.activate = activate;
		self
	}

	/// Sets the `angles` key: the entity's pitch, yaw and roll, in degrees.
	///
	/// [`Self::spawn`] fails if any is not finite.
	pub fn angles(self, angles: QAngle) -> Self {
		self.vector_key(c"angles", [angles.pitch, angles.yaw, angles.roll])
	}

	/// The class the entity is created as.
	pub fn class(&self) -> &CStr {
		&self.class
	}

	/// Sets the key value `key` to `value`, as the map's entity lump would.
	/// Keys are set in the order they are given, so a later value of a key
	/// replaces an earlier one.
	pub fn key(mut self, key: &CStr, value: &CStr) -> Self {
		self.keys.push((key.to_owned(), value.to_owned()));
		self
	}

	/// The key values the entity spawns with, in the order they are set.
	pub fn keys(&self) -> impl Iterator<Item = (&CStr, &CStr)> {
		self.keys
			.iter()
			.map(|(key, value)| (key.as_c_str(), value.as_c_str()))
	}

	/// Sets the `model` key, which most classes precache and set as they
	/// spawn.
	pub fn model(self, model: &CStr) -> Self {
		self.key(c"model", model)
	}

	/// Sets the `targetname` key, the entity's name, by which inputs and
	/// outputs find it.
	pub fn name(self, name: &CStr) -> Self {
		self.key(c"targetname", name)
	}

	/// Sets the `origin` key: where the entity is, relative to its parent if
	/// it has one.
	///
	/// [`Self::spawn`] fails if a coordinate is not finite.
	pub fn origin(self, origin: Vector) -> Self {
		self.vector_key(c"origin", [origin.x, origin.y, origin.z])
	}

	/// Creates the entity, sets its key values, spawns it, and activates it
	/// unless [`Self::activate`] said not to, as `ent_create` does.
	///
	/// Fails, leaving no entity behind, if the class is unknown, a value is
	/// not finite, or the entity refuses a key, which
	/// [`ServerTools::set_key_value`] describes; and fails with
	/// [`SpawnError::RemovedItself`] if the entity marked itself for deletion
	/// as it spawned or activated, which some classes do when a key value is
	/// missing or wrong.
	///
	/// # Safety
	///
	/// The class's constructor, `Spawn`, and, unless it is not activated,
	/// `Activate`, must free entities only through Source's deferred deletion
	/// (condition 4 of [`Server::new`]), as [`ServerTools::create_entity_by_name`],
	/// [`ServerTools::dispatch_spawn`] and [`Entity::activate`] require.
	///
	/// [`Server::new`]: crate::Server::new
	pub unsafe fn spawn<'s>(&self, tools: ServerTools<'s>) -> Result<Entity<'s>, SpawnError> {
		if let Some(key) = self.non_finite {
			return Err(SpawnError::NonFinite {
				key: key.to_owned(),
			});
		}

		// SAFETY: The caller vouches for the constructor.
		let entity = unsafe { tools.create_entity_by_name(&self.class) }.ok_or_else(|| {
			SpawnError::UnknownClass {
				class: self.class.clone(),
			}
		})?;

		for (key, value) in &self.keys {
			if !tools.set_key_value(entity, key, value) {
				// No protected entity is new and unspawned.
				let _ = tools.remove(entity);

				return Err(SpawnError::KeyRejected { key: key.clone() });
			}
		}

		// SAFETY: The entity is new, and the caller vouches for its `Spawn`.
		unsafe { tools.dispatch_spawn(entity) };

		if entity.is_marked_for_deletion() {
			return Err(SpawnError::RemovedItself);
		}

		if self.activate {
			// SAFETY: The entity has just spawned, and the caller vouches for
			// its `Activate`.
			unsafe { entity.activate() };

			if entity.is_marked_for_deletion() {
				return Err(SpawnError::RemovedItself);
			}
		}

		Ok(entity)
	}

	/// Sets `key` to three floats, as `UTIL_StringToVector` reads them, or
	/// notes it to fail the spawn if one is not finite.
	fn vector_key(self, key: &'static CStr, [x, y, z]: [f32; 3]) -> Self {
		if !(x.is_finite() && y.is_finite() && z.is_finite()) {
			return Self {
				non_finite: Some(key),
				..self
			};
		}

		let value = CString::new(format!("{x} {y} {z}")).expect("floats have no NUL");

		self.key(key, &value)
	}
}

/// Why [`EntitySpawn::spawn`] created no entity, or one that removed itself.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SpawnError {
	/// The entity refused a key value, and was removed.
	#[error("the entity refused its {key:?} key")]
	KeyRejected {
		/// The key the entity refused.
		key: CString,
	},

	/// A value given as floats has one that is not finite.
	#[error("the {key:?} key has a value that is not finite")]
	NonFinite {
		/// The key whose value is not finite.
		key: CString,
	},

	/// The entity marked itself for deletion as it spawned or activated, and
	/// the engine frees it at the end of the frame.
	#[error("the entity removed itself as it spawned")]
	RemovedItself,

	/// The game has no factory for the class.
	#[error("the game has no entity class {class:?}")]
	UnknownClass {
		/// The class asked for.
		class: CString,
	},
}

impl<'s> Entity<'s> {
	/// Activates the entity, as the game does for each of a level's entities
	/// once they have all spawned, and `ent_create` for the entity it creates
	/// (`CBaseEntity::Activate`). [`EntitySpawn::spawn`] does it for the
	/// entities it creates.
	///
	/// The base class's changes the entity to its initial team, and finds its
	/// damage filter by name; derived classes find the entities they refer to
	/// by name, and start what they do.
	///
	/// # Safety
	///
	/// The entity must have spawned, and not been activated since. Everything
	/// its `Activate` runs must free entities only through Source's deferred
	/// deletion (condition 4 of [`Server::new`]).
	///
	/// [`Server::new`]: crate::Server::new
	#[doc(alias("Activate", "ActivateEntity"))]
	pub unsafe fn activate(self) {
		// SAFETY: The entity is live on the main thread, and the caller vouches
		// for the rest.
		unsafe { sdk_raw::entities::spawn::activate(self.as_ptr()) };
	}
}
