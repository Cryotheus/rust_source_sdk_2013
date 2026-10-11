//! Tests of what TF2 weapons are and are doing, through fake weapons' classes,
//! datamaps and networked variables.

use super::*;
use crate::Server;
use crate::datatables::PropFlags;
use crate::entities::Entity;

use crate::test_support::datatables::{
	direct_table, int8_proxy, int32_proxy, prop, table, table_prop,
};

use crate::test_support::entities::MOCK_EFLAGS_OFFSET;
use crate::test_support::interfaces::server_game_dll::export_standard_proxies;
use crate::test_support::leak;
use crate::test_support::server::mock_server;
use crate::tf2::weapons::WeaponError;
use sdk_raw::entities::{EFL_KILLME, GET_DATA_DESC_MAP_SLOT};
use sdk_raw::test_support::edicts::mock_edict;
use sdk_raw::test_support::entities::{data_map, field};
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use sdk_raw::vtable_slot;
use std::ffi::c_void;
use std::mem::offset_of;
use std::ptr::{NonNull, null_mut};

const _: () = assert!(offset_of!(FakeWeapon, flags) == MOCK_EFLAGS_OFFSET);

/// A weapon whose datamaps and send tables describe the fields of its class.
#[repr(C)]
struct FakeWeapon {
	vtable: *const *const (),
	map: *mut sys::datamap_t,
	networkable: sys::IServerNetworkable,
	/// Keeps `flags` at the offset every mock entity has it.
	_collideable: *const c_void,
	flags: c_int,
	owner: u32,
	class: *mut sys::ServerClass,
	edict: *mut sys::edict_t,
	/// What `GetWeaponID` returns.
	id: c_int,

	// Networked variables.
	ammo_type: c_int,
	charge_begin_time: f32,
	crit_fire: u8,
}

impl FakeWeapon {
	/// A weapon of the class `class` whose datamap is `map`, reporting `id`.
	fn new(class: *mut sys::ServerClass, map: *mut sys::datamap_t, id: WeaponId) -> *mut Self {
		let slot = |field: usize| field / size_of::<usize>();
		let weapon_id = vtable_slot!(
			sys::CTFWeaponBase__bindgen_vtable,
			CTFWeaponBase_GetWeaponID
		);
		let initial = vtable_slot!(
			sys::CTFWeaponBase__bindgen_vtable,
			CTFWeaponBase_GetInitialAfterburnDuration
		);
		let added = vtable_slot!(
			sys::CTFWeaponBase__bindgen_vtable,
			CTFWeaponBase_GetAfterburnRateOnHit
		);
		let mut vtable = vec![unexpected_call as *const (); weapon_id.max(initial).max(added) + 1];
		vtable[initial] = afterburn_initial as *const ();
		vtable[added] = afterburn_added as *const ();

		vtable[GET_DATA_DESC_MAP_SLOT] = datamap as *const ();
		vtable[slot(offset_of!(
			sys::IServerUnknown__bindgen_vtable,
			IServerUnknown_GetNetworkable
		))] = networkable as *const ();
		vtable[weapon_id] = weapon_id_fn as *const ();

		// SAFETY: The vtable holds only function pointers, `unexpected_call`
		// aborts whichever slot reaches it, and the patch only writes slots of the
		// vtable being built.
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
			vtable: vtable.leak().as_ptr(),
			map,
			networkable: sys::IServerNetworkable {
				vtable_: networkable,
			},
			_collideable: null_mut(),
			flags: 0,
			owner: u32::MAX,
			class,
			edict: leak(mock_edict(1, false)),
			id: id.to_raw(),
			ammo_type: -1,
			charge_begin_time: 0.0,
			crit_fire: 0,
		})
	}
}

