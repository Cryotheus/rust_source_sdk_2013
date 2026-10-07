//! Fake TF2 game rules, found through a `tf_gamerules` entity whose send
//! tables nest the game rules' tables as TF2's do, and the game DLL and tools
//! they are found through.

use crate::Module;
use crate::datatables::PropFlags;
use crate::interfaces::{ServerGameDll, ServerTools};

use crate::test_support::datatables::{
	direct_table, int8_proxy, int32_proxy, prop, proxies, table, table_prop,
};

use crate::test_support::entities::{MockEntity, set_networking};
use crate::test_support::leak;
use crate::test_support::server::export;
use sdk_raw::test_support::edicts::mock_edict;
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use std::cell::{Cell, RefCell};
use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr::null_mut;

/// What a table proxy was called with: the structure, the nested data, the
/// object ID, and whether every recipient was set.
pub type ProxyCall = (*const c_void, *const c_void, c_int, bool);

/// The flags the fake TF2 rules keep from [`FLAGS`] on, in order.
pub const FLAG_NAMES: [&CStr; 11] = [
	c"m_bPlayingKoth",
	c"m_bPlayingMedieval",
	c"m_bPlayingHybrid_CTF_CP",
	c"m_bPlayingSpecialDeliveryMode",
	c"m_bPlayingRobotDestructionMode",
	c"m_bPlayingMannVsMachine",
	c"m_bPowerupMode",
	c"m_bCompetitiveMode",
	c"m_bTruceActive",
	c"m_bHelltowerPlayersInHell",
	c"m_bIsUsingSpells",
];

/// Where the fake TF2 rules keep the first of [`FLAG_NAMES`], each next one
/// in the byte after.
pub const FLAGS: usize = 36;

/// Where the fake TF2 rules keep `m_nForceUpgrades`, after the last of
/// [`FLAG_NAMES`].
pub const FORCE_UPGRADES: usize = 48;

/// Where the fake TF2 rules keep `m_nForceEscortPushLogic`.
pub const FORCE_ESCORT_PUSH: usize = 52;

/// Where the fake TF2 rules keep `m_nGameType`.
pub const GAME_TYPE: usize = 16;

/// Where the fake TF2 rules keep `m_halloweenScenario`.
pub const HALLOWEEN_SCENARIO: usize = 32;

/// Where the fake TF2 rules keep `m_nHudType`.
pub const HUD_TYPE: usize = 24;

/// Where the fake TF2 rules keep `m_nMapHolidayType`.
pub const MAP_HOLIDAY: usize = 28;

/// The flags the fake round-based rules keep from [`ROUND_FLAGS`] on, in
/// order.
pub const ROUND_FLAG_NAMES: [&CStr; 4] = [
	c"m_bInOvertime",
	c"m_bStopWatch",
	c"m_bMultipleTrains",
	c"m_bSwitchedTeamsThisRound",
];

/// Where the fake round-based rules keep the first of [`ROUND_FLAG_NAMES`],
/// each next one in the byte after.
pub const ROUND_FLAGS: usize = 24;

/// Where the fake round-based rules keep `m_iRoundState`.
pub const ROUND_STATE: usize = 8;

/// Where the fake round-based rules keep `m_nRoundsPlayed`.
pub const ROUNDS_PLAYED: usize = 20;

/// The size of each fake game rules object.
pub const RULES_SIZE: usize = 56;

/// Where the fake round-based rules keep `m_bInSetup`.
pub const SETUP: usize = 13;

/// Where the fake TF2 rules keep a variable named as the round-based rules'
/// `m_iRoundState`, which that one shadows.
pub const SHADOWED: usize = 20;

/// Where the fake round-based rules keep `m_bInWaitingForPlayers`.
pub const WAITING: usize = 12;

/// Where the fake round-based rules keep `m_iWinningTeam`.
pub const WINNING_TEAM: usize = 16;

