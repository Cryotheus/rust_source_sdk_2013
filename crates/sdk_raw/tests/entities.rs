//! Tests of the game's data description maps, and of `CBaseEntity`'s vtable
//! slots and header values.

use source_sdk_2013_raw::entities::datamap::{DataMap, DataMaps, FTYPEDESC_KEY, TD_OFFSET_NORMAL};
use source_sdk_2013_raw::entities::{
	ACCEPT_INPUT_SLOT, BASE_ENTITY_FIELD_OFFSET_LIMIT, ENT_ENTRY_MASK, GET_DATA_DESC_MAP_SLOT,
	INVALID_NETWORKED_EHANDLE_VALUE, NUM_ENT_ENTRIES, NUM_SERIAL_NUM_SHIFT_BITS,
	SDK2013_NEXT_BOT_TELEPORT_SLOT, SDK2013_TELEPORT_SLOT, TF2_TELEPORT_SLOT, TeleportSlot,
	accept_input, data_desc_map, find_base_entity_field, teleport,
};
use source_sdk_2013_raw::test_support::entities::{data_map, field};
use source_sdk_2013_raw::test_support::unexpected_call;
use std::cell::Cell;
use std::ffi::{CStr, c_int};
use std::ptr::{null, null_mut};

#[test]
fn chains_run_from_the_class_to_its_bases() {
	let base = data_map(
		c"CBaseEntity",
		vec![field(c"m_iHealth", sys::_fieldtypes_FIELD_INTEGER, 16)],
		null_mut(),
	);
	let derived = data_map(c"CDerived", vec![], base);
	let classes = |first| {
		maps(first)
			.map(|map| map.class_name().unwrap().to_owned())
			.collect::<Vec<_>>()
	};

	assert_eq!(classes(derived), [c"CDerived", c"CBaseEntity"]);
	assert_eq!(maps(null_mut()).count(), 0);

	let base = maps(derived).nth(1).unwrap();

	assert_eq!(
		base.field_offset(c"m_iHealth", sys::_fieldtypes_FIELD_INTEGER),
		Some(16)
	);
	assert_eq!(
		base.field_offset(c"m_iHealth", sys::_fieldtypes_FIELD_FLOAT),
		None
	);
	assert!(maps(derived).next().unwrap().fields().is_empty());

	// A chain that loops back on itself ends after the most maps followed.
	// SAFETY: The map is leaked, and only changed before it is read.
	unsafe { (*derived).baseMap = derived };
	assert_eq!(maps(derived).count(), DataMaps::MAX_MAPS);
}

#[test]
fn implausible_field_arrays_declare_nothing() {
	let map = data_map(
		c"CBaseEntity",
		vec![field(c"m_iHealth", sys::_fieldtypes_FIELD_INTEGER, 16)],
		null_mut(),
	);
	let fields = move || maps(map).next().unwrap().fields().len();

	assert_eq!(fields(), 1);

	// SAFETY: The map is leaked, and only changed between reads.
	unsafe {
		(*map).dataNumFields = -1;
		assert_eq!(fields(), 0);

		(*map).dataNumFields = c_int::try_from(DataMap::MAX_FIELDS + 1).unwrap();
		assert_eq!(fields(), 0);

		(*map).dataNumFields = 1;
		(*map).dataDesc = (*map).dataDesc.byte_add(1);
		assert_eq!(fields(), 0);

		(*map).dataDesc = null_mut();
		assert_eq!(fields(), 0);
	}
}

/// A key field named `key`, at `offset` in its object.
fn key(name: &'static CStr, key: &'static CStr, offset: usize) -> sys::typedescription_t {
	let mut field = field(name, sys::_fieldtypes_FIELD_STRING, offset);

	field.externalName = key.as_ptr();
	field.flags = FTYPEDESC_KEY;
	field
}

