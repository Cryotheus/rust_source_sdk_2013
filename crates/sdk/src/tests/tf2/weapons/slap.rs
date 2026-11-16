//! Tests of the Hot Hand's second slap, through a fake Hot Hand whose smack
//! timer lies where the generated layout places it.

use super::*;
use crate::datatables::PropFlags;
use crate::interfaces::ValveEngine;
use crate::server::Module;
use crate::test_support::datatables::{direct_table, int8_proxy, prop, table, table_prop};
use crate::test_support::entities::MOCK_EFLAGS_OFFSET;
use crate::test_support::interfaces::server_game_dll::export_standard_proxies;
use crate::test_support::leak;
use crate::test_support::sdk_core::change_tracking_engine;
use crate::test_support::server::{export, mock_server};
use sdk_raw::entities::GET_DATA_DESC_MAP_SLOT;
use sdk_raw::test_support::edicts::mock_edict;
use sdk_raw::test_support::entities::{data_map, field};
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use sdk_raw::vtable_slot;
use std::ffi::{CStr, c_void};
use std::ptr::{NonNull, null_mut};

const _: () = assert!(offset_of!(FakeSlap, head.flags) == MOCK_EFLAGS_OFFSET);
const _: () = assert!(offset_of!(FakeSlap, smack_time) == smack_time_offset());

/// What the fake `GetSmackTime` times a smack of the secondary mode at.
const SECOND_SLAP: f32 = 21.5;

/// A Hot Hand whose smack timer is where the generated layout places it.
#[repr(C)]
struct FakeSlap {
	head: Head,
	_padding: [u8; smack_time_offset() - size_of::<Head>()],
	smack_time: f32,
	first_hit: bool,
}

impl FakeSlap {
	/// A Hot Hand of the class `class`, whose datamap is named `map`.
	fn new(class: *mut sys::ServerClass, map: &'static CStr) -> *mut Self {
		let slot = |field: usize| field / size_of::<usize>();
		let smack_time = vtable_slot!(
			sys::CTFWeaponBaseMelee__bindgen_vtable,
			CTFWeaponBaseMelee_GetSmackTime
		);
		let mut vtable = vec![unexpected_call as *const (); smack_time + 1];

		vtable[GET_DATA_DESC_MAP_SLOT] = datamap as *const ();
		vtable[slot(offset_of!(
			sys::IServerUnknown__bindgen_vtable,
			IServerUnknown_GetNetworkable
		))] = networkable as *const ();
		vtable[smack_time] = get_smack_time as *const ();

		// SAFETY: The vtable holds only function pointers, `unexpected_call`
		// aborts whichever slot reaches it, and the patch only writes slots of
		// the vtable being built.
		let networkable = Box::leak(unsafe {
			mock_vtable::<sys::IServerNetworkable__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).IServerNetworkable_GetServerClass).write(server_class);
					(&raw mut (*vtable).IServerNetworkable_GetEdict).write(edict);
				},
			)
		});

		leak(Self {
			head: Head {
				vtable: vtable.leak().as_ptr(),
				map: weapon_map(map),
				networkable: sys::IServerNetworkable {
					vtable_: networkable,
				},
				_collideable: null_mut(),
				flags: 0,
				owner: u32::MAX,
				class,
				edict: leak(mock_edict(1, false)),
			},
			_padding: [0; smack_time_offset() - size_of::<Head>()],
			smack_time: NO_SMACK,
			first_hit: true,
		})
	}
}

/// The fields of a fake Hot Hand before its smack timer.
#[repr(C)]
struct Head {
	vtable: *const *const (),
	map: *mut sys::datamap_t,
	networkable: sys::IServerNetworkable,
	/// Keeps `flags` at the offset every mock entity has it.
	_collideable: *const c_void,
	flags: c_int,
	owner: u32,
	class: *mut sys::ServerClass,
	edict: *mut sys::edict_t,
}

#[test]
fn a_landing_first_slap_times_the_second_once() {
	export_standard_proxies();
	export(
		Module::Engine,
		ValveEngine::VERSION,
		change_tracking_engine().as_ptr(),
	);

	let scope = ();
	let server = mock_server(&scope);
	let class = slap_class();
	let slap = FakeSlap::new(class, c"CTFSlap");
	let landing = LandingSlap::new(weapon(server, slap)).unwrap();

	assert_eq!(landing.finish(), Ok(true));

	// SAFETY: The fake is leaked, and only this test reads it.
	unsafe {
		assert!(!(*slap).first_hit);
		assert_eq!((*slap).smack_time, SECOND_SLAP);
	}

	// The second slap lands as the first did, and times none.
	// SAFETY: As above; the game sets the timer to -1 as a smack lands.
	unsafe { (*slap).smack_time = NO_SMACK };

	let landing = LandingSlap::new(weapon(server, slap)).unwrap();

	assert_eq!(landing.finish(), Ok(false));
	// SAFETY: As above.
	assert_eq!(unsafe { (*slap).smack_time }, NO_SMACK);
}

