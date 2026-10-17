//! TF2 melee hit notifications after the game's `OnEntityHit`.
//!
//! A real traced hit reaches this method even for a teammate whose trace
//! attack does no damage. Hooks observe the native attack; they never alter
//! its arguments or return. Classes, pause, removal, unload and panic
//! containment follow [`crate::hooks::tf2::class`].

use crate::MetamodApi;
use crate::hook::{HookAction, HookTiming, VirtualFunction};
use crate::hooks::tf2::class::ClassHooks;
use source_sdk_2013::entities::Entity;

use source_sdk_2013::raw::tf2::melee::{
	MeleeHitFn as Hit, ON_ENTITY_HIT_SLOT, WEAPON_SOUND_SLOT, WeaponSoundFn, WeaponSoundKind,
};

use source_sdk_2013::raw::vcall;
use source_sdk_2013::tf2::class_targets::TfMeleeWeapon;
use source_sdk_2013::{Server, ServerBinding, sys};
use std::cell::Cell;
use std::ptr::NonNull;

/// The callback-scoped server, melee weapon and entity hit, after the game's
/// hit notification. The target may be a player, building or world entity.
/// Neither entity nor any engine reference may outlive the callback.
pub type MeleeHitFn = for<'s> fn(Server<'s>, Entity<'s>, Entity<'s>);

/// The callback-scoped server, the melee weapon, and the kind of sound
/// (`WeaponSound_t`, such as
/// [`MELEE_HIT_WORLD_SOUND`](source_sdk_2013::raw::tf2::melee::MELEE_HIT_WORLD_SOUND))
/// it is about to play, deciding what it plays. The weapon may be traced in
/// the callback, as a swing is when the sound plays.
pub type MeleeSoundFn = for<'s> fn(Server<'s>, Entity<'s>, WeaponSoundKind) -> MeleeSoundAction;

/// What a melee weapon plays for a kind of sound.
#[must_use]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum MeleeSoundAction {
	/// Plays what the game chose.
	#[default]
	Continue,

	/// Plays this kind of sound instead, from the weapon's script, to the
	/// players the game would send the first one to. The callback is not asked
	/// about it.
	Replace(WeaponSoundKind),

	/// Plays no sound at all, to the players the game would send it to.
	Silence,
}

thread_local! {
	/// Whether a replacement sound is playing, which the hook lets through.
	static REPLACING: Cell<bool> = const { Cell::new(false) };
}

