//! TF2 melee hit notifications after the game's `OnEntityHit`.
//!
//! A real traced hit reaches this method even for a teammate whose trace
//! attack does no damage. Hooks observe the native attack; they never alter
//! its arguments or return. Classes, pause, removal, unload and panic
//! containment follow [`crate::class_hooks`].

use crate::MetamodApi;
use crate::class_hooks::ClassHooks;
use crate::hook::{HookAction, HookTiming, VirtualFunction};
use source_sdk_2013::entities::Entity;
use source_sdk_2013::raw::tf2::melee::{MeleeHitFn as Hit, ON_ENTITY_HIT_SLOT};
use source_sdk_2013::tf2::class_targets::TfMeleeWeapon;
use source_sdk_2013::{Server, ServerBinding};
use std::ptr::NonNull;

/// The callback-scoped server, melee weapon and entity hit, after the game's
/// hit notification. The target may be a player, building or world entity.
/// Neither entity nor any engine reference may outlive the callback.
pub type MeleeHitFn = for<'s> fn(Server<'s>, Entity<'s>, Entity<'s>);

const ON_ENTITY_HIT: VirtualFunction<Hit> = VirtualFunction::new(ON_ENTITY_HIT_SLOT);

impl MetamodApi<'_> {
	/// Observes each `CTFWeaponBaseMelee::OnEntityHit` after the game's
	/// method, for the classes covered by the returned hooks. A teammate hit
	/// reaches it even when TF2 rejects damage to that teammate.
	///
	/// Cover each intended melee class with [`ClassHooks::cover`] or
	/// [`ClassHooks::cover_entity`]; covering a base does not cover derived
	/// classes. An earlier hook may skip the notification's body, but the
	/// callback still observes the hit: native trace damage precedes this
	/// notification. The callback must not immediately delete entities, as
	/// [`Server::new`] requires.
	///
	/// `binding` must describe the same running server as the class targets.
	pub fn hook_melee_hits(
		self,
		binding: ServerBinding,
		callback: MeleeHitFn,
	) -> ClassHooks<TfMeleeWeapon> {
		ClassHooks::new(
			binding,
			callback,
			ON_ENTITY_HIT,
			&[HookTiming::Post],
			|server, callback, weapon, call| {
				let (target, _info) = call.args();

				if let Some(target) = NonNull::new(target) {
					// SAFETY: OnEntityHit's argument is the live entity the
					// native swing hit. Its storage survives this invocation;
					// TF2 marks removed entities for deferred deletion.
					let target = unsafe { Entity::from_live(server, target) };
					callback(server, weapon, target);
				}

				HookAction::Ignore
			},
		)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::hook::{HookCall, HookTarget};
	use crate::test_support::harness::{Harness, on_both};
	use crate::test_support::server::{no_interfaces, tf2_binding};
	use source_sdk_2013::sys;
	use source_sdk_2013::tf2::class_targets::ClassTarget;
	use std::cell::{Cell, RefCell};
	use std::ffi::c_void;
	use std::mem::MaybeUninit;
	use std::ptr;

	thread_local! {
		static CALLS: RefCell<Vec<Seen>> = const { RefCell::new(Vec::new()) };
		static PANIC_NEXT: Cell<bool> = const { Cell::new(false) };
	}

	#[derive(Debug, Eq, PartialEq)]
	enum Seen {
		Native(usize, usize, usize),
		Callback(usize, usize),
	}

	/// A mock entity with only the primary vtable used by these tests.
	#[repr(C)]
	struct Weapon {
		vtable: *mut *mut c_void,
	}

	impl Weapon {
		fn new_class() -> Box<Self> {
			let slots = Vec::leak(vec![game_hit as Hit as *mut c_void; ON_ENTITY_HIT_SLOT + 1]);

			Self::of_class(slots.as_mut_ptr())
		}

		fn of_class(vtable: *mut *mut c_void) -> Box<Self> {
			Box::new(Self { vtable })
		}

		fn address(&self) -> usize {
			ptr::from_ref(self).addr()
		}

		fn ptr(&mut self) -> *mut sys::CBaseEntity {
			ptr::from_mut(self).cast()
		}

		fn target(&self) -> ClassTarget<'static, TfMeleeWeapon> {
			// SAFETY: The leaked primary vtable holds the exact hit signature
			// at its generated slot. These tests call no other entity method.
			unsafe { ClassTarget::from_raw(NonNull::new(self.vtable).unwrap()) }
		}
	}

	#[test]
	fn a_panicking_listener_does_not_unwind_or_disable_later_hits() {
		on_both(|harness| {
			let mut weapon = Weapon::new_class();
			let mut target = Weapon::new_class();
			let mut info = MaybeUninit::<sys::CTakeDamageInfo>::uninit();
			let info = info.as_mut_ptr();
			let target_ptr = target.ptr();
			let address = weapon.address();
			let _hooks = install(harness, &weapon);

			PANIC_NEXT.set(true);

			for _ in 0..2 {
				assert_eq!(
					hit(harness, &mut weapon, target_ptr, info),
					[
						Seen::Native(address, target_ptr.addr(), info.addr()),
						Seen::Callback(address, target_ptr.addr())
					]
				);
			}
		});
	}

	#[test]
	fn covered_hits_preserve_native_arguments_and_notify_once_afterward() {
		on_both(|harness| {
			let mut weapon = Weapon::new_class();
			let mut same_class = Weapon::of_class(weapon.vtable);
			let mut unrelated = Weapon::new_class();
			let mut target = Weapon::new_class();
			let mut info = MaybeUninit::<sys::CTakeDamageInfo>::uninit();
			let info = info.as_mut_ptr();
			let target_ptr = target.ptr();
			let hooks = install(harness, &weapon);

			assert_eq!(hooks.cover(harness.api(), weapon.target()), Ok(false));
			assert_eq!(hooks.len(), 1);

			for object in [&mut weapon, &mut same_class] {
				let address = object.address();

				assert_eq!(
					hit(harness, object, target_ptr, info),
					[
						Seen::Native(address, target_ptr.addr(), info.addr()),
						Seen::Callback(address, target_ptr.addr())
					]
				);
			}

			let address = unrelated.address();

			assert_eq!(
				hit(harness, &mut unrelated, target_ptr, info),
				[Seen::Native(address, target_ptr.addr(), info.addr())]
			);
		});
	}

	/// The native method records the exact pointers the engine supplied.
	unsafe extern "C" fn game_hit(
		weapon: *mut sys::CBaseEntity,
		target: *mut sys::CBaseEntity,
		info: *mut sys::CTakeDamageInfo,
	) {
		CALLS.with_borrow_mut(|calls| {
			calls.push(Seen::Native(weapon.addr(), target.addr(), info.addr()))
		});
	}

	fn hit(
		harness: &Harness,
		weapon: &mut Weapon,
		target: *mut sys::CBaseEntity,
		info: *mut sys::CTakeDamageInfo,
	) -> Vec<Seen> {
		CALLS.take();
		harness.call::<Hit>(weapon.ptr(), ON_ENTITY_HIT_SLOT, (target, info));
		CALLS.take()
	}

	fn install(harness: &Harness, weapon: &Weapon) -> ClassHooks<TfMeleeWeapon> {
		let api = harness.api();
		let hooks = api.hook_melee_hits(tf2_binding(no_interfaces), on_hit);

		assert_eq!(hooks.cover(api, weapon.target()), Ok(true));
		hooks
	}

	#[test]
	fn null_targets_run_the_native_method_without_notifying() {
		on_both(|harness| {
			let mut weapon = Weapon::new_class();
			let mut info = MaybeUninit::<sys::CTakeDamageInfo>::uninit();
			let info = info.as_mut_ptr();
			let _hooks = install(harness, &weapon);
			let address = weapon.address();

			assert_eq!(
				hit(harness, &mut weapon, ptr::null_mut(), info),
				[Seen::Native(address, 0, info.addr())]
			);
		});
	}

	fn on_hit(_server: Server<'_>, weapon: Entity<'_>, target: Entity<'_>) {
		CALLS.with_borrow_mut(|calls| {
			calls.push(Seen::Callback(
				weapon.as_ptr().addr(),
				target.as_ptr().addr(),
			))
		});

		if PANIC_NEXT.replace(false) {
			panic!("the melee listener failed, as this test intends");
		}
	}

	#[test]
	fn pause_removal_and_unload_leave_native_hits_running() {
		on_both(|harness| {
			let mut weapon = Weapon::new_class();
			let mut target = Weapon::new_class();
			let mut info = MaybeUninit::<sys::CTakeDamageInfo>::uninit();
			let info = info.as_mut_ptr();
			let target_ptr = target.ptr();
			let address = weapon.address();
			let hooks = install(harness, &weapon);

			harness.set_status(true, true, harness.generation);
			assert_eq!(
				hit(harness, &mut weapon, target_ptr, info),
				[Seen::Native(address, target_ptr.addr(), info.addr())]
			);

			harness.set_status(true, false, harness.generation);
			assert_eq!(
				hit(harness, &mut weapon, target_ptr, info),
				[
					Seen::Native(address, target_ptr.addr(), info.addr()),
					Seen::Callback(address, target_ptr.addr())
				]
			);

			hooks.remove(harness.api());
			assert_eq!(
				hit(harness, &mut weapon, target_ptr, info),
				[Seen::Native(address, target_ptr.addr(), info.addr())]
			);

			let _reinstalled = install(harness, &weapon);

			harness.set_status(false, false, harness.generation);
			assert_eq!(
				hit(harness, &mut weapon, target_ptr, info),
				[Seen::Native(address, target_ptr.addr(), info.addr())]
			);
		});
	}

	#[test]
	fn superseding_the_notification_keeps_the_post_observer() {
		fn skip(_call: &HookCall<'_, Hit>) -> HookAction<()> {
			HookAction::Supersede(())
		}

		on_both(|harness| {
			let mut weapon = Weapon::new_class();
			let mut target = Weapon::new_class();
			let mut info = MaybeUninit::<sys::CTakeDamageInfo>::uninit();
			let api = harness.api();

			// SAFETY: The mock's leaked primary vtable holds the hit signature
			// at its generated slot, and the hook outlives every test call.
			unsafe {
				api.add_hook(
					ON_ENTITY_HIT,
					HookTarget::vtable(NonNull::new(weapon.vtable).unwrap()),
					HookTiming::Pre,
					&skip,
				)
			}
			.unwrap();

			let _hooks = install(harness, &weapon);
			let expected = Seen::Callback(weapon.address(), target.address());

			// Only the notification's native body was skipped. Post observers
			// still run; the actual melee trace damage precedes this notification.
			assert_eq!(
				hit(harness, &mut weapon, target.ptr(), info.as_mut_ptr()),
				[expected]
			);
		});
	}
}
