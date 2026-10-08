//! Builders of the data description maps the game describes its entity
//! classes with.

use crate::entities::datamap::TD_OFFSET_NORMAL;
use std::ffi::{CStr, c_int};

/// Builds a leaked map for `class`, declaring `fields` and deriving from
/// `base`, so it stays allocated and unmodified as the game's do.
///
/// For tests only.
///
/// # Panics
///
/// If there are more fields than a `c_int` can count.
pub fn data_map(
	class: &'static CStr,
	fields: Vec<sys::typedescription_t>,
	base: *mut sys::datamap_t,
) -> *mut sys::datamap_t {
	let count = c_int::try_from(fields.len()).unwrap();

	// SAFETY: Zero is valid for every field of `datamap_t`.
	let mut map: sys::datamap_t = unsafe { std::mem::zeroed() };

	map.dataDesc = fields.leak().as_mut_ptr();
	map.dataNumFields = count;
	map.dataClassName = class.as_ptr();
	map.baseMap = base;
	Box::into_raw(Box::new(map))
}

/// A field of `field_type` named `name`, at `offset` in its object.
///
/// For tests only. The field's other members are zero.
///
/// # Panics
///
/// If `offset` does not fit a `c_int`.
pub fn field(
	name: &'static CStr,
	field_type: sys::fieldtype_t,
	offset: usize,
) -> sys::typedescription_t {
	// SAFETY: Zero is valid for every field of `typedescription_t`.
	let mut field: sys::typedescription_t = unsafe { std::mem::zeroed() };

	field.fieldType = field_type;
	field.fieldName = name.as_ptr();
	field.fieldOffset[TD_OFFSET_NORMAL] = c_int::try_from(offset).unwrap();
	field
}
