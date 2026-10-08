//! Tests of the `variant_t` values entity inputs carry.

use source_sdk_2013_raw::entities::INVALID_EHANDLE_INDEX;
use source_sdk_2013_raw::inputs::Variant;

/// The bytes every member of the union covers.
fn payload(variant: &sys::variant_t) -> [u8; 12] {
	// SAFETY: The union is plain data, and its every byte was initialized.
	unsafe {
		(&raw const variant.__bindgen_anon_1)
			.cast::<[u8; 12]>()
			.read()
	}
}

#[test]
fn values_are_stored_as_the_setters_store_them() {
	let int = Variant::Int(-5).to_raw();
	let entity = Variant::Entity(9 | 4 << 16).to_raw();
	let void = Variant::Void.to_raw();

	assert_eq!(int.fieldType, sys::_fieldtypes_FIELD_INTEGER);
	assert_eq!(payload(&int), {
		let mut bytes = [0; 12];
		bytes[..4].copy_from_slice(&(-5_i32).to_ne_bytes());
		bytes
	});
	assert_eq!(int.eVal._base.m_Index, INVALID_EHANDLE_INDEX);

	assert_eq!(entity.fieldType, sys::_fieldtypes_FIELD_EHANDLE);
	assert_eq!(payload(&entity), [0; 12]);
	assert_eq!(entity.eVal._base.m_Index, 9 | 4 << 16);

	assert_eq!(void.fieldType, sys::_fieldtypes_FIELD_VOID);
	assert_eq!(payload(&void), [0; 12]);
	assert_eq!(void.eVal._base.m_Index, INVALID_EHANDLE_INDEX);
}
