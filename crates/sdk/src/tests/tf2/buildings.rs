//! Tests of TF2's buildings through fake buildings' and players' networked
//! variables, their `CBaseObject` methods and inputs, and the players'
//! VScript methods.

use super::*;
use crate::Module;
use crate::datatables::PropFlags;
use crate::interfaces::ServerTools;

use crate::test_support::datatables::{
	direct_table, int8_proxy, int32_proxy, prop, table, table_prop,
};

use crate::test_support::entities::MOCK_EFLAGS_OFFSET;
use crate::test_support::interfaces::server_game_dll::export_standard_proxies;
use crate::test_support::leak;
use crate::test_support::server::{export, mock_server, null_server};

use crate::test_support::tf2::script_binding::{
	SCRIPT_DESCRIPTION_SLOT, class_description, member_binding, script_description,
	set_script_description,
};

use sdk_raw::entities::datamap::FTYPEDESC_INPUT;

use sdk_raw::entities::{
	ACCEPT_INPUT_SLOT, EFL_KILLME, GET_DATA_DESC_MAP_SLOT, NUM_NETWORKED_EHANDLE_BITS,
};

use sdk_raw::test_support::edicts::mock_edict;
use sdk_raw::test_support::entities::{data_map, field};
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use sdk_raw::vtable_slot;
use std::cell::{Cell, RefCell};
use std::ffi::{CString, c_char, c_void};
use std::mem::offset_of;
use std::ptr::{NonNull, null_mut};

/// An input a fake entity received: the target's address, and the input's
/// name, activator's and caller's addresses, and integer.
type Input = (usize, CString, usize, usize, c_int);

const _: () = assert!(offset_of!(FakeEntity, flags) == MOCK_EFLAGS_OFFSET);

/// The players' `bool` VScript methods without parameters, which
/// [`answer_adapter`] adapts.
const ANSWERED: [&str; 3] = ["TryToPickupBuilding", "IsSapping", "IsPlacingSapper"];

/// A null handle, in a fake entity's fields.
const NULL: u32 = u32::MAX;

/// A building, player, or other entity, whose datamaps and send tables
/// describe the fields of its class.
#[repr(C)]
struct FakeEntity {
	vtable: *const *const (),
	map: *mut sys::datamap_t,
	networkable: sys::IServerNetworkable,
	/// Keeps `flags` at the offset every mock entity has it.
	_collideable: *const c_void,
	flags: c_int,
	handle: u32,
	class_name: &'static CStr,
	class: *mut sys::ServerClass,
	edict: *mut sys::edict_t,
	/// What `IsDying` returns.
	dying: bool,
	/// What `GetMaxUpgradeLevel` returns.
	max_level: c_int,

	// `CBaseObject`'s networked variables.
	object_type: c_int,
	mode: c_int,
	object_flags: c_int,
	team: c_int,
	level: c_int,
	highest_level: c_int,
	upgrade_metal: c_int,
	upgrade_metal_required: c_int,
	construction: f32,
	builder: u32,
	built_on: u32,
	carried: u8,
	constructing: u8,
	disabled: u8,
	disposable: u8,
	mini: u8,
	placing: u8,
	plasma: u8,
	redeploying: u8,
	sapped: u8,
	map_placed: u8,

	// The subclasses' networked variables.
	state: c_int,
	shells: c_int,
	rockets: c_int,
	kills: c_int,
	assists: c_int,
	shield: c_int,
	enemy: u32,
	wrangled: u8,
	metal: c_int,
	recharge_time: f32,
	recharge_duration: f32,
	times_used: c_int,
	yaw_to_exit: f32,

	// `CTFPlayer`'s.
	carried_object: u32,
}

