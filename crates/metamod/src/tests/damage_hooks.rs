//! Tests of `crate::damage_hooks`: what the dispatch passes the game's
//! damage method.

use super::*;
use crate::test_support::server::{no_interfaces, tf2_binding};
use source_sdk_2013::tf2::damage::{DamageInfo, DamageType};
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
