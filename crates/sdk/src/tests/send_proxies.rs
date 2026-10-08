//! Tests of overriding what clients receive of networked variables.

use super::*;
use crate::datatables::{NetProp, NetPropError, NetValue, PropFlags, StandardSendProxies, Storage};
use crate::test_support::datatables::*;
use crate::test_support::entities::{MockEntity, set_networking};
use crate::test_support::sdk_core::change_tracking_engine;
use sdk_raw::test_support::edicts::mock_edict;
use std::ffi::c_void;
use std::mem::zeroed;
use std::ptr::null_mut;

/// `DT_Rules`, which a relocating proxy nests in `DT_Proxy`, as TF2 nests the
/// game rules' table, and `DT_Proxy`, which also holds a team, a goal and an
/// origin of its own.
struct Tables {
	rules_props: Box<[sys::SendProp]>,
	proxy_props: Box<[sys::SendProp]>,
	rules: Box<sys::SendTable>,
	proxy: Box<sys::SendTable>,
	proxies: Box<sys::CStandardSendProxies>,
}

impl Tables {
	fn new() -> Box<Self> {
		// SAFETY: Tables are plain data, for which zero is valid.
		let empty = || Box::new(unsafe { zeroed::<sys::SendTable>() });
		let mut tables = Box::new(Self {
			rules_props: Box::new([]),
			proxy_props: Box::new([]),
			rules: empty(),
			proxy: empty(),
			proxies: Box::new(proxies(null_mut())),
		});

		tables.rules_props = Box::new([prop(
			c"m_nGameType",
			sys::SendPropType_DPT_Int,
			0,
			PropFlags::default(),
			Some(int32_proxy),
		)]);
		tables.rules = Box::new(table(c"DT_Rules", &mut tables.rules_props));

		tables.proxy_props = Box::new([
			prop(
				c"m_iTeamNum",
				sys::SendPropType_DPT_Int,
				16,
				PropFlags::default(),
				Some(int32_proxy),
			),
			prop(
				c"m_pszGoal",
				sys::SendPropType_DPT_String,
				24,
				PropFlags::default(),
				Some(string_proxy),
			),
			prop(
				c"m_vecOrigin",
				sys::SendPropType_DPT_Vector,
				32,
				PropFlags::default(),
				Some(vector_proxy),
			),
			table_prop(
				c"rules_data",
				0,
				&raw mut *tables.rules,
				Some(pointer_table),
			),
		]);
		tables.proxy = Box::new(table(c"DT_Proxy", &mut tables.proxy_props));

		tables
	}

	fn proxies(&self) -> StandardSendProxies<'_> {
		// SAFETY: The proxies are the fixture's, and have no registered list.
		unsafe { StandardSendProxies::from_raw(NonNull::from(&*self.proxies)) }
	}

	fn proxy(&self) -> SendTable<'_> {
		// SAFETY: The table is the fixture's, whose properties and nested tables
		// stay in place while it is borrowed.
		unsafe { SendTable::from_raw(NonNull::from(&*self.proxy)) }
	}
}

#[test]
fn an_override_changes_what_clients_receive_but_not_the_variable() {
	let tables = Tables::new();
	let (mut mock, _class, slot) = networked(&tables);
	let team = NetProp::resolve(tables.proxy(), c"m_iTeamNum", tables.proxies()).unwrap();

	mock.set_int(16, 2);

	let entity = mock.entity();

	let overridden = SendProxyOverride::install(team.prop(), |sent, value| match value {
		SentValue::Int(team) if sent.entity_index() == 5 && sent.element() == 0 => {
			Some(SentValue::Int(team + 1))
		}

		_ => None,
	})
	.unwrap();

	assert!(overridden.is_installed());
	assert_eq!(team.value(entity), Ok(NetValue::Int(3)));

	// The variable is still read and written as the game's proxy stores it.
	assert_eq!(team.storage(), Storage::I32);
	assert_eq!(team.get::<i32>(entity), Ok(2));

	// SAFETY: No game code reads the mock's team.
	unsafe { team.set(change_tracking_engine(), entity, 1) }.unwrap();
	assert_eq!(team.value(entity), Ok(NetValue::Int(2)));
	assert_ne!(slot._base.m_fStateFlags & 1, 0);

	assert!(matches!(
		SendProxyOverride::install(team.prop(), |_, _| None),
		Err(SendProxyError::AlreadyOverridden { .. })
	));

	drop(overridden);
	assert_eq!(team.value(entity), Ok(NetValue::Int(1)));

	set_networking(null_mut(), null_mut());
}

