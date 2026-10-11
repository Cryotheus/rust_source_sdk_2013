//! Tests of `crate::hooks::tf2::rules`: hooks of the game rules' decisions on a mock
//! game rules class, through the mock SourceHook and KHook.

use super::*;
use crate::hook::Signature;
use crate::test_support::harness::{Harness, on_both};
use crate::test_support::server::{no_interfaces, tf2_binding};
use source_sdk_2013::sys;
use source_sdk_2013::tf2::damage::DamageType;
use std::cell::{Cell, RefCell};
use std::ffi::c_int;
use std::ptr;

/// What ran during a game rules hook's call, in [`CALLS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ran {
	/// The game's method of the name.
	Game(&'static str),

	/// The balance callback.
	Balance,

	/// The ready players callback, with the answer it was given.
	HavePlayers(bool),

	/// The holiday callback, with the holiday and the answer it was given.
	Holiday(Holiday, bool),

	/// The damage callback, with the player's and the attacker's addresses,
	/// the bits of the damage's amount, and the answer it was given.
	Damage(usize, usize, u32, bool),

	/// The scramble callback.
	Scramble,

	/// The switch callback.
	Switch,
}

thread_local! {
	/// What the balance, scramble and switch callbacks decide.
	static TEAMS: Cell<TeamsAction> = const { Cell::new(TeamsAction::Allow) };

	/// What ran during the calls since the last [`rules_call`], in order.
	static CALLS: RefCell<Vec<Ran>> = const { RefCell::new(Vec::new()) };

	/// What the damage callback answers.
	static DAMAGE: Cell<Option<bool>> = const { Cell::new(None) };

	/// What the ready players callback answers.
	static HAVE_PLAYERS: Cell<Option<bool>> = const { Cell::new(None) };

	/// What the holiday callback answers.
	static HOLIDAY: Cell<Option<bool>> = const { Cell::new(None) };
}

/// A game rules object of a C++ class, as far as hooks know it.
#[repr(C)]
struct GameRules {
	vtable: *mut *mut c_void,
}

impl GameRules {
	/// An object of the class whose vtable is `vtable`.
	fn of_class(vtable: NonNull<*mut c_void>) -> Box<Self> {
		Box::new(Self {
			vtable: vtable.as_ptr(),
		})
	}

	fn ptr(&mut self) -> NonNull<c_void> {
		NonNull::from(self).cast()
	}
}

/// Notes that `ran` ran.
fn ran(ran: Ran) {
	CALLS.with_borrow_mut(|calls| calls.push(ran));
}

/// Every game rules callback, each noting that it ran, and deciding as
/// [`TEAMS`], [`HAVE_PLAYERS`], [`HOLIDAY`] and [`DAMAGE`] say.
fn every_callback() -> RulesCallbacks {
	RulesCallbacks {
		balance_teams: Some(|_| {
			ran(Ran::Balance);
			TEAMS.get()
		}),
		have_players: Some(|_, ready| {
			ran(Ran::HavePlayers(ready));
			HAVE_PLAYERS.get()
		}),
		holiday: Some(|_, holiday, active| {
			ran(Ran::Holiday(holiday, active));
			HOLIDAY.get()
		}),
		player_damage: Some(|_, check| {
			ran(Ran::Damage(
				check.player.as_ptr().addr(),
				check.attacker.as_ptr().addr(),
				check.info.amount().to_bits(),
				check.takes_damage,
			));
			DAMAGE.get()
		}),
		scramble_teams: Some(|_| {
			ran(Ran::Scramble);
			TEAMS.get()
		}),
		switch_teams: Some(|_| {
			ran(Ran::Switch);
			TEAMS.get()
		}),
	}
}

