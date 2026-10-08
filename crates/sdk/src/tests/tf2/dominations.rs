//! Tests of TF2 dominations through fake players' networked domination flags
//! and their `GetNumberofDominations` and `SetNumberofDominations`.

use super::*;
use crate::Module;
use crate::datatables::PropFlags;
use crate::interfaces::{ServerTools, ValveEngine};
use crate::test_support::datatables::{direct_table, int8_proxy, prop, table, table_prop};
use crate::test_support::edicts::{change_accessor, shared_change_info};
use crate::test_support::interfaces::server_game_dll::export_standard_proxies;
use crate::test_support::leak;
use crate::test_support::server::{export, mock_server, null_server};
use sdk_raw::edicts::FL_FULL_EDICT_CHANGED;
use sdk_raw::entities::NUM_SERIAL_NUM_SHIFT_BITS;
use sdk_raw::test_support::edicts::mock_edict;
use sdk_raw::test_support::entities::data_map;
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use sdk_raw::tf2::dominations::{GET_NUMBER_OF_DOMINATIONS_SLOT, SET_NUMBER_OF_DOMINATIONS_SLOT};
use std::cell::RefCell;
use std::ffi::{CString, c_char};
use std::mem::offset_of;
use std::ptr::{NonNull, null_mut};

/// Elements of each domination array, as TF2 networks them.
const LEN: usize = MAX_PLAYERS_ARRAY_SAFE;

/// A player, or another entity, whose datamaps and send tables describe these
/// fields.
#[repr(C)]
struct FakeEntity {
	vtable: *const *const (),
	map: *mut sys::datamap_t,
	networkable: sys::IServerNetworkable,
	handle: u32,
	class: *mut sys::ServerClass,
	edict: *mut sys::edict_t,
	count: c_int,
	/// `m_Shared`, whose local data holds the flags.
	shared: Shared,
}

/// A player's `m_Shared.tfsharedlocaldata`.
#[repr(C)]
struct Shared {
	_before: [u8; 3],
	dominated: [bool; LEN],
	dominating_me: [bool; LEN],
}

thread_local! {
	/// The fake entities by edict index, which `GetBaseEntityByEntIndex`
	/// returns.
	static ENTITIES: RefCell<Vec<(c_int, *mut FakeEntity)>> = const { RefCell::new(Vec::new()) };

	/// Each call of `SetNumberofDominations`: the player and the count.
	static SET: RefCell<Vec<(usize, c_int)>> = const { RefCell::new(Vec::new()) };
}

/// Mock interfaces exported on this thread, and the classes of its fake
/// entities.
struct World {
	player_class: *mut sys::ServerClass,
	player_map: *mut sys::datamap_t,
	prop_class: *mut sys::ServerClass,
	prop_map: *mut sys::datamap_t,
	vtable: *const *const (),
	networkable: *const sys::IServerNetworkable__bindgen_vtable,
}

impl World {
	fn new() -> Self {
		let slots = GET_NUMBER_OF_DOMINATIONS_SLOT
			.max(SET_NUMBER_OF_DOMINATIONS_SLOT)
			.max(sdk_raw::entities::GET_DATA_DESC_MAP_SLOT)
			+ 1;
		let mut vtable = vec![unexpected_call as *const (); slots];

		vtable[sdk_raw::entities::GET_DATA_DESC_MAP_SLOT] = datamap as *const ();
		vtable[sdk_raw::vtable_slot!(
			sys::IServerUnknown__bindgen_vtable,
			IServerUnknown_GetNetworkable
		)] = networkable as *const ();
		vtable[sdk_raw::vtable_slot!(
			sys::IServerUnknown__bindgen_vtable,
			IServerUnknown_GetRefEHandle
		)] = handle as *const ();
		vtable[GET_NUMBER_OF_DOMINATIONS_SLOT] = get_count as *const ();
		vtable[SET_NUMBER_OF_DOMINATIONS_SLOT] = set_count as *const ();

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

		let base_map = data_map(c"CBaseEntity", vec![], null_mut());
		let player_map = data_map(
			c"CTFPlayer",
			vec![],
			data_map(c"CBasePlayer", vec![], base_map),
		);
		let prop_map = data_map(c"CDynamicProp", vec![], base_map);

		let flags = offset_of!(Shared, dominated);
		let flagged_by = offset_of!(Shared, dominating_me);
		let local_props = leak([
			bool_array(c"m_bPlayerDominated", flags),
			bool_array(c"m_bPlayerDominatingMe", flagged_by),
		]);
		// SAFETY: The properties are leaked, and only their table, which views
		// them, reaches them from here on. The same holds for the tables below.
		let local_table = leak(table(c"DT_TFPlayerSharedLocal", unsafe {
			&mut *local_props
		}));
		let shared_props = leak([table_prop(
			c"tfsharedlocaldata",
			0,
			local_table,
			Some(direct_table),
		)]);
		// SAFETY: As for the local table.
		let shared_table = leak(table(c"DT_TFPlayerShared", unsafe { &mut *shared_props }));
		let player_props = leak([table_prop(
			c"m_Shared",
			c_int::try_from(offset_of!(FakeEntity, shared)).unwrap(),
			shared_table,
			Some(direct_table),
		)]);
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

		export_interfaces();
		ENTITIES.take();
		SET.take();

		Self {
			player_class: class(c"CTFPlayer", player_table, 1),
			player_map,
			prop_class: class(c"CDynamicProp", prop_table, 2),
			prop_map,
			vtable: vtable.leak().as_ptr(),
			networkable,
		}
	}

