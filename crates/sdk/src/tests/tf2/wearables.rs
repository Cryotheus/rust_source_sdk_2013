//! Tests of TF2 wearables through a fake player's native wearable methods and
//! networked list.

use super::*;
use crate::Module;
use crate::datatables::PropFlags;
use crate::interfaces::{PlayerInfoManager, ValveEngine};

use crate::test_support::datatables::{
	direct_table, int8_proxy, int16_proxy, pointer_table, prop, table, table_prop,
};

use crate::test_support::edicts::{change_accessor, shared_change_info};
use crate::test_support::entities::MOCK_EFLAGS_OFFSET;
use crate::test_support::interfaces::player_info_manager::{global_vars, serve_global_vars};
use crate::test_support::interfaces::server_game_dll::export_standard_proxies;
use crate::test_support::leak;
use crate::test_support::server::{export, mock_server};
use sdk_raw::test_support::edicts::mock_edict;
use sdk_raw::test_support::entities::{data_map, field};
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use std::cell::{Cell, RefCell};
use std::ffi::{c_char, c_void};
use std::mem::offset_of;
use std::ptr::null_mut;

const EQUIP: usize =
	offset_of!(sys::CTFPlayer__bindgen_vtable, CTFPlayer_EquipWearable) / size_of::<usize>();

/// A null handle, in a mock entity's fields and wearable list.
const NULL: u32 = u32::MAX;

const REMOVE: usize =
	offset_of!(sys::CTFPlayer__bindgen_vtable, CTFPlayer_RemoveWearable) / size_of::<usize>();

/// A player or wearable, whose datamaps and send tables describe these
/// fields.
#[repr(C)]
struct FakeEntity {
	vtable: *const *const (),
	map: *mut sys::datamap_t,
	networkable: sys::IServerNetworkable,
	collideable: sys::ICollideable,
	flags: c_int,
	handle: u32,
	owner: u32,
	move_parent: u32,
	validated: bool,
	initialized: bool,
	disguise: bool,
	definition: u16,
	associated: u32,
	origin: sys::Vector,
	class: *mut sys::ServerClass,
	edict: *mut sys::edict_t,
	class_name: *const c_char,
	/// A player's `m_hMyWearables`, as raw handles.
	wearables: Vec<u32>,
	equip_calls: usize,
	remove_calls: usize,
}

thread_local! {
	static DEAD: Cell<bool> = const { Cell::new(false) };
	static ENTITIES: RefCell<Vec<(c_int, *mut FakeEntity)>> = const { RefCell::new(Vec::new()) };
	/// `gEntList`, which `ServerTools::entity_by_handle` reads.
	static ENTITY_LIST: Cell<*mut sys::CGlobalEntityList> = const { Cell::new(null_mut()) };
	/// What `EquipWearable` makes wearables follow instead of the player.
	static FOLLOW: Cell<Option<u32>> = const { Cell::new(None) };
	/// Makes `RemoveWearable` leave the list alone, as if another plugin
	/// intercepted it.
	static IGNORE_REMOVE: Cell<bool> = const { Cell::new(false) };
	static PLAYER_INFO: Cell<*mut sys::IPlayerInfo> = const { Cell::new(null_mut()) };
	/// Makes `EquipWearable` emulate `CanEquip` refusing the wearable.
	static REFUSE_EQUIP: Cell<bool> = const { Cell::new(false) };
	static TEAM: Cell<c_int> = const { Cell::new(FIRST_GAME_TEAM) };
	static TOOL_REMOVALS: Cell<usize> = const { Cell::new(0) };
	static VALIDATED_AT_EQUIP: Cell<Option<bool>> = const { Cell::new(None) };
}

/// Mock interfaces exported on this thread, the classes of its fake
/// entities, and its player.
struct World {
	vtable: *const *const (),
	networkable: *const sys::IServerNetworkable__bindgen_vtable,
	collideable: *const sys::ICollideable__bindgen_vtable,
	base_map: *mut sys::datamap_t,
	wearable_map: *mut sys::datamap_t,
	wearable_class: *mut sys::ServerClass,
	/// The player's `m_hMyWearables` property, nesting `elements_table`.
	wearables_prop: *mut sys::SendProp,
	elements_table: *mut sys::SendTable,
	/// The `m_hMyWearables` table's properties: the length table, then
	/// `000` to `007`.
	elements: *mut [sys::SendProp; MAX_NETWORKED_WEARABLES + 1],
	player: *mut FakeEntity,
}

