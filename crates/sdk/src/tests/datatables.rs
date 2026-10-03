//! Tests of resolving networked variables through a game DLL's send tables,
//! and of reading and writing them in entity memory.

use super::*;
use crate::test_support::datatables::*;
use crate::test_support::entities::{MockEntity, set_networking};
use crate::test_support::sdk_core::change_tracking_engine;
use sdk_raw::test_support::edicts::mock_edict;
use std::mem::zeroed;
use std::ptr::null_mut;

/// `DT_Base` holds `m_iHealth` and a local table; `DT_Derived` embeds it as
/// its base class, then an array, a relocated table, and variables with
/// custom and unsigned proxies.
struct Tables {
	base_props: Box<[sys::SendProp]>,
	local_props: Box<[sys::SendProp]>,
	derived_props: Box<[sys::SendProp]>,
	pointer_props: Box<[sys::SendProp]>,
	other_props: Box<[sys::SendProp]>,
	base: Box<sys::SendTable>,
	local: Box<sys::SendTable>,
	derived: Box<sys::SendTable>,
	pointer: Box<sys::SendTable>,
	other: Box<sys::SendTable>,
	registered: Box<sys::CNonModifiedPointerProxy>,
	registered_head: Box<*mut sys::CNonModifiedPointerProxy>,
	proxies: Box<sys::CStandardSendProxies>,
}

impl Tables {
	fn new() -> Box<Self> {
		// SAFETY: Tables are plain data, for which zero is valid.
		let empty = || Box::new(unsafe { zeroed::<sys::SendTable>() });
		let mut tables = Box::new(Self {
			base_props: Box::new([]),
			local_props: Box::new([]),
			derived_props: Box::new([]),
			pointer_props: Box::new([]),
			other_props: Box::new([]),
			base: empty(),
			local: empty(),
			derived: empty(),
			pointer: empty(),
			other: empty(),
			registered: Box::new(sys::CNonModifiedPointerProxy {
				m_Fn: Some(registered_table),
				m_pNext: null_mut(),
			}),
			registered_head: Box::new(null_mut()),
			proxies: Box::new(proxies(null_mut())),
		});

		*tables.registered_head = &raw mut *tables.registered;
		tables.proxies.m_ppNonModifiedPointerProxies = &raw mut *tables.registered_head;

		tables.local_props = Box::new([
			prop(
				c"m_bDucked",
				sys::SendPropType_DPT_Int,
				4,
				PropFlags::UNSIGNED,
				Some(int8_proxy),
			),
			prop(
				c"m_vecPunchAngle",
				sys::SendPropType_DPT_Vector,
				8,
				PropFlags::default(),
				Some(vector_proxy),
			),
		]);
		tables.local = Box::new(table(c"DT_Local", &mut tables.local_props));

		tables.base_props = Box::new([
			int(c"m_iHealth", 16, Some(int32_proxy)),
			table_prop(
				c"m_Local",
				64,
				&raw mut *tables.local,
				Some(registered_table),
			),
		]);
		tables.base = Box::new(table(c"DT_Base", &mut tables.base_props));

		tables.pointer_props = Box::new([int(c"m_iHidden", 0, Some(int32_proxy))]);
		tables.pointer = Box::new(table(c"DT_Pointer", &mut tables.pointer_props));

		let mut element = int(c"m_iAmmo", 100, Some(int16_proxy));
		element.m_Flags = PropFlags::INSIDE_ARRAY.bits();
		let mut array = prop(
			c"m_iAmmo",
			sys::SendPropType_DPT_Array,
			0,
			PropFlags::default(),
			None,
		);
		array.m_nElements = 4;
		array.m_ElementStride = 2;

		tables.derived_props = Box::new([
			table_prop(c"baseclass", 0, &raw mut *tables.base, Some(direct_table)),
			prop(
				c"m_iHealth",
				sys::SendPropType_DPT_Int,
				0,
				PropFlags::EXCLUDE,
				None,
			),
			element,
			array,
			table_prop(
				c"m_Pointer",
				200,
				&raw mut *tables.pointer,
				Some(pointer_table),
			),
			int(c"m_hOwner", 120, Some(custom_proxy)),
			prop(
				c"m_nFlags",
				sys::SendPropType_DPT_Int,
				124,
				PropFlags::UNSIGNED,
				Some(int32_proxy),
			),
		]);

		let array_prop = &raw mut tables.derived_props[2];
		tables.derived_props[3].m_pArrayProp = array_prop;
		tables.derived = Box::new(table(c"DT_Derived", &mut tables.derived_props));

		tables.other_props = Box::new([int(c"m_iHealth", 16, Some(int32_proxy))]);
		tables.other = Box::new(table(c"DT_Other", &mut tables.other_props));

		tables
	}

	fn table(table: &sys::SendTable) -> SendTable<'_> {
		// SAFETY: The table is one of the fixture's, whose properties and
		// nested tables stay in place while it is borrowed.
		unsafe { SendTable::from_raw(NonNull::from(table)) }
	}

	fn proxies(&self) -> StandardSendProxies<'_> {
		// SAFETY: The proxies and their registered list are the fixture's,
		// which stay in place while it is borrowed.
		unsafe { StandardSendProxies::from_raw(NonNull::from(&*self.proxies)) }
	}
}

/// A signed integer property.
fn int(name: &'static CStr, offset: c_int, proxy: sys::SendVarProxyFn) -> sys::SendProp {
	prop(
		name,
		sys::SendPropType_DPT_Int,
		offset,
		PropFlags::default(),
		proxy,
	)
}