	/// Clears the engine's record of `fake`'s changes.
	fn forget_changes(fake: *mut FakeEntity) {
		// SAFETY: Fake entities and their edicts are leaked, and the mock game is
		// not running while the test reads or writes them.
		unsafe { (*(*fake).edict)._base.m_fStateFlags = 0 };
	}

	/// A player in edict `index`, in the entities `GetBaseEntityByEntIndex`
	/// finds.
	fn player(&self, index: c_int) -> *mut FakeEntity {
		let fake = self.spawn(index, self.player_class, self.player_map);

		ENTITIES.with_borrow_mut(|entities| entities.push((index, fake)));
		fake
	}

	/// An entity in edict `index`, whom no index lookup finds.
	fn spawn(
		&self,
		index: c_int,
		class: *mut sys::ServerClass,
		map: *mut sys::datamap_t,
	) -> *mut FakeEntity {
		leak(FakeEntity {
			vtable: self.vtable,
			map,
			networkable: sys::IServerNetworkable {
				vtable_: self.networkable,
			},
			handle: index.cast_unsigned() | 1 << NUM_SERIAL_NUM_SHIFT_BITS,
			class,
			edict: leak(mock_edict(index, false)),
			count: 0,
			shared: Shared {
				_before: [0; 3],
				dominated: [false; LEN],
				dominating_me: [false; LEN],
			},
		})
	}
}

/// A `SendPropArray3`-shaped array of [`LEN`] `SendPropBool`s at `offset`.
fn bool_array(name: &'static CStr, offset: usize) -> sys::SendProp {
	let elements = (0..LEN)
		.map(|index| {
			let mut element = prop(
				element_name(index),
				sys::SendPropType_DPT_Int,
				c_int::try_from(index).unwrap(),
				PropFlags::UNSIGNED,
				Some(int8_proxy),
			);

			element.m_nBits = 1;
			element
		})
		.collect::<Vec<_>>();
	let nested = leak(table(name, Box::leak(elements.into_boxed_slice())));

	table_prop(
		name,
		c_int::try_from(offset).unwrap(),
		nested,
		Some(direct_table),
	)
}

/// Whether the engine was told that `fake`'s networked variables changed.
fn changed(fake: *mut FakeEntity) -> bool {
	// SAFETY: As for `World::forget_changes`.
	unsafe { (*(*fake).edict)._base.m_fStateFlags & FL_FULL_EDICT_CHANGED != 0 }
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

/// The dominations of a fake player.
fn dominations<'s>(server: Server<'s>, fake: *mut FakeEntity) -> PlayerDominations<'s> {
	PlayerDominations::new(server, entity(fake)).unwrap()
}

unsafe extern "C" fn edict(this: *const sys::IServerNetworkable) -> *mut sys::edict_t {
	// SAFETY: As for `class_name`.
	unsafe { (*fake_of(this)).edict }
}

