//! Native explosion observation preserves trace/type arguments on both backends.
use super::*;

use crate::test_support::{
	harness::on_both,
	server::{no_interfaces, tf2_binding},
};

use source_sdk_2013::tf2::class_targets::ClassTarget;
use std::{cell::RefCell, ffi::c_void, ptr::NonNull};

thread_local! {
	static CALLS: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
}

#[repr(C)]
struct Grenade {
	vtable: *mut *mut c_void,
}

#[test]
fn actual_explosion_is_bracketed_without_rewriting_arguments_and_removal_stops_observation() {
	on_both(|harness| {
		let api = harness.api();
		let slots = Vec::leak(vec![
			explode as NativeExplode as *mut c_void;
			EXPLODE.index() + 1
		]);
		let mut entity = Grenade {
			vtable: slots.as_mut_ptr(),
		};
		// SAFETY: This fixture supplies the exact grenade explosion ABI.
		let target = unsafe {
			ClassTarget::<BaseEntity>::from_raw(NonNull::new(entity.vtable).expect("leaked vtable"))
		};
		// SAFETY: Only the mock grenade class with the matching slot is covered.
		let hooks = unsafe { api.hook_grenade_explosions(tf2_binding(no_interfaces), observe) };
		assert_eq!(hooks.cover(api, target), Ok(true));
		let mut trace: sys::trace_t = unsafe { std::mem::zeroed() };
		let this = std::ptr::from_mut(&mut entity).cast();
		CALLS.take();
		harness.call::<NativeExplode>(this, EXPLODE.index(), (&mut trace, 123));
		assert_eq!(CALLS.take(), ["pre", "native", "post"]);
		hooks.remove(api);
		harness.call::<NativeExplode>(this, EXPLODE.index(), (&mut trace, 123));
		assert_eq!(CALLS.take(), ["native"]);
	});
}

unsafe extern "C" fn explode(_: *mut sys::CBaseEntity, trace: *mut sys::trace_t, kind: c_int) {
	assert!(!trace.is_null());
	assert_eq!(kind, 123);
	CALLS.with_borrow_mut(|calls| calls.push("native"));
}

fn observe(_: Server<'_>, timing: HookTiming, _: Entity<'_>) {
	CALLS.with_borrow_mut(|calls| {
		calls.push(if timing == HookTiming::Pre {
			"pre"
		} else {
			"post"
		})
	});
}