/// The vtable of a new game rules class, which holds the game's methods at
/// their slots, each noting that it ran: no player is ready, only Halloween
/// is active, players take all damage, and the teams are kept balanced,
/// switched and scrambled.
fn new_rules_class() -> NonNull<*mut c_void> {
	unsafe extern "C" fn have_players(_: *mut c_void) -> bool {
		ran(Ran::Game("have players"));
		false
	}

	unsafe extern "C" fn is_holiday_active(_: *mut c_void, holiday: c_int) -> bool {
		ran(Ran::Game("holiday"));
		holiday == Holiday::Halloween.to_raw()
	}

	unsafe extern "C" fn player_can_take_damage(
		_: *mut c_void,
		_: *mut sys::CBasePlayer,
		_: *mut sys::CBaseEntity,
		_: *const sys::CTakeDamageInfo,
	) -> bool {
		ran(Ran::Game("damage"));
		true
	}

	unsafe extern "C" fn should_balance_teams(_: *mut c_void) -> bool {
		ran(Ran::Game("balance"));
		true
	}

	unsafe extern "C" fn should_scramble_teams(_: *mut c_void) -> bool {
		ran(Ran::Game("scramble"));
		true
	}

	unsafe extern "C" fn should_switch_teams(_: *mut c_void) -> bool {
		ran(Ran::Game("switch"));
		true
	}

	let methods: [(usize, *mut c_void); 6] = [
		(HAVE_PLAYERS_SLOT, have_players as HavePlayers as _),
		(
			IS_HOLIDAY_ACTIVE_SLOT,
			is_holiday_active as IsHolidayActive as _,
		),
		(
			PLAYER_CAN_TAKE_DAMAGE_SLOT,
			player_can_take_damage as PlayerCanTakeDamage as _,
		),
		(
			SHOULD_BALANCE_TEAMS_SLOT,
			should_balance_teams as ShouldBalanceTeams as _,
		),
		(
			SHOULD_SCRAMBLE_TEAMS_SLOT,
			should_scramble_teams as ShouldScrambleTeams as _,
		),
		(
			SHOULD_SWITCH_TEAMS_SLOT,
			should_switch_teams as ShouldSwitchTeams as _,
		),
	];

	let length = methods.iter().map(|&(slot, _)| slot).max().unwrap() + 1;
	let filler = should_balance_teams as ShouldBalanceTeams as *mut c_void;
	let slots = Vec::leak(vec![filler; length]);

	for (slot, method) in methods {
		slots[slot] = method;
	}

	NonNull::new(slots.as_mut_ptr()).unwrap()
}

/// Calls `rules`'s hooked method of the signature `S` at `slot`, and returns
/// what it returned and what ran.
fn rules_call<S: Signature<This = c_void>>(
	harness: &Harness,
	rules: &mut GameRules,
	slot: usize,
	args: S::Args,
) -> (S::Output, Vec<Ran>) {
	CALLS.take();

	let output = harness.call::<S>(rules.ptr().as_ptr(), slot, args);

	(output, CALLS.take())
}

#[test]
fn holidays_and_damage_are_decided_again_after_the_game() {
	on_both(|harness| {
		let api = harness.api();
		let class = new_rules_class();
		let mut rules = GameRules::of_class(class);

		// SAFETY: The mock class has the methods at their slots, and is leaked.
		let _hooks =
			unsafe { api.install_rules(class, tf2_binding(no_interfaces), every_callback()) }
				.unwrap();

		let holiday = |harness, rules: &mut GameRules, holiday| {
			rules_call::<IsHolidayActive>(harness, rules, IS_HOLIDAY_ACTIVE_SLOT, (holiday,))
		};
		let halloween = Holiday::Halloween.to_raw();

		HOLIDAY.set(None);
		assert_eq!(
			holiday(harness, &mut rules, halloween),
			(
				true,
				vec![Ran::Game("holiday"), Ran::Holiday(Holiday::Halloween, true)]
			)
		);

		HOLIDAY.set(Some(false));
		assert!(!holiday(harness, &mut rules, halloween).0);

		HOLIDAY.set(Some(true));
		assert_eq!(
			holiday(harness, &mut rules, Holiday::Christmas.to_raw()),
			(
				true,
				vec![
					Ran::Game("holiday"),
					Ran::Holiday(Holiday::Christmas, false)
				]
			)
		);

		// A holiday the crate does not know is left to the game.
		assert_eq!(
			holiday(harness, &mut rules, 99),
			(false, vec![Ran::Game("holiday")])
		);

		let player = ptr::without_provenance_mut::<sys::CBasePlayer>(0x9a7);
		let attacker = ptr::without_provenance_mut::<sys::CBaseEntity>(0xa77);
		let info = DamageInfo::new(12.0, DamageType::BULLET);
		let damage = |harness, rules: &mut GameRules, attacker| {
			rules_call::<PlayerCanTakeDamage>(
				harness,
				rules,
				PLAYER_CAN_TAKE_DAMAGE_SLOT,
				(player, attacker, info.as_ptr()),
			)
		};
		let checked = Ran::Damage(0x9a7, 0xa77, 12.0f32.to_bits(), true);

		DAMAGE.set(None);
		assert_eq!(
			damage(harness, &mut rules, attacker),
			(true, vec![Ran::Game("damage"), checked])
		);

		DAMAGE.set(Some(false));
		assert_eq!(
			damage(harness, &mut rules, attacker),
			(false, vec![Ran::Game("damage"), checked])
		);

		// Without an attacker, the game decides.
		assert_eq!(
			damage(harness, &mut rules, ptr::null_mut()),
			(true, vec![Ran::Game("damage")])
		);
	});
}