/// The server classes of a flame thrower, a sapper and a bottle, whose send
/// tables describe their fields.
fn classes() -> [*mut sys::ServerClass; 3] {
	let offset = |offset: usize| c_int::try_from(offset).unwrap();
	let int = |name, at| {
		prop(
			name,
			sys::SendPropType_DPT_Int,
			offset(at),
			PropFlags::default(),
			Some(int32_proxy),
		)
	};

	let local = vec![int(
		c"m_iPrimaryAmmoType",
		offset_of!(FakeWeapon, ammo_type),
	)];
	let local = leak(table(c"DT_LocalWeaponData", local.leak()));
	let combat = vec![table_prop(c"LocalWeaponData", 0, local, Some(direct_table))];
	let combat = leak(table(c"DT_BaseCombatWeapon", combat.leak()));
	let base = vec![table_prop(c"baseclass", 0, combat, Some(direct_table))];
	let base = leak(table(c"DT_TFWeaponBase", base.leak()));
	let derived = |name, props: Vec<sys::SendProp>| {
		let mut all = vec![table_prop(c"baseclass", 0, base, Some(direct_table))];

		all.extend(props);
		leak(table(name, all.leak()))
	};

	let flame_thrower = derived(
		c"DT_TFFlameThrower",
		vec![
			prop(
				c"m_bCritFire",
				sys::SendPropType_DPT_Int,
				offset(offset_of!(FakeWeapon, crit_fire)),
				PropFlags::UNSIGNED,
				Some(int8_proxy),
			),
			prop(
				c"m_flChargeBeginTime",
				sys::SendPropType_DPT_Float,
				offset(offset_of!(FakeWeapon, charge_begin_time)),
				PropFlags::default(),
				Some(int32_proxy),
			),
		],
	);
	let sapper = derived(c"DT_TFWeaponSapper", vec![]);
	let bottle = derived(c"DT_TFWeaponBottle", vec![]);

	let class = |name: &'static CStr, table, id| {
		leak(sys::ServerClass {
			m_pNetworkName: name.as_ptr(),
			m_pTable: table,
			m_pNext: null_mut(),
			m_ClassID: id,
			m_InstanceBaselineIndex: 0,
		})
	};

	[
		class(c"CTFFlameThrower", flame_thrower, 1),
		class(c"CTFWeaponSapper", sapper, 2),
		class(c"CTFBottle", bottle, 3),
	]
}

unsafe extern "C" fn datamap(entity: *mut sys::CBaseEntity) -> *mut sys::datamap_t {
	// SAFETY: Only fake weapons have this method in their vtables.
	unsafe { (*entity.cast::<FakeWeapon>()).map }
}

unsafe extern "C" fn edict(this: *const sys::IServerNetworkable) -> *mut sys::edict_t {
	// SAFETY: Callers pass the networkable of a fake weapon.
	unsafe { (*fake_of(this)).edict }
}

/// The fake weapon whose networkable is `this`.
fn fake_of(this: *const sys::IServerNetworkable) -> *mut FakeWeapon {
	// SAFETY: Callers pass the networkable of a fake weapon, so the weapon
	// starts that far before it.
	unsafe { this.byte_sub(offset_of!(FakeWeapon, networkable)) }
		.cast_mut()
		.cast()
}

#[test]
fn ids_and_definitions_keep_tf2s_numbers() {
	assert_eq!(WeaponId::NONE.to_raw(), 0);
	assert_eq!(WeaponId::FLAMETHROWER.to_raw(), 25);
	assert_eq!(WeaponId::BUILDER.to_raw(), 49);
	assert_eq!(WeaponId::FLAME_BALL.to_raw(), raw::TF_WEAPON_COUNT - 1);
	assert_eq!(WeaponId::from_raw(58), WeaponId::FLAREGUN);

	// An id the game adds later is kept.
	assert_eq!(WeaponId::from_raw(raw::TF_WEAPON_COUNT).to_raw(), 110);

	let definitions = [
		(ItemDefinitionIndex::HOMEWRECKER, 153),
		(ItemDefinitionIndex::CONCHEROR, 354),
		(ItemDefinitionIndex::COW_MANGLER_5000, 441),
		(ItemDefinitionIndex::MAUL, 466),
		(ItemDefinitionIndex::PHLOGISTINATOR, 594),
		(ItemDefinitionIndex::NEON_ANNIHILATOR, 813),
		(ItemDefinitionIndex::NEON_ANNIHILATOR_GENUINE, 834),
		(ItemDefinitionIndex::DRAGONS_FURY, 1178),
	];

	for (definition, index) in definitions {
		assert_eq!(Some(definition), ItemDefinitionIndex::new(index));
	}
}

/// The datamap of weapons of the class named `class`.
fn map(class: &'static CStr) -> *mut sys::datamap_t {
	let mut flags = field(
		c"m_iEFlags",
		sys::_fieldtypes_FIELD_INTEGER,
		MOCK_EFLAGS_OFFSET,
	);

	flags.fieldSizeInBytes = size_of::<c_int>() as c_int;

	let mut owner = field(
		c"m_hOwner",
		sys::_fieldtypes_FIELD_EHANDLE,
		offset_of!(FakeWeapon, owner),
	);

	owner.fieldSize = 1;
	owner.fieldSizeInBytes = 4;

	let base = data_map(c"CBaseEntity", vec![flags], null_mut());
	let combat = data_map(c"CBaseCombatWeapon", vec![owner], base);
	let weapon = data_map(c"CTFWeaponBase", vec![], combat);

	data_map(class, vec![], weapon)
}

