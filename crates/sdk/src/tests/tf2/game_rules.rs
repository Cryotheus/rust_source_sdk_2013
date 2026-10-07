//! Tests of `crate::tf2::game_rules`: finding fake game rules through a
//! `tf_gamerules` entity whose send tables nest them as TF2's do.

use super::*;
use crate::Module;
use crate::datatables::PropFlags;
use crate::interfaces::{ServerGameDll, ServerTools};
use crate::test_support::datatables::{
	direct_table, int8_proxy, int32_proxy, prop, proxies, table, table_prop,
};
use crate::test_support::entities::MockEntity;
use crate::test_support::entities::set_networking;
use crate::test_support::leak;
use crate::test_support::server::{export, mock_server, null_server};
use sdk_raw::entities::EFL_KILLME;
use sdk_raw::test_support::edicts::mock_edict;
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use std::cell::{Cell, RefCell};
use std::ffi::c_char;
use std::ptr::null_mut;

/// What a table proxy was called with: the structure, the nested data, the
/// object ID, and whether every recipient was set.
type ProxyCall = (*const c_void, *const c_void, c_int, bool);

/// Where the fake game rules keep their round state, waiting flag, setup
/// flag, game type, and the TF2 rules' variable of the round state's name.
const ROUND_STATE: usize = 8;
const WAITING: usize = 12;
const SETUP: usize = 13;
const GAME_TYPE: usize = 16;
const SHADOWED: usize = 20;

thread_local! {
	/// The entities [`find_by_class_name`] finds as `tf_gamerules`, in order.
	static ENTITIES: RefCell<Vec<*mut sys::CBaseEntity>> = const { RefCell::new(Vec::new()) };

	/// The calls of [`rules_proxy`] and [`round_rules_proxy`], in order.
	static PROXY_CALLS: RefCell<Vec<ProxyCall>> = const { RefCell::new(Vec::new()) };

	/// What [`rules_proxy`] and [`round_rules_proxy`] give: the game rules as
	/// `CTFGameRules` and as `CTeamplayRoundBasedRules`, which are separate
	/// here to tell which table is read relative to which.
	static RULES: Cell<(*mut c_void, *mut c_void)> = const { Cell::new((null_mut(), null_mut())) };

	/// The head of the server class list [`server_classes`] returns.
	static SERVER_CLASSES: Cell<*mut sys::ServerClass> = const { Cell::new(null_mut()) };
}

/// How a fake proxy entity's table nests the game rules' tables, which
/// [`Tables::TF2`] does as TF2's does.
#[derive(Clone, Copy)]
struct Tables {
	/// The proxy of the property nesting the round-based rules' table.
	round_proxy: sys::SendTableProxyFn,

	/// The offset and proxy of the property nesting the base class's table.
	base: (c_int, sys::SendTableProxyFn),

	/// The name of the TF2 rules' own table.
	rules_table: &'static CStr,
}

impl Tables {
	const TF2: Self = Self {
		round_proxy: Some(round_rules_proxy),
		base: (0, Some(direct_table)),
		rules_table: c"DT_TFGameRules",
	};
}

/// A fake game: the game rules' tables, the proxy entity's class and edict,
/// and the interfaces they are found through.
struct World {
	class: *mut sys::ServerClass,
	edict: *mut sys::edict_t,
	entity: MockEntity,
	round_rules: *mut [u8; 32],
	rules: *mut [u8; 32],
}

impl World {
	/// A game whose proxy entity nests the round-based rules' table through
	/// `round_proxy`, and otherwise as TF2's does.
	fn new(round_proxy: sys::SendTableProxyFn) -> Self {
		Self::with(Tables {
			round_proxy,
			..Tables::TF2
		})
	}