#[test]
fn ready_players_are_decided_again_after_the_game() {
	on_both(|harness| {
		let api = harness.api();
		let class = new_rules_class();
		let mut rules = GameRules::of_class(class);

		// SAFETY: As above.
		let hooks =
			unsafe { api.install_rules(class, tf2_binding(no_interfaces), every_callback()) }
				.unwrap();

		let ready = |harness, rules: &mut GameRules| {
			rules_call::<HavePlayers>(harness, rules, HAVE_PLAYERS_SLOT, ())
		};

		HAVE_PLAYERS.set(None);
		assert_eq!(
			ready(harness, &mut rules),
			(
				false,
				vec![Ran::Game("have players"), Ran::HavePlayers(false)]
			)
		);

		HAVE_PLAYERS.set(Some(true));
		assert_eq!(
			ready(harness, &mut rules),
			(
				true,
				vec![Ran::Game("have players"), Ran::HavePlayers(false)]
			)
		);

		// Removed hooks run no callback.
		hooks.remove(api);

		assert_eq!(
			ready(harness, &mut rules),
			(false, vec![Ran::Game("have players")])
		);
	});
}

#[test]
fn team_balance_switches_and_scrambles_are_refused_where_the_hooks_say_so() {
	on_both(|harness| {
		let api = harness.api();
		let class = new_rules_class();
		let mut rules = GameRules::of_class(class);

		// SAFETY: As above.
		let hooks =
			unsafe { api.install_rules(class, tf2_binding(no_interfaces), every_callback()) }
				.unwrap();

		let teams = |harness, rules: &mut GameRules, slot| {
			rules_call::<ShouldBalanceTeams>(harness, rules, slot, ())
		};
		let decisions = [
			(SHOULD_BALANCE_TEAMS_SLOT, Ran::Balance, "balance"),
			(SHOULD_SCRAMBLE_TEAMS_SLOT, Ran::Scramble, "scramble"),
			(SHOULD_SWITCH_TEAMS_SLOT, Ran::Switch, "switch"),
		];

		for (slot, callback, game) in decisions {
			TEAMS.set(TeamsAction::Allow);
			assert_eq!(
				teams(harness, &mut rules, slot),
				(true, vec![callback, Ran::Game(game)])
			);

			TEAMS.set(TeamsAction::Refuse);
			assert_eq!(teams(harness, &mut rules, slot), (false, vec![callback]));
		}

		// Removed hooks run no callback.
		hooks.remove(api);

		for (slot, _, game) in decisions {
			assert_eq!(
				teams(harness, &mut rules, slot),
				(true, vec![Ran::Game(game)])
			);
		}
	});
}

#[test]
fn rules_hooks_are_installed_all_at_once() {
	on_both(|harness| {
		let api = harness.api();
		let scope = ();
		let binding = tf2_binding(no_interfaces);
		// SAFETY: The server's game module is this test's executable, which
		// `no_interfaces` is in, and which has no game rules class.
		let server = unsafe { binding.server(&scope) };

		// Without callbacks, nothing is searched for.
		let none = api
			.hook_rules(server, binding, RulesCallbacks::default())
			.unwrap();

		assert!(none.hooks.is_empty());
		assert!(matches!(
			api.hook_rules(server, binding, every_callback()),
			Err(RoundHookError::Target(GameRulesVtableError::NotFound))
		));

		let class = new_rules_class();

		// SAFETY: As above.
		let hooks = unsafe { api.install_rules(class, binding, every_callback()) }.unwrap();

		assert_eq!(hooks.hooks.len(), 6);

		// An installed hook is reported without searching the module again.
		assert!(matches!(
			api.hook_rules(server, binding, every_callback()),
			Err(RoundHookError::Hook(HookError::AlreadyInstalled))
		));

		// Once removed, they can be installed again, a few at a time.
		hooks.remove(api);

		let callbacks = RulesCallbacks {
			holiday: Some(|_, _, _| None),
			..RulesCallbacks::default()
		};
		// SAFETY: As above.
		let hooks = unsafe { api.install_rules(class, binding, callbacks) }.unwrap();

		assert_eq!(hooks.hooks.len(), 1);
	});
}
