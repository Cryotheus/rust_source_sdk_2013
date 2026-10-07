//! Tests of `crate::script_hooks`: pre hooks of `RunVScripts` on mock entity
//! classes, through the mock SourceHook and KHook.

use super::*;
use crate::test_support::harness::{Harness, on_both};
use crate::test_support::server::{no_interfaces, tf2_binding};
use std::cell::RefCell;

use source_sdk_2013::sys;

thread_local! {
	/// What ran during the calls since the last [`run_scripts`], in order,
	/// with the entity each was given.
	static CALLS: RefCell<Vec<(&'static str, usize)>> = const { RefCell::new(Vec::new()) };
}

/// An entity of a C++ class, as hooks know it.
#[repr(C)]
struct Mock {
	vtable: *mut *mut c_void,
}

impl Mock {
	/// An entity of a new class, whose vtable holds `run_vscripts` at
	/// [`RUN_VSCRIPTS_SLOT`].
	fn of_new_class(run_vscripts: RunVScripts) -> Box<Self> {
		let slots = Vec::leak(vec![run_vscripts as *mut c_void; RUN_VSCRIPTS_SLOT + 1]);

		Box::new(Self {
			vtable: slots.as_mut_ptr(),
		})
	}

	fn ptr(&mut self) -> NonNull<sys::CBaseEntity> {
		NonNull::from(self).cast()
	}

	fn vtable(&self) -> NonNull<*mut c_void> {
		NonNull::new(self.vtable).unwrap()
	}
}

/// Calls `entity`'s hooked `RunVScripts`, and returns what ran.
fn run_scripts(harness: &Harness, entity: &mut Mock) -> Vec<(&'static str, usize)> {
	CALLS.take();
	harness.call::<RunVScripts>(entity.ptr().as_ptr(), RUN_VSCRIPTS_SLOT, ());
	CALLS.take()
}

/// The game's `RunVScripts`, which notes the entity it ran on.
unsafe extern "C" fn game_run_vscripts(this: *mut sys::CBaseEntity) {
	CALLS.with_borrow_mut(|calls| calls.push(("game", this.addr())));
}

/// The callback, which notes the entity it saw.
fn on_scripts(_server: Server<'_>, entity: Entity<'_>) {
	CALLS.with_borrow_mut(|calls| calls.push(("callback", entity.as_ptr().addr())));
}

#[test]
fn callbacks_run_before_the_scripts() {
	on_both(|harness| {
		let api = harness.api();
		let mut script = Mock::of_new_class(game_run_vscripts);
		let mut other = Mock::of_new_class(game_run_vscripts);
		let mut blocked = Mock::of_new_class(game_run_vscripts);

		// SAFETY: The mock class has `RunVScripts` at the slot, and is leaked.
		unsafe { api.install_scripts(script.vtable(), tf2_binding(no_interfaces), on_scripts) }
			.unwrap();

		// A second hook of a class is refused, so that each call is seen once.
		assert!(matches!(
			// SAFETY: As above.
			unsafe { api.install_scripts(script.vtable(), tf2_binding(no_interfaces), on_scripts) },
			Err(ScriptHookError::Hook(HookError::AlreadyInstalled))
		));

		// The callback sees the entity, then the game runs its scripts.
		let address = script.ptr().as_ptr().addr();
		assert_eq!(
			run_scripts(harness, &mut script),
			[("callback", address), ("game", address)]
		);

		// Other classes are not hooked.
		let address = other.ptr().as_ptr().addr();
		assert_eq!(run_scripts(harness, &mut other), [("game", address)]);

		// A call an earlier hook superseded reaches neither the callback nor the
		// game.
		fn refuse(_call: &HookCall<'_, RunVScripts>) -> HookAction<()> {
			HookAction::Supersede(())
		}

		// SAFETY: As above.
		unsafe {
			api.add_hook(
				RUN_VSCRIPTS,
				HookTarget::class_of(blocked.ptr()),
				HookTiming::Pre,
				&refuse,
			)
			.unwrap();
			api.install_scripts(blocked.vtable(), tf2_binding(no_interfaces), on_scripts)
				.unwrap();
		}

		assert_eq!(run_scripts(harness, &mut blocked), []);
	});
}

#[test]
fn classes_hooked_by_vtable_cover_their_later_entities() {
	on_both(|harness| {
		let api = harness.api();
		let mut first = Mock::of_new_class(game_run_vscripts);

		// SAFETY: The mock class has `RunVScripts` at the slot, and is leaked.
		unsafe { api.install_scripts(first.vtable(), tf2_binding(no_interfaces), on_scripts) }
			.unwrap();

		// An entity created after the hook, sharing the class's vtable, is
		// covered too.
		let mut later = Box::new(Mock {
			vtable: first.vtable,
		});

		let address = later.ptr().as_ptr().addr();
		assert_eq!(
			run_scripts(harness, &mut later),
			[("callback", address), ("game", address)]
		);

		let address = first.ptr().as_ptr().addr();
		assert_eq!(
			run_scripts(harness, &mut first),
			[("callback", address), ("game", address)]
		);
	});
}
