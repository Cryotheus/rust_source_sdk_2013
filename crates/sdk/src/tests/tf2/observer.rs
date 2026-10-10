//! Tests of TF2 spectating through a fake player's networked observer
//! variables and its `SetObserverTarget`.

use super::*;
use crate::Module;
use crate::datatables::PropFlags;
use crate::interfaces::{ServerTools, ValveEngine};
use crate::test_support::datatables::{direct_table, int32_proxy, prop, table, table_prop};
use crate::test_support::edicts::{change_accessor, shared_change_info};
use crate::test_support::entities::MOCK_EFLAGS_OFFSET;
use crate::test_support::interfaces::server_game_dll::export_standard_proxies;
use crate::test_support::leak;
use crate::test_support::server::{export, mock_server, null_server};
use sdk_raw::edicts::FL_FULL_EDICT_CHANGED;
use sdk_raw::entities::NUM_NETWORKED_EHANDLE_BITS;
use sdk_raw::test_support::edicts::mock_edict;
use sdk_raw::test_support::entities::{data_map, field};
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use std::cell::{Cell, RefCell};
use std::ffi::{c_char, c_void};
use std::mem::offset_of;
use std::ptr::{NonNull, null_mut};

const _: () = assert!(offset_of!(FakeEntity, flags) == MOCK_EFLAGS_OFFSET);

/// A null handle, in a fake entity's fields.
const NULL: u32 = u32::MAX;

/// The view offset of a player following a standing target in first person.
const STANDING: [f32; 3] = [0.0, 0.0, 68.0];

/// A player, or another entity, whose datamaps and send tables describe these
/// fields.
#[repr(C)]
struct FakeEntity {
	vtable: *const *const (),
	map: *mut sys::datamap_t,
	networkable: sys::IServerNetworkable,
	/// Keeps `flags` at the offset every mock entity has it.
	_collideable: *const c_void,
	flags: c_int,
	handle: u32,
	mode: c_int,
	target: u32,
	view_offset: [f32; 3],
	forced_mode: bool,
	class: *mut sys::ServerClass,
	edict: *mut sys::edict_t,
}

thread_local! {
	/// `gEntList`, which `ServerTools::entity_by_handle` reads.
	static ENTITY_LIST: Cell<*mut sys::CGlobalEntityList> = const { Cell::new(null_mut()) };

	/// Each call of `SetObserverTarget`: the player, the target, and the
	/// player's mode during the call.
	static TARGETED: RefCell<Vec<(usize, usize, c_int)>> = const { RefCell::new(Vec::new()) };
}

/// Mock interfaces exported on this thread, the classes of its fake
/// entities, and its player and the player's target.
struct World {
	player: *mut FakeEntity,
	target: *mut FakeEntity,
	/// An entity that is not a player.
	prop: *mut FakeEntity,
}

