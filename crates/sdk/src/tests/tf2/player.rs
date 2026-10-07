//! Tests of TF2's player core through fake players: their networked
//! variables, the native members and vtable methods `TfPlayer` calls, their
//! ragdolls, weapons and reserve ammo.

use super::*;
use crate::Module;
use crate::datatables::PropFlags;
use crate::interfaces::{PluginHelpers, ServerTools, ValveEngine};
use crate::test_support::datatables::{int8_proxy, int32_proxy, prop, table, vector_proxy};
use crate::test_support::edicts::{change_accessor, shared_change_info};
use crate::test_support::entities::MOCK_EFLAGS_OFFSET;
use crate::test_support::interfaces::server_game_dll::export_standard_proxies;
use crate::test_support::leak;
use crate::test_support::server::{export, mock_server, null_server};
use crate::test_support::tf2::game_rules::{ENTITIES, World as RulesWorld, round_rules_proxy};

use crate::test_support::tf2::script_binding::{
	SCRIPT_DESCRIPTION_SLOT, class_description, member_binding,
};

use crate::tf2::ammo::{self, AmmoError, AmmoType};
use crate::tf2::ragdolls::{RagdollFlag, TfRagdollError};
use crate::tf2::scoreboard::ScoringTeam;
use crate::tf2::weapons::{PlayerWeapons, Weapon, WeaponError};
use sdk_raw::edicts::FL_FULL_EDICT_CHANGED;
use sdk_raw::entities::health::IS_ALIVE_SLOT;
use sdk_raw::entities::{EFL_KILLME, GET_DATA_DESC_MAP_SLOT, NUM_NETWORKED_EHANDLE_BITS};
use sdk_raw::test_support::edicts::mock_edict;
use sdk_raw::test_support::entities::{data_map, field};
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use sdk_raw::tf2::script_binding::{FLOAT, QANGLE, STRING, VECTOR, float};
use std::cell::{Cell, RefCell};
use std::ffi::{CString, c_char, c_void};
use std::mem::offset_of;
use std::ptr::{NonNull, null_mut};

const _: () = assert!(offset_of!(FakeEntity, flags) == MOCK_EFLAGS_OFFSET);

/// The native members the fake player declares on `CTFPlayer`: each one's
/// name, return type and parameter types. Others are missing.
const MEMBERS: [(&CStr, sys::ScriptDataType_t, &[sys::ScriptDataType_t]); 13] = [
	(c"SetPlayerClass", binding::VOID, &[INT]),
	(c"ForceChangeTeam", binding::VOID, &[INT, BOOL]),
	(c"AddHudHideFlags", binding::VOID, &[INT]),
	(c"RemoveHudHideFlags", binding::VOID, &[INT]),
	(c"SetHudHideFlags", binding::VOID, &[INT]),
	(c"GetHudHideFlags", INT, &[]),
	(c"SetCustomModel", binding::VOID, &[STRING]),
	(c"ApplyAbsVelocityImpulse", binding::VOID, &[VECTOR]),
	(c"SetCustomModelRotation", binding::VOID, &[QANGLE]),
	(c"SetNextChangeClassTime", binding::VOID, &[FLOAT]),
	(c"GetNextChangeClassTime", FLOAT, &[]),
	(c"Regenerate", binding::VOID, &[BOOL]),
	(c"IsCallingForMedic", BOOL, &[]),
];

/// A null handle, in a fake entity's fields.
const NULL: u32 = u32::MAX;

/// The view offset of a standing player.
const STANDING: [f32; 3] = [0.0, 0.0, 68.0];

/// An argument a native member received.
#[derive(Debug, Clone, PartialEq)]
enum Argument {
	Angles([f32; 3]),
	Bool(bool),
	/// The address of an entity.
	Entity(usize),
	Float(f32),
	Int(c_int),
	String(CString),
	Vector([f32; 3]),
}

/// A player, ragdoll, weapon or prop, whose datamaps and send tables describe
/// the fields of its kind.
#[repr(C)]
struct FakeEntity {
	vtable: *const *const (),
	map: *mut sys::datamap_t,
	networkable: sys::IServerNetworkable,
	/// Keeps `flags` at the offset every mock entity has it.
	_collideable: *const c_void,
	flags: c_int,
	handle: u32,
	class: *mut sys::ServerClass,
	edict: *mut sys::edict_t,
	description: *mut sys::ScriptClassDesc_t,
	alive: bool,
	gib: bool,
	burning: bool,
	player_class: c_int,
	desired_class: c_int,
	state: c_int,
	team: c_int,
	/// `m_iSpawnCounter`, a `bool` that TF2 sends as an 8-bit integer.
	spawn_counter: bool,
	force_bone: c_int,
	damage_custom: c_int,
	/// `m_hRagdoll` of a player.
	ragdoll: u32,
	/// `m_hOwner` of a weapon, or `m_hPlayer` of a ragdoll.
	owner: u32,
	active_weapon: u32,
	weapons: [u32; 3],
	ammo: [c_int; 7],
	view_offset: [f32; 3],
	force: [f32; 3],
	eye_angles: sys::QAngle,
	origin: [f32; 3],
	/// What the HUD flag members change.
	hud: c_int,
	/// What the change class time members change.
	next_class_time: f32,
}