thread_local! {
	/// The entities [`find_by_class_name`] finds as `tf_gamerules`, in order.
	pub static ENTITIES: RefCell<Vec<*mut sys::CBaseEntity>> = const { RefCell::new(Vec::new()) };

	/// The calls of [`rules_proxy`] and [`round_rules_proxy`], in order.
	pub static PROXY_CALLS: RefCell<Vec<ProxyCall>> = const { RefCell::new(Vec::new()) };

	/// What [`rules_proxy`] and [`round_rules_proxy`] give: the game rules as
	/// `CTFGameRules` and as `CTeamplayRoundBasedRules`, which are separate
	/// here to tell which table is read relative to which.
	pub static RULES: Cell<(*mut c_void, *mut c_void)> = const { Cell::new((null_mut(), null_mut())) };

	/// The head of the server class list [`server_classes`] returns.
	static SERVER_CLASSES: Cell<*mut sys::ServerClass> = const { Cell::new(null_mut()) };
}

/// How a fake proxy entity's table nests the game rules' tables, which
/// [`Tables::TF2`] does as TF2's does.
#[derive(Clone, Copy)]
pub struct Tables {
	/// The proxy of the property nesting the round-based rules' table.
	pub round_proxy: sys::SendTableProxyFn,

	/// The offset and proxy of the property nesting the base class's table.
	pub base: (c_int, sys::SendTableProxyFn),

	/// The name of the TF2 rules' own table.
	pub rules_table: &'static CStr,
}

impl Tables {
	pub const TF2: Self = Self {
		round_proxy: Some(round_rules_proxy),
		base: (0, Some(direct_table)),
		rules_table: c"DT_TFGameRules",
	};
}

/// A fake game: the game rules' tables, the proxy entity's class and edict,
/// and the interfaces they are found through, on this thread.
///
/// For tests only. Everything is leaked, so earlier worlds stay valid, but
/// each new one replaces what the interfaces find.
pub struct World {
	pub class: *mut sys::ServerClass,
	pub edict: *mut sys::edict_t,
	pub entity: MockEntity,
	pub round_rules: *mut [u8; RULES_SIZE],
	pub rules: *mut [u8; RULES_SIZE],
}

impl World {
	/// A game whose proxy entity nests the round-based rules' table through
	/// `round_proxy`, and otherwise as TF2's does.
	pub fn new(round_proxy: sys::SendTableProxyFn) -> Self {
		Self::with(Tables {
			round_proxy,
			..Tables::TF2
		})
	}