	/// A game whose proxy entity nests the game rules' tables as `tables` says.
	fn with(tables: Tables) -> Self {
		let int = |name, offset: usize| {
			prop(
				name,
				sys::SendPropType_DPT_Int,
				offset as c_int,
				PropFlags::default(),
				Some(int32_proxy),
			)
		};

		let flag = |name, offset: usize| {
			prop(
				name,
				sys::SendPropType_DPT_Int,
				offset as c_int,
				PropFlags::UNSIGNED,
				Some(int8_proxy),
			)
		};

		let round_rules_table = leak(table(
			c"DT_TeamplayRoundBasedRules",
			Box::leak(Box::new([
				int(c"m_iRoundState", ROUND_STATE),
				flag(c"m_bInWaitingForPlayers", WAITING),
				flag(c"m_bInSetup", SETUP),
			])),
		));

		// The TF2 rules shadow a variable of the round-based rules.
		let rules_table = leak(table(
			tables.rules_table,
			Box::leak(Box::new([
				int(c"m_nGameType", GAME_TYPE),
				int(c"m_iRoundState", SHADOWED),
			])),
		));

		let game_rules_proxy = leak(table(c"DT_GameRulesProxy", &mut []));
		let round_rules_proxy_table = leak(table(
			c"DT_TeamplayRoundBasedRulesProxy",
			Box::leak(Box::new([
				table_prop(c"baseclass", 0, game_rules_proxy, Some(direct_table)),
				table_prop(
					c"teamplayroundbased_gamerules_data",
					0,
					round_rules_table,
					tables.round_proxy,
				),
			])),
		));

		let proxy_table = leak(table(
			c"DT_TFGameRulesProxy",
			Box::leak(Box::new([
				table_prop(
					c"baseclass",
					tables.base.0,
					round_rules_proxy_table,
					tables.base.1,
				),
				table_prop(c"tf_gamerules_data", 0, rules_table, Some(rules_proxy)),
			])),
		));

		let class = server_class(c"CTFGameRulesProxy", proxy_table, null_mut());
		let player_table = leak(table(c"DT_TFPlayer", &mut []));

		SERVER_CLASSES.set(server_class(c"CTFPlayer", player_table, class));
		export_interfaces();

		let rules = leak([0; 32]);
		let round_rules = leak([0; 32]);
		let mut entity = MockEntity::new(1);
		let edict = leak(mock_edict(7, false));

		set_networking(class, edict);
		ENTITIES.set(vec![entity.as_ptr()]);
		RULES.set((rules.cast(), round_rules.cast()));
		PROXY_CALLS.take();

		Self {
			class,
			edict,
			entity,
			round_rules,
			rules,
		}
	}

	/// Writes a byte of the fake round-based rules.
	fn put_round_byte(&self, offset: usize, value: u8) {
		// SAFETY: The rules are leaked, and the offset is within them.
		unsafe { (*self.round_rules)[offset] = value };
	}

	/// Writes an integer of the fake round-based rules, or of the TF2 rules.
	fn put_int(&self, round_rules: bool, offset: usize, value: c_int) {
		let object = if round_rules {
			self.round_rules
		} else {
			self.rules
		};

		// SAFETY: As for `put_round_byte`.
		unsafe {
			object
				.cast::<u8>()
				.add(offset)
				.cast::<c_int>()
				.write_unaligned(value)
		};
	}

	/// Reads an integer of the fake round-based rules, or of the TF2 rules.
	fn int(&self, round_rules: bool, offset: usize) -> c_int {
		let object = if round_rules {
			self.round_rules
		} else {
			self.rules
		};

		// SAFETY: As for `put_round_byte`.
		unsafe {
			object
				.cast::<u8>()
				.add(offset)
				.cast::<c_int>()
				.read_unaligned()
		}
	}

	/// The bytes of the fake round-based rules.
	fn round_bytes(&self) -> [u8; 32] {
		// SAFETY: The rules are leaked.
		unsafe { *self.round_rules }
	}
}

/// Exports a game DLL whose server classes are [`SERVER_CLASSES`] with
/// standard proxies, and tools that find [`ENTITIES`].
fn export_interfaces() {
	// SAFETY: The vtables hold only function pointers, `unexpected_call` aborts
	// whichever slot reaches it, and the patches only write slots of the
	// vtables being built.
	let dll_vtable = unsafe {
		mock_vtable::<sys::IServerGameDLL__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).IServerGameDLL_GetAllServerClasses).write(server_classes);
			(&raw mut (*vtable).IServerGameDLL_GetStandardSendProxies).write(standard_proxies);
		})
	};
	// SAFETY: As for the game DLL's vtable.
	let tools_vtable = unsafe {
		mock_vtable::<sys::IServerTools__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).IServerTools_FindEntityByClassname).write(find_by_class_name);
		})
	};

	export(
		Module::GameServer,
		ServerGameDll::VERSION,
		leak(sys::IServerGameDLL {
			vtable_: Box::leak(dll_vtable),
		}),
	);
	export(
		Module::GameServer,
		ServerTools::VERSION,
		leak(sys::IServerTools {
			vtable_: Box::leak(tools_vtable),
		}),
	);
}