thread_local! {
	/// Whether the native members' adapters accept their calls.
	static ACCEPTS: Cell<bool> = const { Cell::new(true) };

	/// Each call of a native member, by name, with its arguments.
	static CALLS: RefCell<Vec<(&'static CStr, Vec<Argument>)>> = const { RefCell::new(Vec::new()) };

	/// Each client command run through the plugin helpers.
	static COMMANDS: RefCell<Vec<(c_int, CString)>> = const { RefCell::new(Vec::new()) };

	/// `gEntList`, which `ServerTools::entity_by_handle` reads.
	static ENTITY_LIST: Cell<*mut sys::CGlobalEntityList> = const { Cell::new(null_mut()) };

	/// Each call of a vtable method of a player: its name, the player, and
	/// its arguments.
	static VCALLS: RefCell<Vec<(&'static str, usize, Vec<Argument>)>> = const { RefCell::new(Vec::new()) };
}

/// Mock interfaces exported on this thread, and fake entities of each kind.
struct World {
	player: *mut FakeEntity,
	/// A player on no team, with no class, who never spawned.
	newcomer: *mut FakeEntity,
	ragdoll: *mut FakeEntity,
	weapon: *mut FakeEntity,
	/// A weapon another player owns.
	stolen: *mut FakeEntity,
	/// An entity that is neither a player nor a weapon.
	prop: *mut FakeEntity,
}

impl World {
	fn new() -> Self {
		let slot = |field: usize| field / size_of::<usize>();
		let last = [
			GET_DATA_DESC_MAP_SLOT,
			SCRIPT_DESCRIPTION_SLOT,
			IS_ALIVE_SLOT,
			raw::COMMIT_SUICIDE_SLOT,
			raw::EYE_ANGLES_SLOT,
			raw::EYE_POSITION_SLOT,
			raw::WEAPON_SWITCH_SLOT,
		]
		.into_iter()
		.max()
		.unwrap();
		let mut vtable = vec![unexpected_call as *const (); last + 1];

		vtable[GET_DATA_DESC_MAP_SLOT] = datamap as *const ();
		vtable[SCRIPT_DESCRIPTION_SLOT] = description as *const ();
		vtable[IS_ALIVE_SLOT] = is_alive as *const ();
		vtable[raw::COMMIT_SUICIDE_SLOT] = commit_suicide as *const ();
		vtable[raw::EYE_ANGLES_SLOT] = eye_angles as *const ();
		vtable[raw::EYE_POSITION_SLOT] = eye_position as *const ();
		vtable[raw::WEAPON_SWITCH_SLOT] = weapon_switch as *const ();
		vtable[slot(offset_of!(
			sys::IServerUnknown__bindgen_vtable,
			IServerUnknown_GetNetworkable
		))] = networkable as *const ();
		vtable[slot(offset_of!(
			sys::IServerEntity__bindgen_vtable,
			IServerEntity_GetRefEHandle
		))] = ref_handle as *const ();

		let vtable = vtable.leak().as_ptr();

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

		let mut flags = field(
			c"m_iEFlags",
			sys::_fieldtypes_FIELD_INTEGER,
			MOCK_EFLAGS_OFFSET,
		);

		flags.fieldSizeInBytes = size_of::<c_int>() as c_int;

		let base_map = data_map(c"CBaseEntity", vec![flags], null_mut());
		let player_map = data_map(
			c"CTFPlayer",
			vec![],
			data_map(c"CBasePlayer", vec![], base_map),
		);
		let ragdoll_map = data_map(c"CTFRagdoll", vec![], base_map);
		let weapon_map = data_map(
			c"CTFWeaponBase",
			vec![],
			data_map(
				c"CBaseCombatWeapon",
				vec![field(
					c"m_hOwner",
					sys::_fieldtypes_FIELD_EHANDLE,
					offset_of!(FakeEntity, owner),
				)],
				base_map,
			),
		);
		let prop_map = data_map(c"CDynamicProp", vec![], base_map);

		let player_props = leak([
			int_prop(c"m_iClass", offset_of!(FakeEntity, player_class)),
			int_prop(
				c"m_iDesiredPlayerClass",
				offset_of!(FakeEntity, desired_class),
			),
			int_prop(c"m_nPlayerState", offset_of!(FakeEntity, state)),
			int_prop(c"m_iTeamNum", offset_of!(FakeEntity, team)),
			byte_prop(c"m_iSpawnCounter", offset_of!(FakeEntity, spawn_counter)),
			handle_prop(c"m_hRagdoll", offset_of!(FakeEntity, ragdoll)),
			handle_prop(c"m_hActiveWeapon", offset_of!(FakeEntity, active_weapon)),
			inside_array(handle_prop(
				c"m_hMyWeapons",
				offset_of!(FakeEntity, weapons),
			)),
			array_prop(c"m_hMyWeapons", 3),
			inside_array(int_prop(c"m_iAmmo", offset_of!(FakeEntity, ammo))),
			array_prop(c"m_iAmmo", 7),
			float_prop(c"m_vecViewOffset[0]", offset_of!(FakeEntity, view_offset)),
			float_prop(
				c"m_vecViewOffset[1]",
				offset_of!(FakeEntity, view_offset) + 4,
			),
			float_prop(
				c"m_vecViewOffset[2]",
				offset_of!(FakeEntity, view_offset) + 8,
			),
		]);
		let ragdoll_props = leak([
			handle_prop(c"m_hPlayer", offset_of!(FakeEntity, owner)),
			int_prop(c"m_iClass", offset_of!(FakeEntity, player_class)),
			int_prop(c"m_iTeam", offset_of!(FakeEntity, team)),
			int_prop(c"m_nForceBone", offset_of!(FakeEntity, force_bone)),
			int_prop(c"m_iDamageCustom", offset_of!(FakeEntity, damage_custom)),
			bool_prop(c"m_bGib", offset_of!(FakeEntity, gib)),
			bool_prop(c"m_bBurning", offset_of!(FakeEntity, burning)),
			prop(
				c"m_vecForce",
				sys::SendPropType_DPT_Vector,
				offset(offset_of!(FakeEntity, force)),
				PropFlags::NO_SCALE,
				Some(vector_proxy),
			),
		]);

		// SAFETY: The properties are leaked, and only their tables, which view
		// them, reach them from here on. Each array property follows its
		// element, as `SendPropArray` declares them.
		let (player_table, ragdoll_table) = unsafe {
			for array in [8, 10] {
				(*player_props)[array].m_pArrayProp = &raw mut (*player_props)[array - 1];
			}

			(
				leak(table(c"DT_TFPlayer", &mut *player_props)),
				leak(table(c"DT_TFRagdoll", &mut *ragdoll_props)),
			)
		};
		let other_table = leak(table(c"DT_Other", &mut []));

		let class = |name: &'static CStr, table, id| {
			leak(sys::ServerClass {
				m_pNetworkName: name.as_ptr(),
				m_pTable: table,
				m_pNext: null_mut(),
				m_ClassID: id,
				m_InstanceBaselineIndex: 0,
			})
		};
		let player_class = class(c"CTFPlayer", player_table, 1);
		let ragdoll_class = class(c"CTFRagdoll", ragdoll_table, 2);
		let weapon_class = class(c"CTFShotgun", other_table, 3);
		let prop_class = class(c"CDynamicProp", other_table, 4);

		export_interfaces();

		let description = player_description();
		let spawn = |index: u32, map, class| {
			let fake = leak(FakeEntity {
				vtable,
				map,
				networkable: sys::IServerNetworkable {
					vtable_: networkable,
				},
				_collideable: null_mut(),
				flags: 0,
				handle: index | 1 << 16,
				class,
				edict: leak(mock_edict(index.cast_signed(), false)),
				description,
				alive: true,
				gib: false,
				burning: true,
				player_class: PlayerClass::Medic.to_raw(),
				desired_class: PlayerClass::Medic.to_raw(),
				state: raw::TF_STATE_ACTIVE,
				team: Team::Red.to_raw(),
				spawn_counter: true,
				force_bone: 7,
				damage_custom: 1,
				ragdoll: NULL,
				owner: NULL,
				active_weapon: NULL,
				weapons: [NULL; 3],
				ammo: [0, 32, 200, 0, 1, 0, 0],
				view_offset: STANDING,
				force: [10.0, 0.0, 5.0],
				eye_angles: sys::QAngle {
					x: 10.0,
					y: 90.0,
					z: 0.0,
				},
				origin: [100.0, 200.0, 0.0],
				hud: 0,
				next_class_time: 0.0,
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

			fake
		};

		let world = Self {
			player: spawn(1, player_map, player_class),
			newcomer: spawn(2, player_map, player_class),
			ragdoll: spawn(3, ragdoll_map, ragdoll_class),
			weapon: spawn(4, weapon_map, weapon_class),
			stolen: spawn(5, weapon_map, weapon_class),
			prop: spawn(6, prop_map, prop_class),
		};

		// SAFETY: Fake entities are leaked, and the mock game is not running
		// while the test writes their fields.
		unsafe {
			let player = &mut *world.player;
			let newcomer = &mut *world.newcomer;

			player.ragdoll = (*world.ragdoll).handle;
			player.active_weapon = (*world.weapon).handle;
			player.weapons = [(*world.weapon).handle, (*world.prop).handle, NULL];
			(*world.ragdoll).owner = player.handle;
			(*world.weapon).owner = player.handle;
			(*world.stolen).owner = newcomer.handle;
			newcomer.player_class = 0;
			newcomer.desired_class = 0;
			newcomer.state = raw::TF_STATE_WELCOME;
			newcomer.team = Team::Unassigned.to_raw();
			newcomer.spawn_counter = false;
		}

		world
	}

	/// Whether the engine was told that the entity's networked variables
	/// changed, which this then forgets.
	fn changed(&self, fake: *mut FakeEntity) -> bool {
		// SAFETY: As for the fields written in `new`.
		unsafe {
			let flags = &mut (*(*fake).edict)._base.m_fStateFlags;
			let changed = *flags & FL_FULL_EDICT_CHANGED != 0;

			*flags &= !FL_FULL_EDICT_CHANGED;
			changed
		}
	}

	/// A wrapper of the player.
	fn wrap<'s>(&self, scope: &'s ()) -> TfPlayer<'s> {
		TfPlayer::new(mock_server(scope), entity(self.player)).unwrap()
	}
}

/// The array property following an element property, as `SendPropArray`
/// declares it.
fn array_prop(name: &'static CStr, len: c_int) -> sys::SendProp {
	let mut array = prop(
		name,
		sys::SendPropType_DPT_Array,
		0,
		PropFlags::default(),
		None,
	);

	array.m_nElements = len;
	array.m_ElementStride = 4;
	array
}

/// A boolean property, as `SendPropBool` declares one.
fn bool_prop(name: &'static CStr, at: usize) -> sys::SendProp {
	let mut prop = prop(
		name,
		sys::SendPropType_DPT_Int,
		offset(at),
		PropFlags::UNSIGNED,
		Some(int8_proxy),
	);

	prop.m_nBits = 1;
	prop
}

/// An 8-bit integer property, as `SendPropInt` declares one for a `bool`
/// variable, such as `m_iSpawnCounter`.
fn byte_prop(name: &'static CStr, at: usize) -> sys::SendProp {
	let mut prop = prop(
		name,
		sys::SendPropType_DPT_Int,
		offset(at),
		PropFlags::default(),
		Some(int8_proxy),
	);

	prop.m_nBits = 8;
	prop
}

/// Takes the calls of native members made since the last.
fn calls() -> Vec<(&'static CStr, Vec<Argument>)> {
	CALLS.take()
}

#[test]
fn class_menu_state_goes_through_the_clients_commands() {
	let world = World::new();
	let scope = ();
	let player = world.wrap(&scope);

	player.set_class_menu_open(true).unwrap();
	player.set_class_menu_open(false).unwrap();
	assert_eq!(
		COMMANDS.take(),
		[(1, c"menuopen".to_owned()), (1, c"menuclosed".to_owned())]
	);

	// Only players on a team have a class menu.
	let newcomer = TfPlayer::new(mock_server(&scope), entity(world.newcomer)).unwrap();

	assert!(matches!(
		newcomer.show_class_menu(),
		Err(PlayerError::NotPlaying)
	));

	// SAFETY: As for the fields written in `World::new`.
	unsafe { (*world.player).flags = EFL_KILLME };
	assert!(matches!(
		player.set_class_menu_open(true),
		Err(PlayerError::MarkedForDeletion)
	));
	assert!(COMMANDS.take().is_empty());
}

#[test]
fn classes_states_and_teams_read_as_tf2_numbers_them() {
	let world = World::new();
	let scope = ();
	let player = world.wrap(&scope);
	let newcomer = TfPlayer::new(mock_server(&scope), entity(world.newcomer)).unwrap();

	assert_eq!(player.class().unwrap(), Some(PlayerClass::Medic));
	assert_eq!(player.desired_class().unwrap(), Some(PlayerClass::Medic));
	assert_eq!(player.state().unwrap(), PlayerState::Active);
	assert_eq!(player.team().unwrap(), Team::Red);
	assert!(player.spawn_parity().unwrap());
	assert_eq!(
		player.view_offset().unwrap(),
		Vector::new(STANDING[0], STANDING[1], STANDING[2])
	);

	assert_eq!(newcomer.class().unwrap(), None);
	assert_eq!(newcomer.desired_class().unwrap(), None);
	assert_eq!(newcomer.state().unwrap(), PlayerState::Welcome);
	assert_eq!(newcomer.team().unwrap(), Team::Unassigned);
	assert!(!newcomer.spawn_parity().unwrap());

	// The game flips the bit as the player spawns.
	// SAFETY: As for the fields written in `World::new`.
	unsafe { (*world.newcomer).spawn_counter = true };
	assert!(newcomer.spawn_parity().unwrap());

	// Values the game never assigns, such as a script's civilian.
	// SAFETY: As for the fields written in `World::new`.
	unsafe {
		(*world.player).player_class = 10;
		(*world.player).state = raw::TF_STATE_COUNT;
		(*world.player).team = 4;
	}
	assert!(matches!(player.class(), Err(PlayerError::UnknownClass(10))));
	assert!(matches!(
		player.state(),
		Err(PlayerError::UnknownState(raw::TF_STATE_COUNT))
	));
	assert!(matches!(player.team(), Err(PlayerError::UnknownTeam(4))));

	// The desired class is written as the game writes it, and networked.
	assert!(!world.changed(world.player));
	player.set_desired_class(Some(PlayerClass::Spy)).unwrap();
	assert_eq!(player.desired_class().unwrap(), Some(PlayerClass::Spy));
	assert!(world.changed(world.player));
	player.set_desired_class(None).unwrap();
	// SAFETY: As for the fields written in `World::new`.
	assert_eq!(unsafe { (*world.player).desired_class }, 0);
}

/// `IServerPluginHelpers::ClientCommand`, which notes the command.
unsafe extern "C" fn client_command(
	_: *mut sys::IServerPluginHelpers,
	edict: *mut sys::edict_t,
	command: *const c_char,
) {
	// SAFETY: The wrappers pass a fake entity's edict and a NUL-terminated
	// command.
	let (index, command) = unsafe { ((*edict)._base.m_EdictIndex, CStr::from_ptr(command)) };

	COMMANDS.with_borrow_mut(|commands| commands.push((index.into(), command.to_owned())));
}

/// `CTFPlayer::CommitSuicide`, which kills a living player.
unsafe extern "C" fn commit_suicide(player: *mut sys::CTFPlayer, explode: bool, force: bool) {
	let fake = player.cast::<FakeEntity>();

	VCALLS.with_borrow_mut(|calls| {
		calls.push((
			"CommitSuicide",
			fake.addr(),
			vec![Argument::Bool(explode), Argument::Bool(force)],
		));
	});
	// SAFETY: Only fake entities have this method, and they are leaked.
	unsafe { (*fake).alive = false };
}

unsafe extern "C" fn datamap(entity: *mut sys::CBaseEntity) -> *mut sys::datamap_t {
	// SAFETY: As for `commit_suicide`.
	unsafe { (*entity.cast::<FakeEntity>()).map }
}

/// `CBaseEntity::GetScriptDesc`, which returns the fake entity's descriptor.
unsafe extern "C" fn description(entity: *mut sys::CBaseEntity) -> *mut sys::ScriptClassDesc_t {
	// SAFETY: As for `commit_suicide`.
	unsafe { (*entity.cast::<FakeEntity>()).description }
}

unsafe extern "C" fn edict(this: *const sys::IServerNetworkable) -> *mut sys::edict_t {
	// SAFETY: Only fake entities' networkables have this method, and they are
	// fields of their entities.
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

/// Exports the engine and game interfaces the modules use.
fn export_interfaces() {
	export_standard_proxies();

	ENTITY_LIST.set(Box::leak(Box::<sys::CGlobalEntityList>::new_zeroed()).as_mut_ptr());

	// SAFETY: The vtable holds only function pointers, `unexpected_call`
	// aborts whichever slot reaches it, and the patch only writes slots of the
	// vtable being built. The same holds for the vtables below.
	let tools = Box::leak(unsafe {
		mock_vtable::<sys::IServerTools__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).IServerTools_GetEntityList).write(entity_list);
			(&raw mut (*vtable).IServerTools_RemoveEntity).write(remove_entity);
		})
	});
	export(
		Module::GameServer,
		ServerTools::VERSION,
		leak(sys::IServerTools { vtable_: tools }),
	);

	// SAFETY: As for the tools' vtable.
	let engine = Box::leak(unsafe {
		mock_vtable::<sys::IVEngineServer__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).IVEngineServer_GetChangeAccessor).write(change_accessor);
			(&raw mut (*vtable).IVEngineServer_GetSharedEdictChangeInfo).write(shared_change_info);
		})
	});
	export(
		Module::Engine,
		ValveEngine::VERSION,
		leak(sys::IVEngineServer { vtable_: engine }),
	);

	// SAFETY: As for the tools' vtable.
	let helpers = Box::leak(unsafe {
		mock_vtable::<sys::IServerPluginHelpers__bindgen_vtable>(
			unexpected_call as *const (),
			|vtable| {
				(&raw mut (*vtable).IServerPluginHelpers_ClientCommand).write(client_command);
			},
		)
	});
	export(
		Module::Engine,
		PluginHelpers::VERSION,
		leak(sys::IServerPluginHelpers { vtable_: helpers }),
	);
}