thread_local! {
	/// The fake entities, in the order the entity list holds them.
	static ENTITIES: RefCell<Vec<*mut FakeEntity>> = const { RefCell::new(Vec::new()) };

	/// `gEntList`, which `ServerTools::entity_by_handle` reads.
	static ENTITY_LIST: Cell<*mut sys::CGlobalEntityList> = const { Cell::new(null_mut()) };

	/// The inputs fake entities received: the target and the input's name,
	/// activator, caller and integer.
	static INPUTS: RefCell<Vec<Input>> = const { RefCell::new(Vec::new()) };

	/// The calls of buildings' `DetonateObject` and `DestroyObject`, and of
	/// players' VScript methods, with the entity called.
	static CALLS: RefCell<Vec<(&'static str, usize)>> = const { RefCell::new(Vec::new()) };

	/// The calls of buildings' `StartPlacement` and `StartBuilding`, with the
	/// building and the player passed.
	static STARTS: RefCell<Vec<(&'static str, usize, usize)>> = const { RefCell::new(Vec::new()) };

	/// What players' `bool` VScript methods return.
	static ANSWER: Cell<bool> = const { Cell::new(false) };
}

/// The server classes of the fake entities, whose send tables describe their
/// fields.
struct Classes {
	dispenser: *mut sys::ServerClass,
	player: *mut sys::ServerClass,
	prop: *mut sys::ServerClass,
	sapper: *mut sys::ServerClass,
	sentry: *mut sys::ServerClass,
	teleporter: *mut sys::ServerClass,
}

impl Classes {
	fn new() -> Self {
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
		let float = |name, at| {
			prop(
				name,
				sys::SendPropType_DPT_Float,
				offset(at),
				PropFlags::default(),
				Some(int32_proxy),
			)
		};
		let flag = |name, at| {
			prop(
				name,
				sys::SendPropType_DPT_Int,
				offset(at),
				PropFlags::UNSIGNED,
				Some(int8_proxy),
			)
		};
		let handle = |name, at| {
			let mut prop = prop(
				name,
				sys::SendPropType_DPT_Int,
				offset(at),
				PropFlags::UNSIGNED,
				Some(handle_proxy),
			);

			prop.m_nBits = NUM_NETWORKED_EHANDLE_BITS as c_int;
			prop
		};

		let base_props = vec![
			int(c"m_iObjectType", offset_of!(FakeEntity, object_type)),
			int(c"m_iObjectMode", offset_of!(FakeEntity, mode)),
			int(c"m_fObjectFlags", offset_of!(FakeEntity, object_flags)),
			int(c"m_iTeamNum", offset_of!(FakeEntity, team)),
			int(c"m_iUpgradeLevel", offset_of!(FakeEntity, level)),
			int(
				c"m_iHighestUpgradeLevel",
				offset_of!(FakeEntity, highest_level),
			),
			int(c"m_iUpgradeMetal", offset_of!(FakeEntity, upgrade_metal)),
			int(
				c"m_iUpgradeMetalRequired",
				offset_of!(FakeEntity, upgrade_metal_required),
			),
			float(
				c"m_flPercentageConstructed",
				offset_of!(FakeEntity, construction),
			),
			handle(c"m_hBuilder", offset_of!(FakeEntity, builder)),
			handle(c"m_hBuiltOnEntity", offset_of!(FakeEntity, built_on)),
			flag(c"m_bCarried", offset_of!(FakeEntity, carried)),
			flag(c"m_bBuilding", offset_of!(FakeEntity, constructing)),
			flag(c"m_bDisabled", offset_of!(FakeEntity, disabled)),
			flag(c"m_bDisposableBuilding", offset_of!(FakeEntity, disposable)),
			flag(c"m_bMiniBuilding", offset_of!(FakeEntity, mini)),
			flag(c"m_bPlacing", offset_of!(FakeEntity, placing)),
			flag(c"m_bPlasmaDisable", offset_of!(FakeEntity, plasma)),
			flag(c"m_bCarryDeploy", offset_of!(FakeEntity, redeploying)),
			flag(c"m_bHasSapper", offset_of!(FakeEntity, sapped)),
			flag(c"m_bWasMapPlaced", offset_of!(FakeEntity, map_placed)),
		];
		let base_table = leak(table(c"DT_BaseObject", base_props.leak()));
		let derived = |name, props: Vec<sys::SendProp>| {
			let mut all = vec![table_prop(c"baseclass", 0, base_table, Some(direct_table))];

			all.extend(props);
			leak(table(name, all.leak()))
		};

		let local_props = vec![
			int(c"m_iKills", offset_of!(FakeEntity, kills)),
			int(c"m_iAssists", offset_of!(FakeEntity, assists)),
		];
		let local_table = leak(table(c"DT_SentrygunLocalData", local_props.leak()));

		let sentry_table = derived(
			c"DT_ObjectSentrygun",
			vec![
				int(c"m_iAmmoShells", offset_of!(FakeEntity, shells)),
				int(c"m_iAmmoRockets", offset_of!(FakeEntity, rockets)),
				int(c"m_iState", offset_of!(FakeEntity, state)),
				flag(c"m_bPlayerControlled", offset_of!(FakeEntity, wrangled)),
				int(c"m_nShieldLevel", offset_of!(FakeEntity, shield)),
				handle(c"m_hEnemy", offset_of!(FakeEntity, enemy)),
				table_prop(c"SentrygunLocalData", 0, local_table, Some(direct_table)),
			],
		);
		let dispenser_table = derived(
			c"DT_ObjectDispenser",
			vec![
				int(c"m_iState", offset_of!(FakeEntity, state)),
				int(c"m_iAmmoMetal", offset_of!(FakeEntity, metal)),
			],
		);
		let teleporter_table = derived(
			c"DT_ObjectTeleporter",
			vec![
				int(c"m_iState", offset_of!(FakeEntity, state)),
				float(c"m_flRechargeTime", offset_of!(FakeEntity, recharge_time)),
				float(
					c"m_flCurrentRechargeDuration",
					offset_of!(FakeEntity, recharge_duration),
				),
				int(c"m_iTimesUsed", offset_of!(FakeEntity, times_used)),
				float(c"m_flYawToExit", offset_of!(FakeEntity, yaw_to_exit)),
			],
		);
		let sapper_table = derived(c"DT_ObjectSapper", vec![]);
		let player_props = vec![handle(
			c"m_hCarriedObject",
			offset_of!(FakeEntity, carried_object),
		)];
		let player_table = leak(table(c"DT_TFPlayer", player_props.leak()));
		let prop_table = leak(table(c"DT_DynamicProp", &mut []));

		let class = |name: &'static CStr, table, id| {
			leak(sys::ServerClass {
				m_pNetworkName: name.as_ptr(),
				m_pTable: table,
				m_pNext: null_mut(),
				m_ClassID: id,
				m_InstanceBaselineIndex: 0,
			})
		};

		Self {
			dispenser: class(c"CObjectDispenser", dispenser_table, 1),
			player: class(c"CTFPlayer", player_table, 2),
			prop: class(c"CDynamicProp", prop_table, 3),
			sapper: class(c"CObjectSapper", sapper_table, 4),
			sentry: class(c"CObjectSentrygun", sentry_table, 5),
			teleporter: class(c"CObjectTeleporter", teleporter_table, 6),
		}
	}
}

/// The datamaps of the fake entities' classes.
struct Maps {
	cart: *mut sys::datamap_t,
	dispenser: *mut sys::datamap_t,
	player: *mut sys::datamap_t,
	prop: *mut sys::datamap_t,
	sapper: *mut sys::datamap_t,
	sentry: *mut sys::datamap_t,
	teleporter: *mut sys::datamap_t,
}

impl Maps {
	fn new() -> Self {
		let mut flags = field(
			c"m_iEFlags",
			sys::_fieldtypes_FIELD_INTEGER,
			MOCK_EFLAGS_OFFSET,
		);

		flags.fieldSizeInBytes = size_of::<c_int>() as c_int;

		let input = |name: &'static CStr, field_type| {
			// SAFETY: Zero is valid for every field of `typedescription_t`.
			let mut input: sys::typedescription_t = unsafe { std::mem::zeroed() };

			input.fieldType = field_type;
			input.externalName = name.as_ptr();
			input.flags = FTYPEDESC_INPUT;
			input
		};

		let base = data_map(c"CBaseEntity", vec![flags], null_mut());
		let object = data_map(
			c"CBaseObject",
			vec![
				input(c"SetSolidToPlayer", sys::_fieldtypes_FIELD_INTEGER),
				input(c"SetBuilder", sys::_fieldtypes_FIELD_STRING),
				input(c"Show", sys::_fieldtypes_FIELD_VOID),
				input(c"Hide", sys::_fieldtypes_FIELD_VOID),
				input(c"Enable", sys::_fieldtypes_FIELD_VOID),
				input(c"Disable", sys::_fieldtypes_FIELD_VOID),
			],
			base,
		);
		let dispenser = data_map(c"CObjectDispenser", vec![], object);
		let upgrade = data_map(c"CBaseObjectUpgrade", vec![], object);

		Self {
			cart: data_map(c"CObjectCartDispenser", vec![], dispenser),
			dispenser,
			player: data_map(c"CTFPlayer", vec![], data_map(c"CBasePlayer", vec![], base)),
			prop: data_map(c"CDynamicProp", vec![], base),
			sapper: data_map(c"CObjectSapper", vec![], upgrade),
			sentry: data_map(c"CObjectSentrygun", vec![], object),
			teleporter: data_map(c"CObjectTeleporter", vec![], object),
		}
	}
}

/// Mock interfaces exported on this thread, and fake entities of every
/// class.
struct World {
	cart: *mut FakeEntity,
	dispenser: *mut FakeEntity,
	engineer: *mut FakeEntity,
	/// An entity that is neither a building nor a player.
	prop: *mut FakeEntity,
	/// A sapper on the sentry, built by the spy.
	sapper: *mut FakeEntity,
	sentry: *mut FakeEntity,
	spy: *mut FakeEntity,
	teleporter: *mut FakeEntity,
}