#[test]
fn entity_values_are_kept_per_slot() {
	static PASSABLE: EntityValues<bool> = EntityValues::new();
	let speeds = EntityValues::<f32>::new();

	assert_eq!(PASSABLE.get(3), None);
	assert!(PASSABLE.set(3, Some(true)));
	assert!(!PASSABLE.set(3, Some(true)));
	assert_eq!(PASSABLE.get(3), Some(true));
	assert!(PASSABLE.set(3, Some(false)));
	assert_eq!(PASSABLE.get(3), Some(false));
	assert!(PASSABLE.set(3, None));
	assert_eq!(PASSABLE.get(3), None);

	// Past the edict table, nothing is kept.
	assert!(!PASSABLE.set(MAX_EDICTS as usize, Some(true)));
	assert_eq!(PASSABLE.get(MAX_EDICTS as usize), None);

	assert!(speeds.set(0, Some(-0.5)));
	assert!(speeds.set(2047, Some(f32::MAX)));
	assert_eq!(
		(speeds.get(0), speeds.get(2047)),
		(Some(-0.5), Some(f32::MAX))
	);

	speeds.clear();
	assert_eq!((speeds.get(0), speeds.get(2047)), (None, None));

	let teams = EntityValues::<i32>::default();
	assert!(teams.set(1, Some(-1)));
	assert_eq!(teams.get(1), Some(-1));
}

/// A mock entity networked as a `DT_Proxy`, with its slot, which the caller
/// keeps until it calls `set_networking(null_mut(), null_mut())`.
fn networked(tables: &Tables) -> (MockEntity, Box<sys::ServerClass>, Box<sys::edict_t>) {
	let mock = MockEntity::new(5);
	let mut class = Box::new(sys::ServerClass {
		m_pNetworkName: c"CProxy".as_ptr(),
		m_pTable: (&raw const *tables.proxy).cast_mut(),
		m_pNext: null_mut(),
		m_ClassID: 1,
		m_InstanceBaselineIndex: 0,
	});
	let mut slot = Box::new(mock_edict(5, false));

	set_networking(&raw mut *class, &raw mut *slot);
	(mock, class, slot)
}

#[test]
fn relocated_properties_are_found_to_override_and_tables_are_refused() {
	let tables = Tables::new();
	let proxy = tables.proxy();

	// `NetProp` refuses what it cannot address, but the property can still be
	// overridden.
	assert!(matches!(
		NetProp::resolve(proxy, c"m_nGameType", tables.proxies()),
		Err(NetPropError::Relocated { .. })
	));

	let game_type = proxy.find_prop(c"m_nGameType").unwrap();
	assert_eq!(
		game_type.as_ptr(),
		&raw const tables.rules_props[0] as *mut _
	);
	assert!(proxy.find_prop(c"m_nMissing").is_none());

	assert!(holds_prop(
		proxy,
		NonNull::new(game_type.as_ptr()).unwrap(),
		0
	));
	// SAFETY: The table is the fixture's, which outlives the handle.
	let rules = unsafe { SendTable::from_raw(NonNull::from(&*tables.rules)) };
	assert!(!holds_prop(
		rules,
		NonNull::new(proxy.find_prop(c"m_iTeamNum").unwrap().as_ptr()).unwrap(),
		0
	));

	let rules_data = proxy.find_prop(c"rules_data").unwrap();
	assert!(matches!(
		SendProxyOverride::install(rules_data, |_, _| None),
		Err(SendProxyError::NotAValue {
			kind: PropKind::DataTable,
			..
		})
	));

	let overridden = SendProxyOverride::install(game_type, |_, _| Some(SentValue::Int(0))).unwrap();
	assert!(overridden.is_installed());
}