impl World {
	fn new() -> Self {
		let slot = |field: usize| field / size_of::<usize>();
		let mut vtable = vec![
			unexpected_call as *const ();
			EQUIP
				.max(REMOVE)
				.max(sdk_raw::entities::GET_DATA_DESC_MAP_SLOT)
				+ 1
		];

		vtable[sdk_raw::entities::GET_DATA_DESC_MAP_SLOT] = datamap as *const ();
		vtable[slot(offset_of!(
			sys::IServerUnknown__bindgen_vtable,
			IServerUnknown_GetRefEHandle
		))] = handle as *const ();
		vtable[slot(offset_of!(
			sys::IServerUnknown__bindgen_vtable,
			IServerUnknown_GetNetworkable
		))] = networkable as *const ();
		vtable[slot(offset_of!(
			sys::IServerUnknown__bindgen_vtable,
			IServerUnknown_GetCollideable
		))] = collideable as *const ();
		vtable[EQUIP] = equip_wearable as *const ();
		vtable[REMOVE] = remove_wearable as *const ();

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
		// SAFETY: As for the networkable's vtable.
		let collideable = Box::leak(unsafe {
			mock_vtable::<sys::ICollideable__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).ICollideable_GetCollisionOrigin).write(origin);
				},
			)
		});

		let base_map = data_map(
			c"CBaseEntity",
			vec![
				flags_field(),
				handle_field(c"m_hOwnerEntity", offset_of!(FakeEntity, owner)),
				handle_field(c"m_hMoveParent", offset_of!(FakeEntity, move_parent)),
			],
			null_mut(),
		);
		let player_map = data_map(
			c"CTFPlayer",
			vec![],
			data_map(c"CBasePlayer", vec![], base_map),
		);
		let wearable_map = data_map(
			c"CTFWearable",
			vec![],
			data_map(c"CEconWearable", vec![], base_map),
		);

		let wearable_props = leak([
			int_prop(
				c"m_bValidatedAttachedEntity",
				offset_of!(FakeEntity, validated),
				PropFlags::UNSIGNED,
				Some(int8_proxy),
			),
			int_prop(
				c"m_bInitialized",
				offset_of!(FakeEntity, initialized),
				PropFlags::UNSIGNED,
				Some(int8_proxy),
			),
			int_prop(
				c"m_bDisguiseWearable",
				offset_of!(FakeEntity, disguise),
				PropFlags::UNSIGNED,
				Some(int8_proxy),
			),
			int_prop(
				c"m_iItemDefinitionIndex",
				offset_of!(FakeEntity, definition),
				PropFlags::UNSIGNED,
				Some(int16_proxy),
			),
			int_prop(
				c"m_hWeaponAssociatedWith",
				offset_of!(FakeEntity, associated),
				PropFlags::default(),
				Some(handle_proxy),
			),
		]);
		// SAFETY: The properties are leaked, and only their table, which views
		// them, reaches them from here on. The same holds for the tables below.
		let wearable_table = leak(table(c"DT_TFWearable", unsafe { &mut *wearable_props }));

		let length_props = leak([int_prop(
			c"lengthprop8",
			0,
			PropFlags::UNSIGNED,
			Some(int8_proxy),
		)]);
		// SAFETY: As for the wearable's table.
		let length_table = leak(table(c"_LPT_m_hMyWearables_8", unsafe {
			&mut *length_props
		}));
		let elements: [sys::SendProp; MAX_NETWORKED_WEARABLES + 1] =
			std::array::from_fn(|index| match index.checked_sub(1) {
				None => table_prop(c"lengthproxy", 0, length_table, Some(pointer_table)),

				Some(element) => {
					let mut prop = int_prop(
						ELEMENT_NAMES[element],
						0,
						PropFlags::default(),
						Some(wearable_element),
					);

					// `SendPropUtlVector` stores each element's index here.
					prop.m_ElementStride = element as c_int;
					prop
				}
			});
		let elements = leak(elements);
		// SAFETY: As for the wearable's table. The test changes the elements
		// only through `World::elements`, between the wrappers' reads.
		let elements_table = leak(table(c"_ST_m_hMyWearables_8", unsafe { &mut *elements }));
		let player_props = leak([table_prop(
			c"m_hMyWearables",
			0,
			elements_table,
			Some(direct_table),
		)]);
		// SAFETY: As for the elements' table.
		let player_table = leak(table(c"DT_TFPlayer", unsafe { &mut *player_props }));

		let player_class = leak(sys::ServerClass {
			m_pNetworkName: c"CTFPlayer".as_ptr(),
			m_pTable: player_table,
			m_pNext: null_mut(),
			m_ClassID: 1,
			m_InstanceBaselineIndex: 0,
		});
		let wearable_class = leak(sys::ServerClass {
			m_pNetworkName: c"CTFWearable".as_ptr(),
			m_pTable: wearable_table,
			m_pNext: null_mut(),
			m_ClassID: 2,
			m_InstanceBaselineIndex: 0,
		});

		Self::export_interfaces();

		let mut world = Self {
			vtable: vtable.leak().as_ptr(),
			networkable,
			collideable,
			base_map,
			wearable_map,
			wearable_class,
			wearables_prop: player_props.cast(),
			elements_table,
			elements,
			player: null_mut(),
		};

		world.player = world.spawn(1, 1, player_map, player_class, c"player");
		world
	}

	/// Exports the engine and game interfaces the module uses.
	fn export_interfaces() {
		serve_global_vars(8);

		// SAFETY: The vtable holds only function pointers, `unexpected_call`
		// aborts whichever slot reaches it, and the patch only writes slots of
		// the vtable being built.
		let info = Box::leak(unsafe {
			mock_vtable::<sys::IPlayerInfo__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).IPlayerInfo_IsDead).write(is_dead);
					(&raw mut (*vtable).IPlayerInfo_GetTeamIndex).write(team_index);
				},
			)
		});
		PLAYER_INFO.set(leak(sys::IPlayerInfo { vtable_: info }));

		// SAFETY: As for the player info's vtable.
		let manager = Box::leak(unsafe {
			mock_vtable::<sys::IPlayerInfoManager__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).IPlayerInfoManager_GetGlobalVars).write(global_vars);
					(&raw mut (*vtable).IPlayerInfoManager_GetPlayerInfo).write(player_info);
				},
			)
		});
		export(
			Module::GameServer,
			PlayerInfoManager::VERSION,
			leak(sys::IPlayerInfoManager { vtable_: manager }),
		);

		export_standard_proxies();

		ENTITY_LIST.set(Box::leak(Box::<sys::CGlobalEntityList>::new_zeroed()).as_mut_ptr());

		// SAFETY: As for the player info's vtable.
		let tools = Box::leak(unsafe {
			mock_vtable::<sys::IServerTools__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).IServerTools_GetBaseEntityByEntIndex)
						.write(entity_by_index);
					(&raw mut (*vtable).IServerTools_GetEntityList).write(entity_list);
					(&raw mut (*vtable).IServerTools_RemoveEntity).write(remove_entity);
				},
			)
		});
		export(
			Module::GameServer,
			ServerTools::VERSION,
			leak(sys::IServerTools { vtable_: tools }),
		);

		// SAFETY: As for the player info's vtable.
		let engine = Box::leak(unsafe {
			mock_vtable::<sys::IVEngineServer__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).IVEngineServer_GetChangeAccessor).write(change_accessor);
					(&raw mut (*vtable).IVEngineServer_GetSharedEdictChangeInfo)
						.write(shared_change_info);
				},
			)
		});
		export(
			Module::Engine,
			ValveEngine::VERSION,
			leak(sys::IVEngineServer { vtable_: engine }),
		);
	}

	/// Creates an entity in edict slot `index`, which
	/// `GetBaseEntityByEntIndex` and the entity list find.
	fn spawn(
		&self,
		index: c_int,
		serial: u32,
		map: *mut sys::datamap_t,
		class: *mut sys::ServerClass,
		class_name: &'static CStr,
	) -> *mut FakeEntity {
		let fake = leak(FakeEntity {
			vtable: self.vtable,
			map,
			networkable: sys::IServerNetworkable {
				vtable_: self.networkable,
			},
			collideable: sys::ICollideable {
				vtable_: self.collideable,
			},
			flags: 0,
			handle: index.cast_unsigned() | serial << 16,
			owner: NULL,
			move_parent: NULL,
			validated: false,
			initialized: true,
			disguise: false,
			definition: 378,
			associated: NULL,
			origin: sys::Vector {
				x: 1.0,
				y: 2.0,
				z: 3.0,
			},
			class,
			edict: leak(mock_edict(index, false)),
			class_name: class_name.as_ptr(),
			wearables: Vec::new(),
			equip_calls: 0,
			remove_calls: 0,
		});

		ENTITIES.with_borrow_mut(|entities| entities.push((index, fake)));

		// SAFETY: The entity list is a leaked, zeroed `CGlobalEntityList`, and
		// tests spawn entities in slots within its `m_EntPtrArray`.
		unsafe {
			let info = (&raw mut (*ENTITY_LIST.get())._base.m_EntPtrArray)
				.cast::<sys::CEntInfo>()
				.add(usize::try_from(index).unwrap());

			(&raw mut (*info).m_pEntity).write(fake.cast());
			(&raw mut (*info).m_SerialNumber).write(c_int::try_from(serial).unwrap());
		}

		fake
	}

	/// A spawned, unowned `tf_wearable` in edict slot `index`.
	fn wearable(&self, index: c_int) -> *mut FakeEntity {
		self.spawn(
			index,
			1,
			self.wearable_map,
			self.wearable_class,
			c"tf_wearable",
		)
	}
}