impl World {
	fn new() -> Self {
		let slot = |field: usize| field / size_of::<usize>();
		let is_dying = vtable_slot!(sys::CBaseObject__bindgen_vtable, CBaseObject_IsDying);
		let detonate = vtable_slot!(sys::CBaseObject__bindgen_vtable, CBaseObject_DetonateObject);
		let destroy = vtable_slot!(sys::CBaseObject__bindgen_vtable, CBaseObject_DestroyObject);
		let max_level = vtable_slot!(
			sys::CBaseObject__bindgen_vtable,
			CBaseObject_GetMaxUpgradeLevel
		);
		let placement = vtable_slot!(sys::CBaseObject__bindgen_vtable, CBaseObject_StartPlacement);
		let building = vtable_slot!(sys::CBaseObject__bindgen_vtable, CBaseObject_StartBuilding);
		let slots = [
			is_dying,
			detonate,
			destroy,
			max_level,
			placement,
			building,
			SCRIPT_DESCRIPTION_SLOT,
		]
		.into_iter()
		.max()
		.unwrap()
			+ 1;
		let mut vtable = vec![unexpected_call as *const (); slots];

		vtable[GET_DATA_DESC_MAP_SLOT] = datamap as *const ();
		vtable[ACCEPT_INPUT_SLOT] = accept_input as *const ();
		vtable[slot(offset_of!(
			sys::IServerUnknown__bindgen_vtable,
			IServerUnknown_GetNetworkable
		))] = networkable as *const ();
		vtable[slot(offset_of!(
			sys::IServerEntity__bindgen_vtable,
			IServerEntity_GetRefEHandle
		))] = handle as *const ();
		vtable[is_dying] = is_dying_fn as *const ();
		vtable[detonate] = detonate_object as *const ();
		vtable[destroy] = destroy_object as *const ();
		vtable[max_level] = max_upgrade_level as *const ();
		vtable[placement] = start_placement as *const ();
		vtable[building] = start_building as *const ();
		vtable[SCRIPT_DESCRIPTION_SLOT] = script_description as *const ();

		let vtable = vtable.leak().as_ptr();

		// SAFETY: The vtable holds only function pointers, `unexpected_call`
		// aborts whichever slot reaches it, and the patch only writes slots of
		// the vtable being built.
		let networkable = Box::leak(unsafe {
			mock_vtable::<sys::IServerNetworkable__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).IServerNetworkable_GetClassName).write(class_name);
					(&raw mut (*vtable).IServerNetworkable_GetServerClass).write(server_class);
					(&raw mut (*vtable).IServerNetworkable_GetEdict).write(edict);
				},
			)
		});

		let maps = Maps::new();
		let classes = Classes::new();

		export_interfaces();
		export_bindings();
		ENTITIES.take();
		INPUTS.take();
		CALLS.take();
		STARTS.take();

		let spawn = |index: u32, class_name, map, class| {
			let fake = leak(FakeEntity {
				vtable,
				map,
				networkable: sys::IServerNetworkable {
					vtable_: networkable,
				},
				_collideable: null_mut(),
				flags: 0,
				handle: index | 1 << 16,
				class_name,
				class,
				edict: leak(mock_edict(index.cast_signed(), false)),
				dying: false,
				max_level: 3,
				object_type: 0,
				mode: 0,
				object_flags: 0,
				team: 0,
				level: 1,
				highest_level: 1,
				upgrade_metal: 0,
				upgrade_metal_required: 200,
				construction: 1.0,
				builder: NULL,
				built_on: NULL,
				carried: 0,
				constructing: 0,
				disabled: 0,
				disposable: 0,
				mini: 0,
				placing: 0,
				plasma: 0,
				redeploying: 0,
				sapped: 0,
				map_placed: 0,
				state: 0,
				shells: 0,
				rockets: 0,
				kills: 0,
				assists: 0,
				shield: 0,
				enemy: NULL,
				wrangled: 0,
				metal: 0,
				recharge_time: 0.0,
				recharge_duration: 0.0,
				times_used: 0,
				yaw_to_exit: 0.0,
				carried_object: NULL,
			});

			// SAFETY: The entity list is a leaked, zeroed `CGlobalEntityList`,
			// and the slots are within its `m_EntPtrArray`.
			unsafe {
				let info = (&raw mut (*ENTITY_LIST.get())._base.m_EntPtrArray)
					.cast::<sys::CEntInfo>()
					.add(index as usize);

				(&raw mut (*info).m_pEntity).write(fake.cast());
				(&raw mut (*info).m_SerialNumber).write(1);
			}

			ENTITIES.with_borrow_mut(|entities| entities.push(fake));
			fake
		};

		let world = Self {
			engineer: spawn(1, c"player", maps.player, classes.player),
			spy: spawn(2, c"player", maps.player, classes.player),
			sentry: spawn(3, c"obj_sentrygun", maps.sentry, classes.sentry),
			dispenser: spawn(4, c"obj_dispenser", maps.dispenser, classes.dispenser),
			teleporter: spawn(5, c"obj_teleporter", maps.teleporter, classes.teleporter),
			sapper: spawn(6, c"obj_attachment_sapper", maps.sapper, classes.sapper),
			prop: spawn(7, c"prop_dynamic", maps.prop, classes.prop),
			cart: spawn(8, c"mapobj_cart_dispenser", maps.cart, classes.dispenser),
		};

		// SAFETY: Fake entities are leaked, and the mock game is not running
		// while the test writes their fields.
		unsafe {
			let engineer = (*world.engineer).handle;

			for (building, kind) in [
				(world.sentry, raw::OBJ_SENTRYGUN),
				(world.dispenser, raw::OBJ_DISPENSER),
				(world.teleporter, raw::OBJ_TELEPORTER),
				(world.cart, raw::OBJ_DISPENSER),
			] {
				(*building).object_type = kind;
				(*building).team = 3;
			}

			for building in [world.sentry, world.dispenser, world.teleporter] {
				(*building).builder = engineer;
			}

			(*world.sentry).sapped = 1;
			(*world.sapper).object_type = raw::OBJ_ATTACHMENT_SAPPER;
			(*world.sapper).team = 2;
			(*world.sapper).builder = (*world.spy).handle;
			(*world.sapper).built_on = (*world.sentry).handle;
			(*world.sapper).max_level = 1;
		}

		world
	}

	/// Sets a field of a fake entity, as the game would have.
	fn set(&self, fake: *mut FakeEntity, change: impl FnOnce(&mut FakeEntity)) {
		// SAFETY: As for the fields set in `new`.
		change(unsafe { &mut *fake });
	}
}

/// `CBaseEntity::AcceptInput`, which notes the input.
unsafe extern "C" fn accept_input(
	this: *mut sys::CBaseEntity,
	name: *const c_char,
	activator: *mut sys::CBaseEntity,
	caller: *mut sys::CBaseEntity,
	value: *mut sys::variant_t,
	_: c_int,
) -> bool {
	// SAFETY: The wrappers pass a NUL-terminated name and a live `variant_t`,
	// whose integer member covers its first bytes.
	let input = unsafe {
		(
			this.addr(),
			CStr::from_ptr(name).to_owned(),
			activator.addr(),
			caller.addr(),
			value.cast::<c_int>().read(),
		)
	};

	INPUTS.with_borrow_mut(|inputs| inputs.push(input));
	true
}

/// Adapts the players' `bool` VScript method named `ANSWERED[METHOD]`,
/// without parameters, which notes the call and returns [`ANSWER`].
unsafe extern "C" fn answer_adapter<const METHOD: usize>(
	_: sys::ScriptFunctionBindingStorageType_t,
	object: *mut c_void,
	_: *mut sys::ScriptVariant_t,
	count: c_int,
	result: *mut sys::ScriptVariant_t,
) -> bool {
	assert_eq!(count, 0);
	CALLS.with_borrow_mut(|calls| calls.push((ANSWERED[METHOD], object.addr())));

	// SAFETY: `call` passes a writable result for a `bool` method.
	unsafe { result.write(boolean(ANSWER.get())) };
	true
}

/// A callback-scoped building for a fake one.
fn building<'s>(server: Server<'s>, fake: *mut FakeEntity) -> Building<'s> {
	Building::new(server, entity(fake)).unwrap()
}