/// Stands in for a string proxy, like `SendProxy_StringToString`, which sends
/// the string the variable points to.
///
/// # Safety
///
/// `data` must point to a pointer to a string, and `out` to a writable
/// `DVariant`.
unsafe extern "C" fn string_proxy(
	_: *const sys::SendProp,
	_: *const c_void,
	data: *const c_void,
	out: *mut sys::DVariant,
	_: c_int,
	_: c_int,
) {
	// SAFETY: As the caller promises.
	unsafe { (*out).__bindgen_anon_1.m_pString = data.cast::<*const c_char>().read() };
}

#[test]
fn strings_and_vectors_are_sent_in_place_of_the_games() {
	let tables = Tables::new();
	let (mut mock, _class, _slot) = networked(&tables);
	let entity = mock.entity();
	let goal = NetProp::resolve(tables.proxy(), c"m_pszGoal", tables.proxies()).unwrap();
	let origin = NetProp::resolve(tables.proxy(), c"m_vecOrigin", tables.proxies()).unwrap();
	let game_goal = c"#capture_the_flag";

	// SAFETY: The string pointer and the vector lie within the mock's
	// storage, aligned for them, and the string is static.
	unsafe {
		entity
			.as_ptr()
			.byte_add(24)
			.cast::<*const c_char>()
			.write(game_goal.as_ptr());
		entity
			.as_ptr()
			.byte_add(32)
			.cast::<[f32; 3]>()
			.write([1.0, 2.0, 3.0]);
	}

	let hidden_goal = SendProxyOverride::install(goal.prop(), |_, value| match value {
		SentValue::String(goal) if goal.to_bytes().starts_with(b"#capture") => {
			Some(SentValue::String(c""))
		}

		_ => None,
	})
	.unwrap();

	// A value of another type than the property's is not sent.
	let mistyped =
		SendProxyOverride::install(origin.prop(), |_, _| Some(SentValue::Int(7))).unwrap();

	assert_eq!(goal.value(entity), Ok(NetValue::String(c"".to_owned())));
	assert_eq!(
		origin.value(entity),
		Ok(NetValue::Vector(Vector::new(1.0, 2.0, 3.0)))
	);

	drop(mistyped);

	let raised = SendProxyOverride::install(origin.prop(), |_, value| match value {
		SentValue::Vector(origin) => Some(SentValue::Vector(Vector(origin.0 + glam::Vec3::Z))),
		_ => None,
	})
	.unwrap();

	assert_eq!(
		origin.value(entity),
		Ok(NetValue::Vector(Vector::new(1.0, 2.0, 4.0)))
	);

	drop((hidden_goal, raised));
	assert_eq!(
		goal.value(entity),
		Ok(NetValue::String(game_goal.to_owned()))
	);

	set_networking(null_mut(), null_mut());
}

#[test]
fn values_changing_resend_their_entity() {
	static GROUPS: EntityValues<i32> = EntityValues::new();
	let engine = change_tracking_engine();
	let mut slot = mock_edict(9, false);

	// SAFETY: The slot is the test's, which outlives the handle.
	let edict = unsafe { Edict::from_raw(NonNull::from(&mut slot)) };

	GROUPS.set_and_resend(engine, edict, Some(4));
	assert_eq!(GROUPS.get(9), Some(4));
	assert_ne!(slot._base.m_fStateFlags & 1, 0);

	slot._base.m_fStateFlags = 0;
	GROUPS.set_and_resend(engine, edict, Some(4));
	assert_eq!(slot._base.m_fStateFlags & 1, 0, "unchanged, so not resent");
}