unsafe extern "C" fn datamap(entity: *mut sys::CBaseEntity) -> *mut sys::datamap_t {
	// SAFETY: Only fake Hot Hands have this method in their vtables.
	unsafe { (*entity.cast::<FakeSlap>()).head.map }
}

unsafe extern "C" fn edict(this: *const sys::IServerNetworkable) -> *mut sys::edict_t {
	// SAFETY: Callers pass the networkable of a fake Hot Hand.
	unsafe { (*fake_of(this)).head.edict }
}

/// The fake Hot Hand whose networkable is `this`.
fn fake_of(this: *const sys::IServerNetworkable) -> *mut FakeSlap {
	// SAFETY: Callers pass the networkable of a fake Hot Hand, so it starts
	// that far before it.
	unsafe { this.byte_sub(offset_of!(FakeSlap, head.networkable)) }
		.cast_mut()
		.cast()
}

/// `CTFWeaponBaseMelee::GetSmackTime`, which times a smack of the secondary
/// mode at [`SECOND_SLAP`].
unsafe extern "C" fn get_smack_time(_: *mut sys::CTFWeaponBaseMelee, mode: c_int) -> f32 {
	assert_eq!(mode, SECONDARY_MODE);
	SECOND_SLAP
}

unsafe extern "C" fn networkable(entity: *mut sys::IServerUnknown) -> *mut sys::IServerNetworkable {
	// SAFETY: As for `datamap`.
	unsafe { &raw mut (*entity.cast::<FakeSlap>()).head.networkable }
}

#[test]
fn only_hot_hands_whose_smack_lands_are_taken() {
	export_standard_proxies();

	let scope = ();
	let server = mock_server(&scope);
	let class = slap_class();

	// A timed smack that has not landed shows a layout the slap cannot trust.
	let waiting = FakeSlap::new(class, c"CTFSlap");

	// SAFETY: The fake is leaked, and only this test writes it.
	unsafe { (*waiting).smack_time = 3.0 };
	assert_eq!(
		LandingSlap::new(weapon(server, waiting)).unwrap_err(),
		SlapError::Layout
	);

	// Other melee weapons are no Hot Hands.
	let bottle = FakeSlap::new(class, c"CTFBottle");

	assert_eq!(
		LandingSlap::new(weapon(server, bottle)).unwrap_err(),
		SlapError::NotSlap
	);
}

unsafe extern "C" fn server_class(this: *mut sys::IServerNetworkable) -> *mut sys::ServerClass {
	// SAFETY: As for `edict`.
	unsafe { (*fake_of(this)).head.class }
}

/// The server class of the Hot Hand, whose send table networks
/// `m_bFirstHit`.
fn slap_class() -> *mut sys::ServerClass {
	let base = leak(table(c"DT_TFWeaponBaseMelee", &mut []));
	let props = vec![
		table_prop(c"baseclass", 0, base, Some(direct_table)),
		prop(
			c"m_bFirstHit",
			sys::SendPropType_DPT_Int,
			c_int::try_from(offset_of!(FakeSlap, first_hit)).unwrap(),
			PropFlags::UNSIGNED,
			Some(int8_proxy),
		),
	];

	leak(sys::ServerClass {
		m_pNetworkName: c"CTFSlap".as_ptr(),
		m_pTable: leak(table(c"DT_TFWeaponSlap", props.leak())),
		m_pNext: null_mut(),
		m_ClassID: 1,
		m_InstanceBaselineIndex: 0,
	})
}

/// A callback-scoped weapon for a fake one.
fn weapon<'s>(server: Server<'s>, fake: *mut FakeSlap) -> Weapon<'s> {
	// SAFETY: Fake Hot Hands are leaked, so they outlive every scope, and
	// their vtables answer what the wrappers call of a weapon.
	let entity = unsafe { Entity::from_raw(NonNull::new(fake).unwrap().cast()) };

	Weapon::new(server, entity).unwrap()
}

/// The datamap of weapons whose class is named `class`, a melee weapon's.
fn weapon_map(class: &'static CStr) -> *mut sys::datamap_t {
	let mut flags = field(
		c"m_iEFlags",
		sys::_fieldtypes_FIELD_INTEGER,
		MOCK_EFLAGS_OFFSET,
	);

	flags.fieldSizeInBytes = size_of::<c_int>() as c_int;

	let mut owner = field(
		c"m_hOwner",
		sys::_fieldtypes_FIELD_EHANDLE,
		offset_of!(FakeSlap, head.owner),
	);

	owner.fieldSize = 1;
	owner.fieldSizeInBytes = 4;

	let base = data_map(c"CBaseEntity", vec![flags], null_mut());
	let combat = data_map(c"CBaseCombatWeapon", vec![owner], base);
	let weapon = data_map(c"CTFWeaponBase", vec![], combat);
	let melee = data_map(c"CTFWeaponBaseMelee", vec![], weapon);

	data_map(class, vec![], melee)
}