#[test]
fn building_classes_are_listed_in_the_order_of_their_vtables() {
	for (index, class) in BuildingClass::ALL.into_iter().enumerate() {
		assert_eq!(class as usize, index);
	}

	let kinds = BuildingClass::ALL.map(BuildingClass::kind);

	assert_eq!(
		kinds,
		[
			BuildingKind::Dispenser,
			BuildingKind::Dispenser,
			BuildingKind::Dispenser,
			BuildingKind::Dispenser,
			BuildingKind::Sapper,
			BuildingKind::Sentry,
			BuildingKind::Teleporter,
		]
	);
	assert_eq!(BuildingClass::Sentry.name(), "CObjectSentrygun");
	assert_eq!(BuildingClass::RobotDispenser.name(), "CRobotDispenser");
}

#[test]
fn building_inputs_are_sent_to_the_building() {
	let world = World::new();
	let scope = ();
	let sentry = building(mock_server(&scope), world.sentry);
	let at = world.sentry.addr();

	sentry.disable().unwrap();
	sentry.enable().unwrap();
	sentry.hide().unwrap();
	sentry.show().unwrap();
	sentry.set_solid_to_players(SolidToPlayers::No).unwrap();
	sentry
		.set_solid_to_players(SolidToPlayers::Default)
		.unwrap();

	assert_eq!(
		INPUTS.take(),
		[
			(at, c"Disable".to_owned(), at, at, 0),
			(at, c"Enable".to_owned(), at, at, 0),
			(at, c"Hide".to_owned(), at, at, 0),
			(at, c"Show".to_owned(), at, at, 0),
			(
				at,
				c"SetSolidToPlayer".to_owned(),
				at,
				at,
				raw::SOLID_TO_PLAYER_NO
			),
			(
				at,
				c"SetSolidToPlayer".to_owned(),
				at,
				at,
				raw::SOLID_TO_PLAYER_USE_DEFAULT
			),
		]
	);

	world.set(world.sentry, |sentry| sentry.flags |= EFL_KILLME);
	assert!(matches!(
		sentry.enable(),
		Err(BuildingError::Input(InputError::MarkedForDeletion))
	));
	assert!(INPUTS.take().is_empty());
}

#[test]
fn building_vtables_are_only_searched_for_on_tf2() {
	let scope = ();

	assert!(matches!(
		building_vtables(null_server(Game::SourceSdk2013, &scope)),
		Err(BuildingVtableError::WrongGame)
	));

	// The tests' own executable has no building class.
	assert!(matches!(
		building_vtables(mock_server(&scope)),
		Err(BuildingVtableError::NotFound(BuildingClass::CartDispenser))
	));

	// Nor in a snapshot other searches share.
	let targets = ClassTargets::load(mock_server(&scope)).unwrap();

	assert!(matches!(
		BuildingVtables::find(&targets),
		Err(BuildingVtableError::NotFound(BuildingClass::CartDispenser))
	));
}

#[test]
fn buildings_are_found_by_class_and_builder() {
	let world = World::new();
	let scope = ();
	let server = mock_server(&scope);
	let found = |buildings: Vec<Building<'_>>| {
		buildings
			.into_iter()
			.map(|building| building.entity().as_ptr().addr())
			.collect::<Vec<_>>()
	};

	assert_eq!(
		found(Building::all(server).unwrap().collect()),
		[
			world.sentry.addr(),
			world.dispenser.addr(),
			world.teleporter.addr(),
			world.sapper.addr(),
			world.cart.addr(),
		]
	);

	let engineer = PlayerBuildings::new(server, entity(world.engineer)).unwrap();
	let spy = PlayerBuildings::new(server, entity(world.spy)).unwrap();

	assert_eq!(
		found(engineer.all().unwrap().collect()),
		[
			world.sentry.addr(),
			world.dispenser.addr(),
			world.teleporter.addr(),
		]
	);
	assert_eq!(found(spy.all().unwrap().collect()), [world.sapper.addr()]);

	// Buildings marked for deletion are gone as far as the game is concerned.
	world.set(world.dispenser, |dispenser| dispenser.flags |= EFL_KILLME);
	assert_eq!(
		found(engineer.all().unwrap().collect()),
		[world.sentry.addr(), world.teleporter.addr()]
	);

	assert!(matches!(
		Building::all(null_server(Game::SourceSdk2013, &scope)),
		Err(BuildingError::WrongGame)
	));
}

#[test]
fn buildings_built_for_players_start_as_placed_blueprints() {
	let world = World::new();
	let scope = ();
	let server = mock_server(&scope);
	let engineer = entity(world.engineer);
	let spawn = BuildingSpawn::new(ObjectKind::Dispenser);

	// SAFETY: Each build fails before creating anything.
	unsafe {
		assert!(matches!(
			spawn.build(null_server(Game::SourceSdk2013, &scope), engineer),
			Err(BuildingError::WrongGame)
		));

		for fake in [world.prop, world.dispenser] {
			assert!(matches!(
				spawn.build(server, entity(fake)),
				Err(BuildingError::NotTfPlayer)
			));
		}

		assert!(matches!(
			spawn.clone().level(4).build(server, engineer),
			Err(BuildingError::InvalidLevel(4))
		));

		world.set(world.engineer, |engineer| engineer.flags |= EFL_KILLME);
		assert!(matches!(
			spawn.build(server, engineer),
			Err(BuildingError::MarkedForDeletion)
		));
	}

	world.set(world.engineer, |engineer| engineer.flags &= !EFL_KILLME);

	// SAFETY: The fake dispenser's methods only note their calls.
	assert!(unsafe { building(server, world.dispenser).start_construction(engineer) });

	// Placed for the engineer, then built without taking their metal.
	assert_eq!(
		STARTS.take(),
		[
			(
				"StartPlacement",
				world.dispenser.addr(),
				world.engineer.addr()
			),
			("StartBuilding", world.dispenser.addr(), 0),
		]
	);
}