unsafe extern "C" fn class_name(this: *const sys::IServerNetworkable) -> *const c_char {
	// SAFETY: Only fake entities' networkables have this method, and they
	// are fields of their entities.
	unsafe { (*fake_of(this, offset_of!(FakeEntity, networkable))).class_name }
}

unsafe extern "C" fn collideable(entity: *mut sys::IServerUnknown) -> *mut sys::ICollideable {
	// SAFETY: Only fake entities have this method in their vtables.
	unsafe { &raw mut (*entity.cast::<FakeEntity>()).collideable }
}

unsafe extern "C" fn datamap(entity: *mut sys::CBaseEntity) -> *mut sys::datamap_t {
	// SAFETY: As for `collideable`.
	unsafe { (*entity.cast::<FakeEntity>()).map }
}

unsafe extern "C" fn edict(this: *const sys::IServerNetworkable) -> *mut sys::edict_t {
	// SAFETY: As for `class_name`.
	unsafe { (*fake_of(this, offset_of!(FakeEntity, networkable))).edict }
}

/// Encodes a raw handle as `SendProxy_EHandleToInt` does.
fn encode(handle: u32) -> c_int {
	if handle == NULL {
		INVALID_NETWORKED_EHANDLE_VALUE.cast_signed()
	} else {
		((handle & 0x7FF) | ((handle >> 16) & 0x3FF) << 11).cast_signed()
	}
}

/// A callback-scoped entity for a fake one.
fn entity<'s>(fake: *mut FakeEntity) -> Entity<'s> {
	// SAFETY: Fake entities are leaked, so they outlive every scope, and
	// their vtables answer what the wrappers call of an entity.
	unsafe { Entity::from_raw(NonNull::new(fake).unwrap().cast()) }
}

unsafe extern "C" fn entity_by_index(
	_: *mut sys::IServerTools,
	index: c_int,
) -> *mut sys::CBaseEntity {
	ENTITIES.with_borrow(|entities| {
		entities
			.iter()
			.find(|(slot, _)| *slot == index)
			.map_or(null_mut(), |&(_, fake)| fake.cast())
	})
}