/// `CBasePlayer::EyeAngles`, returning the fake player's angles.
unsafe extern "C" fn eye_angles(player: *mut sys::CTFPlayer) -> *const sys::QAngle {
	// SAFETY: As for `commit_suicide`.
	unsafe { &raw const (*player.cast::<FakeEntity>()).eye_angles }
}

/// Where a fake player sees from.
///
/// # Safety
///
/// `fake` must be a fake entity.
unsafe fn eye_of(fake: *mut FakeEntity) -> sys::Vector {
	// SAFETY: As the caller promises.
	let (origin, offset) = unsafe { ((*fake).origin, (*fake).view_offset) };

	sys::Vector {
		x: origin[0] + offset[0],
		y: origin[1] + offset[1],
		z: origin[2] + offset[2],
	}
}

/// `CBasePlayer::EyePosition`: the fake player's origin raised by its view
/// offset, constructed through the hidden result pointer of the MSVC ABI.
#[cfg(target_os = "windows")]
unsafe extern "C" fn eye_position(
	player: *mut sys::CTFPlayer,
	result: *mut sys::Vector,
) -> *mut sys::Vector {
	// SAFETY: As for `commit_suicide`, and the caller passes writable storage.
	unsafe { result.write(eye_of(player.cast())) };
	result
}

