//! Members of entities that their classes' datamaps declare, read and written
//! by name and type, as SourceMod's `GetEntProp` and `SetEntProp` do with
//! `Prop_Data`.
//!
//! A class's datamap (`BEGIN_DATADESC`) declares the members the game saves,
//! reads from key values, or takes as inputs, each with its type, size and
//! offset. Unlike networked variables, which [`NetProp`] reads, they cover
//! server-only members, and members of entities no client is sent.
//!
//! The offsets are the game's own, so a member is only read or written as the
//! type its map declares: [`FieldValue`] says which types a Rust type stands
//! for. Writes record the change for networking, as the game's network
//! variables do, but run none of the game's code around them: prefer a
//! dedicated accessor, an input, or a method where one exists.
//!
//! [`NetProp`]: crate::datatables::NetProp

#[cfg(test)]
#[path = "../tests/entities/fields.rs"]
mod tests;

use crate::entities::{Entity, EntityHandle};
use crate::interfaces::ValveEngine;
use crate::math::{Color32, QAngle, Vector};
use sdk_raw::entities::datamap::{DataField, DataMaps};
use std::ffi::{CStr, CString, c_char, c_int};
use std::marker::PhantomData;
use std::sync::OnceLock;

/// The deepest nesting of embedded objects searched for a member, as
/// [`DataMaps::find_key_field`] searches them.
const MAX_EMBEDDING_DEPTH: usize = DataMaps::MAX_EMBEDDING_DEPTH;

/// A member `CBaseEntity`'s own datamap declares, read and written as `T`,
/// whose offset is found once and kept: every entity shares it through its
/// `CBaseEntity` base.
pub(super) struct BaseField<T> {
	name: &'static CStr,
	field_type: sys::fieldtype_t,
	offset: OnceLock<usize>,
	_value: PhantomData<fn() -> T>,
}

impl<T: FieldValue> BaseField<T> {
	/// The member `name`, declared as `field_type`, which must be one of
	/// `T`'s.
	pub(super) const fn new(name: &'static CStr, field_type: sys::fieldtype_t) -> Self {
		Self {
			name,
			field_type,
			offset: OnceLock::new(),
			_value: PhantomData,
		}
	}

	/// The member's offset, found through `entity`'s datamaps, under the
	/// limit and with the alignment [`find_base_entity_field`] checks.
	///
	/// Only a found offset is kept, so an entity of a class whose maps lack
	/// `CBaseEntity`'s, which no game has, does not hide it from others.
	///
	/// [`find_base_entity_field`]: sdk_raw::entities::find_base_entity_field
	pub(super) fn offset(&self, entity: Entity<'_>) -> Result<usize, FieldError> {
		debug_assert!(T::FIELD_TYPES.contains(&self.field_type));

		if let Some(&offset) = self.offset.get() {
			return Ok(offset);
		}

		let offset = entity
			.find_base_entity_field(self.name, self.field_type, size_of::<T::Stored>())
			.ok_or_else(|| FieldError::NotFound {
				name: self.name.to_owned(),
			})?;

		Ok(*self.offset.get_or_init(|| offset))
	}

	/// Reads the member of `entity`.
	pub(super) fn read(&self, entity: Entity<'_>) -> Result<T, FieldError> {
		let offset = self.offset(entity)?;

		// SAFETY: The offset was validated against `CBaseEntity`'s datamap,
		// which every entity shares through its base, for a member of `T`'s
		// stored type and size, aligned for it. The member is read without
		// forming a reference, as the game writes it too.
		let stored = unsafe { entity.as_ptr().byte_add(offset).cast::<T::Stored>().read() };

		Ok(T::from_stored(stored))
	}

	/// Writes the member of `entity`, and records the change for networking.
	pub(super) fn write(
		&self,
		engine: ValveEngine<'_>,
		entity: Entity<'_>,
		value: T,
	) -> Result<(), FieldError> {
		let offset = self.offset(entity)?;

		// SAFETY: As for `read`. The game writes the member the same way,
		// through its own pointers, on the main thread.
		unsafe {
			entity
				.as_ptr()
				.byte_add(offset)
				.cast::<T::Stored>()
				.write(value.into_stored())
		};

		entity.network_state_changed(engine, offset);
		Ok(())
	}
}

/// Why a datamap member could not be read or written.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FieldError {
	/// No datamap of the entity's class or its bases declares a member of
	/// this name.
	#[error("the entity's datamaps declare no member {name:?}")]
	NotFound {
		/// The member's name.
		name: CString,
	},

	/// The element lies past the end of the member's array.
	#[error("member {name:?} has {count} elements, so has no element {element}")]
	OutOfBounds {
		/// The member's name.
		name: CString,

		/// The number of elements the member has.
		count: usize,

		/// The element asked for.
		element: usize,
	},

	/// The member is of another type than the value, or its declared size or
	/// offset does not fit the value.
	#[error("member {name:?} is declared as field type {field_type}, not as the value's")]
	WrongType {
		/// The member's name.
		name: CString,

		/// The type the member is declared as (`fieldtype_t`).
		field_type: sys::fieldtype_t,
	},
}