impl World {
	fn new() -> Self {
		let slot = |field: usize| field / size_of::<usize>();
		let slots =
			raw::SET_OBSERVER_TARGET_SLOT.max(sdk_raw::entities::GET_DATA_DESC_MAP_SLOT) + 1;
		let mut vtable = vec![unexpected_call as *const (); slots];

		vtable[sdk_raw::entities::GET_DATA_DESC_MAP_SLOT] = datamap as *const ();
		vtable[slot(offset_of!(
			sys::IServerUnknown__bindgen_vtable,
			IServerUnknown_GetNetworkable
		))] = networkable as *const ();
		vtable[raw::SET_OBSERVER_TARGET_SLOT] = set_observer_target as *const ();

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

		let mut flags = field(
			c"m_iEFlags",
			sys::_fieldtypes_FIELD_INTEGER,
			MOCK_EFLAGS_OFFSET,
		);

		flags.fieldSizeInBytes = size_of::<c_int>() as c_int;

		let base_map = data_map(c"CBaseEntity", vec![flags], null_mut());
		let mut forced_mode = field(
			c"m_bForcedObserverMode",
			sys::_fieldtypes_FIELD_BOOLEAN,
			offset_of!(FakeEntity, forced_mode),
		);
		forced_mode.fieldSize = 1;
		forced_mode.fieldSizeInBytes = size_of::<bool>() as c_int;
		let player_map = data_map(
			c"CTFPlayer",
			vec![],
			data_map(c"CBasePlayer", vec![forced_mode], base_map),
		);
		let prop_map = data_map(c"CDynamicProp", vec![], base_map);

		let float_prop = |name, offset: usize| {
			prop(
				name,
				sys::SendPropType_DPT_Float,
				c_int::try_from(offset).unwrap(),
				PropFlags::default(),
				Some(int32_proxy),
			)
		};
		let view_offset = offset_of!(FakeEntity, view_offset);
		let local_props = leak([
			float_prop(c"m_vecViewOffset[0]", view_offset),
			float_prop(c"m_vecViewOffset[1]", view_offset + size_of::<f32>()),
			float_prop(c"m_vecViewOffset[2]", view_offset + 2 * size_of::<f32>()),
		]);
		// SAFETY: The properties are leaked, and only their table, which views
		// them, reaches them from here on. The same holds for the tables below.
		let local_table = leak(table(c"DT_LocalPlayerExclusive", unsafe {
			&mut *local_props
		}));

		let mut target_prop = prop(
			c"m_hObserverTarget",
			sys::SendPropType_DPT_Int,
			c_int::try_from(offset_of!(FakeEntity, target)).unwrap(),
			PropFlags::UNSIGNED,
			Some(handle_proxy),
		);

		target_prop.m_nBits = NUM_NETWORKED_EHANDLE_BITS as c_int;

		let player_props = leak([
			table_prop(c"localdata", 0, local_table, Some(direct_table)),
			prop(
				c"m_iObserverMode",
				sys::SendPropType_DPT_Int,
				c_int::try_from(offset_of!(FakeEntity, mode)).unwrap(),
				PropFlags::UNSIGNED,
				Some(int32_proxy),
			),
			target_prop,
		]);
		// SAFETY: As for the local table.
		let player_table = leak(table(c"DT_TFPlayer", unsafe { &mut *player_props }));
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
		let player_class = class(c"CTFPlayer", player_table, 1);
		let prop_class = class(c"CDynamicProp", prop_table, 2);

		export_interfaces();

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
				mode: raw::OBS_MODE_IN_EYE,
				target: NULL,
				view_offset: STANDING,
				forced_mode: false,
				class,
				edict: leak(mock_edict(index.cast_signed(), false)),
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
			target: spawn(2, player_map, player_class),
			prop: spawn(3, prop_map, prop_class),
		};

		// SAFETY: Fake entities are leaked, and the mock game is not running
		// while the test writes their fields.
		unsafe { (*world.player).target = (*world.target).handle };
		world
	}

	/// Whether the engine was told that the player's networked variables
	/// changed.
	fn player_changed(&self) -> bool {
		// SAFETY: As for the target's handle in `new`.
		unsafe { (*(*self.player).edict)._base.m_fStateFlags & FL_FULL_EDICT_CHANGED != 0 }
	}

	/// The player's fields: its mode, target and view offset.
	fn player_state(&self) -> (c_int, u32, [f32; 3]) {
		// SAFETY: As for the target's handle in `new`.
		unsafe {
			(
				(*self.player).mode,
				(*self.player).target,
				(*self.player).view_offset,
			)
		}
	}

	/// Sets the player's mode, as the game would have.
	fn set_mode(&self, mode: c_int) {
		// SAFETY: As for the target's handle in `new`.
		unsafe { (*self.player).mode = mode };
	}
}

#[test]
fn already_roaming_clears_only_the_forced_mode_flag() {
	let world = World::new();
	let scope = ();
	let observer = PlayerObserver::new(mock_server(&scope), entity(world.player)).unwrap();
	world.set_mode(raw::OBS_MODE_ROAMING);
	// SAFETY: The fake player is leaked, and no engine runs during this test.
	unsafe { (*world.player).forced_mode = true };
	let initial = world.player_state();
	observer.roam().unwrap();
	assert_eq!(world.player_state(), initial);
	// SAFETY: As above.
	assert!(!unsafe { (*world.player).forced_mode });
	assert!(world.player_changed());
	assert!(TARGETED.take().is_empty());
}

unsafe extern "C" fn class_name(this: *const sys::IServerNetworkable) -> *const c_char {
	// SAFETY: Only fake entities' networkables have this method, and they
	// are fields of their entities.
	unsafe { (*(*fake_of(this)).class).m_pNetworkName }
}