/// `CBasePlayer::EyePosition`: the fake player's origin raised by its view
/// offset, returned in registers, as the Itanium ABI returns a `Vector`.
#[cfg(not(target_os = "windows"))]
unsafe extern "C" fn eye_position(player: *mut sys::CTFPlayer) -> sys::Vector {
	// SAFETY: As for `commit_suicide`.
	unsafe { eye_of(player.cast()) }
}

/// The fake entity whose networkable is `this`.
fn fake_of(this: *const sys::IServerNetworkable) -> *mut FakeEntity {
	// SAFETY: Callers pass the networkable of a fake entity, so the entity
	// starts that far before it.
	unsafe { this.byte_sub(offset_of!(FakeEntity, networkable)) }
		.cast_mut()
		.cast()
}

/// A float property, read through the folded standard proxy.
fn float_prop(name: &'static CStr, at: usize) -> sys::SendProp {
	prop(
		name,
		sys::SendPropType_DPT_Float,
		offset(at),
		PropFlags::default(),
		Some(int32_proxy),
	)
}

/// An entity handle property, as `SendPropEHandle` declares one.
fn handle_prop(name: &'static CStr, at: usize) -> sys::SendProp {
	let mut prop = prop(
		name,
		sys::SendPropType_DPT_Int,
		offset(at),
		PropFlags::UNSIGNED,
		Some(handle_proxy),
	);

	prop.m_nBits = NUM_NETWORKED_EHANDLE_BITS as c_int;
	prop
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

/// `prop`, as the element of the array property that follows it.
fn inside_array(mut prop: sys::SendProp) -> sys::SendProp {
	prop.m_Flags |= PropFlags::INSIDE_ARRAY.bits();
	prop
}

/// A signed integer property.
fn int_prop(name: &'static CStr, at: usize) -> sys::SendProp {
	prop(
		name,
		sys::SendPropType_DPT_Int,
		offset(at),
		PropFlags::default(),
		Some(int32_proxy),
	)
}

/// `CBaseEntity::IsAlive`, reading the fake entity's field.
unsafe extern "C" fn is_alive(entity: *mut sys::CBaseEntity) -> bool {
	// SAFETY: As for `commit_suicide`.
	unsafe { (*entity.cast::<FakeEntity>()).alive }
}

#[test]
fn max_reserves_are_the_lowest_counts_without_room() {
	/// Each type's max, by its index, which `can_have_ammo` compares the
	/// reserve with.
	const MAXES: [c_int; 7] = [0, 32, 36, 200, 1, -1, c_int::MAX];

	thread_local! {
		/// How often `can_have_ammo` was asked.
		static ASKED: Cell<usize> = const { Cell::new(0) };
	}

	/// `CTFGameRules::CanHaveAmmo`, which compares a fake player's reserve
	/// with its type's max, as the game compares it with `GetMaxAmmo`.
	unsafe extern "C" fn can_have_ammo(
		_: *mut sys::CTFGameRules,
		player: *mut sys::CBaseCombatCharacter,
		ammo_type: c_int,
	) -> bool {
		let index = usize::try_from(ammo_type).unwrap();

		ASKED.set(ASKED.get() + 1);

		// SAFETY: Only fake players are asked about, and they are leaked.
		unsafe { (*player.cast::<FakeEntity>()).ammo[index] < MAXES[index] }
	}

	// The game rules' game DLL is exported first, so it is the one found.
	let rules = RulesWorld::new(Some(round_rules_proxy));
	let world = World::new();
	let scope = ();
	let server = mock_server(&scope);
	let player = entity(world.player);

	// SAFETY: The vtable holds only function pointers, `unexpected_call` aborts
	// whichever slot reaches it, and the patch only writes a slot of the vtable
	// being built.
	let vtable = unsafe {
		mock_vtable::<sys::CTFGameRules__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).CTFGameRules_CanHaveAmmo).write(can_have_ammo);
		})
	};

	rules.set_vtable(Box::leak(vtable));

	// A max below none reads as none.
	assert_eq!(
		[
			AmmoType::Primary,
			AmmoType::Secondary,
			AmmoType::Metal,
			AmmoType::Grenades1,
			AmmoType::Grenades2,
			AmmoType::Grenades3,
		]
		.map(|ammo_type| ammo::max_reserve(server, player, ammo_type).unwrap()),
		[32, 36, 200, 1, 0, c_int::MAX]
	);

	// The reserves are put back, and clients are told of no change.
	// SAFETY: As for the fields written in `World::new`.
	assert_eq!(unsafe { (*world.player).ammo }, [0, 32, 200, 0, 1, 0, 0]);
	assert!(!world.changed(world.player));

	// Ten counts double from none to 256, then seven halve down to 200.
	ASKED.set(0);
	assert_eq!(
		ammo::max_reserve(server, player, AmmoType::Metal).unwrap(),
		200
	);
	assert_eq!(ASKED.get(), 17);

	assert!(matches!(
		ammo::max_reserve(server, entity(world.prop), AmmoType::Metal),
		Err(AmmoError::NotTfPlayer)
	));

	// Before a level's entities are created, there are no game rules to ask.
	ENTITIES.set(Vec::new());
	assert!(matches!(
		ammo::max_reserve(server, player, AmmoType::Metal),
		Err(AmmoError::GameRules(_))
	));
	assert_eq!(ASKED.get(), 17);
}

