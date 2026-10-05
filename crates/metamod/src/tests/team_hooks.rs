//! Tests of `crate::team_hooks`: pre hooks of `ChangeTeam` on mock entity
//! classes, through the mock SourceHook and KHook.

use super::*;
use crate::test_support::harness::{Harness, on_both};
use crate::test_support::server::{no_interfaces, tf2_binding};
use std::cell::RefCell;

thread_local! {
	/// What ran during the calls since the last [`change_team`], in order,
	/// with the team each was given.
	static CALLS: RefCell<Vec<(&'static str, c_int)>> = const { RefCell::new(Vec::new()) };
}

/// An entity of a C++ class, as hooks know it.
#[repr(C)]
struct Mock {
	vtable: *mut *mut c_void,
}

impl Mock {
	/// An entity of a new class, whose vtable holds `change_team` at
	/// [`CHANGE_TEAM_SLOT`].
	fn of_new_class(change_team: ChangeTeam) -> Box<Self> {
		let slots = Vec::leak(vec![change_team as *mut c_void; CHANGE_TEAM_SLOT + 1]);

		Box::new(Self {
			vtable: slots.as_mut_ptr(),
		})
	}

	fn ptr(&mut self) -> NonNull<sys::CBaseEntity> {
		NonNull::from(self).cast()
	}
}

/// Calls `entity`'s hooked `ChangeTeam` with `team`, and returns what ran.
fn change_team(harness: &Harness, entity: &mut Mock, team: c_int) -> Vec<(&'static str, c_int)> {
	CALLS.take();
	harness.call::<ChangeTeam>(entity.ptr().as_ptr(), CHANGE_TEAM_SLOT, (team,));
	CALLS.take()
}

/// The game's `ChangeTeam`, which notes the team it was given.
unsafe extern "C" fn game_change_team(_this: *mut sys::CBaseEntity, team: c_int) {
	CALLS.with_borrow_mut(|calls| calls.push(("game", team)));
}

/// The callback, which notes the team, keeps entities off team 1, moves
/// teams 2 and 3 to 0, and keeps team 0 as it is.
fn on_change(_server: Server<'_>, _entity: Entity<'_>, team: c_int) -> TeamChange {
	CALLS.with_borrow_mut(|calls| calls.push(("callback", team)));

	match team {
		1 => TeamChange::Refuse,
		2 | 3 => TeamChange::Replace(0),
		0 => TeamChange::Replace(0),
		_ => TeamChange::Allow,
	}
}

#[test]
fn team_changes_are_allowed_replaced_or_refused() {
	on_both(|harness| {
		let api = harness.api();
		let mut room = Mock::of_new_class(game_change_team);
		let mut regenerate = Mock::of_new_class(game_change_team);
		let mut blocked = Mock::of_new_class(game_change_team);

		for object in [room.ptr(), regenerate.ptr()] {
			// SAFETY: The mock classes have `ChangeTeam` at the slot, and are
			// leaked.
			unsafe { api.install_team(object, tf2_binding(no_interfaces), on_change) }.unwrap();
		}

		// A second hook of a class is refused, so that each change is decided
		// once.
		assert!(matches!(
			// SAFETY: As above.
			unsafe { api.install_team(room.ptr(), tf2_binding(no_interfaces), on_change) },
			Err(TeamHookError::Hook(HookError::AlreadyInstalled))
		));

		// A replaced team reaches the game's method instead of the asked one.
		assert_eq!(
			change_team(harness, &mut room, 2),
			[("callback", 2), ("game", 0)]
		);
		assert_eq!(
			change_team(harness, &mut regenerate, 3),
			[("callback", 3), ("game", 0)]
		);

		// Replacing a team with itself lets the call proceed.
		assert_eq!(
			change_team(harness, &mut room, 0),
			[("callback", 0), ("game", 0)]
		);
		assert_eq!(
			change_team(harness, &mut room, 5),
			[("callback", 5), ("game", 5)]
		);
		assert_eq!(change_team(harness, &mut room, 1), [("callback", 1)]);

		// A change an earlier hook superseded reaches neither the callback nor
		// the game.
		fn refuse(_call: &HookCall<'_, ChangeTeam>) -> HookAction<()> {
			HookAction::Supersede(())
		}

		// SAFETY: As above.
		unsafe {
			api.add_hook(
				CHANGE_TEAM,
				HookTarget::class_of(blocked.ptr()),
				HookTiming::Pre,
				&refuse,
			)
			.unwrap();
			api.install_team(blocked.ptr(), tf2_binding(no_interfaces), on_change)
				.unwrap();
		}

		assert_eq!(change_team(harness, &mut blocked, 2), []);
	});
}