unsafe extern "C" fn entity_list(_: *mut sys::IServerTools) -> *mut sys::CGlobalEntityList {
	ENTITY_LIST.get()
}

#[test]
fn equip_remove_and_strip_go_through_the_players_list() {
	let world = World::new();
	let scope = ();
	let server = mock_server(&scope);
	let player = world.player;
	let wearables = PlayerWearables::new(server, entity(player)).unwrap();
	// SAFETY: Fake entities are leaked, and the mock game is not running
	// while the test reads or writes their fields, as the game would. The
	// same holds for every access to them below.
	let player_handle = unsafe { (*player).handle };
	let wear = |index| Wearable::new(server, entity(world.wearable(index))).unwrap();
	let fake = |wearable: Wearable<'_>| wearable.entity().as_ptr().cast::<FakeEntity>();

	// SAFETY: The mock game never iterates the list while these run.
	let equip = |wearable| unsafe { wearables.equip(wearable) };
	// SAFETY: As for `equip`.
	let remove = |wearable| unsafe { wearables.remove(wearable) };

	let owned = wear(2);
	// SAFETY: As for the player's handle.
	unsafe { (*fake(owned)).owner = 9 | 1 << 16 };
	assert!(matches!(equip(owned), Err(WearableError::DifferentOwner)));

	let hat = wear(3);
	assert!(matches!(remove(hat), Err(WearableError::NotEquipped)));
	// SAFETY: As for the player's handle.
	assert_eq!(unsafe { (*player).remove_calls }, 0);

	equip(hat).unwrap();
	equip(hat).unwrap();
	// SAFETY: As for the player's handle.
	unsafe {
		assert!((*fake(hat)).validated);
		assert_eq!((*fake(hat)).owner, player_handle);
		assert_eq!((*player).equip_calls, 1);
	}

	// The game's removal takes a null entry behind the wearable first.
	// SAFETY: As for the player's handle.
	unsafe { (*player).wearables.push(NULL) };
	remove(hat).unwrap();
	// SAFETY: As for the player's handle.
	unsafe {
		assert_eq!((*fake(hat)).flags, 1);
		assert_eq!(((*fake(hat)).owner, (*fake(hat)).move_parent), (NULL, NULL));
		assert_eq!((*player).remove_calls, 2);
		assert!((*player).wearables.is_empty());
	}
	assert!(matches!(remove(hat), Err(WearableError::MarkedForDeletion)));
	assert!(matches!(equip(hat), Err(WearableError::MarkedForDeletion)));

	// Behind more null entries, it takes one call per entry.
	let removals = TOOL_REMOVALS.get();
	let behind = wear(4);
	equip(behind).unwrap();
	// SAFETY: As for the player's handle.
	unsafe { (*player).wearables.extend([NULL; 3]) };
	remove(behind).unwrap();
	// SAFETY: As for the player's handle.
	unsafe {
		assert_eq!((*fake(behind)).flags, 1);
		assert_eq!((*player).remove_calls, 6);
		assert!((*player).wearables.is_empty());
	}
	assert_eq!(TOOL_REMOVALS.get(), removals);

	// If the game never removes it, it is deleted through `ServerTools`.
	let stubborn = wear(15);
	equip(stubborn).unwrap();
	IGNORE_REMOVE.set(true);
	remove(stubborn).unwrap();
	IGNORE_REMOVE.set(false);
	assert_eq!(TOOL_REMOVALS.get(), removals + 1);
	// SAFETY: As for the player's handle.
	unsafe {
		assert_eq!((*fake(stubborn)).flags, 1);
		assert_eq!((*player).remove_calls, 6 + MAX_REMOVE_ATTEMPTS);
		(*player).wearables = vec![NULL; MAX_NETWORKED_WEARABLES];
	}

	assert!(matches!(equip(wear(5)), Err(WearableError::Full)));
	// SAFETY: As for the player's handle.
	unsafe { (*player).wearables.clear() };

	// Only what the filter selects is stripped.
	let cosmetic = wear(6);
	let disguise = wear(7);
	let extra = wear(8);
	let boots = wear(9);
	for wearable in [cosmetic, disguise, extra, boots] {
		equip(wearable).unwrap();
	}
	// SAFETY: As for the player's handle.
	unsafe {
		(*fake(disguise)).disguise = true;
		(*fake(extra)).initialized = false;
		(*fake(boots)).definition = 133;
	}
	let mut seen = 0;
	// SAFETY: As for `equip`.
	let removed = unsafe {
		wearables.strip(|wearable| {
			seen += 1;
			!wearable.is_game_managed().unwrap()
				&& wearable.definition().unwrap() != ItemDefinitionIndex::new(133)
		})
	}
	.unwrap();
	assert_eq!((removed, seen), (1, 4));
	// SAFETY: As for the player's handle.
	unsafe {
		assert_eq!((*fake(cosmetic)).flags, 1);
		assert_eq!(
			[
				(*fake(disguise)).flags,
				(*fake(extra)).flags,
				(*fake(boots)).flags
			],
			[0; 3]
		);
		assert_eq!((*player).wearables.len(), 3);
	}

	// Dead players are given nothing, but can still lose wearables.
	DEAD.set(true);
	assert!(matches!(
		equip(wear(10)),
		Err(WearableError::PlayerNotPlaying)
	));
	// SAFETY: As for `equip`.
	assert_eq!(unsafe { wearables.strip(|_| true) }.unwrap(), 3);
	assert!(wearables.list().unwrap().is_empty());
	DEAD.set(false);

	// A view model wearable follows the view model, which follows the player.
	let view_model = world.spawn(11, 1, world.base_map, world.wearable_class, c"tf_viewmodel");
	// SAFETY: As for the player's handle.
	unsafe { (*view_model).move_parent = player_handle };
	// SAFETY: As for the player's handle.
	FOLLOW.set(Some(unsafe { (*view_model).handle }));
	let sleeve = wear(12);
	equip(sleeve).unwrap();

	// Following anything else is not attached, and is undone.
	FOLLOW.set(Some(13 | 1 << 16));
	let detached = wear(14);
	assert!(matches!(equip(detached), Err(WearableError::Rejected)));
	FOLLOW.set(None);
	// SAFETY: As for the player's handle.
	unsafe {
		assert_eq!((*fake(detached)).flags, 1);
		assert_eq!((*player).wearables, [(*fake(sleeve)).handle]);
	}
}