unsafe extern "C" fn networkable(entity: *mut sys::IServerUnknown) -> *mut sys::IServerNetworkable {
	// SAFETY: As for `datamap`.
	unsafe { &raw mut (*entity.cast::<FakeWeapon>()).networkable }
}

unsafe extern "C" fn server_class(this: *mut sys::IServerNetworkable) -> *mut sys::ServerClass {
	// SAFETY: As for `edict`.
	unsafe { (*fake_of(this)).class }
}

/// A callback-scoped weapon for a fake one.
fn weapon<'s>(server: Server<'s>, fake: *mut FakeWeapon) -> Weapon<'s> {
	// SAFETY: Fake weapons are leaked, so they outlive every scope, and their
	// vtables answer what the wrappers call of a weapon.
	let entity = unsafe { Entity::from_raw(NonNull::new(fake).unwrap().cast()) };

	Weapon::new(server, entity).unwrap()
}

/// `CTFWeaponBase::GetWeaponID`.
unsafe extern "C" fn weapon_id_fn(this: *const sys::CTFWeaponBase) -> c_int {
	// SAFETY: As for `datamap`.
	unsafe { (*this.cast::<FakeWeapon>()).id }
}

#[test]
fn weapons_report_what_they_are_and_are_doing() {
	export_standard_proxies();

	let scope = ();
	let server = mock_server(&scope);
	let [flame_thrower_class, sapper_class, bottle_class] = classes();
	let flame_thrower = FakeWeapon::new(
		flame_thrower_class,
		map(c"CTFFlameThrower"),
		WeaponId::FLAMETHROWER,
	);
	let sapper = FakeWeapon::new(sapper_class, map(c"CTFWeaponBuilder"), WeaponId::BUILDER);
	let bottle = FakeWeapon::new(bottle_class, map(c"CTFBottle"), WeaponId::BOTTLE);

	// SAFETY: The fake weapons are leaked, and only this test writes them.
	unsafe {
		(*flame_thrower).ammo_type = sdk_raw::tf2::ammo::AMMO_PRIMARY;
		(*flame_thrower).charge_begin_time = 12.5;
		(*flame_thrower).crit_fire = 1;
	}

	let flames = weapon(server, flame_thrower);

	assert_eq!(flames.weapon_id().unwrap(), WeaponId::FLAMETHROWER);
	assert_eq!(flames.afterburn_duration().unwrap(), (25.0, 0.4));
	assert_eq!(flames.ammo_type().unwrap(), Some(AmmoType::Primary));
	assert_eq!(flames.charge_begin_time().unwrap(), 12.5);
	assert!(flames.is_firing_crits().unwrap());
	assert!(!flames.is_sapper());

	// A sapper reports the toolboxes' id.
	let sapper = weapon(server, sapper);

	assert_eq!(sapper.weapon_id().unwrap(), WeaponId::BUILDER);
	assert!(sapper.is_sapper());

	// A bottle fires no ammo, never charges and has no flames.
	let bottle_weapon = weapon(server, bottle);

	assert_eq!(bottle_weapon.weapon_id().unwrap(), WeaponId::BOTTLE);
	assert_eq!(bottle_weapon.ammo_type().unwrap(), None);
	assert!(matches!(
		bottle_weapon.charge_begin_time(),
		Err(WeaponError::NetProp(_))
	));
	assert!(matches!(
		bottle_weapon.is_firing_crits(),
		Err(WeaponError::NetProp(_))
	));

	// SAFETY: As above.
	unsafe { (*bottle).flags |= EFL_KILLME };
	assert!(matches!(
		bottle_weapon.afterburn_duration(),
		Err(WeaponError::MarkedForDeletion)
	));

	assert!(matches!(
		bottle_weapon.weapon_id(),
		Err(WeaponError::MarkedForDeletion)
	));
	assert!(matches!(
		bottle_weapon.ammo_type(),
		Err(WeaponError::MarkedForDeletion)
	));
}

/// Const getter mock returns a weapon-dependent value to detect bad this/slots.
unsafe extern "C" fn afterburn_initial(this: *const sys::CTFWeaponBase) -> f32 {
	// SAFETY: Installed only on the leaked fake weapon's vtable.
	unsafe { (*this.cast::<FakeWeapon>()).id as f32 }
}
unsafe extern "C" fn afterburn_added(_this: *const sys::CTFWeaponBase) -> f32 {
	0.4
}
