//! What an entity is: its classes, its name and its parent's, its team, and
//! the spawn flags its map gave it.

#[cfg(test)]
#[path = "../tests/entities/identity.rs"]
mod tests;

use crate::entities::Entity;
use crate::entities::fields::{BaseField, FieldError, PooledString};
use crate::interfaces::ValveEngine;
use sdk_raw::util::cstr::borrow_cstr;
use std::ffi::{CStr, c_int};

/// `m_iParent`, the `parentname` key value.
static PARENT_NAME: BaseField<PooledString> =
	BaseField::new(c"m_iParent", sys::_fieldtypes_FIELD_STRING);

/// `m_spawnflags`, the `spawnflags` key value.
static SPAWN_FLAGS: BaseField<c_int> =
	BaseField::new(c"m_spawnflags", sys::_fieldtypes_FIELD_INTEGER);

/// `m_iTeamNum`, the `teamnumber` key value.
static TEAM: BaseField<c_int> = BaseField::new(c"m_iTeamNum", sys::_fieldtypes_FIELD_INTEGER);

impl<'s> Entity<'s> {
	/// Whether a datamap of the entity's class or one of its bases is named
	/// `class`, such as `CBaseAnimating` or `CTFPlayer`: whether the entity's
	/// class is `class` or derives from it, as far as datamaps tell.
	///
	/// Classes without a datamap of their own share their base's, so this
	/// never finds them.
	#[doc(alias("HasEntProp", "dynamic_cast"))]
	pub fn is_a(self, class: &CStr) -> bool {
		self.has_data_map_class(class)
	}

	/// The entity's name (`m_iName`), which the `targetname` key value sets,
	/// and by which inputs and outputs find it, or `None` if it has none.
	#[doc(alias("GetEntityName", "targetname", "m_iName"))]
	pub fn name(self) -> Option<&'s CStr> {
		let field = self.name_field()?;

		// SAFETY: The field is the entity's `m_iName`, read without forming a
		// reference. Its string is pooled, and pooled strings live until the
		// level ends, past `'s`.
		unsafe { borrow_cstr(field.read().pszValue) }.filter(|name| !name.is_empty())
	}

	/// The name of the entity the map parents this one to (`m_iParent`), the
	/// `parentname` key value, or `None` if it has none.
	///
	/// The game parents the entity as it activates, so this only says which
	/// entity it was asked to follow, not what [`Self::move_parent`] it has.
	#[doc(alias("parentname", "m_iParent"))]
	pub fn parent_name(self) -> Result<Option<&'s CStr>, FieldError> {
		let name = PARENT_NAME.read(self)?;

		// SAFETY: As for `name`.
		Ok(unsafe { borrow_cstr(name.0) }.filter(|name| !name.is_empty()))
	}

	/// Sets the entity's spawn flags (`m_spawnflags`).
	///
	/// Classes read their flags as they spawn, and some as they run, such as
	/// a trigger checking which entities may touch it.
	#[doc(alias("AddSpawnFlags", "RemoveSpawnFlags", "m_spawnflags"))]
	pub fn set_spawn_flags(self, engine: ValveEngine<'_>, flags: c_int) -> Result<(), FieldError> {
		SPAWN_FLAGS.write(engine, self, flags)
	}

	/// The entity's spawn flags (`m_spawnflags`), the `spawnflags` key value,
	/// whose meaning each class gives.
	#[doc(alias("GetSpawnFlags", "HasSpawnFlags", "m_spawnflags"))]
	pub fn spawn_flags(self) -> Result<c_int, FieldError> {
		SPAWN_FLAGS.read(self)
	}

	/// The number of the entity's team (`m_iTeamNum`), such as TF2's
	/// `TF_TEAM_RED`, or 0 for none.
	#[doc(alias("GetTeamNumber", "m_iTeamNum"))]
	pub fn team(self) -> Result<c_int, FieldError> {
		TEAM.read(self)
	}
}
