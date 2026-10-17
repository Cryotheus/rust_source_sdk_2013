//! Tests of `crate::hooks::tf2::body_target`: hooks telling of where the game
//! aims at an entity, on a mock entity class, through the mock SourceHook and
//! KHook.

use super::*;
use crate::test_support::harness::on_both;
use crate::test_support::server::{no_interfaces, tf2_binding};
use source_sdk_2013::sys;
use source_sdk_2013::tf2::class_targets::ClassTarget;
use std::cell::RefCell;
use std::ffi::c_void;

#[cfg(target_os = "windows")]
use std::mem::MaybeUninit;

use std::ptr::{self, NonNull};

/// Where the game aims at the mock entity.
const CENTRE: Vector = Vector::new(1.0, 2.0, 3.0);

thread_local! {
	/// What ran during the calls since the last check, in order, with the
	/// entity asked, the point the aim is from, and whether it is noisy.
	static CALLS: RefCell<Vec<(&'static str, usize, Vector, bool)>> = const { RefCell::new(Vec::new()) };
}

/// The game's `BodyTarget`, which notes that it ran, and aims at [`CENTRE`].
#[cfg(not(target_os = "windows"))]
unsafe extern "C" fn game_body_target(
	this: *mut sys::CBaseEntity,
	from: *const sys::Vector,
	noisy: bool,
) -> sys::Vector {
	// SAFETY: The test passes a point.
	let from = Vector::from(unsafe { from.read() });

	CALLS.with_borrow_mut(|calls| calls.push(("game", this.addr(), from, noisy)));
	CENTRE.into()
}

/// The game's `BodyTarget`, which notes that it ran, and aims at [`CENTRE`].
#[cfg(target_os = "windows")]
unsafe extern "C" fn game_body_target(
	this: *mut sys::CBaseEntity,
	result: *mut sys::Vector,
	from: *const sys::Vector,
	noisy: bool,
) -> *mut sys::Vector {
	// SAFETY: The test passes a point, and storage for the result.
	let from = Vector::from(unsafe { from.read() });

	CALLS.with_borrow_mut(|calls| calls.push(("game", this.addr(), from, noisy)));

	// SAFETY: As above.
	unsafe { result.write(CENTRE.into()) };
	result
}

#[test]
fn hooks_tell_of_where_the_game_aims_at_an_entity() {
	/// An entity of a C++ class, as far as hooks know it.
	#[repr(C)]
	struct Mock {
		vtable: *mut *mut c_void,
	}

	on_both(|harness| {
		let api = harness.api();
		let slots = Vec::leak(vec![
			game_body_target as BodyTarget as *mut c_void;
			BODY_TARGET_SLOT + 1
		]);
		let mut entity = Mock {
			vtable: slots.as_mut_ptr(),
		};
		let this = ptr::from_mut(&mut entity).cast::<sys::CBaseEntity>();
		let hooks = api.hook_body_targets(tf2_binding(no_interfaces), on_body_target);
		// SAFETY: The test only calls the hooked slot, which holds a method of
		// its signature.
		let target =
			unsafe { ClassTarget::<BaseEntity>::from_raw(NonNull::new(entity.vtable).unwrap()) };
		let aim = |from: Vector, noisy| {
			let from = sys::Vector::from(from);

			CALLS.take();

			#[cfg(not(target_os = "windows"))]
			let point =
				harness.call::<BodyTarget>(this, BODY_TARGET_SLOT, (ptr::from_ref(&from), noisy));

			#[cfg(target_os = "windows")]
			let point = {
				let mut point = MaybeUninit::<sys::Vector>::uninit();
				let returned = harness.call::<BodyTarget>(
					this,
					BODY_TARGET_SLOT,
					(point.as_mut_ptr(), ptr::from_ref(&from), noisy),
				);

				assert_eq!(returned, point.as_mut_ptr());

				// SAFETY: The game constructed its point there.
				unsafe { point.assume_init() }
			};

			(Vector::from(point), CALLS.take())
		};
		let (from, addr) = (Vector::new(-4.0, 5.5, 64.0), this.addr());

		assert_eq!(
			aim(from, false),
			(CENTRE, vec![("game", addr, from, false)])
		);
		assert_eq!(hooks.cover(api, target), Ok(true));

		assert_eq!(
			aim(from, false),
			(
				CENTRE,
				vec![("hook", addr, from, false), ("game", addr, from, false)]
			)
		);

		assert_eq!(
			aim(from, true),
			(
				CENTRE,
				vec![("hook", addr, from, true), ("game", addr, from, true)]
			)
		);
	});
}

/// The callback, which notes the call.
fn on_body_target(_server: Server<'_>, entity: Entity<'_>, from: Vector, noisy: bool) {
	CALLS.with_borrow_mut(|calls| calls.push(("hook", entity.as_ptr().addr(), from, noisy)));
}