/// A leaked name for element `index` of an array, as `SendPropArray3` names
/// them.
fn element_name(index: usize) -> &'static CStr {
	Box::leak(
		CString::new(format!("{index:03}"))
			.unwrap()
			.into_boxed_c_str(),
	)
}

/// A callback-scoped entity for a fake one.
fn entity<'s>(fake: *mut FakeEntity) -> Entity<'s> {
	// SAFETY: Fake entities are leaked, so they outlive every scope, and their
	// vtables answer what the wrappers call of an entity.
	unsafe { Entity::from_raw(NonNull::new(fake).unwrap().cast()) }
}

unsafe extern "C" fn entity_by_index(
	_: *mut sys::IServerTools,
	index: c_int,
) -> *mut sys::CBaseEntity {
	ENTITIES.with_borrow(|entities| {
		entities
			.iter()
			.find(|(found, _)| *found == index)
			.map_or(null_mut(), |(_, fake)| fake.cast())
	})
}

/// Exports the engine and game interfaces the module uses.
fn export_interfaces() {
	export_standard_proxies();

	// SAFETY: The vtable holds only function pointers, `unexpected_call`
	// aborts whichever slot reaches it, and the patch only writes slots of the
	// vtable being built.
	let tools = Box::leak(unsafe {
		mock_vtable::<sys::IServerTools__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).IServerTools_GetBaseEntityByEntIndex).write(entity_by_index);
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

/// `CTFPlayer::GetNumberofDominations`, which returns the fake's count.
unsafe extern "C" fn get_count(player: *mut sys::CTFPlayer) -> c_int {
	// SAFETY: Only fake players have this method in their vtables.
	unsafe { (*player.cast::<FakeEntity>()).count }
}

/// `IHandleEntity::GetRefEHandle`, which returns the fake entity's handle.
unsafe extern "C" fn handle(this: *const sys::IServerUnknown) -> *const sys::CBaseHandle {
	// SAFETY: As for `datamap`.
	unsafe { (&raw const (*this.cast::<FakeEntity>()).handle).cast() }
}

unsafe extern "C" fn networkable(entity: *mut sys::IServerUnknown) -> *mut sys::IServerNetworkable {
	// SAFETY: As for `datamap`.
	unsafe { &raw mut (*entity.cast::<FakeEntity>()).networkable }
}

unsafe extern "C" fn server_class(this: *mut sys::IServerNetworkable) -> *mut sys::ServerClass {
	// SAFETY: As for `class_name`.
	unsafe { (*fake_of(this)).class }
}

/// `CTFPlayer::SetNumberofDominations`, which notes the call and clamps the
/// count as the game does.
unsafe extern "C" fn set_count(player: *mut sys::CTFPlayer, count: c_int) {
	SET.with_borrow_mut(|set| set.push((player.addr(), count)));

	// SAFETY: As for `get_count`.
	unsafe { (*player.cast::<FakeEntity>()).count = count.clamp(0, 100) };
}

/// Marks `dominator` as dominating `victim` in both halves of the pair, and
/// counts it, as `CalcDominationAndRevenge` does.
fn dominate(dominator: (c_int, *mut FakeEntity), victim: (c_int, *mut FakeEntity)) {
	// SAFETY: As for `World::forget_changes`.
	unsafe {
		(*dominator.1).shared.dominated[victim.0 as usize] = true;
		(*victim.1).shared.dominating_me[dominator.0 as usize] = true;
		(*dominator.1).count += 1;
	}
}

#[test]
fn only_tf2_players_with_player_indices_are_wrapped() {
	let world = World::new();
	let scope = ();
	let server = mock_server(&scope);
	let player = world.player(1);

	assert!(matches!(
		PlayerDominations::new(null_server(Game::SourceSdk2013, &scope), entity(player)),
		Err(DominationError::NotTfPlayer)
	));
	assert!(matches!(
		PlayerDominations::new(
			server,
			entity(world.spawn(3, world.prop_class, world.prop_map))
		),
		Err(DominationError::NotTfPlayer)
	));

	for index in [0, MAX_PLAYERS_ARRAY_SAFE as c_int] {
		assert!(matches!(
			PlayerDominations::new(server, entity(world.player(index))),
			Err(DominationError::NotTfPlayer)
		));
	}

	assert_eq!(dominations(server, player).player().as_ptr(), player.cast());
}

#[test]
fn dominations_are_read_from_each_half_and_the_count() {
	let world = World::new();
	let scope = ();
	let server = mock_server(&scope);
	let (scout, soldier) = ((1, world.player(1)), (101, world.player(101)));

	dominate(scout, soldier);

	let scouts = dominations(server, scout.1);
	let soldiers = dominations(server, soldier.1);

	assert!(scouts.dominates(soldiers).unwrap());
	assert!(!scouts.is_dominated_by(soldiers).unwrap());
	assert!(soldiers.is_dominated_by(scouts).unwrap());
	assert!(!soldiers.dominates(scouts).unwrap());
	assert_eq!((scouts.count(), soldiers.count()), (1, 0));
	assert!(SET.take().is_empty());
}

#[test]
fn ending_a_domination_clears_both_halves_and_counts_it_once() {
	let world = World::new();
	let scope = ();
	let server = mock_server(&scope);
	let (scout, soldier, pyro) = (
		(1, world.player(1)),
		(2, world.player(2)),
		(3, world.player(3)),
	);

	dominate(scout, soldier);
	dominate(scout, pyro);

	let scouts = dominations(server, scout.1);
	let soldiers = dominations(server, soldier.1);

	assert!(scouts.end(soldiers).unwrap());
	assert!(!scouts.dominates(soldiers).unwrap());
	assert!(!soldiers.is_dominated_by(scouts).unwrap());
	assert!(scouts.dominates(dominations(server, pyro.1)).unwrap());
	assert_eq!(scouts.count(), 1);
	assert_eq!(SET.take(), [(scout.1.addr(), 1)]);
	assert!(changed(scout.1) && changed(soldier.1) && !changed(pyro.1));

	// Nothing is left to end, so nothing is written.
	World::forget_changes(scout.1);
	World::forget_changes(soldier.1);
	assert!(!scouts.end(soldiers).unwrap());
	assert!(!soldiers.end(scouts).unwrap());
	assert_eq!(scouts.count(), 1);
	assert!(SET.take().is_empty());
	assert!(!changed(scout.1) && !changed(soldier.1));

	// A half left alone is cleared, without counting a domination that was not.
	// SAFETY: As for `World::forget_changes`.
	unsafe { (*soldier.1).shared.dominating_me[scout.0 as usize] = true };
	assert!(!scouts.end(soldiers).unwrap());
	assert!(!soldiers.is_dominated_by(scouts).unwrap());
	assert!(SET.take().is_empty());
}

#[test]
fn ending_all_clears_every_pair_of_the_player_and_their_counts() {
	let world = World::new();
	let scope = ();
	let server = mock_server(&scope);
	let scout = (1, world.player(1));
	let soldier = (2, world.player(2));
	let pyro = (3, world.player(3));
	let demoman = (4, world.player(4));
	// A player who left, whose half the scout still holds.
	let gone = 9;

	dominate(scout, soldier);
	dominate(scout, pyro);
	dominate(demoman, scout);
	dominate(demoman, pyro);

	// SAFETY: As for `World::forget_changes`.
	unsafe {
		(*scout.1).shared.dominated[gone] = true;
		(*scout.1).count += 1;
	}

	assert_eq!(dominations(server, scout.1).end_all().unwrap(), 4);

	// SAFETY: As for `World::forget_changes`.
	unsafe {
		assert!(!(*scout.1).shared.dominated.contains(&true));
		assert!(!(*scout.1).shared.dominating_me.contains(&true));
		assert!(!(*soldier.1).shared.dominating_me[scout.0 as usize]);
		assert!(!(*pyro.1).shared.dominating_me[scout.0 as usize]);
		assert!(!(*demoman.1).shared.dominated[scout.0 as usize]);

		// The demoman still dominates the pyro.
		assert!((*demoman.1).shared.dominated[pyro.0 as usize]);
		assert!((*pyro.1).shared.dominating_me[demoman.0 as usize]);
		assert_eq!(((*scout.1).count, (*demoman.1).count), (0, 1));
	}

	assert_eq!(SET.take(), [(demoman.1.addr(), 1), (scout.1.addr(), 0)]);

	// With nothing left, the count is not written again.
	assert_eq!(dominations(server, scout.1).end_all().unwrap(), 0);
	assert!(SET.take().is_empty());
}