#[test]
fn buildings_read_their_networked_state() {
	let world = World::new();
	let scope = ();
	let sentry = building(mock_server(&scope), world.sentry);

	assert_eq!(sentry.kind().unwrap(), BuildingKind::Sentry);
	assert_eq!(
		sentry.builder().unwrap().map(Entity::as_ptr),
		Some(world.engineer.cast())
	);
	assert_eq!(sentry.built_on().unwrap(), None);
	assert_eq!(sentry.team().unwrap(), 3);
	assert_eq!(sentry.level().unwrap(), 1);
	assert_eq!(sentry.max_level(), 3);
	assert!(sentry.is_sapped().unwrap());
	assert!(!sentry.is_dying());

	world.set(world.sentry, |sentry| {
		sentry.mode = raw::MODE_SENTRYGUN_DISPOSABLE;
		sentry.object_flags = raw::OF_DOESNT_HAVE_A_MODEL | 1 << 7;
		sentry.level = 2;
		sentry.highest_level = 3;
		sentry.upgrade_metal = 25;
		sentry.upgrade_metal_required = 200;
		sentry.construction = 0.5;
		sentry.carried = 1;
		sentry.constructing = 1;
		sentry.disabled = 1;
		sentry.disposable = 1;
		sentry.mini = 1;
		sentry.placing = 1;
		sentry.plasma = 1;
		sentry.redeploying = 1;
		sentry.map_placed = 1;
		sentry.max_level = 1;
		sentry.dying = true;
	});

	assert_eq!(sentry.mode().unwrap(), raw::MODE_SENTRYGUN_DISPOSABLE);
	assert_eq!(
		sentry.flags().unwrap(),
		ObjectFlags::NO_MODEL | ObjectFlags::from_bits_retain(1 << 7)
	);
	assert_eq!(sentry.level().unwrap(), 2);
	assert_eq!(sentry.highest_level().unwrap(), 3);
	assert_eq!(sentry.max_level(), 1);
	assert_eq!(sentry.upgrade_metal().unwrap(), 25);
	assert_eq!(sentry.upgrade_metal_required().unwrap(), 200);
	assert_eq!(sentry.construction_progress().unwrap(), 0.5);
	assert!(sentry.is_carried().unwrap());
	assert!(sentry.is_constructing().unwrap());
	assert!(sentry.is_disabled().unwrap());
	assert!(sentry.is_disposable().unwrap());
	assert!(sentry.is_mini().unwrap());
	assert!(sentry.is_placing().unwrap());
	assert!(sentry.is_plasma_disabled().unwrap());
	assert!(sentry.is_redeploying().unwrap());
	assert!(sentry.was_map_placed().unwrap());
	assert!(sentry.is_dying());

	// A builder who no longer exists is none.
	world.set(world.sentry, |sentry| sentry.builder = 9 | 1 << 16);
	assert_eq!(sentry.builder().unwrap(), None);

	world.set(world.sentry, |sentry| sentry.object_type = raw::OBJ_LAST);
	assert!(matches!(
		sentry.kind(),
		Err(BuildingError::UnknownKind(raw::OBJ_LAST))
	));
}

#[test]
fn buildings_spawn_only_at_tf2s_levels() {
	let scope = ();

	for level in [0, 4, u8::MAX] {
		assert!(matches!(
			BuildingSpawn::new(ObjectKind::Sentry)
				.level(level)
				.entity_spawn(),
			Err(BuildingError::InvalidLevel(invalid)) if invalid == level
		));
	}

	let spawn = |spawn: BuildingSpawn, game| {
		// SAFETY: The server exports no interfaces, so nothing is created.
		unsafe { spawn.spawn(null_server(game, &scope)) }
	};

	assert!(matches!(
		spawn(BuildingSpawn::new(ObjectKind::Sentry), Game::SourceSdk2013),
		Err(BuildingError::WrongGame)
	));
	assert!(matches!(
		spawn(
			BuildingSpawn::new(ObjectKind::Sentry).level(4),
			Game::TeamFortress2
		),
		Err(BuildingError::InvalidLevel(4))
	));
	assert!(matches!(
		spawn(
			BuildingSpawn::new(ObjectKind::Sentry).level(3),
			Game::TeamFortress2
		),
		Err(BuildingError::Interface(_))
	));
}

#[test]
fn buildings_spawn_with_the_key_values_maps_give_them() {
	let keys = |spawn: BuildingSpawn| -> Vec<String> {
		spawn
			.entity_spawn()
			.unwrap()
			.keys()
			.map(|(key, value)| format!("{}={}", key.to_str().unwrap(), value.to_str().unwrap()))
			.collect()
	};

	let dispenser = BuildingSpawn::new(ObjectKind::Dispenser);

	assert_eq!(dispenser.kind(), ObjectKind::Dispenser);
	assert_eq!(dispenser.entity_spawn().unwrap().class(), c"obj_dispenser");
	assert!(keys(dispenser).is_empty());

	let sentry = BuildingSpawn::new(ObjectKind::Sentry)
		.team(Team::Red)
		.solid_to_players(SolidToPlayers::No)
		.name(c"guard")
		.level(3)
		.invulnerable(true)
		.upgradable(true)
		.infinite_ammo(true)
		.infinite_ammo(false)
		.teleporter_end(TeleporterEnd::Entrance);

	assert_eq!(sentry.entity_spawn().unwrap().class(), c"obj_sentrygun");
	assert_eq!(
		keys(sentry),
		[
			"TeamNum=2".to_owned(),
			format!("SolidToPlayer={}", raw::SOLID_TO_PLAYER_NO),
			"targetname=guard".to_owned(),
			"defaultupgrade=2".to_owned(),
			format!(
				"spawnflags={}",
				raw::SF_BASEOBJ_INVULN | raw::SF_SENTRY_UPGRADEABLE
			),
		]
	);

	// Only sentries take their flags, and only teleporters their end.
	let teleporter = BuildingSpawn::new(ObjectKind::Teleporter)
		.teleporter_end(TeleporterEnd::Entrance)
		.upgradable(true)
		.infinite_ammo(true)
		.level(1);

	assert_eq!(
		teleporter.entity_spawn().unwrap().class(),
		c"obj_teleporter"
	);
	assert_eq!(
		keys(teleporter),
		[
			format!("teleporterType={}", raw::TTYPE_ENTRANCE),
			"defaultupgrade=0".to_owned(),
		]
	);
	assert_eq!(
		keys(BuildingSpawn::new(ObjectKind::Teleporter).teleporter_end(TeleporterEnd::Exit)),
		[format!("teleporterType={}", raw::TTYPE_EXIT)]
	);
}

#[test]
fn buildings_without_a_builder_are_given_to_players() {
	let world = World::new();
	let scope = ();
	let sentry = building(mock_server(&scope), world.sentry);
	let engineer = entity(world.engineer);

	assert!(matches!(
		sentry.set_builder(engineer),
		Err(BuildingError::HasBuilder)
	));

	world.set(world.sentry, |sentry| sentry.builder = NULL);

	for fake in [world.prop, world.dispenser] {
		assert!(matches!(
			sentry.set_builder(entity(fake)),
			Err(BuildingError::NotTfPlayer)
		));
	}

	world.set(world.engineer, |engineer| engineer.flags |= EFL_KILLME);
	assert!(matches!(
		sentry.set_builder(engineer),
		Err(BuildingError::MarkedForDeletion)
	));

	world.set(world.engineer, |engineer| engineer.flags &= !EFL_KILLME);
	world.set(world.sentry, |sentry| sentry.dying = true);
	assert!(matches!(
		sentry.set_builder(engineer),
		Err(BuildingError::Dying)
	));
	assert!(INPUTS.take().is_empty());

	world.set(world.sentry, |sentry| sentry.dying = false);
	sentry.set_builder(engineer).unwrap();

	assert_eq!(
		INPUTS.take(),
		[(
			world.sentry.addr(),
			c"SetBuilder".to_owned(),
			world.engineer.addr(),
			world.sentry.addr(),
			0
		)]
	);
}

unsafe extern "C" fn class_name(this: *const sys::IServerNetworkable) -> *const c_char {
	// SAFETY: Callers pass the networkable of a fake entity.
	unsafe { (*fake_of(this)).class_name.as_ptr() }
}