#[test]
fn lookups_follow_nested_tables_and_report_why_they_fail() {
	let tables = Tables::new();
	let proxies = tables.proxies();
	let derived = Tables::table(&tables.derived);
	let resolve = |name: &CStr| NetProp::resolve(derived, name, proxies);

	assert_eq!(derived.base().map(SendTable::name), Some(c"DT_Base"));
	assert!(derived.derives_from(Tables::table(&tables.base)));
	assert!(!Tables::table(&tables.base).derives_from(derived));

	// Found in the base class rather than as the exclude property.
	let health = resolve(c"m_iHealth").unwrap();
	assert_eq!((health.offset(), health.storage()), (16, Storage::I32));

	// Found through a registered, pointer-preserving table proxy.
	let ducked = resolve(c"m_bDucked").unwrap();
	assert_eq!((ducked.offset(), ducked.storage()), (68, Storage::U8));
	assert_eq!(
		resolve(c"m_vecPunchAngle").unwrap().storage(),
		Storage::Vector
	);

	// Unsigned flags pick the signedness even when proxies were folded.
	assert_eq!(resolve(c"m_nFlags").unwrap().storage(), Storage::U32);
	assert_eq!(resolve(c"m_hOwner").unwrap().storage(), Storage::Unknown);

	// Arrays resolve to their array property, and elements through it.
	let ammo = resolve(c"m_iAmmo").unwrap();
	assert_eq!(
		(ammo.prop().kind(), ammo.element_count()),
		(PropKind::Array, Some(4))
	);
	let third = ammo.element(2).unwrap();
	assert_eq!(
		(third.offset(), third.storage(), third.element),
		(104, Storage::I16, 2)
	);
	assert_eq!(
		ammo.element(4).unwrap_err().to_string(),
		"`m_iAmmo` has 4 elements, so it has no element 4"
	);

	let local = resolve(c"m_Local").unwrap();
	assert_eq!(local.element(1).unwrap().offset(), 72);

	assert_eq!(
		resolve(c"m_iHidden").unwrap_err().to_string(),
		"`m_iHidden` is inside `m_Pointer`, whose send proxy relocates its data, so its address is unknown"
	);
	assert_eq!(
		resolve(c"m_iMissing").unwrap_err().to_string(),
		"`DT_Derived` has no networked variable named `m_iMissing`"
	);
}

#[test]
fn variables_are_read_and_written_only_as_stored() {
	let tables = Tables::new();
	let proxies = tables.proxies();
	let derived = Tables::table(&tables.derived);
	let mut mock = MockEntity::new(3);

	// Make the mock entity networked, as a `DT_Derived`.
	let mut class = sys::ServerClass {
		m_pNetworkName: c"CDerived".as_ptr(),
		m_pTable: (&raw const *tables.derived).cast_mut(),
		m_pNext: null_mut(),
		m_ClassID: 1,
		m_InstanceBaselineIndex: 0,
	};
	let mut slot = mock_edict(3, false);
	set_networking(&raw mut class, &raw mut slot);

	let entity = mock.entity();
	// SAFETY: Every offset read or written here lies within the mock's
	// storage.
	let at = |offset: usize| unsafe { entity.as_ptr().cast::<u8>().add(offset) };

	// SAFETY: The offsets are those the tables declare, within the mock's
	// storage, and aligned for what is written there.
	unsafe {
		at(16).cast::<i32>().write(125);
		at(68).write(1);
		at(104).cast::<i16>().write(-3);
		at(120).cast::<i32>().write(41);
	}

	let health = NetProp::resolve(derived, c"m_iHealth", proxies).unwrap();
	assert_eq!(health.get::<i32>(entity), Ok(125));
	assert_eq!(health.get::<u32>(entity), Ok(125));
	assert!(matches!(
		health.get::<i16>(entity),
		Err(NetPropError::TypeMismatch { .. })
	));
	assert_eq!(health.value(entity), Ok(NetValue::Int(125)));
	assert_eq!(
		health.get::<f32>(entity).unwrap_err().to_string(),
		"`m_iHealth` is stored as i32, not f32"
	);

	let ducked = NetProp::resolve(derived, c"m_bDucked", proxies).unwrap();
	assert_eq!(ducked.get::<bool>(entity), Ok(true));

	let third_ammo = NetProp::resolve(derived, c"m_iAmmo", proxies)
		.unwrap()
		.element(2)
		.unwrap();
	assert_eq!(third_ammo.get::<i16>(entity), Ok(-3));

	// Custom proxies hide the storage, but still produce the networked value.
	let owner = NetProp::resolve(derived, c"m_hOwner", proxies).unwrap();
	assert_eq!(owner.value(entity), Ok(NetValue::Int(42)));
	assert!(matches!(
		owner.get::<i32>(entity),
		Err(NetPropError::UnknownStorage { .. })
	));

	// Writes land at the offset and mark the edict changed. Without the
	// engine's change tracking, the whole edict is marked changed.
	let engine = change_tracking_engine();

	// SAFETY: No game code reads the mock's health, so it accepts any value.
	unsafe { health.set(engine, entity, 300) }.unwrap();
	assert_eq!(health.get::<i32>(entity), Ok(300));
	assert_ne!(slot._base.m_fStateFlags & 1, 0);

	// Offsets resolved for another class are refused.
	let other = NetProp::resolve(Tables::table(&tables.other), c"m_iHealth", proxies).unwrap();
	assert_eq!(
		other.get::<i32>(entity).unwrap_err().to_string(),
		"`m_iHealth` belongs to `DT_Other`, which `tf_player`'s table `DT_Derived` does not derive from"
	);

	set_networking(null_mut(), null_mut());
}