/// The adapter of every native member the fake player declares, which notes
/// its arguments, and acts as the member does on the fake player's fields.
unsafe extern "C" fn member(
	function: sys::ScriptFunctionBindingStorageType_t,
	object: *mut c_void,
	arguments: *mut sys::ScriptVariant_t,
	count: c_int,
	result: *mut sys::ScriptVariant_t,
) -> bool {
	let (name, returns, parameters) = MEMBERS[function.val_0 as usize];

	assert_eq!(count as usize, parameters.len());
	assert_eq!(result.is_null(), returns == binding::VOID);

	// SAFETY: The binding is only declared by fake entities' descriptors, so
	// the object is a fake entity, which is leaked. Its arguments have the
	// declared types, whose union members are read.
	let (object, received) = unsafe {
		let received: Vec<_> = parameters
			.iter()
			.enumerate()
			.map(|(index, &kind)| {
				let value = &(*arguments.add(index)).__bindgen_anon_1;

				match kind {
					BOOL => Argument::Bool(value.m_bool),
					FLOAT => Argument::Float(value.m_float),
					INT => Argument::Int(value.m_int),
					STRING => Argument::String(CStr::from_ptr(value.m_pszString).to_owned()),

					VECTOR => {
						let vector = *value.m_pVector;

						Argument::Vector([vector.x, vector.y, vector.z])
					}

					QANGLE => {
						let angles = *value.m_pData.cast::<sys::QAngle>();

						Argument::Angles([angles.x, angles.y, angles.z])
					}

					kind => panic!("no fake member takes type {kind}"),
				}
			})
			.collect();

		(&mut *object.cast::<FakeEntity>(), received)
	};

	CALLS.with_borrow_mut(|calls| calls.push((name, received.clone())));

	if !ACCEPTS.get() {
		return false;
	}

	let value = match (name.to_bytes(), received.as_slice()) {
		(b"AddHudHideFlags", &[Argument::Int(flags)]) => {
			object.hud |= flags;
			None
		}

		(b"RemoveHudHideFlags", &[Argument::Int(flags)]) => {
			object.hud &= !flags;
			None
		}

		(b"SetHudHideFlags", &[Argument::Int(flags)]) => {
			object.hud = flags;
			None
		}

		(b"SetNextChangeClassTime", &[Argument::Float(time)]) => {
			object.next_class_time = time;
			None
		}

		(b"GetHudHideFlags", []) => Some(int(object.hud)),
		(b"GetNextChangeClassTime", []) => Some(float(object.next_class_time)),
		(b"IsCallingForMedic", []) => Some(boolean(true)),
		_ => None,
	};

	if let Some(value) = value {
		// SAFETY: The member returns a value, so the caller passes a writable
		// result.
		unsafe { result.write(value) };
	}

	true
}