/// Emulates `CBasePlayer::EquipWearable` and `CEconWearable::Equip`.
unsafe extern "C" fn equip_wearable(player: *mut sys::CTFPlayer, item: *mut sys::CEconWearable) {
	// SAFETY: The wrappers pass the fake player and a fake wearable, which
	// are leaked and distinct.
	unsafe {
		let player = player.cast::<FakeEntity>();
		let item = item.cast::<FakeEntity>();

		(*player).equip_calls += 1;
		VALIDATED_AT_EQUIP.set(Some((*item).validated));
		(*player).wearables.insert(0, (*item).handle);

		if REFUSE_EQUIP.get() {
			// `CanEquip` failed: `RemoveFrom` calls `RemoveWearable`.
			remove_wearable(player.cast(), item.cast());
		} else {
			(*item).owner = (*player).handle;
			(*item).move_parent = FOLLOW.get().unwrap_or((*player).handle);
		}
	}
}

/// The fake entity containing an interface at `offset`.
fn fake_of<T>(interface: *const T, offset: usize) -> *mut FakeEntity {
	// SAFETY: Callers pass an interface that is a field of a fake entity at
	// `offset`, so the entity starts that far before it.
	unsafe { interface.byte_sub(offset) }.cast_mut().cast()
}

/// The `m_iEFlags` field of `CBaseEntity`'s datamap.
fn flags_field() -> sys::typedescription_t {
	let mut flags = field(
		c"m_iEFlags",
		sys::_fieldtypes_FIELD_INTEGER,
		MOCK_EFLAGS_OFFSET,
	);

	flags.fieldSizeInBytes = size_of::<c_int>() as c_int;
	flags
}

#[test]
fn give_validates_before_equipping_and_deletes_what_the_game_refuses() {
	let world = World::new();
	let scope = ();
	let server = mock_server(&scope);
	let wearables = PlayerWearables::new(server, entity(world.player)).unwrap();
	// SAFETY: Fake entities are leaked, and the mock game is not running
	// while the test reads or writes their fields, as the game would. The
	// same holds for every access to them below.
	let player_handle = unsafe { (*world.player).handle };
	let created = |fake: *mut FakeEntity| {
		move |origin| {
			assert_eq!(origin, Vector::new(1.0, 2.0, 3.0));
			Ok(NonNull::new(fake).unwrap().cast())
		}
	};

	let hat = world.wearable(2);
	// SAFETY: The fake wearable is live through the call, as if newly
	// created, and the mock game creates, spawns and deletes nothing
	// itself, nor iterates the list while it runs. The same holds for every
	// `give_with` below.
	let given = unsafe { wearables.give_with(created(hat)) }.unwrap();

	assert_eq!(given.entity().as_ptr(), hat.cast());
	assert_eq!(VALIDATED_AT_EQUIP.get(), Some(true));
	// SAFETY: As for the player's handle.
	unsafe {
		assert_eq!(
			((*hat).owner, (*hat).move_parent, (*hat).flags),
			(player_handle, player_handle, 0)
		);
		assert_eq!((*world.player).equip_calls, 1);
	}
	assert!(wearables.list().unwrap().contains(given));

	// Nothing is created for a player who could not wear it.
	DEAD.set(true);
	assert!(matches!(
		// SAFETY: As for the first `give_with`.
		unsafe { wearables.give_with(|_| unreachable!()) },
		Err(WearableError::PlayerNotPlaying)
	));
	DEAD.set(false);
	TEAM.set(1);
	assert!(matches!(
		// SAFETY: As for the first `give_with`.
		unsafe { wearables.give_with(|_| unreachable!()) },
		Err(WearableError::PlayerNotPlaying)
	));
	TEAM.set(FIRST_GAME_TEAM);

	// SAFETY: As for the player's handle.
	unsafe { (*world.player).wearables = vec![(*hat).handle; MAX_NETWORKED_WEARABLES] };
	assert!(matches!(
		// SAFETY: As for the first `give_with`.
		unsafe { wearables.give_with(|_| unreachable!()) },
		Err(WearableError::Full)
	));
	// SAFETY: As for the player's handle.
	unsafe { (*world.player).wearables = vec![(*hat).handle] };

	// A definition that creates a weapon is deleted unequipped.
	let weapon = world.spawn(
		3,
		1,
		data_map(c"CTFWeaponBase", vec![], world.base_map),
		world.wearable_class,
		c"tf_weapon_bottle",
	);
	assert!(matches!(
		// SAFETY: As for the first `give_with`.
		unsafe { wearables.give_with(created(weapon)) },
		Err(WearableError::NotWearable)
	));
	// SAFETY: As for the player's handle.
	assert_eq!(unsafe { (*weapon).flags }, 1);
	// SAFETY: As for the player's handle.
	assert_eq!(unsafe { (*world.player).equip_calls }, 1);

	// `CanEquip` refusing a holiday item makes the game delete it already.
	let removals = TOOL_REMOVALS.get();
	let holiday = world.wearable(4);
	REFUSE_EQUIP.set(true);
	assert!(matches!(
		// SAFETY: As for the first `give_with`.
		unsafe { wearables.give_with(created(holiday)) },
		Err(WearableError::Rejected)
	));
	// SAFETY: As for the player's handle.
	unsafe {
		assert_eq!((*holiday).flags, 1);
		assert_eq!((*world.player).remove_calls, 1);
		assert_eq!((*world.player).wearables, [(*hat).handle]);
	}
	assert_eq!(TOOL_REMOVALS.get(), removals);

	// With a null entry at the end, the game's own removal takes the null
	// entry instead, leaving the refused item listed but unowned.
	let quirk = world.wearable(5);
	// SAFETY: As for the player's handle.
	unsafe { (*world.player).wearables.push(NULL) };
	assert!(matches!(
		// SAFETY: As for the first `give_with`.
		unsafe { wearables.give_with(created(quirk)) },
		Err(WearableError::Rejected)
	));
	REFUSE_EQUIP.set(false);
	// SAFETY: As for the player's handle.
	unsafe {
		assert_eq!((*quirk).flags, 1);
		assert_eq!((*world.player).remove_calls, 3);
		assert_eq!((*world.player).wearables, [(*hat).handle]);
	}
	assert_eq!(TOOL_REMOVALS.get(), removals);

	// SAFETY: As for the player's handle.
	unsafe { (*world.player).flags = 1 };
	assert!(matches!(
		// SAFETY: As for the first `give_with`.
		unsafe { wearables.give_with(|_| unreachable!()) },
		Err(WearableError::MarkedForDeletion)
	));
	assert!(matches!(
		wearables.list(),
		Err(WearableError::MarkedForDeletion)
	));
}

