//! Tests of `crate::round_hooks`: pre hooks of `CleanUpMap` on a mock
//! game rules class, through the mock SourceHook and KHook.

use super::*;
use crate::test_support::harness::{Harness, on_both};
use crate::test_support::server::{no_interfaces, tf2_binding};
use source_sdk_2013::InterfaceFactory;
use std::cell::RefCell;
use std::ffi::c_char;
use std::ptr;

thread_local! {
	/// What ran during the calls since the last [`clean_up`], in order, with
	/// the object each ran for.
	static CALLS: RefCell<Vec<(&'static str, usize)>> = const { RefCell::new(Vec::new()) };

	/// Whether the callback skips the cleanup.
	static SKIP: Cell<bool> = const { Cell::new(false) };
}

/// A game rules object of a C++ class, as far as hooks know it.
#[repr(C)]
struct GameRules {
	vtable: *mut *mut c_void,
}

impl GameRules {
	/// The vtable of a new class, which holds `clean_up` at
	/// [`CLEAN_UP_MAP_SLOT`].
	fn new_class(clean_up: CleanUpMap) -> NonNull<*mut c_void> {
		let slots = Vec::leak(vec![clean_up as *mut c_void; CLEAN_UP_MAP_SLOT + 1]);

		NonNull::new(slots.as_mut_ptr()).unwrap()
	}

	/// An object of the class whose vtable is `vtable`, as each level makes.
	fn of_class(vtable: NonNull<*mut c_void>) -> Box<Self> {
		Box::new(Self {
			vtable: vtable.as_ptr(),
		})
	}

	fn ptr(&mut self) -> NonNull<c_void> {
		NonNull::from(self).cast()
	}
}

/// The game's `CleanUpMap`, which notes that it ran.
unsafe extern "C" fn game_clean_up(this: *mut c_void) {
	CALLS.with_borrow_mut(|calls| calls.push(("game", this.addr())));
}

/// The callback, which notes that it ran, and skips the cleanup if [`SKIP`].
fn on_clean_up(_server: Server<'_>) -> MapCleanupAction {
	CALLS.with_borrow_mut(|calls| calls.push(("callback", 0)));

	match SKIP.get() {
		true => MapCleanupAction::Skip,
		false => MapCleanupAction::Allow,
	}
}

/// Calls `rules`'s hooked `CleanUpMap`, and returns what ran.
fn clean_up(harness: &Harness, rules: &mut GameRules) -> Vec<(&'static str, usize)> {
	CALLS.take();
	harness.call::<CleanUpMap>(rules.ptr().as_ptr(), CLEAN_UP_MAP_SLOT, ());
	CALLS.take()
}

#[test]
fn cleanups_run_unless_the_hook_skips_them() {
	on_both(|harness| {
		let api = harness.api();
		let class = GameRules::new_class(game_clean_up);
		let mut rules = GameRules::of_class(class);
		let address = rules.ptr().addr().get();

		// SAFETY: The mock class has `CleanUpMap` at the slot, and is leaked.
		unsafe { api.install_map_cleanup(class, tf2_binding(no_interfaces), on_clean_up) }.unwrap();

		// A second hook of the class is refused, so that each cleanup is decided
		// once, and so is a hook of another class.
		assert!(matches!(
			// SAFETY: As above.
			unsafe { api.install_map_cleanup(class, tf2_binding(no_interfaces), on_clean_up) },
			Err(HookError::AlreadyInstalled)
		));
		assert!(matches!(
			// SAFETY: As above.
			unsafe {
				api.install_map_cleanup(
					GameRules::new_class(game_clean_up),
					tf2_binding(no_interfaces),
					on_clean_up,
				)
			},
			Err(HookError::TooManyFunctions)
		));

		SKIP.set(false);
		assert_eq!(
			clean_up(harness, &mut rules),
			[("callback", 0), ("game", address)]
		);

		SKIP.set(true);
		assert_eq!(clean_up(harness, &mut rules), [("callback", 0)]);
	});
}

#[test]
fn one_hook_covers_the_game_rules_of_every_level() {
	on_both(|harness| {
		let api = harness.api();
		let class = GameRules::new_class(game_clean_up);

		// SAFETY: As above.
		unsafe { api.install_map_cleanup(class, tf2_binding(no_interfaces), on_clean_up) }.unwrap();
		SKIP.set(true);

		// The game rules of three levels, the first gone before the next.
		for _level in 0..3 {
			let mut rules = GameRules::of_class(class);

			assert_eq!(clean_up(harness, &mut rules), [("callback", 0)]);
			drop(rules);
		}
	});
}

#[test]
fn pauses_and_reloads_let_the_game_clean_up() {
	on_both(|harness| {
		let api = harness.api();
		let class = GameRules::new_class(game_clean_up);
		let mut rules = GameRules::of_class(class);
		let address = rules.ptr().addr().get();

		// SAFETY: As above.
		let hook =
			unsafe { api.install_map_cleanup(class, tf2_binding(no_interfaces), on_clean_up) }
				.unwrap();
		SKIP.set(true);

		harness.set_status(true, true, harness.generation);
		assert_eq!(clean_up(harness, &mut rules), [("game", address)]);

		// A later load, which has not installed its own hook yet. Its
		// generation is one no later harness takes, as it installs hooks.
		harness.set_status(true, false, harness.generation | 1 << 63);
		assert_eq!(clean_up(harness, &mut rules), [("game", address)]);
		assert!(!api.has_hook(hook));

		// SAFETY: As above.
		unsafe { api.install_map_cleanup(class, tf2_binding(no_interfaces), on_clean_up) }.unwrap();
		assert_eq!(clean_up(harness, &mut rules), [("callback", 0)]);
	});
}

#[test]
fn removed_or_superseded_hooks_run_no_callback() {
	on_both(|harness| {
		let api = harness.api();
		let class = GameRules::new_class(game_clean_up);
		let mut rules = GameRules::of_class(class);
		let address = rules.ptr().addr().get();

		// SAFETY: As above.
		let hook =
			unsafe { api.install_map_cleanup(class, tf2_binding(no_interfaces), on_clean_up) }
				.unwrap();
		SKIP.set(true);

		assert!(api.remove_hook(hook));
		assert_eq!(clean_up(harness, &mut rules), [("game", address)]);

		// A cleanup an earlier hook skipped reaches neither the callback nor the
		// game.
		fn skip(_call: &HookCall<'_, CleanUpMap>) -> HookAction<()> {
			HookAction::Supersede(())
		}

		// SAFETY: As above.
		unsafe {
			api.add_hook(
				CLEAN_UP_MAP,
				HookTarget::vtable(class),
				HookTiming::Pre,
				&skip,
			)
			.unwrap();
			api.install_map_cleanup(class, tf2_binding(no_interfaces), on_clean_up)
				.unwrap();
		}

		assert_eq!(clean_up(harness, &mut rules), []);
	});
}

#[test]
fn the_game_rules_class_is_searched_for_unless_already_hooked() {
	on_both(|harness| {
		let api = harness.api();
		let scope = ();

		// SAFETY: As for `tf2_binding`, but for another game, whose servers reach
		// no interface before the game is checked.
		let other = unsafe {
			ServerBinding::new(
				InterfaceFactory::new(no_interfaces),
				InterfaceFactory::new(no_interfaces),
				Game::SourceSdk2013,
			)
		};
		// SAFETY: As above.
		let other_server = unsafe { other.server(&scope) };

		assert!(matches!(
			api.hook_map_cleanup(other_server, other, on_clean_up),
			Err(MapCleanupHookError::Target(GameRulesVtableError::WrongGame))
		));

		let binding = tf2_binding(no_interfaces);
		// SAFETY: The server's game module is this test's executable, which
		// `no_interfaces` is in, and which has no game rules class.
		let server = unsafe { binding.server(&scope) };

		assert!(matches!(
			api.hook_map_cleanup(server, binding, on_clean_up),
			Err(MapCleanupHookError::Target(GameRulesVtableError::NotFound))
		));

		// SAFETY: As above.
		unsafe {
			api.install_map_cleanup(GameRules::new_class(game_clean_up), binding, on_clean_up)
		}
		.unwrap();

		// An installed hook is reported without searching the module again.
		assert!(matches!(
			api.hook_map_cleanup(server, binding, on_clean_up),
			Err(MapCleanupHookError::Hook(HookError::AlreadyInstalled))
		));
	});
}

/// What ran during a round hook's call, in [`ROUND_CALLS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ran {
	/// The game's method of the name.
	Game(&'static str),

	/// The win callback, with its event.
	Win(RoundWinEvent),

	/// The stalemate callback, with its event.
	Stalemate(StalemateEvent),

	/// The setup callback.
	Setup,

	/// The running callback.
	Running,

	/// The points callback.
	Points,

	/// The team capture callback, with what it was asked.
	Team(TeamCaptureCheck),

	/// The player capture callback, with the player's address and the point.
	Player(usize, c_int),

	/// The flags callback.
	Flags,

	/// The player block callback, with the player's address and the point.
	Block(usize, c_int),

	/// The cleanup keep callback, with the entity's address.
	Keep(usize),

	/// The cleanup create callback, with the class name's address.
	Create(usize),
}

thread_local! {
	/// What the player block callback decides.
	static BLOCK: Cell<Option<bool>> = const { Cell::new(None) };

	/// What the cleanup create callback decides.
	static CREATE_ACTION: Cell<CleanupCreateAction> = const { Cell::new(CleanupCreateAction::Allow) };

	/// What the cleanup keep callback decides.
	static KEEP_ACTION: Cell<CleanupKeepAction> = const { Cell::new(CleanupKeepAction::Allow) };

	/// What the capture callbacks decide.
	static CAPTURE_ACTION: Cell<CaptureAction> = const { Cell::new(CaptureAction::Allow) };

	/// What the win and stalemate callbacks decide.
	static END_ACTION: Cell<RoundEndAction> = const { Cell::new(RoundEndAction::Allow) };

	/// What ran during the round calls since the last [`round_call`], in
	/// order.
	static ROUND_CALLS: RefCell<Vec<Ran>> = const { RefCell::new(Vec::new()) };
}

/// Notes that `ran` ran.
fn ran(ran: Ran) {
	ROUND_CALLS.with_borrow_mut(|calls| calls.push(ran));
}

/// Every round callback, each noting that it ran, and deciding as
/// [`END_ACTION`], [`CAPTURE_ACTION`], [`BLOCK`], [`KEEP_ACTION`] and
/// [`CREATE_ACTION`] say.
fn every_callback() -> RoundCallbacks {
	RoundCallbacks {
		cleanup_create: Some(|_, class_name| {
			ran(Ran::Create(class_name.as_ptr().addr()));
			CREATE_ACTION.get()
		}),
		cleanup_keep: Some(|_, entity| {
			ran(Ran::Keep(entity.as_ptr().addr()));
			KEEP_ACTION.get()
		}),
		flags_capturable: Some(|_| {
			ran(Ran::Flags);
			CAPTURE_ACTION.get()
		}),
		player_block: Some(|_, check| {
			ran(Ran::Block(check.player.as_ptr().addr(), check.point));
			BLOCK.get()
		}),
		player_capture: Some(|_, check| {
			ran(Ran::Player(check.player.as_ptr().addr(), check.point));
			CAPTURE_ACTION.get()
		}),
		points_capturable: Some(|_| {
			ran(Ran::Points);
			CAPTURE_ACTION.get()
		}),
		running: Some(|_| ran(Ran::Running)),
		setup: Some(|_| ran(Ran::Setup)),
		stalemate: Some(|_, event| {
			ran(Ran::Stalemate(event));
			END_ACTION.get()
		}),
		team_capture: Some(|_, check| {
			ran(Ran::Team(check));
			CAPTURE_ACTION.get()
		}),
		win: Some(|_, event| {
			ran(Ran::Win(event));
			END_ACTION.get()
		}),
	}
}

/// The vtable of a new game rules class, which holds the game's round methods
/// at their slots, each noting that it ran.
fn new_round_class() -> NonNull<*mut c_void> {
	unsafe extern "C" fn flags_may_be_capped(_: *mut c_void) -> bool {
		ran(Ran::Game("flags"));
		true
	}

	unsafe extern "C" fn player_may_block(
		_: *mut c_void,
		_: *mut source_sdk_2013::sys::CBasePlayer,
		_: c_int,
		reason: *mut c_char,
		_: c_int,
	) -> bool {
		if !reason.is_null() {
			// SAFETY: The tests pass a buffer of at least a byte.
			unsafe { reason.write(b'x' as c_char) };
		}

		ran(Ran::Game("block"));
		false
	}

	unsafe extern "C" fn round_cleanup_should_ignore(
		_: *mut c_void,
		_: *mut source_sdk_2013::sys::CBaseEntity,
	) -> bool {
		ran(Ran::Game("keep"));
		false
	}

	unsafe extern "C" fn should_create_entity(_: *mut c_void, _: *const c_char) -> bool {
		ran(Ran::Game("create"));
		true
	}

	unsafe extern "C" fn player_may_capture(
		_: *mut c_void,
		_: *mut source_sdk_2013::sys::CBasePlayer,
		_: c_int,
		reason: *mut c_char,
		_: c_int,
	) -> bool {
		if !reason.is_null() {
			// SAFETY: The tests pass a buffer of at least a byte.
			unsafe { reason.write(b'x' as c_char) };
		}

		ran(Ran::Game("player"));
		true
	}

	unsafe extern "C" fn points_may_be_captured(_: *mut c_void) -> bool {
		ran(Ran::Game("points"));
		true
	}

	unsafe extern "C" fn running(_: *mut c_void) {
		ran(Ran::Game("running"));
	}

	unsafe extern "C" fn setup(_: *mut c_void) {
		ran(Ran::Game("setup"));
	}

	unsafe extern "C" fn stalemate(_: *mut c_void, _: c_int, _: bool, _: bool) {
		ran(Ran::Game("stalemate"));
	}

	unsafe extern "C" fn team_may_capture(_: *mut c_void, _: c_int, _: c_int) -> bool {
		ran(Ran::Game("team"));
		true
	}

	unsafe extern "C" fn win(
		_: *mut c_void,
		_: c_int,
		_: c_int,
		_: bool,
		_: bool,
		_: bool,
		_: bool,
	) {
		ran(Ran::Game("win"));
	}

	let methods: [(usize, *mut c_void); 11] = [
		(
			FLAGS_MAY_BE_CAPPED_SLOT,
			flags_may_be_capped as FlagsMayBeCapped as _,
		),
		(
			PLAYER_MAY_BLOCK_POINT_SLOT,
			player_may_block as PlayerMayBlockPoint as _,
		),
		(
			ROUND_CLEANUP_SHOULD_IGNORE_SLOT,
			round_cleanup_should_ignore as RoundCleanupShouldIgnore as _,
		),
		(
			SHOULD_CREATE_ENTITY_SLOT,
			should_create_entity as ShouldCreateEntity as _,
		),
		(
			PLAYER_MAY_CAPTURE_POINT_SLOT,
			player_may_capture as PlayerMayCapturePoint as _,
		),
		(
			POINTS_MAY_BE_CAPTURED_SLOT,
			points_may_be_captured as PointsMayBeCaptured as _,
		),
		(SETUP_ON_ROUND_RUNNING_SLOT, running as RoundSetup as _),
		(SETUP_ON_ROUND_START_SLOT, setup as RoundSetup as _),
		(SET_STALEMATE_SLOT, stalemate as SetStalemate as _),
		(
			TEAM_MAY_CAPTURE_POINT_SLOT,
			team_may_capture as TeamMayCapturePoint as _,
		),
		(SET_WINNING_TEAM_SLOT, win as SetWinningTeam as _),
	];

	let length = methods.iter().map(|&(slot, _)| slot).max().unwrap() + 1;
	let slots = Vec::leak(vec![game_clean_up as CleanUpMap as *mut c_void; length]);

	for (slot, method) in methods {
		slots[slot] = method;
	}

	NonNull::new(slots.as_mut_ptr()).unwrap()
}

/// Calls `rules`'s hooked method of the signature `S` at `slot`, and returns
/// what it returned and what ran.
fn round_call<S: Signature<This = c_void>>(
	harness: &Harness,
	rules: &mut GameRules,
	slot: usize,
	args: S::Args,
) -> (S::Output, Vec<Ran>) {
	ROUND_CALLS.take();

	let output = harness.call::<S>(rules.ptr().as_ptr(), slot, args);

	(output, ROUND_CALLS.take())
}

#[test]
fn round_ends_run_unless_the_hooks_refuse_them() {
	on_both(|harness| {
		let api = harness.api();
		let class = new_round_class();
		let mut rules = GameRules::of_class(class);

		// SAFETY: The mock class has the round methods at their slots, and is
		// leaked.
		let hooks =
			unsafe { api.install_rounds(class, tf2_binding(no_interfaces), every_callback()) }
				.unwrap();

		let win = |harness, rules: &mut GameRules, team| {
			round_call::<SetWinningTeam>(
				harness,
				rules,
				SET_WINNING_TEAM_SLOT,
				(
					team,
					WinReason::FlagCaptureLimit.to_raw(),
					false,
					true,
					true,
					true,
				),
			)
			.1
		};
		let red_win = Ran::Win(RoundWinEvent {
			winner: Some(ScoringTeam::Red),
			reason: Some(WinReason::FlagCaptureLimit),
			options: WinOptions {
				reset_map: false,
				switch_teams: true,
				add_score: false,
				final_round: true,
			},
		});

		END_ACTION.set(RoundEndAction::Allow);
		assert_eq!(
			win(harness, &mut rules, ScoringTeam::Red.to_raw()),
			[red_win, Ran::Game("win")]
		);

		// A win without a winner, for a reason unknown to the crate.
		assert_eq!(
			round_call::<SetWinningTeam>(
				harness,
				&mut rules,
				SET_WINNING_TEAM_SLOT,
				(TEAM_UNASSIGNED, 99, true, false, false, false),
			)
			.1,
			[
				Ran::Win(RoundWinEvent {
					winner: None,
					reason: None,
					options: WinOptions::default(),
				}),
				Ran::Game("win")
			]
		);

		// The game refuses a spectators' win itself.
		END_ACTION.set(RoundEndAction::Refuse);
		assert_eq!(win(harness, &mut rules, 1), [Ran::Game("win")]);
		assert_eq!(
			win(harness, &mut rules, ScoringTeam::Red.to_raw()),
			[red_win]
		);

		let stalemate = |harness, rules: &mut GameRules| {
			round_call::<SetStalemate>(
				harness,
				rules,
				SET_STALEMATE_SLOT,
				(StalemateReason::TimeLimit.to_raw(), false, true),
			)
			.1
		};
		let time_limit = Ran::Stalemate(StalemateEvent {
			reason: Some(StalemateReason::TimeLimit),
			options: StalemateOptions {
				reset_map: false,
				switch_teams: true,
			},
		});

		assert_eq!(stalemate(harness, &mut rules), [time_limit]);

		END_ACTION.set(RoundEndAction::Allow);
		assert_eq!(
			stalemate(harness, &mut rules),
			[time_limit, Ran::Game("stalemate")]
		);

		// Removed hooks run no callback.
		hooks.remove(api);
		END_ACTION.set(RoundEndAction::Refuse);
		assert_eq!(
			win(harness, &mut rules, ScoringTeam::Blue.to_raw()),
			[Ran::Game("win")]
		);
		assert_eq!(stalemate(harness, &mut rules), [Ran::Game("stalemate")]);
	});
}

#[test]
fn round_setup_and_start_are_reported_after_the_game() {
	on_both(|harness| {
		let api = harness.api();
		let class = new_round_class();
		let mut rules = GameRules::of_class(class);

		// SAFETY: As above.
		let _hooks =
			unsafe { api.install_rounds(class, tf2_binding(no_interfaces), every_callback()) }
				.unwrap();

		assert_eq!(
			round_call::<RoundSetup>(harness, &mut rules, SETUP_ON_ROUND_START_SLOT, ()).1,
			[Ran::Game("setup"), Ran::Setup]
		);
		assert_eq!(
			round_call::<RoundSetup>(harness, &mut rules, SETUP_ON_ROUND_RUNNING_SLOT, ()).1,
			[Ran::Game("running"), Ran::Running]
		);
	});
}

#[test]
fn captures_are_refused_where_the_hooks_say_so() {
	on_both(|harness| {
		let api = harness.api();
		let class = new_round_class();
		let mut rules = GameRules::of_class(class);
		let player = ptr::without_provenance_mut::<source_sdk_2013::sys::CBasePlayer>(0x9a7);

		// SAFETY: As above.
		let _hooks =
			unsafe { api.install_rounds(class, tf2_binding(no_interfaces), every_callback()) }
				.unwrap();

		let points = |harness, rules: &mut GameRules| {
			round_call::<PointsMayBeCaptured>(harness, rules, POINTS_MAY_BE_CAPTURED_SLOT, ())
		};
		let team = |harness, rules: &mut GameRules, team| {
			round_call::<TeamMayCapturePoint>(
				harness,
				rules,
				TEAM_MAY_CAPTURE_POINT_SLOT,
				(team, 4),
			)
		};
		let blue = Ran::Team(TeamCaptureCheck {
			team: Team::Blue,
			point: 4,
		});

		CAPTURE_ACTION.set(CaptureAction::Allow);
		assert_eq!(
			points(harness, &mut rules),
			(true, vec![Ran::Points, Ran::Game("points")])
		);
		assert_eq!(
			team(harness, &mut rules, Team::Blue.to_raw()),
			(true, vec![blue, Ran::Game("team")])
		);

		CAPTURE_ACTION.set(CaptureAction::Refuse);
		assert_eq!(points(harness, &mut rules), (false, vec![Ran::Points]));
		assert_eq!(
			team(harness, &mut rules, Team::Blue.to_raw()),
			(false, vec![blue])
		);

		// A team the crate does not know is left to the game.
		assert_eq!(
			team(harness, &mut rules, 9),
			(true, vec![Ran::Game("team")])
		);

		// A refusal clears the reason buffer the game would write.
		let mut reason = [b'?' as c_char; 8];
		let mut player_call = |harness, rules: &mut GameRules, player| {
			round_call::<PlayerMayCapturePoint>(
				harness,
				rules,
				PLAYER_MAY_CAPTURE_POINT_SLOT,
				(player, 2, reason.as_mut_ptr(), reason.len() as c_int),
			)
		};

		assert_eq!(
			player_call(harness, &mut rules, player),
			(false, vec![Ran::Player(0x9a7, 2)])
		);
		assert_eq!(reason[0], 0);

		CAPTURE_ACTION.set(CaptureAction::Allow);
		let mut player_call = |harness, rules: &mut GameRules, player| {
			round_call::<PlayerMayCapturePoint>(
				harness,
				rules,
				PLAYER_MAY_CAPTURE_POINT_SLOT,
				(player, 2, reason.as_mut_ptr(), reason.len() as c_int),
			)
		};

		assert_eq!(
			player_call(harness, &mut rules, player),
			(true, vec![Ran::Player(0x9a7, 2), Ran::Game("player")])
		);

		// Without a player, the game decides.
		assert_eq!(
			player_call(harness, &mut rules, ptr::null_mut()),
			(true, vec![Ran::Game("player")])
		);
		assert_eq!(reason[0], b'x' as c_char);
	});
}

#[test]
fn round_hooks_are_installed_all_at_once() {
	on_both(|harness| {
		let api = harness.api();
		let scope = ();
		let binding = tf2_binding(no_interfaces);
		// SAFETY: As for `the_game_rules_class_is_searched_for_unless_already_hooked`.
		let server = unsafe { binding.server(&scope) };

		// Without callbacks, nothing is searched for.
		let none = api
			.hook_rounds(server, binding, RoundCallbacks::default())
			.unwrap();

		assert!(none.hooks.is_empty());
		assert!(matches!(
			api.hook_rounds(server, binding, every_callback()),
			Err(RoundHookError::Target(GameRulesVtableError::NotFound))
		));

		let class = new_round_class();

		// SAFETY: As above.
		let hooks = unsafe { api.install_rounds(class, binding, every_callback()) }.unwrap();

		assert_eq!(hooks.hooks.len(), 11);

		// An installed hook is reported without searching the module again.
		assert!(matches!(
			api.hook_rounds(server, binding, every_callback()),
			Err(RoundHookError::Hook(HookError::AlreadyInstalled))
		));

		// Once removed, they can be installed again, a few at a time.
		hooks.remove(api);

		let callbacks = RoundCallbacks {
			setup: Some(|_| ran(Ran::Setup)),
			..RoundCallbacks::default()
		};
		// SAFETY: As above.
		let hooks = unsafe { api.install_rounds(class, binding, callbacks) }.unwrap();

		assert_eq!(hooks.hooks.len(), 1);
	});
}

#[test]
fn flags_blocks_and_cleanups_follow_the_hooks() {
	on_both(|harness| {
		let api = harness.api();
		let class = new_round_class();
		let mut rules = GameRules::of_class(class);
		let player = ptr::without_provenance_mut::<source_sdk_2013::sys::CBasePlayer>(0x9a7);
		let entity = ptr::without_provenance_mut::<source_sdk_2013::sys::CBaseEntity>(0xe47);
		let prop = c"prop_dynamic".as_ptr();

		// SAFETY: As above.
		let _hooks =
			unsafe { api.install_rounds(class, tf2_binding(no_interfaces), every_callback()) }
				.unwrap();

		let flags = |harness, rules: &mut GameRules| {
			round_call::<FlagsMayBeCapped>(harness, rules, FLAGS_MAY_BE_CAPPED_SLOT, ())
		};

		CAPTURE_ACTION.set(CaptureAction::Allow);
		assert_eq!(
			flags(harness, &mut rules),
			(true, vec![Ran::Flags, Ran::Game("flags")])
		);

		CAPTURE_ACTION.set(CaptureAction::Refuse);
		assert_eq!(flags(harness, &mut rules), (false, vec![Ran::Flags]));

		// Without an answer, the game decides, and gives its reason.
		let mut reason = [b'?' as c_char; 8];
		let mut block = |harness, rules: &mut GameRules| {
			round_call::<PlayerMayBlockPoint>(
				harness,
				rules,
				PLAYER_MAY_BLOCK_POINT_SLOT,
				(player, 3, reason.as_mut_ptr(), reason.len() as c_int),
			)
		};

		BLOCK.set(None);
		assert_eq!(
			block(harness, &mut rules),
			(false, vec![Ran::Block(0x9a7, 3), Ran::Game("block")])
		);

		BLOCK.set(Some(true));
		assert_eq!(
			block(harness, &mut rules),
			(true, vec![Ran::Block(0x9a7, 3)])
		);
		assert_eq!(reason[0], 0);

		let keep = |harness, rules: &mut GameRules, entity| {
			round_call::<RoundCleanupShouldIgnore>(
				harness,
				rules,
				ROUND_CLEANUP_SHOULD_IGNORE_SLOT,
				(entity,),
			)
		};

		KEEP_ACTION.set(CleanupKeepAction::Allow);
		assert_eq!(
			keep(harness, &mut rules, entity),
			(false, vec![Ran::Keep(0xe47), Ran::Game("keep")])
		);

		KEEP_ACTION.set(CleanupKeepAction::Keep);
		assert_eq!(
			keep(harness, &mut rules, entity),
			(true, vec![Ran::Keep(0xe47)])
		);
		assert_eq!(
			keep(harness, &mut rules, ptr::null_mut()),
			(false, vec![Ran::Game("keep")])
		);

		let create = |harness, rules: &mut GameRules, class_name| {
			round_call::<ShouldCreateEntity>(
				harness,
				rules,
				SHOULD_CREATE_ENTITY_SLOT,
				(class_name,),
			)
		};

		CREATE_ACTION.set(CleanupCreateAction::Allow);
		assert_eq!(
			create(harness, &mut rules, prop),
			(true, vec![Ran::Create(prop.addr()), Ran::Game("create")])
		);

		CREATE_ACTION.set(CleanupCreateAction::Skip);
		assert_eq!(
			create(harness, &mut rules, prop),
			(false, vec![Ran::Create(prop.addr())])
		);
		assert_eq!(
			create(harness, &mut rules, ptr::null()),
			(true, vec![Ran::Game("create")])
		);
	});
}