/// `IServerTools::FindEntityByClassname`, which finds [`ENTITIES`] in order as
/// `tf_gamerules`.
unsafe extern "C" fn find_by_class_name(
	_: *mut sys::IServerTools,
	after: *mut sys::CBaseEntity,
	name: *const c_char,
) -> *mut sys::CBaseEntity {
	// SAFETY: The tools are passed a NUL-terminated class name.
	let name = unsafe { CStr::from_ptr(name) };

	if name != c"tf_gamerules" {
		return null_mut();
	}

	ENTITIES.with_borrow(|entities| {
		let next = match after.is_null() {
			true => Some(0),
			false => entities
				.iter()
				.position(|&entity| entity == after)
				.map(|index| index + 1),
		};

		next.and_then(|index| entities.get(index).copied())
			.unwrap_or(null_mut())
	})
}

/// Records a table proxy's call, and gives `object`.
fn record(
	struct_base: *const c_void,
	data: *const c_void,
	recipients: *mut sys::CSendProxyRecipients,
	object_id: c_int,
	object: *mut c_void,
) -> *mut c_void {
	// SAFETY: The recipients are 255 bits in eight 32-bit words.
	let bits = unsafe { recipients.cast::<[u32; 8]>().read() };

	PROXY_CALLS.with_borrow_mut(|calls| {
		calls.push((struct_base, data, object_id, bits == [u32::MAX; 8]));
	});
	object
}

/// `SendProxy_TeamplayRoundBasedRules`, which gives the round-based rules.
unsafe extern "C" fn round_rules_proxy(
	_: *const sys::SendProp,
	struct_base: *const c_void,
	data: *const c_void,
	recipients: *mut sys::CSendProxyRecipients,
	object_id: c_int,
) -> *mut c_void {
	record(struct_base, data, recipients, object_id, RULES.get().1)
}

/// `SendProxy_TFGameRules`, which gives the TF2 rules.
unsafe extern "C" fn rules_proxy(
	_: *const sys::SendProp,
	struct_base: *const c_void,
	data: *const c_void,
	recipients: *mut sys::CSendProxyRecipients,
	object_id: c_int,
) -> *mut c_void {
	record(struct_base, data, recipients, object_id, RULES.get().0)
}

/// A table proxy that gives nothing, as TF2's do without game rules.
unsafe extern "C" fn no_rules_proxy(
	_: *const sys::SendProp,
	struct_base: *const c_void,
	data: *const c_void,
	recipients: *mut sys::CSendProxyRecipients,
	object_id: c_int,
) -> *mut c_void {
	record(struct_base, data, recipients, object_id, null_mut())
}

/// A leaked server class named `name` describing `table`, followed by `next`.
fn server_class(
	name: &'static CStr,
	table: *mut sys::SendTable,
	next: *mut sys::ServerClass,
) -> *mut sys::ServerClass {
	leak(sys::ServerClass {
		m_pNetworkName: name.as_ptr(),
		m_pTable: table,
		m_pNext: next,
		m_ClassID: 1,
		m_InstanceBaselineIndex: 0,
	})
}

/// `IServerGameDLL::GetAllServerClasses`, which returns [`SERVER_CLASSES`].
unsafe extern "C" fn server_classes(_: *mut sys::IServerGameDLL) -> *mut sys::ServerClass {
	SERVER_CLASSES.get()
}

/// `IServerGameDLL::GetStandardSendProxies`, which returns leaked standard
/// proxies.
unsafe extern "C" fn standard_proxies(
	_: *mut sys::IServerGameDLL,
) -> *mut sys::CStandardSendProxies {
	leak(proxies(null_mut()))
}