unsafe extern "C" fn handle(entity: *const sys::IServerUnknown) -> *const sys::CBaseHandle {
	// SAFETY: As for `collideable`.
	unsafe { (&raw const (*entity.cast::<FakeEntity>()).handle).cast() }
}

/// An `EHANDLE` field of `CBaseEntity`'s datamap.
fn handle_field(name: &'static CStr, offset: usize) -> sys::typedescription_t {
	let mut handle = field(name, sys::_fieldtypes_FIELD_EHANDLE, offset);

	handle.fieldSize = 1;
	handle.fieldSizeInBytes = size_of::<u32>() as c_int;
	handle
}

/// Stands in for `SendProxy_EHandleToInt`.
unsafe extern "C" fn handle_proxy(
	_: *const sys::SendProp,
	_: *const c_void,
	data: *const c_void,
	out: *mut sys::DVariant,
	_: c_int,
	_: c_int,
) {
	// SAFETY: The wrappers pass a fake entity's handle field as the data,
	// and a variant to write.
	unsafe { (*out).__bindgen_anon_1.m_Int = encode(data.cast::<u32>().read()) };
}

/// An integer property at `offset` into a fake entity.
fn int_prop(
	name: &'static CStr,
	offset: usize,
	flags: PropFlags,
	proxy: sys::SendVarProxyFn,
) -> sys::SendProp {
	prop(
		name,
		sys::SendPropType_DPT_Int,
		c_int::try_from(offset).unwrap(),
		flags,
		proxy,
	)
}

unsafe extern "C" fn is_dead(_: *mut sys::IPlayerInfo) -> bool {
	DEAD.get()
}

