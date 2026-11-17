//! Tuning a stock dispenser's heal rate and reach, building by building.
//! `CObjectDispenser::GetHealRate` and `GetDispenserRadius` take TF2's values
//! (by level, and the builder's `mult_dispenser_rate` and
//! `mult_dispenser_radius` attributes); a post hook on each hands the callback
//! the dispenser and the value TF2 found, and returns what it answers instead.
//!
//! TF2 reads the heal rate as a dispenser starts healing a player, and keeps
//! it for as long as it heals them, so a new rate reaches only players it
//! starts healing after. It sizes the touch trigger by the radius as it goes
//! active, and again from its dispense think, every 0.1 seconds, whenever the
//! radius differs from the last it used, so a new radius takes effect at once.
//!
//! Only `obj_dispenser`'s table is hooked: payload carts' dispensers keep
//! TF2's values. Dropping the handle does not remove the hooks; pausing or
//! unloading the plugin stops their routing.

use crate::MetamodApi;

use crate::hook::{
	Handler, HookAction, HookCall, HookError, HookId, HookTarget, HookTiming, VirtualFunction,
};

use source_sdk_2013::entities::Entity;

use source_sdk_2013::raw::tf2::dispenser_hooks::{
	DispenserFloatFn, GET_DISPENSER_RADIUS_SLOT, GET_HEAL_RATE_SLOT,
};

use source_sdk_2013::tf2::buildings::{BuildingClass, BuildingVtableError};
use source_sdk_2013::tf2::class_targets::ClassTargets;
use source_sdk_2013::{Game, Server, ServerBinding};
use std::cell::Cell;
use std::ffi::c_void;
use std::marker::PhantomData;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::NonNull;
use std::rc::Rc;

/// Answers the value a dispenser uses for `value`, given the dispenser and
/// the value TF2 found. Answering the given value leaves it as it is.
pub type DispenserTuningFn = for<'s> fn(Server<'s>, Entity<'s>, DispenserValue, f32) -> f32;

const GET_DISPENSER_RADIUS: VirtualFunction<DispenserFloatFn> =
	VirtualFunction::new(GET_DISPENSER_RADIUS_SLOT);

const GET_HEAL_RATE: VirtualFunction<DispenserFloatFn> = VirtualFunction::new(GET_HEAL_RATE_SLOT);

static HEAL_RATE: TuningRoute = TuningRoute::new(DispenserValue::HealRate);
static RADIUS: TuningRoute = TuningRoute::new(DispenserValue::Radius);