#[test]
fn class_views_read_what_their_kinds_add() {
	let world = World::new();
	let scope = ();
	let server = mock_server(&scope);

	world.set(world.sentry, |sentry| {
		sentry.state = raw::SENTRY_STATE_ATTACKING;
		sentry.shells = 150;
		sentry.rockets = 20;
		sentry.kills = 4;
		sentry.assists = 2;
		sentry.shield = 1;
		sentry.wrangled = 1;
		sentry.enemy = handle_of(world.spy);
	});

	let sentry = Sentry::new(building(server, world.sentry)).unwrap();

	assert_eq!(sentry.state().unwrap(), SentryState::Attacking);
	assert_eq!(sentry.shells().unwrap(), 150);
	assert_eq!(sentry.rockets().unwrap(), 20);
	assert_eq!(sentry.kills().unwrap(), 4);
	assert_eq!(sentry.assists().unwrap(), 2);
	assert_eq!(sentry.shield_level().unwrap(), 1);
	assert!(sentry.is_wrangled().unwrap());
	assert_eq!(
		sentry.enemy().unwrap().map(Entity::as_ptr),
		Some(world.spy.cast())
	);
	assert_eq!(sentry.building().entity().as_ptr(), world.sentry.cast());

	world.set(world.dispenser, |dispenser| {
		dispenser.state = raw::DISPENSER_STATE_UPGRADING;
		dispenser.metal = 400;
	});

	let dispenser = Dispenser::new(building(server, world.dispenser)).unwrap();
	let cart = Dispenser::new(building(server, world.cart)).unwrap();

	assert_eq!(dispenser.state().unwrap(), DispenserState::Upgrading);
	assert_eq!(dispenser.metal().unwrap(), 400);
	assert!(!dispenser.is_cart());
	assert!(cart.is_cart());

	world.set(world.teleporter, |teleporter| {
		teleporter.mode = raw::MODE_TELEPORTER_EXIT;
		teleporter.state = raw::TELEPORTER_STATE_RECHARGING;
		teleporter.recharge_time = 12.5;
		teleporter.recharge_duration = 10.0;
		teleporter.times_used = 7;
		teleporter.yaw_to_exit = 90.0;
	});

	let teleporter = Teleporter::new(building(server, world.teleporter)).unwrap();

	assert_eq!(teleporter.end().unwrap(), TeleporterEnd::Exit);
	assert_eq!(teleporter.state().unwrap(), TeleporterState::Recharging);
	assert_eq!(teleporter.recharge_time().unwrap(), 12.5);
	assert_eq!(teleporter.recharge_duration().unwrap(), 10.0);
	assert_eq!(teleporter.times_used().unwrap(), 7);
	assert_eq!(teleporter.yaw_to_exit().unwrap(), 90.0);

	world.set(world.teleporter, |teleporter| {
		teleporter.mode = raw::MODE_TELEPORTER_ENTRANCE;
		teleporter.state = 8;
	});
	assert_eq!(teleporter.end().unwrap(), TeleporterEnd::Entrance);
	assert!(matches!(
		teleporter.state(),
		Err(BuildingError::UnknownState(8))
	));

	// Each view only wraps its own kind.
	assert!(matches!(
		Sentry::new(building(server, world.dispenser)),
		Err(BuildingError::NotKind(BuildingKind::Sentry))
	));
	assert!(matches!(
		Dispenser::new(building(server, world.teleporter)),
		Err(BuildingError::NotKind(BuildingKind::Dispenser))
	));
	assert!(matches!(
		Teleporter::new(building(server, world.sapper)),
		Err(BuildingError::NotKind(BuildingKind::Teleporter))
	));
}

#[test]
fn construction_native_preserves_completion_result_and_refuses_invalid_lifecycle() {
	thread_local! {
		static WORK: RefCell<Vec<(usize, f32)>> = const { RefCell::new(Vec::new()) };
		static COMPLETE: Cell<bool> = const { Cell::new(false) };
	}
	unsafe extern "C" fn construct(this: *mut sys::CBaseObject, amount: f32) -> bool {
		WORK.with_borrow_mut(|work| work.push((this as usize, amount)));
		COMPLETE.get()
	}
	let world = World::new();
	let slot = vtable_slot!(sys::CBaseObject__bindgen_vtable, CBaseObject_Construct);
	let last = vtable_slot!(sys::CBaseObject__bindgen_vtable, CBaseObject_IsDying);
	assert!(slot <= last, "World's mock table includes IsDying");
	// SAFETY: World allocates a leaked mutable function table through at
	// least IsDying; only this test's table slot is replaced.
	unsafe {
		(*world.dispenser)
			.vtable
			.cast_mut()
			.add(slot)
			.write(construct as *const ());
	}
	let scope = ();
	let server = mock_server(&scope);
	let dispenser = building(server, world.dispenser);
	assert!(!dispenser.construct(20.0).unwrap());
	COMPLETE.set(true);
	assert!(dispenser.construct(5.0).unwrap());
	assert_eq!(
		WORK.take(),
		[
			(world.dispenser as usize, 20.0),
			(world.dispenser as usize, 5.0)
		]
	);
	for amount in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0] {
		assert!(matches!(
			dispenser.construct(amount),
			Err(BuildingError::InvalidHealth)
		));
	}
	world.set(world.dispenser, |entity| entity.placing = 1);
	assert!(matches!(
		dispenser.construct(20.0),
		Err(BuildingError::Blueprint)
	));
	world.set(world.dispenser, |entity| {
		entity.placing = 0;
		entity.dying = true;
	});
	assert!(matches!(
		dispenser.construct(20.0),
		Err(BuildingError::Dying)
	));
	world.set(world.dispenser, |entity| {
		entity.dying = false;
		entity.flags |= EFL_KILLME;
	});
	assert!(matches!(
		dispenser.construct(20.0),
		Err(BuildingError::MarkedForDeletion)
	));
	assert!(WORK.take().is_empty());
}

unsafe extern "C" fn datamap(entity: *mut sys::CBaseEntity) -> *mut sys::datamap_t {
	// SAFETY: Only fake entities have this method in their vtables.
	unsafe { (*entity.cast::<FakeEntity>()).map }
}

/// `CBaseObject::DestroyObject`, which notes the call and marks the building
/// for deletion, as the game does.
unsafe extern "C" fn destroy_object(this: *mut sys::CBaseObject) {
	CALLS.with_borrow_mut(|calls| calls.push(("DestroyObject", this.addr())));

	// SAFETY: Only fake entities have this method in their vtables.
	unsafe { (*this.cast::<FakeEntity>()).flags |= EFL_KILLME };
}

/// `CBaseObject::DetonateObject`, as `DestroyObject` is.
unsafe extern "C" fn detonate_object(this: *mut sys::CBaseObject) {
	CALLS.with_borrow_mut(|calls| calls.push(("DetonateObject", this.addr())));

	// SAFETY: As for `destroy_object`.
	unsafe { (*this.cast::<FakeEntity>()).flags |= EFL_KILLME };
}

#[test]
fn detonating_and_destroying_refuse_buildings_being_removed() {
	let world = World::new();
	let scope = ();
	let server = mock_server(&scope);
	let sentry = building(server, world.sentry);
	let dispenser = building(server, world.dispenser);

	sentry.detonate().unwrap();
	dispenser.destroy().unwrap();
	assert_eq!(
		CALLS.take(),
		[
			("DetonateObject", world.sentry.addr()),
			("DestroyObject", world.dispenser.addr()),
		]
	);

	// The game marked both for deletion.
	assert!(matches!(
		sentry.destroy(),
		Err(BuildingError::MarkedForDeletion)
	));
	assert!(matches!(
		dispenser.detonate(),
		Err(BuildingError::MarkedForDeletion)
	));

	let teleporter = building(server, world.teleporter);

	world.set(world.teleporter, |teleporter| teleporter.dying = true);
	assert!(matches!(teleporter.detonate(), Err(BuildingError::Dying)));

	world.set(world.teleporter, |teleporter| {
		teleporter.dying = false;
		teleporter.placing = 1;
	});
	assert!(matches!(
		teleporter.destroy(),
		Err(BuildingError::Blueprint)
	));
	assert!(CALLS.take().is_empty());
}