#[test]
fn lists_decode_networked_handles_and_wearables_read_their_variables() {
	assert_eq!(offset_of!(FakeEntity, flags), MOCK_EFLAGS_OFFSET);

	let world = World::new();
	let scope = ();
	let server = mock_server(&scope);
	let player = entity(world.player);
	let first = world.wearable(2);
	let second = world.wearable(3);
	let stale = world.wearable(4);

	// Newest first, with a null entry and one whose serial number is stale.
	// SAFETY: Fake entities are leaked, and the mock game is not running
	// while the test reads or writes their fields, or the send tables, as
	// the game would. The same holds for every access to them below.
	unsafe {
		(*world.player).wearables = vec![
			(*second).handle,
			NULL,
			(*stale).handle + (1 << 16),
			(*first).handle,
		];
	}

	let wearables = PlayerWearables::new(server, player).unwrap();
	let list = wearables.list().unwrap();

	assert_eq!((list.networked_len(), list.len()), (4, 2));
	assert!(!list.is_empty() && !list.is_full());
	assert_eq!(
		list.iter()
			.map(|wearable| wearable.entity().as_ptr())
			.collect::<Vec<_>>(),
		[second.cast(), first.cast()]
	);
	assert!(!list.contains(Wearable::new(server, entity(stale)).unwrap()));

	// SAFETY: As above.
	unsafe { (*world.player).wearables = vec![(*first).handle; MAX_NETWORKED_WEARABLES + 1] };
	let list = wearables.list().unwrap();
	assert!(list.is_full());
	assert_eq!(list.networked_len(), MAX_NETWORKED_WEARABLES);

	// SAFETY: As above.
	unsafe { (*world.player).wearables.clear() };
	assert!(wearables.list().unwrap().is_empty());

	// A list the game sends in another shape is refused.
	let refused = || matches!(wearables.list(), Err(WearableError::UnsupportedLayout));

	// SAFETY: As above.
	unsafe { (*world.elements)[4].m_pVarName = c"bad".as_ptr() };
	assert!(refused());
	// SAFETY: As above.
	unsafe { (*world.elements)[4].m_pVarName = ELEMENT_NAMES[3].as_ptr() };

	// SAFETY: As above.
	unsafe { (*world.elements)[4].m_Type = sys::SendPropType_DPT_Float };
	assert!(refused());
	// SAFETY: As above.
	unsafe { (*world.elements)[4].m_Type = sys::SendPropType_DPT_Int };

	// SAFETY: As above.
	unsafe { (*world.elements)[1].m_ProxyFn = None };
	assert!(refused());
	// SAFETY: As above.
	unsafe { (*world.elements)[1].m_ProxyFn = Some(wearable_element) };

	// SAFETY: As above.
	unsafe { (*world.elements_table).m_nProps -= 1 };
	assert!(refused());
	// SAFETY: As above.
	unsafe { (*world.elements_table).m_nProps += 1 };

	// SAFETY: As above.
	unsafe { (*world.wearables_prop).m_Type = sys::SendPropType_DPT_Int };
	assert!(refused());
	// SAFETY: As above.
	unsafe { (*world.wearables_prop).m_Type = sys::SendPropType_DPT_DataTable };

	// A table proxy that moves the elements elsewhere hides where they are.
	// SAFETY: As above.
	unsafe { (*world.wearables_prop).m_DataTableProxyFn = Some(pointer_table) };
	assert!(refused());
	// SAFETY: As above.
	unsafe { (*world.wearables_prop).m_DataTableProxyFn = Some(direct_table) };

	// Classes are checked, down to `CTFWearable`, and so is the game.
	let econ_only = world.spawn(
		5,
		1,
		data_map(c"CEconWearable", vec![], world.base_map),
		world.wearable_class,
		c"wearable_item",
	);

	// An entry that is no TF2 wearable takes room but is not listed, while
	// one whose entity data cannot be read fails the whole list.
	let no_handles = world.spawn(
		7,
		1,
		data_map(
			c"CTFWearable",
			vec![],
			data_map(c"CBaseEntity", vec![flags_field()], null_mut()),
		),
		world.wearable_class,
		c"tf_wearable",
	);
	// SAFETY: As above.
	unsafe { (*world.player).wearables = vec![(*econ_only).handle, (*first).handle] };
	let list = wearables.list().unwrap();
	assert_eq!((list.networked_len(), list.len()), (2, 1));
	// SAFETY: As above.
	unsafe { (*world.player).wearables.push((*no_handles).handle) };
	assert!(refused());
	// SAFETY: As above.
	unsafe { (*world.player).wearables.clear() };
	assert!(matches!(
		PlayerWearables::new(server, entity(first)),
		Err(WearableError::NotTfPlayer)
	));
	assert!(matches!(
		Wearable::new(server, player),
		Err(WearableError::NotWearable)
	));
	assert!(matches!(
		Wearable::new(server, entity(econ_only)),
		Err(WearableError::NotWearable)
	));
	// SAFETY: The factories are the mock server's, whose exports outlive the
	// scope, and the server only differs in the game it reports.
	let sdk_server = unsafe {
		Server::new(
			server.engine_factory(),
			server.game_server_factory(),
			Game::SourceSdk2013,
			&scope,
		)
	};
	assert!(matches!(
		PlayerWearables::new(sdk_server, player),
		Err(WearableError::NotTfPlayer)
	));
	assert!(matches!(
		Wearable::new(sdk_server, entity(first)),
		Err(WearableError::NotWearable)
	));

	let wearable = Wearable::new(server, entity(first)).unwrap();
	assert_eq!(wearable.owner().unwrap(), None);
	assert_eq!(wearable.kind(), WearableKind::Plain);
	assert_eq!(
		wearable.definition().unwrap(),
		ItemDefinitionIndex::new(378)
	);
	assert!(!wearable.is_game_managed().unwrap());
	assert!(!wearable.is_validated().unwrap());

	wearable.set_validated(true).unwrap();
	// SAFETY: As above.
	assert!(unsafe { (*first).validated });
	assert!(wearable.is_validated().unwrap());
	// SAFETY: As above. The fake's edict is leaked too.
	assert_ne!(unsafe { (*(*first).edict)._base.m_fStateFlags } & 1, 0);

	// SAFETY: As above.
	unsafe { (*first).definition = u16::MAX };
	assert_eq!(wearable.definition().unwrap(), None);

	// Weapons' extra wearables never initialize their item.
	// SAFETY: As above.
	unsafe {
		(*first).definition = 378;
		(*first).initialized = false;
	}
	assert_eq!(wearable.definition().unwrap(), None);
	assert!(wearable.is_game_managed().unwrap());

	// SAFETY: As above.
	unsafe {
		(*first).initialized = true;
		(*first).disguise = true;
	}
	assert!(wearable.is_disguise().unwrap());
	assert!(wearable.is_game_managed().unwrap());

	let weapon = world.spawn(
		6,
		3,
		world.base_map,
		world.wearable_class,
		c"tf_weapon_rocketlauncher",
	);
	// SAFETY: As above.
	unsafe {
		(*first).disguise = false;
		(*first).associated = (*weapon).handle;
	}
	assert_eq!(
		wearable.associated_weapon().unwrap().map(Entity::as_ptr),
		Some(weapon.cast())
	);
	assert!(wearable.is_game_managed().unwrap());

	// SAFETY: As above.
	unsafe { (*first).associated = (*weapon).handle + (1 << 16) };
	assert!(wearable.associated_weapon().unwrap().is_none());

	// SAFETY: As above.
	unsafe {
		(*first).associated = NULL;
		(*first).owner = (*world.player).handle;
	}
	assert!(!wearable.is_game_managed().unwrap());
	assert_eq!(wearable.owner().unwrap(), Some(player.handle()));

	// SAFETY: As above.
	unsafe { (*first).flags = 1 };
	assert!(matches!(
		wearable.owner(),
		Err(WearableError::MarkedForDeletion)
	));
}