unsafe extern "C" fn datamap(entity: *mut sys::CBaseEntity) -> *mut sys::datamap_t {
	// SAFETY: Only fake entities have this method in their vtables.
	unsafe { (*entity.cast::<FakeEntity>()).map }
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
}

/// The fake entity whose networkable is `this`.
fn fake_of(this: *const sys::IServerNetworkable) -> *mut FakeEntity {
	// SAFETY: Callers pass the networkable of a fake entity, so the entity
	// starts that far before it.
	unsafe { this.byte_sub(offset_of!(FakeEntity, networkable)) }
		.cast_mut()
		.cast()
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

#[test]
fn missing_or_mistyped_forced_mode_leaves_the_view_unchanged() {
	let world = World::new();
	let scope = ();
	let observer = PlayerObserver::new(mock_server(&scope), entity(world.player)).unwrap();
	let initial = world.player_state();
	// SAFETY: World builds this leaked CTFPlayer/CBasePlayer/CBaseEntity chain.
	let base_map = unsafe { (*(*(*world.player).map).baseMap).baseMap };
	// SAFETY: The fake player's storage is leaked.
	unsafe { (*world.player).forced_mode = true };
	for field_type in [None, Some(sys::_fieldtypes_FIELD_INTEGER)] {
		let fields = field_type
			.map(|kind| {
				let mut description = field(
					c"m_bForcedObserverMode",
					kind,
					offset_of!(FakeEntity, forced_mode),
				);
				description.fieldSize = 1;
				description.fieldSizeInBytes = size_of::<bool>() as c_int;
				description
			})
			.into_iter()
			.collect();
		// SAFETY: The fake player and its datamaps are leaked; changing its
		// map emulates a game image missing or changing the declared field.
		unsafe {
			(*world.player).map = data_map(
				c"CTFPlayer",
				vec![],
				data_map(c"CBasePlayer", fields, base_map),
			);
		}
		assert!(matches!(observer.roam(), Err(ObserverError::Field(_))));
		assert_eq!(world.player_state(), initial);
		// SAFETY: The fake player's storage remains live.
		assert!(unsafe { (*world.player).forced_mode });
		assert!(!world.player_changed());
		assert!(TARGETED.take().is_empty());
	}
}

#[test]
fn modes_keep_tf2s_numbers() {
	for raw in raw::OBS_MODE_NONE..raw::NUM_OBSERVER_MODES {
		assert_eq!(ObserverMode::from_raw(raw).unwrap().to_raw(), raw);
	}

	assert_eq!(
		ObserverMode::from_raw(raw::OBS_MODE_ROAMING),
		Some(ObserverMode::Roaming)
	);
	assert_eq!(
		ObserverMode::from_raw(raw::OBS_MODE_POI),
		Some(ObserverMode::PointOfInterest)
	);
	assert_eq!(ObserverMode::from_raw(-1), None);
	assert_eq!(ObserverMode::from_raw(raw::NUM_OBSERVER_MODES), None);
}

unsafe extern "C" fn networkable(entity: *mut sys::IServerUnknown) -> *mut sys::IServerNetworkable {
	// SAFETY: As for `datamap`.
	unsafe { &raw mut (*entity.cast::<FakeEntity>()).networkable }
}

#[test]
fn only_tf2_players_are_wrapped() {
	let world = World::new();
	let scope = ();

	assert!(matches!(
		PlayerObserver::new(
			null_server(Game::SourceSdk2013, &scope),
			entity(world.player)
		),
		Err(ObserverError::NotTfPlayer)
	));
	assert!(matches!(
		PlayerObserver::new(mock_server(&scope), entity(world.prop)),
		Err(ObserverError::NotTfPlayer)
	));
	assert!(PlayerObserver::new(mock_server(&scope), entity(world.player)).is_ok());
}

#[test]
fn roaming_leaves_roamers_and_refuses_players_not_spectating() {
	let world = World::new();
	let scope = ();
	let observer = PlayerObserver::new(mock_server(&scope), entity(world.player)).unwrap();
	// SAFETY: As for the target's handle in `World::new`.
	let target = unsafe { (*world.target).handle };

	world.set_mode(raw::OBS_MODE_ROAMING);
	observer.roam().unwrap();
	assert_eq!(
		world.player_state(),
		(raw::OBS_MODE_ROAMING, target, STANDING)
	);
	assert!(!world.player_changed());

	for (raw, mode) in [
		(raw::OBS_MODE_NONE, ObserverMode::None),
		(raw::OBS_MODE_DEATHCAM, ObserverMode::DeathCam),
		(raw::OBS_MODE_FREEZECAM, ObserverMode::FreezeCam),
	] {
		world.set_mode(raw);
		assert!(matches!(
			observer.roam(),
			Err(ObserverError::NotObserving(refused)) if refused == mode
		));
		assert_eq!(world.player_state(), (raw, target, STANDING));
	}

	world.set_mode(raw::NUM_OBSERVER_MODES);
	assert!(matches!(
		observer.roam(),
		Err(ObserverError::UnknownMode(raw)) if raw == raw::NUM_OBSERVER_MODES
	));
	assert!(!world.player_changed());
	assert!(TARGETED.take().is_empty());

	// SAFETY: As for the target's handle in `World::new`.
	unsafe { (*world.player).flags |= sdk_raw::entities::EFL_KILLME };
	world.set_mode(raw::OBS_MODE_IN_EYE);
	assert!(matches!(
		observer.roam(),
		Err(ObserverError::MarkedForDeletion)
	));
	assert_eq!(world.player_state().0, raw::OBS_MODE_IN_EYE);
}

#[test]
fn roaming_switches_the_mode_and_moves_behind_the_target() {
	let world = World::new();
	let scope = ();
	let observer = PlayerObserver::new(mock_server(&scope), entity(world.player)).unwrap();
	// SAFETY: As for the target's handle in `World::new`.
	let target = unsafe { (*world.target).handle };

	assert_eq!(observer.mode().unwrap(), ObserverMode::InEye);
	assert_eq!(
		observer.target().unwrap().map(|target| target.as_ptr()),
		Some(world.target.cast())
	);

	// SAFETY: The fake player's storage is leaked.
	unsafe { (*world.player).forced_mode = true };
	observer.roam().unwrap();
	assert_eq!(observer.mode().unwrap(), ObserverMode::Roaming);
	assert_eq!(
		world.player_state(),
		(raw::OBS_MODE_ROAMING, target, [0.0; 3])
	);
	assert!(world.player_changed());
	// SAFETY: The fake player is leaked and no engine runs during this test.
	assert!(!unsafe { (*world.player).forced_mode });

	// The game moves the player behind the target as they roam already.
	assert_eq!(
		TARGETED.take(),
		[(
			world.player.addr(),
			world.target.addr(),
			raw::OBS_MODE_ROAMING
		)]
	);

	// Without a target, the player roams from where they are.
	for mode in [raw::OBS_MODE_FIXED, raw::OBS_MODE_CHASE, raw::OBS_MODE_POI] {
		// SAFETY: As for the target's handle in `World::new`.
		unsafe {
			(*world.player).target = NULL;
			(*world.player).view_offset = STANDING;
			(*world.player).forced_mode = true;
		}
		world.set_mode(mode);

		assert_eq!(observer.target().unwrap(), None);
		observer.roam().unwrap();
		assert_eq!(
			world.player_state(),
			(raw::OBS_MODE_ROAMING, NULL, [0.0; 3])
		);
		// SAFETY: As above.
		assert!(!unsafe { (*world.player).forced_mode });
	}

	assert!(TARGETED.take().is_empty());
}

unsafe extern "C" fn server_class(this: *mut sys::IServerNetworkable) -> *mut sys::ServerClass {
	// SAFETY: As for `class_name`.
	unsafe { (*fake_of(this)).class }
}

/// Notes the call, and follows the target, as `CBasePlayer::SetObserverTarget`
/// does for a valid one.
unsafe extern "C" fn set_observer_target(
	player: *mut sys::CTFPlayer,
	target: *mut sys::CBaseEntity,
) -> bool {
	let player = player.cast::<FakeEntity>();
	let target = target.cast::<FakeEntity>();

	// SAFETY: The wrappers pass the fake player and its fake target, which are
	// leaked.
	let mode = unsafe {
		(*player).target = (*target).handle;
		(*player).mode
	};

	TARGETED.with_borrow_mut(|targeted| targeted.push((player.addr(), target.addr(), mode)));
	true
}