#[test]
fn dispenser_stock_writes_keep_nonnegative_inventory_and_refuse_deleted_entities() {
	let world = World::new();
	let scope = ();
	let engine = crate::test_support::sdk_core::change_tracking_engine();
	export(
		Module::Engine,
		crate::interfaces::ValveEngine::VERSION,
		engine.as_ptr(),
	);
	let server = mock_server(&scope);
	let dispenser = Dispenser::new(building(server, world.dispenser)).unwrap();
	dispenser.set_metal(40).unwrap();
	assert_eq!(dispenser.metal().unwrap(), 40);
	// SAFETY: The fake edict is leaked with the world.
	assert_ne!(
		unsafe { (*(*world.dispenser).edict)._base.m_fStateFlags } & 1,
		0
	);
	assert!(matches!(
		dispenser.set_metal(-1),
		Err(BuildingError::InvalidMetal(-1))
	));
	assert_eq!(dispenser.metal().unwrap(), 40);
	dispenser.set_metal(0).unwrap();
	assert_eq!(dispenser.metal().unwrap(), 0);
	world.set(world.dispenser, |entity| entity.flags |= EFL_KILLME);
	assert!(matches!(
		dispenser.set_metal(400),
		Err(BuildingError::MarkedForDeletion)
	));
	// SAFETY: The fake object is leaked and remains allocated after marking.
	assert_eq!(unsafe { (*world.dispenser).metal }, 0);
}

unsafe extern "C" fn edict(this: *const sys::IServerNetworkable) -> *mut sys::edict_t {
	// SAFETY: As for `class_name`.
	unsafe { (*fake_of(this)).edict }
}

/// A callback-scoped entity for a fake one.
fn entity<'s>(fake: *mut FakeEntity) -> Entity<'s> {
	// SAFETY: Fake entities are leaked, so they outlive every scope, and their
	// vtables answer what the wrappers call of an entity.
	unsafe { Entity::from_raw(NonNull::new(fake).unwrap().cast()) }
}

unsafe extern "C" fn entity_list(_: *mut sys::IServerTools) -> *mut sys::CGlobalEntityList {
	ENTITY_LIST.get()
}

/// Makes every fake entity's script class `CTFPlayer`, with the VScript
/// methods the wrappers call.
fn export_bindings() {
	let bool_parameter = leak([BOOL]);
	// SAFETY: The parameters are leaked, and only their binding views them.
	let bindings = leak([
		member_binding(
			c"RemoveAllObjects",
			binding::VOID,
			unsafe { &mut *bool_parameter },
			Some(remove_adapter),
		),
		member_binding(
			c"TryToPickupBuilding",
			BOOL,
			&mut [],
			Some(answer_adapter::<0>),
		),
		member_binding(c"IsSapping", BOOL, &mut [], Some(answer_adapter::<1>)),
		member_binding(c"IsPlacingSapper", BOOL, &mut [], Some(answer_adapter::<2>)),
	]);

	// SAFETY: The bindings are leaked, and only their description views them.
	set_script_description(leak(class_description(
		c"CTFPlayer",
		unsafe { &mut *bindings },
		null_mut(),
	)));
}

/// Exports the engine and game interfaces the module uses.
fn export_interfaces() {
	export_standard_proxies();

	ENTITY_LIST.set(Box::leak(Box::<sys::CGlobalEntityList>::new_zeroed()).as_mut_ptr());

	// SAFETY: The vtable holds only function pointers, `unexpected_call`
	// aborts whichever slot reaches it, and the patch only writes slots of the
	// vtable being built.
	let tools = Box::leak(unsafe {
		mock_vtable::<sys::IServerTools__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).IServerTools_GetEntityList).write(entity_list);
			(&raw mut (*vtable).IServerTools_FindEntityByClassname).write(find_by_class_name);
		})
	});
	export(
		Module::GameServer,
		ServerTools::VERSION,
		leak(sys::IServerTools { vtable_: tools }),
	);
}

/// The fake entity whose networkable is `this`.
fn fake_of(this: *const sys::IServerNetworkable) -> *mut FakeEntity {
	// SAFETY: Callers pass the networkable of a fake entity, so the entity
	// starts that far before it.
	unsafe { this.byte_sub(offset_of!(FakeEntity, networkable)) }
		.cast_mut()
		.cast()
}

/// `IServerTools::FindEntityByClassname`, which finds the fake entities whose
/// class names match, in order, as the game does, with a `*` ending the
/// pattern matching any rest.
unsafe extern "C" fn find_by_class_name(
	_: *mut sys::IServerTools,
	after: *mut sys::CBaseEntity,
	pattern: *const c_char,
) -> *mut sys::CBaseEntity {
	// SAFETY: The tools are passed a NUL-terminated class name.
	let pattern = unsafe { CStr::from_ptr(pattern) }.to_bytes();
	let matches = |name: &CStr| match pattern.strip_suffix(b"*") {
		Some(prefix) => name.to_bytes().starts_with(prefix),
		None => name.to_bytes() == pattern,
	};

	ENTITIES.with_borrow(|entities| {
		let start = entities
			.iter()
			.position(|&entity| entity.cast() == after)
			.map_or(0, |index| index + 1);

		entities[start..]
			.iter()
			// SAFETY: Fake entities are leaked.
			.find(|&&entity| matches(unsafe { (*entity).class_name }))
			.map_or(null_mut(), |&entity| entity.cast())
	})
}

unsafe extern "C" fn handle(this: *const sys::IServerEntity) -> *const sys::CBaseHandle {
	// SAFETY: Only fake entities have this method in their vtables.
	unsafe { (&raw const (*this.cast::<FakeEntity>()).handle).cast() }
}

/// The handle of a fake entity.
fn handle_of(fake: *mut FakeEntity) -> u32 {
	// SAFETY: Fake entities are leaked.
	unsafe { (*fake).handle }
}

/// Stands in for `SendProxy_EHandleToInt`, which no test reaches: the wrappers
/// read handles as stored.
unsafe extern "C" fn handle_proxy(
	_: *const sys::SendProp,
	_: *const c_void,
	_: *const c_void,
	_: *mut sys::DVariant,
	_: c_int,
	_: c_int,
) {
	unexpected_call();
}

/// `CBaseObject::IsDying`.
unsafe extern "C" fn is_dying_fn(this: *mut sys::CBaseObject) -> bool {
	// SAFETY: As for `destroy_object`.
	unsafe { (*this.cast::<FakeEntity>()).dying }
}