#[test]
fn members_are_called_with_their_arguments() {
	let world = World::new();
	let scope = ();
	let player = world.wrap(&scope);

	player.set_class(PlayerClass::Pyro).unwrap();
	player.force_change_team(Team::Blue, true).unwrap();
	player
		.set_custom_model(c"models/bots/bot_scout.mdl")
		.unwrap();
	player.clear_custom_model().unwrap();
	player.apply_impulse(Vector::new(0.0, 0.0, 300.0)).unwrap();
	player
		.set_custom_model_rotation(QAngle {
			pitch: 0.0,
			yaw: 45.0,
			roll: 0.0,
		})
		.unwrap();
	player.regenerate(false).unwrap();

	assert_eq!(
		calls(),
		[
			(c"SetPlayerClass", vec![Argument::Int(7)]),
			(
				c"ForceChangeTeam",
				vec![Argument::Int(3), Argument::Bool(true)]
			),
			(
				c"SetCustomModel",
				vec![Argument::String(c"models/bots/bot_scout.mdl".to_owned())]
			),
			(c"SetCustomModel", vec![Argument::String(c"".to_owned())]),
			(
				c"ApplyAbsVelocityImpulse",
				vec![Argument::Vector([0.0, 0.0, 300.0])]
			),
			(
				c"SetCustomModelRotation",
				vec![Argument::Angles([0.0, 45.0, 0.0])]
			),
			(c"Regenerate", vec![Argument::Bool(false)]),
		]
	);

	// The HUD flags round-trip, unknown bits included.
	player
		.set_hud_hide_flags(HideHud::CROSSHAIR | HideHud::HEALTH)
		.unwrap();
	player.add_hud_hide_flags(HideHud::CHAT).unwrap();
	player.remove_hud_hide_flags(HideHud::HEALTH).unwrap();
	assert_eq!(
		player.hud_hide_flags().unwrap(),
		HideHud::CROSSHAIR | HideHud::CHAT
	);
	player
		.set_hud_hide_flags(HideHud::from_bits_retain(1 << 20))
		.unwrap();
	assert_eq!(player.hud_hide_flags().unwrap().bits(), 1 << 20);

	player.set_next_change_class_time(12.5).unwrap();
	assert_eq!(player.next_change_class_time().unwrap(), 12.5);
	assert!(player.is_calling_for_medic().unwrap());
	calls();

	// Members the player's script class lacks.
	assert!(matches!(
		player.set_forced_taunt_cam(ForcedTauntCam::Always),
		Err(PlayerError::UnsupportedMethod)
	));
	assert!(matches!(
		player.next_change_team_time(),
		Err(PlayerError::UnsupportedMethod)
	));

	// Members whose adapters refuse the call.
	ACCEPTS.set(false);
	assert!(matches!(
		player.set_class(PlayerClass::Scout),
		Err(PlayerError::Rejected)
	));
	assert_eq!(calls().len(), 1);
}