#[test]
fn key_fields_are_found_as_extract_keyvalue_does() {
	let inner = data_map(c"CInner", vec![key(c"m_iszInner", c"inner", 8)], null_mut());

	let mut embedded = field(c"m_Inner", sys::_fieldtypes_FIELD_EMBEDDED, 64);
	embedded.fieldSize = 1;
	embedded.td = inner;

	let mut array = embedded;
	array.fieldName = c"m_Inners".as_ptr();
	array.fieldSize = 2;
	array.fieldOffset[TD_OFFSET_NORMAL] = 128;

	let base = data_map(
		c"CBaseEntity",
		vec![
			key(c"m_iName", c"targetname", 24),
			key(c"m_iszShadowed", c"shadowed", 32),
		],
		null_mut(),
	);
	let derived = data_map(
		c"CDerived",
		vec![
			array,
			embedded,
			key(c"m_iszOwn", c"Shadowed", 40),
			field(c"m_iNotKey", sys::_fieldtypes_FIELD_INTEGER, 48),
		],
		base,
	);
	let find = |name: &[u8]| {
		maps(derived)
			.find_key_field(name)
			.map(|(field, offset)| (field.name().unwrap(), offset))
	};

	// Through the single embedded object, but not the array of them.
	assert_eq!(find(b"INNER"), Some((c"m_iszInner", 72)));
	// The derived class's key shadows its base's.
	assert_eq!(find(b"shadowed"), Some((c"m_iszOwn", 40)));
	assert_eq!(find(b"targetname"), Some((c"m_iName", 24)));
	assert_eq!(find(b"m_iNotKey"), None);
	assert_eq!(find(b"missing"), None);
}

#[test]
fn key_fields_are_walked_as_find_key_field_searches() {
	let inner = data_map(c"CInner", vec![key(c"m_iszInner", c"inner", 8)], null_mut());

	let mut embedded = field(c"m_Inner", sys::_fieldtypes_FIELD_EMBEDDED, 64);
	embedded.fieldSize = 1;
	embedded.td = inner;

	let mut array = embedded;
	array.fieldName = c"m_Inners".as_ptr();
	array.fieldSize = 2;
	array.fieldOffset[TD_OFFSET_NORMAL] = 128;

	let mut negative = key(c"m_iszNegative", c"negative", 0);
	negative.fieldOffset[TD_OFFSET_NORMAL] = -1;

	let base = data_map(
		c"CBaseEntity",
		vec![
			key(c"m_iName", c"targetname", 24),
			key(c"m_iszShadowed", c"shadowed", 32),
			negative,
		],
		null_mut(),
	);
	let derived = data_map(
		c"CDerived",
		vec![
			array,
			embedded,
			key(c"m_iszOwn", c"Shadowed", 40),
			field(c"m_iNotKey", sys::_fieldtypes_FIELD_INTEGER, 48),
		],
		base,
	);
	let walked = maps(derived)
		.key_fields()
		.map(|(field, offset)| (field.name().unwrap(), offset))
		.collect::<Vec<_>>();

	// Through the single embedded object, but not the array of them, and the
	// derived class's keys before its base's.
	assert_eq!(
		walked,
		[
			(c"m_iszInner", Some(72)),
			(c"m_iszOwn", Some(40)),
			(c"m_iName", Some(24)),
			(c"m_iszShadowed", Some(32)),
			(c"m_iszNegative", None),
		]
	);

	// `find_key_field` finds the first field of each external name.
	for (field, _) in maps(derived).key_fields() {
		let name = field.external_name().unwrap().to_bytes();
		let (first, offset) = maps(derived)
			.key_fields()
			.find(|(other, _)| {
				other
					.external_name()
					.is_some_and(|other| other.to_bytes().eq_ignore_ascii_case(name))
			})
			.unwrap();

		assert_eq!(
			maps(derived)
				.find_key_field(name)
				.map(|(found, offset)| (std::ptr::from_ref(found), offset)),
			offset.map(|offset| (std::ptr::from_ref(first), offset))
		);
	}

	// An object embedding itself is searched only as deep as `find_key_field`
	// searches it.
	let mut itself = field(c"m_Itself", sys::_fieldtypes_FIELD_EMBEDDED, 4);
	itself.fieldSize = 1;

	let looping = data_map(
		c"CLooping",
		vec![itself, key(c"m_iszLoop", c"loop", 0)],
		null_mut(),
	);

	// SAFETY: The map is leaked, and only changed before it is read.
	unsafe { (*(*looping).dataDesc).td = looping };

	let depths = 0..=DataMaps::MAX_EMBEDDING_DEPTH;

	assert_eq!(
		maps(looping)
			.key_fields()
			.map(|(_, offset)| offset)
			.collect::<Vec<_>>(),
		depths
			.rev()
			.map(|depth| Some(4 * depth))
			.collect::<Vec<_>>()
	);
	assert_eq!(
		maps(looping)
			.find_key_field(b"loop")
			.map(|(_, offset)| offset),
		Some(4 * DataMaps::MAX_EMBEDDING_DEPTH)
	);
}