#[derive(Debug, thiserror::Error)]
pub enum DispenserTuningHookError {
	#[error(transparent)]
	Hook(#[from] HookError),
	#[error(transparent)]
	Target(#[from] BuildingVtableError),
}

/// The registered stock-dispenser heal rate and radius hooks.
#[must_use = "retain the hooks to support explicitly removing them"]
#[derive(Debug)]
pub struct DispenserTuningHooks {
	hooks: [HookId; 2],
	_not_thread_safe: PhantomData<Rc<()>>,
}

impl DispenserTuningHooks {
	/// Removes this generation's hooks, without clearing newer routes.
	pub fn remove(self, api: MetamodApi<'_>) {
		for (route, hook) in [&HEAL_RATE, &RADIUS].into_iter().zip(self.hooks) {
			api.remove_hook(hook);

			if route.state.get().is_some_and(|routed| routed.hook == hook) {
				route.state.set(None);
			}
		}
	}
}

/// A value of a dispenser the callback tunes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DispenserValue {
	/// The health a second it heals each player (`GetHealRate`): TF2's
	/// `g_flDispenserHealRates` by level, 10, 15 and 20.
	#[doc(alias("GetHealRate"))]
	HealRate,

	/// The half-width of the cube of its touch trigger, which finds whom it
	/// serves (`GetDispenserRadius`): 64 units in TF2.
	#[doc(alias("GetDispenserRadius"))]
	Radius,
}

#[derive(Clone, Copy)]
struct Routed {
	binding: ServerBinding,
	callback: DispenserTuningFn,
	hook: HookId,
}

struct TuningRoute {
	value: DispenserValue,
	state: Cell<Option<Routed>>,
}

impl TuningRoute {
	const fn new(value: DispenserValue) -> Self {
		Self {
			value,
			state: Cell::new(None),
		}
	}
}

impl Handler<DispenserFloatFn> for TuningRoute {
	fn call(&self, call: &HookCall<'_, DispenserFloatFn>) -> HookAction<f32> {
		let (Some(route), Some(entity), Some(native)) = (
			self.state.get(),
			NonNull::new(call.this()),
			call.return_value(),
		) else {
			return HookAction::Ignore;
		};
		let scope = ();
		// SAFETY: The hook runs during a live stock-dispenser method on the
		// main thread; registration bound it to this server's generation.
		let server = unsafe { route.binding.server(&scope) };
		// SAFETY: The native method's receiver is the live stock dispenser.
		let entity = unsafe { Entity::from_live(server, entity) };

		match catch_unwind(AssertUnwindSafe(|| {
			(route.callback)(server, entity, self.value, native)
		})) {
			Ok(tuned) if tuned.is_finite() && tuned.to_bits() != native.to_bits() => {
				HookAction::Override(tuned)
			}

			Ok(_) | Err(_) => HookAction::Ignore,
		}
	}
}

// SAFETY: Installation and dispatch are restricted to the server's main thread.
// The route is copied before calling user code; no Cell borrow crosses it.
unsafe impl Sync for TuningRoute {}

impl MetamodApi<'_> {
	/// Hooks stock dispensers' heal rate and radius getters after TF2's own,
	/// so that `callback` answers each value they return. A callback that
	/// panics, or answers no finite number, leaves TF2's value.
	///
	/// # Safety
	///
	/// `targets` and `binding` must describe the same live TF2 module. Its
	/// CObjectDispenser primary vtable must have the generated no-argument
	/// float GetHealRate and GetDispenserRadius methods and remain loaded until
	/// plugin unload. RTTI and the generated signatures do not independently
	/// prove binary identity.
	pub unsafe fn hook_dispenser_tuning(
		self,
		targets: &ClassTargets<'_>,
		binding: ServerBinding,
		callback: DispenserTuningFn,
	) -> Result<DispenserTuningHooks, DispenserTuningHookError> {
		let vtable = targets
			.find_all(
				&[BuildingClass::Dispenser.name()],
				GET_DISPENSER_RADIUS_SLOT.max(GET_HEAL_RATE_SLOT),
			)
			.pop()
			.flatten()
			.ok_or(BuildingVtableError::NotFound(BuildingClass::Dispenser))?;

		// SAFETY: The caller supplies the live module and generated ABI above.
		unsafe { self.install_dispenser_tuning(vtable.as_ptr(), binding, callback) }
	}