#[test]
fn game_rules_are_read_from_the_objects_the_proxies_give() {
	let mut world = World::new(Some(round_rules_proxy));
	let scope = ();
	let server = mock_server(&scope);

	world.put_int(true, ROUND_STATE, GR_STATE_PREGAME);
	world.put_round_byte(WAITING, 1);
	world.put_int(false, GAME_TYPE, 4);
	world.put_int(false, SHADOWED, GR_STATE_BONUS);

	let rules = GameRules::get(server).unwrap();
	let entity = world.entity.as_ptr().cast::<c_void>().cast_const();

	// Each proxy is given the entity, its edict index, and every recipient.
	assert_eq!(
		PROXY_CALLS.take(),
		[(entity, entity, 7, true), (entity, entity, 7, true)]
	);
	assert_eq!(rules.as_ptr(), world.rules.cast());

	// A name in both tables is the round-based rules' variable.
	assert_eq!(rules.round_state(), Ok(RoundState::Pregame));
	assert_eq!(rules.is_waiting_for_players(), Ok(true));
	assert_eq!(rules.is_in_setup(), Ok(false));

	// The TF2 rules' own variables are read from the object their proxy gave.
	assert_eq!(rules.read::<i32>(c"m_nGameType"), Ok(4));

	world.put_int(true, ROUND_STATE, GR_STATE_STARTGAME);
	world.put_round_byte(SETUP, 1);
	assert_eq!(rules.round_state(), Ok(RoundState::StartGame));
	assert_eq!(rules.is_in_setup(), Ok(true));

	world.put_int(true, ROUND_STATE, GR_STATE_BETWEEN_RNDS + 1);
	assert_eq!(
		rules.round_state(),
		Err(GameRulesError::UnknownRoundState(GR_STATE_BETWEEN_RNDS + 1))
	);

	assert!(matches!(
		rules.read::<i32>(c"m_bMissing"),
		Err(GameRulesError::NetProp(NetPropError::NotFound { table, .. })) if table == "DT_TFGameRules"
	));
	assert!(matches!(
		rules.read::<i16>(c"m_iRoundState"),
		Err(GameRulesError::NetProp(NetPropError::TypeMismatch { .. }))
	));
}

#[test]
fn game_rules_are_written_where_they_are_read() {
	let world = World::new(Some(round_rules_proxy));
	let scope = ();
	let rules = GameRules::get(mock_server(&scope)).unwrap();

	// SAFETY: The fake game rules accept any value of their variables.
	unsafe { rules.write(c"m_bInWaitingForPlayers", true) }.unwrap();

	// The flag's byte alone changed, to the value the game stores for true.
	let mut expected = [0; 32];
	expected[WAITING] = 1;
	assert_eq!(world.round_bytes(), expected);
	assert_eq!(rules.is_waiting_for_players(), Ok(true));

	// A name in both tables is the round-based rules' variable, and the TF2
	// rules' own variables are written in the object their proxy gave.
	// SAFETY: As above.
	unsafe {
		rules.write(c"m_iRoundState", GR_STATE_BONUS).unwrap();
		rules.write(c"m_nGameType", 4).unwrap();
		rules.write(c"m_bInWaitingForPlayers", false).unwrap();
	}

	assert_eq!(world.int(true, ROUND_STATE), GR_STATE_BONUS);
	assert_eq!(world.int(false, SHADOWED), 0);
	assert_eq!(world.int(false, GAME_TYPE), 4);
	assert_eq!(rules.is_waiting_for_players(), Ok(false));

	// Variables stored as another type, or in neither table, are not written.
	assert!(matches!(
		// SAFETY: As above.
		unsafe { rules.write::<i16>(c"m_iRoundState", 1) },
		Err(GameRulesError::NetProp(NetPropError::TypeMismatch { .. }))
	));
	assert!(matches!(
		// SAFETY: As above.
		unsafe { rules.write(c"m_bMissing", true) },
		Err(GameRulesError::NetProp(NetPropError::NotFound { .. }))
	));
	assert_eq!(world.int(true, ROUND_STATE), GR_STATE_BONUS);
}