#[test]
fn kinds_states_and_flags_keep_tf2s_numbers() {
	for (raw, kind) in (raw::OBJ_DISPENSER..raw::OBJ_LAST).zip(BuildingKind::ALL) {
		assert_eq!(BuildingKind::from_raw(raw), Some(kind));
		assert_eq!(kind.to_raw(), raw);
	}

	assert_eq!(BuildingKind::from_raw(-1), None);
	assert_eq!(BuildingKind::from_raw(raw::OBJ_LAST), None);

	for kind in ObjectKind::ALL {
		assert_eq!(BuildingKind::from(kind).object_kind(), Some(kind));
	}

	assert_eq!(BuildingKind::Sapper.object_kind(), None);

	for raw in raw::SENTRY_STATE_INACTIVE..=raw::SENTRY_STATE_UPGRADING {
		assert_eq!(SentryState::from_raw(raw).unwrap().to_raw(), raw);
	}

	for raw in raw::DISPENSER_STATE_IDLE..=raw::DISPENSER_STATE_UPGRADING {
		assert_eq!(DispenserState::from_raw(raw).unwrap().to_raw(), raw);
	}

	for raw in raw::TELEPORTER_STATE_BUILDING..=raw::TELEPORTER_STATE_UPGRADING {
		assert_eq!(TeleporterState::from_raw(raw).unwrap().to_raw(), raw);
	}

	assert_eq!(SentryState::from_raw(4), None);
	assert_eq!(DispenserState::from_raw(2), None);
	assert_eq!(TeleporterState::from_raw(-1), None);

	assert_eq!(ObjectFlags::all().bits(), !0);
	assert_eq!(
		(ObjectFlags::ALLOW_REPEAT_PLACEMENT
			| ObjectFlags::BUILT_ON_ATTACHMENT
			| ObjectFlags::NO_MODEL
			| ObjectFlags::PLAYER_DESTRUCTION)
			.bits(),
		0x0f
	);
	assert_eq!(SolidToPlayers::Default.to_raw(), 0);
	assert_eq!(SolidToPlayers::Yes.to_raw(), 1);
	assert_eq!(SolidToPlayers::No.to_raw(), 2);
}

/// `CBaseObject::GetMaxUpgradeLevel`.
unsafe extern "C" fn max_upgrade_level(this: *mut sys::CBaseObject) -> c_int {
	// SAFETY: As for `destroy_object`.
	unsafe { (*this.cast::<FakeEntity>()).max_level }
}

unsafe extern "C" fn networkable(entity: *mut sys::IServerUnknown) -> *mut sys::IServerNetworkable {
	// SAFETY: As for `datamap`.
	unsafe { &raw mut (*entity.cast::<FakeEntity>()).networkable }
}

#[test]
fn only_tf2_buildings_and_players_are_wrapped() {
	let world = World::new();
	let scope = ();
	let server = mock_server(&scope);

	assert!(matches!(
		Building::new(
			null_server(Game::SourceSdk2013, &scope),
			entity(world.sentry)
		),
		Err(BuildingError::WrongGame)
	));

	for fake in [world.prop, world.engineer] {
		assert!(matches!(
			Building::new(server, entity(fake)),
			Err(BuildingError::NotBuilding)
		));
	}

	for fake in [
		world.sentry,
		world.dispenser,
		world.cart,
		world.teleporter,
		world.sapper,
	] {
		assert!(Building::new(server, entity(fake)).is_ok());
	}

	assert!(matches!(
		PlayerBuildings::new(server, entity(world.sentry)),
		Err(BuildingError::NotTfPlayer)
	));
	assert!(matches!(
		PlayerBuildings::new(
			null_server(Game::SourceSdk2013, &scope),
			entity(world.engineer)
		),
		Err(BuildingError::WrongGame)
	));
	assert!(PlayerBuildings::new(server, entity(world.engineer)).is_ok());
}

#[test]
fn players_carry_pick_up_and_remove_their_buildings() {
	let world = World::new();
	let scope = ();
	let server = mock_server(&scope);
	let engineer = PlayerBuildings::new(server, entity(world.engineer)).unwrap();
	let spy = PlayerBuildings::new(server, entity(world.spy)).unwrap();
	let at = world.engineer.addr();

	assert!(engineer.carried().unwrap().is_none());

	world.set(world.engineer, |engineer| {
		engineer.carried_object = handle_of(world.dispenser);
	});
	assert_eq!(
		engineer
			.carried()
			.unwrap()
			.map(|building| building.entity().as_ptr()),
		Some(world.dispenser.cast())
	);

	ANSWER.set(true);
	assert!(engineer.pick_up().unwrap());
	assert!(spy.is_sapping().unwrap());
	ANSWER.set(false);
	assert!(!spy.is_placing_sapper().unwrap());

	engineer.remove_all(true).unwrap();
	engineer.remove_all(false).unwrap();
	assert_eq!(
		CALLS.take(),
		[
			("TryToPickupBuilding", at),
			("IsSapping", world.spy.addr()),
			("IsPlacingSapper", world.spy.addr()),
			("RemoveAllObjects(true)", at),
			("RemoveAllObjects(false)", at),
		]
	);

	world.set(world.engineer, |engineer| engineer.flags |= EFL_KILLME);
	assert!(matches!(
		engineer.remove_all(true),
		Err(BuildingError::MarkedForDeletion)
	));
	assert!(CALLS.take().is_empty());
}

/// Adapts `CTFPlayer::RemoveAllObjects`, which notes the call with whether it
/// explodes the buildings.
unsafe extern "C" fn remove_adapter(
	_: sys::ScriptFunctionBindingStorageType_t,
	object: *mut c_void,
	arguments: *mut sys::ScriptVariant_t,
	count: c_int,
	result: *mut sys::ScriptVariant_t,
) -> bool {
	assert_eq!(count, 1);
	assert!(result.is_null(), "a void member gets no result");

	// SAFETY: `call` passes the one argument it checked to be a `bool`.
	let name = match unsafe { (*arguments).__bindgen_anon_1.m_bool } {
		true => "RemoveAllObjects(true)",
		false => "RemoveAllObjects(false)",
	};

	CALLS.with_borrow_mut(|calls| calls.push((name, object.addr())));
	true
}

#[test]
fn sappers_are_found_on_the_buildings_they_sap() {
	let world = World::new();
	let scope = ();
	let server = mock_server(&scope);
	let sentry = building(server, world.sentry);
	let sapper = building(server, world.sapper);

	assert_eq!(
		sentry
			.sapper()
			.unwrap()
			.map(|sapper| sapper.entity().as_ptr()),
		Some(world.sapper.cast())
	);
	assert_eq!(sapper.kind().unwrap(), BuildingKind::Sapper);
	assert_eq!(
		sapper.built_on().unwrap().map(Entity::as_ptr),
		Some(world.sentry.cast())
	);
	assert!(
		building(server, world.dispenser)
			.sapper()
			.unwrap()
			.is_none()
	);

	world.set(world.sapper, |sapper| sapper.flags |= EFL_KILLME);
	assert!(sentry.sapper().unwrap().is_none());
}

unsafe extern "C" fn server_class(this: *mut sys::IServerNetworkable) -> *mut sys::ServerClass {
	// SAFETY: Callers pass the networkable of a fake entity.
	unsafe { (*fake_of(this)).class }
}

/// `CBaseObject::StartBuilding`, which notes the call and the builder, and
/// starts building.
unsafe extern "C" fn start_building(
	this: *mut sys::CBaseObject,
	builder: *mut sys::CBaseEntity,
) -> bool {
	STARTS.with_borrow_mut(|starts| starts.push(("StartBuilding", this.addr(), builder.addr())));
	true
}

/// `CBaseObject::StartPlacement`, which notes the call and the player.
unsafe extern "C" fn start_placement(this: *mut sys::CBaseObject, player: *mut sys::CTFPlayer) {
	STARTS.with_borrow_mut(|starts| starts.push(("StartPlacement", this.addr(), player.addr())));
}
