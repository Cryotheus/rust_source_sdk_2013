//! Tests of `crate::damage_hooks`: what the dispatch passes the game's
//! damage method.

use super::*;
use crate::test_support::harness::{Harness, on_both};
use crate::test_support::server::{no_interfaces, tf2_binding};
use source_sdk_2013::tf2::damage::{DamageInfo, DamageType};
use std::cell::RefCell;
use std::mem::MaybeUninit;

thread_local! {
	/// How many times [`original`] ran since the last [`probe`].
	static CALLS: Cell<usize> = const { Cell::new(0) };
}

#[test]
fn changed_damage_reaches_original_once_without_overwriting_source() {
	let (action, calls) = probe(|_, _, event| {
		event.info.set_amount(37.0);
		DamageAction::Apply
	});
	assert_eq!(action, HookAction::Supersede(37));
	assert_eq!(calls, 1);
}

unsafe extern "C" fn original(
	_: *mut sys::CBaseEntity,
	info: *const sys::CTakeDamageInfo,
) -> c_int {
	CALLS.with(|calls| calls.set(calls.get() + 1));
	// SAFETY: The test dispatch supplies a constructed local damage record.
	unsafe { (&raw const (*info).m_flDamage).read() as c_int }
}

/// Dispatches 11 bullet damage to `callback`, with [`original`] as the game's
/// method. Returns the hook's action and how many times the method ran.
fn probe(callback: DamageFn) -> (HookAction<c_int>, usize) {
	CALLS.with(|calls| calls.set(0));
	let scope = ();
	// SAFETY: These tests exercise no engine interface; their only virtual
	// call is the explicit mock original below, within this stack scope.
	let server = unsafe { tf2_binding(no_interfaces).server(&scope) };
	let mut victim = MaybeUninit::<sys::CBaseEntity>::zeroed();
	let info = DamageInfo::new(11.0, DamageType::BULLET);
	// SAFETY: Both allocations live through dispatch. The mock original
	// and callbacks never dereference the mock entity or call its methods.
	let action = unsafe {
		dispatch(
			server,
			DamageStage::Incoming,
			original,
			callback,
			NonNull::new(victim.as_mut_ptr()).unwrap(),
			NonNull::new(info.as_ptr().cast_mut()).unwrap(),
		)
	};
	assert_eq!(
		info.amount(),
		11.0,
		"a const source record must never be overwritten"
	);
	(action, CALLS.with(Cell::get))
}

thread_local! {
	/// The damage [`on_taken`] saw since the last [`take_damage`], with its
	/// stage and what the call returned.
	static TAKEN: RefCell<Vec<(DamageStage, f32, c_int)>> = const { RefCell::new(Vec::new()) };
}

/// A victim of a C++ class, as far as hooks know it.
#[repr(C)]
struct Mock {
	vtable: *mut *mut c_void,
}

impl Mock {
	/// A victim of a new class, whose vtable holds [`original`] in both
	/// stages' slots.
	fn of_new_class() -> Box<Self> {
		let slots = Vec::leak(vec![
			original as TakeDamage as *mut c_void;
			ON_TAKE_DAMAGE_SLOT.max(ON_TAKE_DAMAGE_ALIVE_SLOT) + 1
		]);

		Box::new(Self {
			vtable: slots.as_mut_ptr(),
		})
	}

	fn ptr(&mut self) -> *mut sys::CBaseEntity {
		std::ptr::from_mut(self).cast()
	}

	/// The mock's class, as a class of TF2 players.
	fn target(&self) -> ClassTarget<'static, TfPlayer> {
		// SAFETY: The tests only call the stages' slots, which hold a method of
		// their signature.
		unsafe { ClassTarget::from_raw(NonNull::new(self.vtable).unwrap()) }
	}
}

/// Blocks the damage.
fn block(_: Server<'_>, _: DamageStage, _: &mut DamageEvent<'_>) -> DamageAction {
	DamageAction::Block
}

/// Notes the damage taken.
fn on_taken(_: Server<'_>, stage: DamageStage, event: &DamageEvent<'_>, dealt: c_int) {
	TAKEN.with_borrow_mut(|taken| taken.push((stage, event.info.amount(), dealt)));
}

/// Calls `victim`'s method of `stage` with 11 bullet damage. Returns what it
/// returned, how many times [`original`] ran, and the damage [`on_taken`] saw.
fn take_damage(
	harness: &Harness,
	victim: &mut Mock,
	stage: DamageStage,
) -> (c_int, usize, Vec<(DamageStage, f32, c_int)>) {
	CALLS.with(|calls| calls.set(0));
	TAKEN.take();
	let info = DamageInfo::new(11.0, DamageType::BULLET);
	let result =
		harness.call::<TakeDamage>(victim.ptr(), stage.function().index(), (info.as_ptr(),));

	(result, CALLS.with(Cell::get), TAKEN.take())
}

#[test]
fn class_hooks_see_the_damage_taken_after_the_game() {
	on_both(|harness| {
		let api = harness.api();
		let mut player = Mock::of_new_class();
		let binding = tf2_binding(no_interfaces);

		for stage in [DamageStage::Incoming, DamageStage::Alive] {
			api.hook_player_class_damage_taken(player.target(), stage, binding, on_taken)
				.unwrap();

			assert_eq!(
				take_damage(harness, &mut player, stage),
				(11, 1, vec![(stage, 11.0, 11)])
			);
		}

		// The same callback hooks a class's stage once.
		assert!(matches!(
			api.hook_player_class_damage_taken(
				player.target(),
				DamageStage::Alive,
				binding,
				on_taken
			),
			Err(DamageHookError::Hook(HookError::AlreadyInstalled))
		));

		// Blocked damage is taken as what the blocking hook returned.
		api.hook_player_class_damage(player.target(), DamageStage::Alive, binding, block)
			.unwrap();

		assert_eq!(
			take_damage(harness, &mut player, DamageStage::Alive),
			(0, 0, vec![(DamageStage::Alive, 11.0, 0)])
		);

		// Entity classes have the incoming stage.
		let mut prop = Mock::of_new_class();

		api.hook_entity_class_damage(prop.target().upcast::<BaseEntity>(), binding, block)
			.unwrap();
		api.hook_entity_class_damage_taken(prop.target().upcast::<BaseEntity>(), binding, on_taken)
			.unwrap();

		assert_eq!(
			take_damage(harness, &mut prop, DamageStage::Incoming),
			(0, 0, vec![(DamageStage::Incoming, 11.0, 0)])
		);
	});
}