#[test]
fn members_refuse_bad_values_and_deleted_players_before_the_call() {
	let world = World::new();
	let scope = ();
	let player = world.wrap(&scope);

	for result in [
		player.apply_impulse(Vector::new(f32::NAN, 0.0, 0.0)),
		player.set_custom_model_offset(Vector::new(0.0, f32::INFINITY, 0.0)),
		player.set_custom_model_rotation(QAngle {
			pitch: 0.0,
			yaw: 0.0,
			roll: f32::NEG_INFINITY,
		}),
		player.set_next_change_class_time(f32::NAN),
	] {
		assert!(matches!(result, Err(PlayerError::NonFinite)));
	}

	// Only living players are resupplied.
	// SAFETY: As for the fields written in `World::new`.
	unsafe { (*world.player).alive = false };
	assert!(matches!(
		player.regenerate(true),
		Err(PlayerError::NotAlive)
	));

	// SAFETY: As above.
	unsafe { (*world.player).flags = EFL_KILLME };
	assert!(matches!(
		player.set_class(PlayerClass::Scout),
		Err(PlayerError::MarkedForDeletion)
	));
	assert!(matches!(
		player.hud_hide_flags(),
		Err(PlayerError::MarkedForDeletion)
	));
	assert!(matches!(
		player.eye_angles(),
		Err(PlayerError::MarkedForDeletion)
	));
	assert!(calls().is_empty());
	assert!(vcalls().is_empty());
}

unsafe extern "C" fn networkable(entity: *mut sys::IServerUnknown) -> *mut sys::IServerNetworkable {
	// SAFETY: As for `commit_suicide`.
	unsafe { &raw mut (*entity.cast::<FakeEntity>()).networkable }
}

/// The offset of a field, as a property declares it.
fn offset(at: usize) -> c_int {
	c_int::try_from(at).unwrap()
}

#[test]
fn only_tf2_players_are_wrapped() {
	let world = World::new();
	let scope = ();

	assert!(matches!(
		TfPlayer::new(
			null_server(Game::SourceSdk2013, &scope),
			entity(world.player)
		),
		Err(PlayerError::NotTfPlayer)
	));

	for other in [world.prop, world.ragdoll, world.weapon] {
		assert!(matches!(
			TfPlayer::new(mock_server(&scope), entity(other)),
			Err(PlayerError::NotTfPlayer)
		));
	}

	assert!(TfPlayer::new(mock_server(&scope), entity(world.player)).is_ok());
}

/// The descriptors of a TF2 player's script class, declaring [`MEMBERS`],
/// leaked.
fn player_description() -> *mut sys::ScriptClassDesc_t {
	let bindings: Vec<_> = MEMBERS
		.iter()
		.enumerate()
		.map(|(index, &(name, returns, parameters))| {
			let parameters = Vec::from(parameters).leak();
			let mut binding = member_binding(name, returns, parameters, Some(member));

			binding.m_pFunction.val_0 = index as isize;
			binding
		})
		.collect();
	let base = leak(class_description(c"CBasePlayer", &mut [], null_mut()));

	leak(class_description(c"CTFPlayer", bindings.leak(), base))
}

#[test]
fn ragdolls_are_found_read_flagged_and_removed() {
	let world = World::new();
	let scope = ();
	let player = world.wrap(&scope);
	let ragdoll = player.ragdoll().unwrap().unwrap();

	assert_eq!(ragdoll.entity().as_ptr(), world.ragdoll.cast());
	assert_eq!(
		ragdoll.player().unwrap().map(Entity::as_ptr),
		Some(world.player.cast())
	);
	assert_eq!(ragdoll.class().unwrap(), Some(PlayerClass::Medic));
	assert_eq!(ragdoll.team().unwrap(), Team::Red);
	assert_eq!(ragdoll.force().unwrap(), Vector::new(10.0, 0.0, 5.0));
	assert_eq!(ragdoll.force_bone().unwrap(), 7);
	assert_eq!(ragdoll.damage_custom().unwrap(), 1);
	assert!(ragdoll.flag(RagdollFlag::Burning).unwrap());
	assert!(!ragdoll.flag(RagdollFlag::Gib).unwrap());

	// Flags are written and networked.
	ragdoll.set_flag(RagdollFlag::Gib, true).unwrap();
	ragdoll.set_flag(RagdollFlag::Burning, false).unwrap();
	assert!(world.changed(world.ragdoll));
	// SAFETY: As for the fields written in `World::new`.
	assert_eq!(
		unsafe { ((*world.ragdoll).gib, (*world.ragdoll).burning) },
		(true, false)
	);

	// The fake ragdoll declares no other flag.
	assert!(matches!(
		ragdoll.flag(RagdollFlag::Gold),
		Err(TfRagdollError::NetProp(_))
	));

	// Other entities are no ragdolls.
	assert!(matches!(
		TfRagdoll::new(mock_server(&scope), entity(world.prop)),
		Err(TfRagdollError::NotTfRagdoll)
	));

	// Removal clears the player's handle, after which they have none.
	assert!(player.remove_ragdoll().unwrap());
	// SAFETY: As above.
	unsafe {
		assert_eq!((*world.player).ragdoll, EntityHandle::INVALID.to_raw());
		assert_ne!((*world.ragdoll).flags & EFL_KILLME, 0);
	}
	assert!(player.ragdoll().unwrap().is_none());
	assert!(!player.remove_ragdoll().unwrap());
	assert!(matches!(
		ragdoll.set_flag(RagdollFlag::Gib, false),
		Err(TfRagdollError::MarkedForDeletion)
	));

	// A handle to another kind of entity is no ragdoll.
	// SAFETY: As above.
	unsafe { (*world.player).ragdoll = (*world.prop).handle };
	assert!(player.ragdoll().unwrap().is_none());
}

/// `IServerEntity::GetRefEHandle`, pointing to the fake entity's handle.
unsafe extern "C" fn ref_handle(entity: *const sys::IServerEntity) -> *const sys::CBaseHandle {
	// SAFETY: As for `commit_suicide`.
	unsafe { (&raw const (*entity.cast::<FakeEntity>()).handle).cast() }
}

/// `IServerTools::RemoveEntity`, which marks the entity for deletion, as the
/// game's deferred deletion does.
unsafe extern "C" fn remove_entity(_: *mut sys::IServerTools, entity: *mut sys::CBaseEntity) {
	// SAFETY: As for `commit_suicide`.
	unsafe { (*entity.cast::<FakeEntity>()).flags |= EFL_KILLME };
}