unsafe extern "C" fn networkable(entity: *mut sys::IServerUnknown) -> *mut sys::IServerNetworkable {
	// SAFETY: As for `collideable`.
	unsafe { &raw mut (*entity.cast::<FakeEntity>()).networkable }
}

#[test]
fn networked_handles_keep_the_index_and_low_serial_bits() {
	let handle = NetworkedHandle::decode(5 | 3 << 11).unwrap();

	assert_eq!(handle.index(), 5);
	assert!(handle.matches(EntityHandle::from_raw(5 | 3 << 16)));
	assert!(handle.matches(EntityHandle::from_raw(5 | (3 + 1024) << 16)));
	assert!(!handle.matches(EntityHandle::from_raw(5 | 4 << 16)));
	assert!(!handle.matches(EntityHandle::from_raw(6 | 3 << 16)));
	assert!(!handle.matches(EntityHandle::INVALID));
	assert_eq!(encode(5 | (3 + 1024) << 16), 5 | 3 << 11);

	assert_eq!(
		NetworkedHandle::decode(2047 | 1022 << 11).map(NetworkedHandle::index),
		Some(2047)
	);

	for raw in [
		0,
		INVALID_NETWORKED_EHANDLE_VALUE.cast_signed(),
		1 << 21,
		-1,
	] {
		assert_eq!(NetworkedHandle::decode(raw), None);
	}
}

unsafe extern "C" fn origin(this: *const sys::ICollideable) -> *const sys::Vector {
	// SAFETY: Only fake entities' collideables have this method, and they
	// are fields of their entities.
	unsafe { &raw const (*fake_of(this, offset_of!(FakeEntity, collideable))).origin }
}

unsafe extern "C" fn player_info(
	_: *mut sys::IPlayerInfoManager,
	_: *mut sys::edict_t,
) -> *mut sys::IPlayerInfo {
	PLAYER_INFO.get()
}

unsafe extern "C" fn remove_entity(_: *mut sys::IServerTools, entity: *mut sys::CBaseEntity) {
	// SAFETY: The wrappers only remove fake entities.
	unsafe { (*entity.cast::<FakeEntity>()).flags |= 1 };
	TOOL_REMOVALS.set(TOOL_REMOVALS.get() + 1);
}

/// Emulates `CBasePlayer::RemoveWearable`, which removes the first null
/// entry it meets from the end instead of the wearable.
unsafe extern "C" fn remove_wearable(player: *mut sys::CTFPlayer, item: *mut sys::CEconWearable) {
	assert!(!item.is_null(), "RemoveWearable unequips null entries");

	// SAFETY: As for `equip_wearable`. The list is borrowed while no other
	// access to the player happens.
	unsafe {
		let player = player.cast::<FakeEntity>();
		let item = item.cast::<FakeEntity>();

		(*player).remove_calls += 1;

		if IGNORE_REMOVE.get() {
			return;
		}

		let wearables = &mut (*player).wearables;

		for index in (0..wearables.len()).rev() {
			let entry = wearables[index];

			if entry == (*item).handle {
				(*item).owner = NULL;
				(*item).move_parent = NULL;
				(*item).flags |= 1;
				wearables.remove(index);
				break;
			}

			if entry == NULL {
				wearables.remove(index);
				break;
			}
		}
	}
}

unsafe extern "C" fn server_class(this: *mut sys::IServerNetworkable) -> *mut sys::ServerClass {
	// SAFETY: As for `class_name`.
	unsafe { (*fake_of(this, offset_of!(FakeEntity, networkable))).class }
}

unsafe extern "C" fn team_index(_: *mut sys::IPlayerInfo) -> c_int {
	TEAM.get()
}

/// Stands in for `SendProxy_UtlVectorElement` over a fake player's list.
unsafe extern "C" fn wearable_element(
	prop: *const sys::SendProp,
	structure: *const c_void,
	_: *const c_void,
	out: *mut sys::DVariant,
	_: c_int,
	_: c_int,
) {
	// SAFETY: The wrappers pass one of the list's element properties, the
	// fake player as the structure, and a variant to write.
	unsafe {
		let index = usize::try_from((*prop).m_ElementStride).unwrap();
		let wearables = &(*structure.cast::<FakeEntity>()).wearables;

		(*out).__bindgen_anon_1.m_Int = wearables.get(index).map_or(0, |&handle| encode(handle));
	}
}