	/// A game whose proxy entity nests the game rules' tables as `tables` says.
	pub fn with(tables: Tables) -> Self {
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

		let mut round_rules_props = vec![
			int(c"m_iRoundState", ROUND_STATE),
			flag(c"m_bInWaitingForPlayers", WAITING),
			flag(c"m_bInSetup", SETUP),
			int(c"m_iWinningTeam", WINNING_TEAM),
			int(c"m_nRoundsPlayed", ROUNDS_PLAYED),
		];

		round_rules_props.extend(
			ROUND_FLAG_NAMES
				.into_iter()
				.zip(ROUND_FLAGS..)
				.map(|(name, offset)| flag(name, offset)),
		);

		let round_rules_table = leak(table(
			c"DT_TeamplayRoundBasedRules",
			round_rules_props.leak(),
		));

		// The TF2 rules shadow a variable of the round-based rules.
		let mut rules_props = vec![
			int(c"m_nGameType", GAME_TYPE),
			int(c"m_iRoundState", SHADOWED),
			int(c"m_nHudType", HUD_TYPE),
			int(c"m_nMapHolidayType", MAP_HOLIDAY),
			int(c"m_halloweenScenario", HALLOWEEN_SCENARIO),
			int(c"m_nForceUpgrades", FORCE_UPGRADES),
			int(c"m_nForceEscortPushLogic", FORCE_ESCORT_PUSH),
		];

		rules_props.extend(
			FLAG_NAMES
				.into_iter()
				.zip(FLAGS..)
				.map(|(name, offset)| flag(name, offset)),
		);

		let rules_table = leak(table(tables.rules_table, rules_props.leak()));
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

		let rules = leak([0; RULES_SIZE]);
		let round_rules = leak([0; RULES_SIZE]);
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

	/// Reads a byte of the fake round-based rules, or of the TF2 rules.
	pub fn byte(&self, round_rules: bool, offset: usize) -> u8 {
		// SAFETY: The rules are leaked.
		unsafe { (*self.object(round_rules))[offset] }
	}

	/// Reads an integer of the fake round-based rules, or of the TF2 rules.
	pub fn int(&self, round_rules: bool, offset: usize) -> c_int {
		// SAFETY: The rules are leaked, and the integer lies within them.
		unsafe {
			self.object(round_rules)
				.cast::<u8>()
				.add(offset)
				.cast::<c_int>()
				.read_unaligned()
		}
	}

	/// The fake round-based rules, or the TF2 rules.
	fn object(&self, round_rules: bool) -> *mut [u8; RULES_SIZE] {
		match round_rules {
			true => self.round_rules,
			false => self.rules,
		}
	}

	/// Writes a byte of the fake round-based rules, or of the TF2 rules.
	pub fn put_byte(&self, round_rules: bool, offset: usize, value: u8) {
		// SAFETY: The rules are leaked, and only the tests write them.
		unsafe { (*self.object(round_rules))[offset] = value };
	}

	/// Writes an integer of the fake round-based rules, or of the TF2 rules.
	pub fn put_int(&self, round_rules: bool, offset: usize, value: c_int) {
		// SAFETY: As for `put_byte`, and the integer lies within the rules.
		unsafe {
			self.object(round_rules)
				.cast::<u8>()
				.add(offset)
				.cast::<c_int>()
				.write_unaligned(value)
		};
	}

	/// Writes a byte of the fake round-based rules.
	pub fn put_round_byte(&self, offset: usize, value: u8) {
		self.put_byte(true, offset, value);
	}

	/// The bytes of the fake round-based rules.
	pub fn round_bytes(&self) -> [u8; RULES_SIZE] {
		// SAFETY: The rules are leaked.
		unsafe { *self.round_rules }
	}

	/// The `m_fStateFlags` of the proxy entity's edict.
	pub fn state_flags(&self) -> c_int {
		// SAFETY: The edict is leaked.
		unsafe { (*self.edict)._base.m_fStateFlags }
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

/// A table proxy that gives nothing, as TF2's do without game rules.
///
/// # Safety
///
/// `recipients` must point to a recipient set, as the engine's do.
pub unsafe extern "C" fn no_rules_proxy(
	_: *const sys::SendProp,
	struct_base: *const c_void,
	data: *const c_void,
	recipients: *mut sys::CSendProxyRecipients,
	object_id: c_int,
) -> *mut c_void {
	// SAFETY: The caller passes a recipient set.
	unsafe { record(struct_base, data, recipients, object_id, null_mut()) }
}

/// Records a table proxy's call, and gives `object`.
///
/// # Safety
///
/// As for [`no_rules_proxy`].
unsafe fn record(
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
///
/// # Safety
///
/// As for [`no_rules_proxy`].
pub unsafe extern "C" fn round_rules_proxy(
	_: *const sys::SendProp,
	struct_base: *const c_void,
	data: *const c_void,
	recipients: *mut sys::CSendProxyRecipients,
	object_id: c_int,
) -> *mut c_void {
	// SAFETY: The caller passes a recipient set.
	unsafe { record(struct_base, data, recipients, object_id, RULES.get().1) }
}

/// `SendProxy_TFGameRules`, which gives the TF2 rules.
///
/// # Safety
///
/// As for [`no_rules_proxy`].
pub unsafe extern "C" fn rules_proxy(
	_: *const sys::SendProp,
	struct_base: *const c_void,
	data: *const c_void,
	recipients: *mut sys::CSendProxyRecipients,
	object_id: c_int,
) -> *mut c_void {
	// SAFETY: The caller passes a recipient set.
	unsafe { record(struct_base, data, recipients, object_id, RULES.get().0) }
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