#[test]
fn reserve_ammo_is_read_and_set_by_type() {
	let world = World::new();
	let scope = ();
	let server = mock_server(&scope);
	let player = entity(world.player);

	assert_eq!(
		ammo::reserve(server, player, AmmoType::Primary).unwrap(),
		32
	);
	assert_eq!(ammo::reserve(server, player, AmmoType::Metal).unwrap(), 0);

	ammo::set_reserve(server, player, AmmoType::Secondary, 500).unwrap();
	assert!(world.changed(world.player));
	// SAFETY: As for the fields written in `World::new`.
	assert_eq!(unsafe { (*world.player).ammo }, [0, 32, 500, 0, 1, 0, 0]);

	assert!(matches!(
		ammo::set_reserve(server, player, AmmoType::Secondary, -1),
		Err(AmmoError::InvalidReserve(-1))
	));
	assert!(matches!(
		ammo::reserve(server, entity(world.prop), AmmoType::Primary),
		Err(AmmoError::NotTfPlayer)
	));
	assert!(!world.changed(world.player));
}

unsafe extern "C" fn server_class(this: *mut sys::IServerNetworkable) -> *mut sys::ServerClass {
	// SAFETY: As for `edict`.
	unsafe { (*fake_of(this)).class }
}

#[test]
fn teams_states_and_flags_keep_tf2s_numbers() {
	for team in Team::ALL {
		assert_eq!(Team::from_raw(team.to_raw()), Some(team));
	}

	assert_eq!(Team::ALL.map(Team::to_raw), [0, 1, 2, 3]);
	assert_eq!(Team::from_raw(4), None);
	assert_eq!(Team::Red.opponent(), Some(Team::Blue));
	assert_eq!(Team::Spectator.opponent(), None);
	assert_eq!(Team::Blue.scoring(), Some(ScoringTeam::Blue));
	assert_eq!(Team::Unassigned.scoring(), None);
	assert_eq!(Team::from(ScoringTeam::Red), Team::Red);
	assert!(Team::PLAYING.iter().all(|team| team.is_playing()));

	for raw in raw::TF_STATE_ACTIVE..raw::TF_STATE_COUNT {
		assert_eq!(PlayerState::from_raw(raw).unwrap().to_raw(), raw);
	}

	assert_eq!(PlayerState::from_raw(raw::TF_STATE_COUNT), None);

	// The named flags are the bits clients receive.
	assert_eq!(HideHud::all().bits(), (1 << raw::HIDEHUD_BITCOUNT) - 1);
	assert_eq!(
		[
			ForcedTauntCam::Off,
			ForcedTauntCam::WhileAlive,
			ForcedTauntCam::Always
		]
		.map(ForcedTauntCam::to_raw),
		[0, 1, 2]
	);

	assert_eq!(
		RagdollFlag::ALL.map(RagdollFlag::net_prop_name).len(),
		RagdollFlag::ALL.len()
	);
}

/// Takes the vtable calls made since the last.
fn vcalls() -> Vec<(&'static str, usize, Vec<Argument>)> {
	VCALLS.take()
}

#[test]
fn view_methods_read_the_player_and_suicide_kills_the_living() {
	let world = World::new();
	let scope = ();
	let player = world.wrap(&scope);

	assert_eq!(
		player.eye_angles().unwrap(),
		QAngle {
			pitch: 10.0,
			yaw: 90.0,
			roll: 0.0,
		}
	);
	assert_eq!(
		player.eye_position().unwrap(),
		Vector::new(100.0, 200.0, 68.0)
	);

	assert!(player.commit_suicide(true, false).unwrap());
	assert_eq!(
		vcalls(),
		[(
			"CommitSuicide",
			world.player.addr(),
			vec![Argument::Bool(true), Argument::Bool(false)]
		)]
	);

	// The dead are left alone.
	assert!(!player.commit_suicide(false, true).unwrap());
	assert!(vcalls().is_empty());
}

/// `CTFPlayer::Weapon_Switch`, which makes the weapon the active one.
unsafe extern "C" fn weapon_switch(
	player: *mut sys::CTFPlayer,
	weapon: *mut sys::CBaseCombatWeapon,
	view_model: c_int,
) -> bool {
	let fake = player.cast::<FakeEntity>();

	VCALLS.with_borrow_mut(|calls| {
		calls.push((
			"Weapon_Switch",
			fake.addr(),
			vec![Argument::Entity(weapon.addr()), Argument::Int(view_model)],
		));
	});
	// SAFETY: As for `commit_suicide`.
	unsafe { (*fake).active_weapon = (*weapon.cast::<FakeEntity>()).handle };
	true
}

#[test]
fn weapons_are_listed_and_switched_to() {
	let world = World::new();
	let scope = ();
	let server = mock_server(&scope);
	let weapons = PlayerWeapons::new(server, entity(world.player)).unwrap();

	assert_eq!(
		weapons
			.active()
			.unwrap()
			.map(|weapon| weapon.entity().as_ptr()),
		Some(world.weapon.cast())
	);

	// The prop in the inventory, and the empty slot, are skipped.
	assert_eq!(
		weapons
			.all()
			.unwrap()
			.iter()
			.map(|weapon| weapon.entity().as_ptr())
			.collect::<Vec<_>>(),
		[world.weapon.cast()]
	);

	// SAFETY: As for the fields written in `World::new`.
	unsafe { (*world.player).active_weapon = (*world.prop).handle };
	assert!(weapons.active().unwrap().is_none());

	let weapon = Weapon::new(server, entity(world.weapon)).unwrap();

	assert!(weapons.switch_to(weapon).unwrap());
	assert_eq!(
		vcalls(),
		[(
			"Weapon_Switch",
			world.player.addr(),
			vec![Argument::Entity(world.weapon.addr()), Argument::Int(0)]
		)]
	);
	// SAFETY: As above.
	assert_eq!(unsafe { (*world.player).active_weapon }, unsafe {
		(*world.weapon).handle
	});

	// Another player's weapon is refused before the game is called.
	let stolen = Weapon::new(server, entity(world.stolen)).unwrap();

	assert!(matches!(
		weapons.switch_to(stolen),
		Err(WeaponError::DifferentOwner)
	));
	assert!(vcalls().is_empty());
}