/// The maps from `first`, which the tests leak.
fn maps(first: *mut sys::datamap_t) -> DataMaps<'static> {
	// SAFETY: The tests' maps are leaked and never changed.
	unsafe { DataMaps::new(first) }
}

/// An entity whose primary vtable holds the thunks [`with_entity`] installs.
#[repr(C)]
struct FakeEntity {
	vtable: *const *const (),
}

thread_local! {
	static CALLS: Cell<[usize; 4]> = const { Cell::new([0; 4]) };
}

unsafe extern "C" fn accept(
	_: *mut sys::CBaseEntity,
	input: *const std::ffi::c_char,
	_: *mut sys::CBaseEntity,
	_: *mut sys::CBaseEntity,
	value: *mut sys::variant_t,
	output_id: c_int,
) -> bool {
	// SAFETY: `accept_input` passes the caller's name and value.
	unsafe {
		(&raw mut (*value).fieldType).write(sys::_fieldtypes_FIELD_VOID);
		CStr::from_ptr(input) == c"Kill" && output_id == 7
	}
}

#[test]
fn base_entity_fields_are_found_where_plausible() {
	let size = size_of::<c_int>();
	let mut flags = field(c"m_iEFlags", sys::_fieldtypes_FIELD_INTEGER, 32);
	flags.fieldSizeInBytes = c_int::try_from(size).unwrap();

	let mut misaligned = flags;
	misaligned.fieldName = c"m_iMisaligned".as_ptr();
	misaligned.fieldOffset[TD_OFFSET_NORMAL] = 34;

	let mut distant = flags;
	distant.fieldName = c"m_iDistant".as_ptr();
	distant.fieldOffset[TD_OFFSET_NORMAL] =
		c_int::try_from(BASE_ENTITY_FIELD_OFFSET_LIMIT).unwrap();

	// A vector of floats needs their alignment, not one of its size.
	let vector_size = size_of::<sys::Vector>();
	let mut velocity = field(c"m_vecAbsVelocity", sys::_fieldtypes_FIELD_VECTOR, 44);
	velocity.fieldSizeInBytes = c_int::try_from(vector_size).unwrap();

	let mut misaligned_velocity = velocity;
	misaligned_velocity.fieldName = c"m_vecMisaligned".as_ptr();
	misaligned_velocity.fieldOffset[TD_OFFSET_NORMAL] = 58;

	let base = data_map(
		c"CBaseEntity",
		vec![flags, misaligned, distant, velocity, misaligned_velocity],
		null_mut(),
	);
	let derived = data_map(c"CDerived", vec![flags], base);
	let find = |name, field_type, size| {
		// SAFETY: The tests' maps are leaked and never changed.
		find_base_entity_field(unsafe { DataMaps::new(derived) }, name, field_type, size)
	};

	assert_eq!(
		find(c"m_iEFlags", sys::_fieldtypes_FIELD_INTEGER, size),
		Some(32)
	);
	assert_eq!(find(c"m_iEFlags", sys::_fieldtypes_FIELD_FLOAT, size), None);
	assert_eq!(find(c"m_iEFlags", sys::_fieldtypes_FIELD_INTEGER, 2), None);
	assert_eq!(
		find(c"m_iMisaligned", sys::_fieldtypes_FIELD_INTEGER, size),
		None
	);
	assert_eq!(
		find(c"m_iDistant", sys::_fieldtypes_FIELD_INTEGER, size),
		None
	);
	assert_eq!(
		find(
			c"m_vecAbsVelocity",
			sys::_fieldtypes_FIELD_VECTOR,
			vector_size
		),
		Some(44)
	);
	assert_eq!(
		find(
			c"m_vecMisaligned",
			sys::_fieldtypes_FIELD_VECTOR,
			vector_size
		),
		None
	);
}