/// A type a datamap member can be read and written as, by the member's
/// declared field types.
///
/// | Type | Field types |
/// | --- | --- |
/// | `bool` | `FIELD_BOOLEAN` |
/// | `u8` | `FIELD_CHARACTER` |
/// | `i16` | `FIELD_SHORT` |
/// | `c_int` | `FIELD_INTEGER`, `FIELD_TICK`, `FIELD_MODELINDEX`, `FIELD_MATERIALINDEX` |
/// | `f32` | `FIELD_FLOAT`, `FIELD_TIME` |
/// | [`Vector`] | `FIELD_VECTOR`, `FIELD_POSITION_VECTOR` |
/// | [`QAngle`] | `FIELD_VECTOR` |
/// | [`Color32`] | `FIELD_COLOR32` |
/// | [`EntityHandle`] | `FIELD_EHANDLE` |
pub trait FieldValue: sealed::Sealed + Copy + 'static {}

mod sealed {
	/// How a [`FieldValue`] is stored.
	pub trait Sealed: Sized {
		/// The value as the game stores it.
		type Stored: Copy;

		/// The field types the value is stored as.
		const FIELD_TYPES: &'static [sys::fieldtype_t];

		fn from_stored(stored: Self::Stored) -> Self;

		fn into_stored(self) -> Self::Stored;
	}
}

/// Implements [`FieldValue`] for types stored as themselves.
macro_rules! stored_as_is {
	($($ty:ty => [$($field_type:ident),+ $(,)?]),+ $(,)?) => {$(
		impl FieldValue for $ty {}

		impl sealed::Sealed for $ty {
			type Stored = Self;

			const FIELD_TYPES: &'static [sys::fieldtype_t] = &[$(sys::$field_type),+];

			fn from_stored(stored: Self) -> Self {
				stored
			}

			fn into_stored(self) -> Self {
				self
			}
		}
	)+};
}

stored_as_is!(
	bool => [_fieldtypes_FIELD_BOOLEAN],
	u8 => [_fieldtypes_FIELD_CHARACTER],
	i16 => [_fieldtypes_FIELD_SHORT],
	c_int => [
		_fieldtypes_FIELD_INTEGER,
		_fieldtypes_FIELD_TICK,
		_fieldtypes_FIELD_MODELINDEX,
		_fieldtypes_FIELD_MATERIALINDEX,
	],
	f32 => [_fieldtypes_FIELD_FLOAT, _fieldtypes_FIELD_TIME],
);

/// A `string_t` member's pooled string, by its address, which only the crate
/// reads, as its lifetime is the level's.
#[derive(Clone, Copy)]
pub(crate) struct PooledString(pub(crate) *const c_char);

impl FieldValue for PooledString {}

impl sealed::Sealed for PooledString {
	type Stored = sys::string_t;

	const FIELD_TYPES: &'static [sys::fieldtype_t] = &[
		sys::_fieldtypes_FIELD_STRING,
		sys::_fieldtypes_FIELD_MODELNAME,
		sys::_fieldtypes_FIELD_SOUNDNAME,
	];

	fn from_stored(stored: sys::string_t) -> Self {
		Self(stored.pszValue)
	}

	fn into_stored(self) -> sys::string_t {
		sys::string_t { pszValue: self.0 }
	}
}

impl FieldValue for Color32 {}

impl sealed::Sealed for Color32 {
	type Stored = sys::color32;

	const FIELD_TYPES: &'static [sys::fieldtype_t] = &[sys::_fieldtypes_FIELD_COLOR32];

	fn from_stored(stored: sys::color32) -> Self {
		stored.into()
	}

	fn into_stored(self) -> sys::color32 {
		self.into()
	}
}

impl<'s> Entity<'s> {
	/// Reads element `element` of the array member `name` that a datamap of
	/// the entity's class or its bases declares, as `T`, as
	/// [`Self::data_field`] reads a member.
	pub fn data_element<T: FieldValue>(self, name: &CStr, element: usize) -> Result<T, FieldError> {
		let offset = self.data_element_offset::<T>(name, element)?;

		// SAFETY: The entity is live, and its datamap declares a member of
		// `T`'s stored type at the offset, aligned for it. It is read without
		// forming a reference, as the game writes it through its own pointers.
		let stored = unsafe { self.as_ptr().byte_add(offset).cast::<T::Stored>().read() };

		Ok(T::from_stored(stored))
	}