	/// # Safety
	///
	/// The table remains live and contains [`DispenserFloatFn`] at both named
	/// generated slots, on stock dispenser objects with a primary entity base.
	unsafe fn install_dispenser_tuning(
		self,
		vtable: NonNull<*mut c_void>,
		binding: ServerBinding,
		callback: DispenserTuningFn,
	) -> Result<DispenserTuningHooks, DispenserTuningHookError> {
		if binding.game() != Game::TeamFortress2 {
			return Err(BuildingVtableError::WrongGame.into());
		}

		if [&HEAL_RATE, &RADIUS].iter().any(|route| {
			route
				.state
				.get()
				.is_some_and(|routed| self.has_hook(routed.hook))
		}) {
			return Err(HookError::AlreadyInstalled.into());
		}

		// SAFETY: As promised by the caller for this exact generated slot.
		let heal_rate = unsafe {
			self.add_hook(
				GET_HEAL_RATE,
				HookTarget::vtable(vtable),
				HookTiming::Post,
				&HEAL_RATE,
			)
		}?;

		// SAFETY: As above.
		let radius = match unsafe {
			self.add_hook(
				GET_DISPENSER_RADIUS,
				HookTarget::vtable(vtable),
				HookTiming::Post,
				&RADIUS,
			)
		} {
			Ok(hook) => hook,

			Err(error) => {
				self.remove_hook(heal_rate);
				return Err(error.into());
			}
		};

		for (route, hook) in [(&HEAL_RATE, heal_rate), (&RADIUS, radius)] {
			route.state.set(Some(Routed {
				binding,
				callback,
				hook,
			}));
		}

		Ok(DispenserTuningHooks {
			hooks: [heal_rate, radius],
			_not_thread_safe: PhantomData,
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::test_support::harness::{Harness, on_both};
	use crate::test_support::server::{no_interfaces, tf2_binding};
	use source_sdk_2013::sys;

	thread_local! {
		static TUNED: Cell<usize> = const { Cell::new(0) };
		static PANIC: Cell<bool> = const { Cell::new(false) };
	}

	#[repr(C)]
	struct NativeDispenser {
		vtable: *mut *mut c_void,
	}

	impl NativeDispenser {
		fn ptr(&mut self) -> *mut sys::CBaseEntity {
			std::ptr::from_mut(self).cast()
		}
	}

	fn callback(_: Server<'_>, entity: Entity<'_>, value: DispenserValue, native: f32) -> f32 {
		assert!(!PANIC.get(), "fixture callback panic");

		if entity.as_ptr().addr() != TUNED.get() {
			return native;
		}

		match value {
			DispenserValue::HealRate => native * 1.5,
			DispenserValue::Radius => native * 3.0,
		}
	}

	unsafe extern "C" fn native_heal_rate(_: *mut sys::CBaseEntity) -> f32 {
		10.0
	}

	unsafe extern "C" fn native_radius(_: *mut sys::CBaseEntity) -> f32 {
		64.0
	}

	#[test]
	fn only_the_tuned_dispenser_changes_and_a_panic_keeps_tf2s_values() {
		on_both(|harness| {
			let vtable = table();
			// SAFETY: The leaked mock table holds the generated float signature at both slots.
			let hooks = unsafe {
				harness
					.api()
					.install_dispenser_tuning(vtable, tf2_binding(no_interfaces), callback)
			}
			.unwrap();
			let mut tuned = NativeDispenser {
				vtable: vtable.as_ptr(),
			};
			let mut stock = NativeDispenser {
				vtable: vtable.as_ptr(),
			};

			TUNED.set(tuned.ptr().addr());
			PANIC.set(false);

			assert_eq!(values(harness, &mut tuned), (15.0, 192.0));
			assert_eq!(values(harness, &mut stock), (10.0, 64.0));

			PANIC.set(true);
			assert_eq!(values(harness, &mut tuned), (10.0, 64.0));
			PANIC.set(false);

			hooks.remove(harness.api());
			assert_eq!(values(harness, &mut tuned), (10.0, 64.0));
		});
	}

	fn table() -> NonNull<*mut c_void> {
		let mut slots = vec![
			native_heal_rate as *mut c_void;
			GET_HEAL_RATE_SLOT.max(GET_DISPENSER_RADIUS_SLOT) + 1
		];

		slots[GET_DISPENSER_RADIUS_SLOT] = native_radius as *mut c_void;

		NonNull::new(Vec::leak(slots).as_mut_ptr()).unwrap()
	}

	fn values(harness: &Harness, dispenser: &mut NativeDispenser) -> (f32, f32) {
		(
			harness.call::<DispenserFloatFn>(dispenser.ptr(), GET_HEAL_RATE_SLOT, ()),
			harness.call::<DispenserFloatFn>(dispenser.ptr(), GET_DISPENSER_RADIUS_SLOT, ()),
		)
	}
}
