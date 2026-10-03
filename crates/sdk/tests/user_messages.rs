//! Tests of user messages' payloads, which must match what the game's own
//! senders write, and of finding a team's players through the engine and the
//! game's player info.

use sdk_raw::test_support::{mock_vtable, unexpected_call};
use source_sdk_2013::Module;
use source_sdk_2013::interfaces::{PlayerInfoManager, ValveEngine};
use source_sdk_2013::math::Color32;
use source_sdk_2013::test_support::edicts::{edict_of_index, edict_table, serve_edicts};

use source_sdk_2013::test_support::interfaces::player_info_manager::{
	global_vars, serve_global_vars,
};

use source_sdk_2013::test_support::server::{export, mock_server};
use source_sdk_2013::test_support::user_messages::payload;
use source_sdk_2013::user_messages::Recipients;

use source_sdk_2013::user_messages::messages::{
	Fade, FadeFlags, Shake, ShakeCommand, TextDestination, TextMsg,
};

use std::cell::Cell;
use std::ffi::c_int;
use std::ptr::null_mut;

thread_local! {
	/// The players [`player_info`] reports, by edict index.
	static PLAYERS: Cell<*const [*mut MockPlayer]> =
		const { Cell::new(std::ptr::slice_from_raw_parts(std::ptr::null(), 0)) };
}

/// A player's `IPlayerInfo`, which reports the team it was made with.
#[repr(C)]
struct MockPlayer {
	interface: sys::IPlayerInfo,
	team: c_int,
}

#[test]
fn fade_times_clamp_to_their_range() {
	// The fade's duration and hold time, as written.
	let times = |duration, hold| {
		let fade = Fade {
			duration,
			hold,
			flags: FadeFlags::empty(),
			color: Color32::rgb(0, 0, 0),
		};
		let bits = payload(&fade);
		let mut reader = bits.reader();

		(reader.read_u16().unwrap(), reader.read_u16().unwrap())
	};

	assert_eq!(times(-1.0, 1000.0), (0, u16::MAX));
	assert_eq!(times(f32::NAN, 1.5), (0, 768));
}

#[test]
fn fixed_size_messages_have_their_registered_sizes() {
	let fade = Fade {
		duration: 1.5,
		hold: 100.0,
		flags: FadeFlags::OUT.union(FadeFlags::STAY_OUT),
		color: Color32::rgb(0, 0, 0),
	};
	let bits = payload(&fade);
	let mut reader = bits.reader();

	// `Fade` is registered with 10 bytes, `Shake` with 13.
	assert_eq!(bits.byte_len(), 10);
	assert_eq!(reader.read_u16(), Ok(768));
	assert_eq!(reader.read_u16(), Ok(51200));
	assert_eq!(reader.read_u16(), Ok(0xA));

	let shake = Shake {
		command: ShakeCommand::Start,
		amplitude: 10.0,
		frequency: 150.0,
		duration: 2.0,
	};

	assert_eq!(payload(&shake).byte_len(), 13);
}

/// `IPlayerInfoManager::GetPlayerInfo`, which returns the player of the
/// edict's index in [`PLAYERS`], or null.
unsafe extern "C" fn player_info(
	_: *mut sys::IPlayerInfoManager,
	edict: *mut sys::edict_t,
) -> *mut sys::IPlayerInfo {
	// SAFETY: The wrapper passes an edict of the mock table, and the players
	// are leaked.
	unsafe {
		let index = (*edict)._base.m_EdictIndex;

		usize::try_from(index)
			.ok()
			.and_then(|index| (&*PLAYERS.get()).get(index))
			.map_or(null_mut(), |&player| player.cast())
	}
}

/// `IPlayerInfo::GetTeamIndex`, which returns the team the player was made
/// with.
unsafe extern "C" fn team_index(this: *mut sys::IPlayerInfo) -> c_int {
	// SAFETY: Every player info the mock returns is a `MockPlayer`.
	unsafe { (*this.cast::<MockPlayer>()).team }
}

#[test]
fn teams_are_the_players_reporting_them() {
	// SAFETY: The vtables hold only function pointers, `unexpected_call`
	// aborts whichever slot reaches it, and each patch only writes slots of the
	// vtable being built.
	let (player_vtable, engine_vtable, manager_vtable) = unsafe {
		(
			mock_vtable::<sys::IPlayerInfo__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| (&raw mut (*vtable).IPlayerInfo_GetTeamIndex).write(team_index),
			),
			mock_vtable::<sys::IVEngineServer__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).IVEngineServer_PEntityOfEntIndex).write(edict_of_index);
				},
			),
			mock_vtable::<sys::IPlayerInfoManager__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).IPlayerInfoManager_GetGlobalVars).write(global_vars);
					(&raw mut (*vtable).IPlayerInfoManager_GetPlayerInfo).write(player_info);
				},
			),
		)
	};
	let player = |team| {
		Box::into_raw(Box::new(MockPlayer {
			interface: sys::IPlayerInfo {
				vtable_: &raw const *player_vtable,
			},
			team,
		}))
	};

	// Slot 3 is a client still connecting, which has no player, slot 5 is
	// free, and slot 6 lies past the client limit.
	let players = vec![
		null_mut(),
		player(2),
		player(3),
		null_mut(),
		player(2),
		player(2),
		player(2),
	];
	let mut table = edict_table(7, |slot| slot == 5);

	serve_global_vars(5);
	serve_edicts(table.as_mut_ptr(), table.len());
	PLAYERS.set(Vec::leak(players));

	let mut engine = sys::IVEngineServer {
		vtable_: &raw const *engine_vtable,
	};
	let mut manager = sys::IPlayerInfoManager {
		vtable_: &raw const *manager_vtable,
	};

	export(Module::Engine, ValveEngine::VERSION, &raw mut engine);
	export(
		Module::GameServer,
		PlayerInfoManager::VERSION,
		&raw mut manager,
	);

	let scope = ();
	let server = mock_server(&scope);
	let team = |team| Recipients::team(server, team).unwrap();

	assert_eq!(team(2).players(), [1, 4]);
	assert_eq!(team(3).players(), [2]);
	assert!(team(0).is_empty());
}

#[test]
fn text_messages_carry_every_argument() {
	let message = TextMsg {
		destination: TextDestination::Center,
		message: c"Run.",
		arguments: [c"", c"", c"", c""],
	};
	let bits = payload(&message);
	let mut reader = bits.reader();

	assert_eq!(reader.read_u8(), Ok(4));
	assert_eq!(reader.read_cstring().as_deref(), Ok(c"Run."));

	for _ in 0..4 {
		assert_eq!(reader.read_cstring().as_deref(), Ok(c""));
	}

	assert_eq!(reader.remaining(), 0);
}
