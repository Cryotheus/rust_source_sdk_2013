//! Hand-written ABI of entity inputs: the `variant_t` values they carry, built
//! as the setters of `game/server/variant_t.h` build them.

use crate::entities::INVALID_EHANDLE_INDEX;
use std::ffi::c_int;
use std::mem::MaybeUninit;

/// A value for a `variant_t`, which [`Self::to_raw`] stores as the game's
/// setters do.
#[doc(alias("variant_t"))]
#[derive(Debug, Clone, Copy)]
pub enum Variant {
	/// No value (`FIELD_VOID`), as `variant_t`'s constructor leaves it.
	Void,

	/// A boolean (`FIELD_BOOLEAN`), as `SetBool` stores it.
	#[doc(alias("SetBool"))]
	Bool(bool),

	/// An integer (`FIELD_INTEGER`), as `SetInt` stores it.
	#[doc(alias("SetInt"))]
	Int(c_int),

	/// A float (`FIELD_FLOAT`), as `SetFloat` stores it.
	#[doc(alias("SetFloat"))]
	Float(f32),

	/// A string (`FIELD_STRING`), as `SetString` stores it.
	#[doc(alias("SetString"))]
	String(sys::string_t),

	/// A vector (`FIELD_VECTOR`), as `SetVector3D` stores it.
	#[doc(alias("SetVector3D"))]
	Vector([f32; 3]),

	/// A color (`FIELD_COLOR32`), as `SetColor32` stores it.
	#[doc(alias("SetColor32"))]
	Color(sys::color32),

	/// An entity handle (`FIELD_EHANDLE`), as `SetEntity` stores it, by the
	/// raw value of its `CBaseHandle`.
	#[doc(alias("SetEntity"))]
	Entity(u32),
}

impl Variant {
	/// The `fieldtype_t` a `variant_t` holding this value records.
	pub const fn field_type(self) -> sys::fieldtype_t {
		match self {
			Self::Void => sys::_fieldtypes_FIELD_VOID,
			Self::Bool(_) => sys::_fieldtypes_FIELD_BOOLEAN,
			Self::Int(_) => sys::_fieldtypes_FIELD_INTEGER,
			Self::Float(_) => sys::_fieldtypes_FIELD_FLOAT,
			Self::String(_) => sys::_fieldtypes_FIELD_STRING,
			Self::Vector(_) => sys::_fieldtypes_FIELD_VECTOR,
			Self::Color(_) => sys::_fieldtypes_FIELD_COLOR32,
			Self::Entity(_) => sys::_fieldtypes_FIELD_EHANDLE,
		}
	}

	/// Builds the `variant_t` the game's constructor and setter would: the
	/// value's member of the union set and every other byte of it zero, the
	/// handle left as `CHandle`'s constructor leaves it,
	/// [`INVALID_EHANDLE_INDEX`], unless the value is one, and the field type
	/// recorded.
	pub const fn to_raw(self) -> sys::variant_t {
		// SAFETY: Zero is valid for every field: a null string, a handle, and
		// `FIELD_VOID`.
		let mut variant = unsafe { MaybeUninit::<sys::variant_t>::zeroed().assume_init() };
		let mut handle = INVALID_EHANDLE_INDEX;

		match self {
			Self::Void => {}
			Self::Bool(value) => variant.__bindgen_anon_1.bVal = value,
			Self::Int(value) => variant.__bindgen_anon_1.iVal = value,
			Self::Float(value) => variant.__bindgen_anon_1.flVal = value,
			Self::String(value) => variant.__bindgen_anon_1.iszVal = value,
			Self::Vector(value) => variant.__bindgen_anon_1.vecVal = value,
			Self::Color(value) => variant.__bindgen_anon_1.rgbaVal = value,
			Self::Entity(value) => handle = value,
		}

		variant.eVal._base.m_Index = handle;
		variant.fieldType = self.field_type();
		variant
	}
}