#[test]
fn game_rules_need_tf2_its_tables_and_a_live_proxy_entity() {
	let scope = ();

	assert_eq!(
		GameRules::get(null_server(Game::SourceSdk2013, &scope)),
		Err(GameRulesError::WrongGame)
	);
	assert!(matches!(
		GameRules::get(null_server(Game::TeamFortress2, &scope)),
		Err(GameRulesError::Interface(_))
	));

	// A proxy that keeps the entity's address would not reach the game rules,
	// a base class's table must be the entity's own, and the nested tables
	// must be the game rules'.
	let tf2 = Tables::TF2;

	for tables in [
		Tables {
			round_proxy: Some(direct_table),
			..tf2
		},
		Tables {
			base: (8, Some(direct_table)),
			..tf2
		},
		Tables {
			base: (0, Some(round_rules_proxy)),
			..tf2
		},
		Tables {
			rules_table: c"DT_GameRules",
			..tf2
		},
	] {
		let _world = World::with(tables);
		assert_eq!(
			GameRules::get(mock_server(&scope)),
			Err(GameRulesError::UnexpectedTables)
		);
	}

	// Without game rules, the proxies give nothing.
	let _world = World::new(Some(no_rules_proxy));
	assert_eq!(
		GameRules::get(mock_server(&scope)),
		Err(GameRulesError::NoGameRules)
	);

	let world = World::new(Some(round_rules_proxy));
	RULES.set((null_mut(), world.round_rules.cast()));
	assert_eq!(
		GameRules::get(mock_server(&scope)),
		Err(GameRulesError::NoGameRules)
	);

	// A proxy entity being removed, or of another class, is not the game's.
	let mut world = World::new(Some(round_rules_proxy));
	world.entity.set_eflags(EFL_KILLME);
	assert_eq!(
		GameRules::get(mock_server(&scope)),
		Err(GameRulesError::NoProxyEntity)
	);

	world.entity.set_eflags(0);
	set_networking(null_mut(), world.edict);
	assert_eq!(
		GameRules::get(mock_server(&scope)),
		Err(GameRulesError::NoProxyEntity)
	);

	set_networking(world.class, world.edict);
	assert!(GameRules::get(mock_server(&scope)).is_ok());

	ENTITIES.set(Vec::new());
	assert_eq!(
		GameRules::get(mock_server(&scope)),
		Err(GameRulesError::NoProxyEntity)
	);
}

#[test]
fn the_proxy_entity_a_maps_own_replaces_is_skipped() {
	let scope = ();
	let mut world = World::new(Some(round_rules_proxy));
	let mut replaced = MockEntity::new(2);

	// Building another mock entity resets what they report.
	replaced.set_eflags(EFL_KILLME);
	set_networking(world.class, world.edict);
	ENTITIES.set(vec![replaced.as_ptr(), world.entity.as_ptr()]);

	let rules = GameRules::get(mock_server(&scope)).unwrap();
	let entity = world.entity.as_ptr().cast::<c_void>().cast_const();

	assert_eq!(rules.as_ptr(), world.rules.cast());
	assert_eq!(
		PROXY_CALLS.take(),
		[(entity, entity, 7, true), (entity, entity, 7, true)]
	);
}

#[test]
fn round_states_round_trip_through_their_raw_values() {
	for (state, raw) in RoundState::ALL.into_iter().zip(GR_STATE_INIT..) {
		assert_eq!(state.to_raw(), raw);
		assert_eq!(RoundState::from_raw(raw), Some(state));
	}

	assert_eq!(
		RoundState::ALL.last().unwrap().to_raw(),
		GR_STATE_BETWEEN_RNDS
	);
	assert_eq!(RoundState::from_raw(GR_STATE_INIT - 1), None);
	assert_eq!(RoundState::from_raw(GR_STATE_BETWEEN_RNDS + 1), None);
}

#[test]
fn the_game_rules_vtable_is_only_searched_for_on_tf2() {
	let scope = ();

	assert!(matches!(
		game_rules_vtable(null_server(Game::SourceSdk2013, &scope)),
		Err(GameRulesVtableError::WrongGame)
	));

	// The tests' own executable has no `CTFGameRules`.
	assert!(matches!(
		game_rules_vtable(mock_server(&scope)),
		Err(GameRulesVtableError::NotFound)
	));
}