	/// Finds the offset of element `element` of the member `name`, which must
	/// be declared as one of `T`'s field types, with elements of its size.
	fn data_element_offset<T: FieldValue>(
		self,
		name: &CStr,
		element: usize,
	) -> Result<usize, FieldError> {
		let (field, offset) =
			find_field(self.data_maps(), name, 0).ok_or_else(|| FieldError::NotFound {
				name: name.to_owned(),
			})?;

		let wrong_type = || FieldError::WrongType {
			name: name.to_owned(),
			field_type: field.fieldType,
		};

		let count = usize::from(field.fieldSize);
		let bytes = usize::try_from(field.fieldSizeInBytes).map_err(|_| wrong_type())?;
		let size = size_of::<T::Stored>();

		if !T::FIELD_TYPES.contains(&field.fieldType)
			|| count == 0
			|| bytes != size * count
			|| !offset.is_multiple_of(align_of::<T::Stored>())
		{
			return Err(wrong_type());
		}

		if element >= count {
			return Err(FieldError::OutOfBounds {
				name: name.to_owned(),
				count,
				element,
			});
		}

		Ok(offset + element * size)
	}

	/// Reads the member `name` that a datamap of the entity's class or its
	/// bases declares, as `T`. An array's first element is read.
	///
	/// Fails if no map declares such a member, or declares it as another type
	/// than `T` stands for, as [`FieldValue`] lists.
	#[doc(alias("GetEntProp", "GetEntPropFloat", "GetEntPropEnt", "GetEntPropVector"))]
	pub fn data_field<T: FieldValue>(self, name: &CStr) -> Result<T, FieldError> {
		self.data_element(name, 0)
	}

	/// Writes element `element` of the array member `name` that a datamap of
	/// the entity's class or its bases declares, as [`Self::set_data_field`]
	/// writes a member.
	pub fn set_data_element<T: FieldValue>(
		self,
		engine: ValveEngine<'_>,
		name: &CStr,
		element: usize,
		value: T,
	) -> Result<(), FieldError> {
		let offset = self.data_element_offset::<T>(name, element)?;

		// SAFETY: As for `data_element`. The game writes the member the same
		// way, on the main thread.
		unsafe {
			self.as_ptr()
				.byte_add(offset)
				.cast::<T::Stored>()
				.write(value.into_stored())
		};

		self.network_state_changed(engine, offset);
		Ok(())
	}

	/// Writes the member `name` that a datamap of the entity's class or its
	/// bases declares, as `T`, and records the change for networking. An
	/// array's first element is written.
	///
	/// Nothing else the game does when it changes the member runs, such as
	/// the transmit state update `CBaseEntity::AddEffects` does for
	/// `EF_NODRAW`, or the collision update of `SetSolidFlags`: prefer a
	/// dedicated accessor, an input, or a method where one exists. Fails as
	/// [`Self::data_field`] does.
	#[doc(alias("SetEntProp", "SetEntPropFloat", "SetEntPropEnt", "SetEntPropVector"))]
	pub fn set_data_field<T: FieldValue>(
		self,
		engine: ValveEngine<'_>,
		name: &CStr,
		value: T,
	) -> Result<(), FieldError> {
		self.set_data_element(engine, name, 0, value)
	}
}

impl FieldValue for EntityHandle {}

impl sealed::Sealed for EntityHandle {
	type Stored = u32;

	const FIELD_TYPES: &'static [sys::fieldtype_t] = &[sys::_fieldtypes_FIELD_EHANDLE];

	fn from_stored(stored: u32) -> Self {
		Self::from_raw(stored)
	}

	fn into_stored(self) -> u32 {
		self.to_raw()
	}
}

impl FieldValue for QAngle {}

impl sealed::Sealed for QAngle {
	type Stored = sys::QAngle;

	const FIELD_TYPES: &'static [sys::fieldtype_t] = &[sys::_fieldtypes_FIELD_VECTOR];

	fn from_stored(stored: sys::QAngle) -> Self {
		stored.into()
	}

	fn into_stored(self) -> sys::QAngle {
		self.into()
	}
}

impl FieldValue for Vector {}

impl sealed::Sealed for Vector {
	type Stored = sys::Vector;

	const FIELD_TYPES: &'static [sys::fieldtype_t] = &[
		sys::_fieldtypes_FIELD_VECTOR,
		sys::_fieldtypes_FIELD_POSITION_VECTOR,
	];

	fn from_stored(stored: sys::Vector) -> Self {
		stored.into()
	}

	fn into_stored(self) -> sys::Vector {
		self.into()
	}
}

/// Finds the member `name` in the object `maps` describe, nested `depth`
/// deep, and its offset in that object: from the class's own map towards its
/// bases', and through each embedded object before the fields after it.
fn find_field<'a>(maps: DataMaps<'a>, name: &CStr, depth: usize) -> Option<(&'a DataField, usize)> {
	for map in maps {
		for field in map.fields() {
			if field.fieldType == sys::_fieldtypes_FIELD_EMBEDDED
				&& field.fieldSize == 1
				&& depth < MAX_EMBEDDING_DEPTH
				&& let Some((found, inner)) = find_field(field.embedded(), name, depth + 1)
			{
				return Some((found, field.offset()?.checked_add(inner)?));
			}

			if field.name() == Some(name) {
				return Some((field, field.offset()?));
			}
		}
	}

	None
}