const ON_ENTITY_HIT: VirtualFunction<Hit> = VirtualFunction::new(ON_ENTITY_HIT_SLOT);
const WEAPON_SOUND: VirtualFunction<WeaponSoundFn> = VirtualFunction::new(WEAPON_SOUND_SLOT);

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

	/// Runs `callback` before each `WeaponSound` of the melee weapons of the
	/// classes the returned hooks cover, which are none until
	/// [`ClassHooks::cover`] covers them: as a weapon plays the sound of its
	/// swing, a miss or a hit.
	///
	/// TF2 plays a swing's hit sound when the swing lands, from the kind of
	/// entity it found: the flesh sound for a player, and the sound of hitting
	/// the world for anything else. [`MeleeSoundAction::Replace`] plays another
	/// kind in its place, and [`MeleeSoundAction::Silence`] plays none, as the
	/// swing sound, played as the swing starts, may need to. The swinging player's client predicts its own swing's
	/// sounds, which this does not change; the game sends the sound to the other
	/// players.
	///
	/// Cover each intended melee class with [`ClassHooks::cover`] or
	/// [`ClassHooks::cover_entity`]; covering a base does not cover derived
	/// classes. `binding` must describe the running server. The callback must not
	/// delete entities immediately, as [`Server::new`] requires.
	pub fn hook_melee_sounds(
		self,
		binding: ServerBinding,
		callback: MeleeSoundFn,
	) -> ClassHooks<TfMeleeWeapon> {
		ClassHooks::new(
			binding,
			callback,
			WEAPON_SOUND,
			&[HookTiming::Pre],
			|server, callback, weapon, call| {
				if call.superseded() == Some(true) || REPLACING.get() {
					return HookAction::Ignore;
				}

				let (kind, time) = call.args();

				match callback(server, weapon, kind) {
					MeleeSoundAction::Replace(other) if other != kind => {
						let this = weapon.as_ptr().cast::<sys::CTFWeaponBaseMelee>();

						REPLACING.set(true);

						// SAFETY: The hooked method is the melee weapons' own
						// `WeaponSound`, and the weapon is live for the call.
						// The native method only plays the sound.
						unsafe {
							vcall!(this as sys::CTFWeaponBaseMelee__bindgen_vtable => CTFWeaponBaseMelee_WeaponSound(other, time))
						};

						REPLACING.set(false);

						HookAction::Supersede(())
					}

					MeleeSoundAction::Silence => HookAction::Supersede(()),
					_ => HookAction::Ignore,
				}
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
	use source_sdk_2013::raw::tf2::melee::BURST_SOUND;
	use source_sdk_2013::raw::tf2::melee::MELEE_HIT_SOUND;
	use source_sdk_2013::raw::tf2::melee::MELEE_HIT_WORLD_SOUND;
	use source_sdk_2013::raw::tf2::melee::MELEE_MISS_SOUND;
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

	thread_local! {
		static SOUNDS: RefCell<Vec<Sound>> = const { RefCell::new(Vec::new()) };
	}

	#[derive(Debug, PartialEq)]
	enum Sound {
		Asked(WeaponSoundKind),
		Native(WeaponSoundKind, f32),
	}

	#[test]
	fn a_replaced_sound_plays_once_in_place_of_the_game_s_and_is_not_asked_about() {
		on_both(|harness| {
			let slots = Vec::leak(vec![game_hit as Hit as *mut c_void; WEAPON_SOUND_SLOT + 1]);

			slots[WEAPON_SOUND_SLOT] = game_sound as WeaponSoundFn as *mut c_void;

			let mut weapon = Weapon::of_class(slots.as_mut_ptr());
			let api = harness.api();
			let hooks = api.hook_melee_sounds(tf2_binding(no_interfaces), on_sound);

			assert_eq!(hooks.cover(api, weapon.target()), Ok(true));

			let mut play = |kind: WeaponSoundKind| {
				SOUNDS.take();
				harness.call::<WeaponSoundFn>(weapon.ptr(), WEAPON_SOUND_SLOT, (kind, 0.5));
				SOUNDS.take()
			};

			assert_eq!(
				play(MELEE_HIT_WORLD_SOUND),
				[
					Sound::Asked(MELEE_HIT_WORLD_SOUND),
					Sound::Native(MELEE_HIT_SOUND, 0.5)
				]
			);
			assert_eq!(
				play(MELEE_MISS_SOUND),
				[
					Sound::Asked(MELEE_MISS_SOUND),
					Sound::Native(MELEE_MISS_SOUND, 0.5)
				]
			);

			// A silenced sound is asked about and never played.
			assert_eq!(play(BURST_SOUND), [Sound::Asked(BURST_SOUND)]);
		});
	}

	/// The native method records the kind and time it plays.
	unsafe extern "C" fn game_sound(
		_weapon: *mut sys::CBaseEntity,
		kind: WeaponSoundKind,
		time: f32,
	) {
		SOUNDS.with_borrow_mut(|sounds| sounds.push(Sound::Native(kind, time)));
	}

	/// Plays the flesh hit for what the game would play the world's for.
	fn on_sound(
		_server: Server<'_>,
		_weapon: Entity<'_>,
		kind: WeaponSoundKind,
	) -> MeleeSoundAction {
		SOUNDS.with_borrow_mut(|sounds| sounds.push(Sound::Asked(kind)));

		match kind {
			MELEE_HIT_WORLD_SOUND => MeleeSoundAction::Replace(MELEE_HIT_SOUND),
			BURST_SOUND => MeleeSoundAction::Silence,
			_ => MeleeSoundAction::Continue,
		}
	}
}