#[test]
fn calls_go_through_the_entitys_slots() {
	with_entity(|entity| {
		// SAFETY: The entity's vtable has every slot called, and the
		// arguments are locals or null.
		unsafe {
			let mut value: sys::variant_t = std::mem::zeroed();

			value.fieldType = sys::_fieldtypes_FIELD_INTEGER;
			assert!(accept_input(
				entity,
				c"Kill",
				null_mut(),
				null_mut(),
				&raw mut value,
				7
			));
			assert_eq!(value.fieldType, sys::_fieldtypes_FIELD_VOID);
			assert!(!accept_input(
				entity,
				c"Use",
				null_mut(),
				null_mut(),
				&raw mut value,
				7
			));

			let maps = DataMaps::new(data_desc_map(entity));
			let classes: Vec<_> = maps.map(|map| map.class_name()).collect();

			assert_eq!(classes, [Some(c"CBaseEntity")]);

			let origin = sys::Vector {
				x: 1.0,
				y: 2.0,
				z: 3.0,
			};

			teleport(entity, TeleportSlot::TeamFortress2, &origin, null(), null());
			assert_eq!(CALLS.get(), [1, 0, 0, 0]);
			teleport(entity, TeleportSlot::SourceSdk2013, null(), null(), null());
			assert_eq!(CALLS.get(), [1, 1, 0, 1]);
			teleport(
				entity,
				TeleportSlot::SourceSdk2013NextBot,
				null(),
				null(),
				null(),
			);
			assert_eq!(CALLS.get(), [1, 1, 1, 2]);
		}
	});
}

/// The entity's `GetDataDescMap`, returning a map of `CBaseEntity`.
unsafe extern "C" fn datamap(_: *mut sys::CBaseEntity) -> *mut sys::datamap_t {
	data_map(c"CBaseEntity", vec![], null_mut())
}

#[test]
fn handle_values_follow_the_header() {
	assert_eq!(NUM_ENT_ENTRIES, 8192);
	assert_eq!(ENT_ENTRY_MASK, 0xFFFF);
	assert_eq!(NUM_SERIAL_NUM_SHIFT_BITS, 16);
	assert_eq!(INVALID_NETWORKED_EHANDLE_VALUE, 0x1F_FFFF);
}

/// A `Teleport` at one of the Source SDK 2013 slots, counting its calls in
/// `CALLS[KIND]`, and those without an origin in `CALLS[3]`.
unsafe extern "C" fn sdk2013_teleport<const KIND: usize>(
	_: *mut sys::CBaseEntity,
	origin: *const sys::Vector,
	_: *const sys::QAngle,
	_: *const sys::Vector,
) {
	let mut calls = CALLS.get();

	calls[KIND] += 1;
	calls[3] += usize::from(origin.is_null());
	CALLS.set(calls);
}

unsafe extern "C" fn tf2_teleport(
	_: *mut sys::CBaseEntity,
	origin: *const sys::Vector,
	_: *const sys::QAngle,
	_: *const sys::Vector,
) {
	// SAFETY: The test passes a local origin.
	assert_eq!(unsafe { origin.read() }.z, 3.0);

	let mut calls = CALLS.get();

	calls[0] += 1;
	CALLS.set(calls);
}

/// Calls `test` with an entity whose vtable has every slot these functions
/// call.
fn with_entity(test: impl FnOnce(*mut sys::CBaseEntity)) {
	let slot_count = TF2_TELEPORT_SLOT
		.max(SDK2013_TELEPORT_SLOT)
		.max(SDK2013_NEXT_BOT_TELEPORT_SLOT)
		+ 1;
	let mut vtable = vec![unexpected_call as *const (); slot_count];

	vtable[ACCEPT_INPUT_SLOT] = accept as *const ();
	vtable[GET_DATA_DESC_MAP_SLOT] = datamap as *const ();
	vtable[TF2_TELEPORT_SLOT] = tf2_teleport as *const ();
	vtable[SDK2013_TELEPORT_SLOT] = sdk2013_teleport::<1> as *const ();
	vtable[SDK2013_NEXT_BOT_TELEPORT_SLOT] = sdk2013_teleport::<2> as *const ();

	let mut entity = FakeEntity {
		vtable: vtable.as_ptr(),
	};

	CALLS.set([0; 4]);
	test((&raw mut entity).cast());
}
