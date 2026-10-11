//! Timed construction workers can contribute to stock dispensers through
//! TF2's native `CBaseObject::GetConstructionMultiplier`.
//!
//! The post hook retains the native constructor and additive builder terms,
//! then adds the caller's worker contribution. TF2 reads the result for both
//! `DeterminePlaybackRate` and `Construct`; the hook never writes animation,
//! construction clocks, health, or completion state. Reverse construction
//! and nonfinite native results are retained.
//!
//! Only the stock `CObjectDispenser` table is covered. Installation consumes
//! one shared SourceHook manager. KHook may miss calls the game has inlined
//! or devirtualized; observe a callback on each building before admitting a
//! worker rather than treating registration as an activation witness.

use crate::MetamodApi;

use crate::hook::{
	Handler, HookAction, HookCall, HookError, HookId, HookTarget, HookTiming, VirtualFunction,
};

use source_sdk_2013::entities::Entity;

use source_sdk_2013::raw::tf2::buildings::{
	GET_CONSTRUCTION_MULTIPLIER_SLOT, GetConstructionMultiplierFn,
};

use source_sdk_2013::tf2::buildings::{BuildingClass, BuildingVtableError};
use source_sdk_2013::tf2::class_targets::ClassTargets;
use source_sdk_2013::{Game, Server, ServerBinding};
use std::cell::Cell;
use std::ffi::c_void;
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::rc::Rc;

/// The current native multiplier and a callback-scoped stock dispenser.
/// Return a finite nonnegative worker contribution to add, or `None`.
/// Native reverse construction is preserved regardless of the callback.
pub type ConstructionWorkerFn = for<'s> fn(Server<'s>, Entity<'s>, f32) -> Option<f32>;

const CONSTRUCTION_MULTIPLIER: VirtualFunction<GetConstructionMultiplierFn> =
	VirtualFunction::new(GET_CONSTRUCTION_MULTIPLIER_SLOT);

static ROUTE: ConstructionRoute = ConstructionRoute {
	state: Cell::new(None),
};

struct ConstructionRoute {
	state: Cell<Option<RoutedConstruction>>,
}

impl Handler<GetConstructionMultiplierFn> for ConstructionRoute {
	fn call(&self, call: &HookCall<'_, GetConstructionMultiplierFn>) -> HookAction<f32> {
		let (Some(route), Some(building), Some(multiplier)) = (
			self.state.get(),
			NonNull::new(call.this()),
			call.return_value(),
		) else {
			return HookAction::Ignore;
		};
		let scope = ();
		// SAFETY: Registration supplied this server's binding; the dispatcher
		// runs on the main thread during one live native invocation.
		let server = unsafe { route.binding.server(&scope) };
		// SAFETY: The stock class method receives a live primary entity base.
		// Callbacks must defer removal until the native invocation returns.
		let building = unsafe { Entity::from_live(server, building) };
		let worker = (route.callback)(server, building, multiplier);
		match added_worker(multiplier, worker) {
			Some(multiplier) => HookAction::Override(multiplier),
			None => HookAction::Ignore,
		}
	}
}

// SAFETY: Installation and dispatch require the server's main thread. Route
// state is copied before callbacks; no Cell borrow crosses an engine call.
unsafe impl Sync for ConstructionRoute {}

/// A stock dispenser construction multiplier hook, retained for early removal.
/// Metamod pauses and removes it with the plugin; no map entity is retained.
#[must_use = "retain the hook to support explicitly removing it"]
#[derive(Debug)]
pub struct DispenserConstructionHook {
	hook: HookId,
	_not_thread_safe: PhantomData<Rc<()>>,
}

impl DispenserConstructionHook {
	pub fn remove(self, api: MetamodApi<'_>) {
		api.remove_hook(self.hook);
		if ROUTE
			.state
			.get()
			.is_some_and(|route| route.hook == self.hook)
		{
			ROUTE.state.set(None);
		}
	}
}

#[derive(Debug, thiserror::Error)]
pub enum DispenserConstructionHookError {
	#[error(transparent)]
	Hook(#[from] HookError),
	#[error(transparent)]
	Target(#[from] BuildingVtableError),
}

#[derive(Clone, Copy)]
struct RoutedConstruction {
	binding: ServerBinding,
	callback: ConstructionWorkerFn,
	hook: HookId,
}

impl MetamodApi<'_> {
	/// Adds callback-owned workers after native stock-dispenser constructor
	/// aggregation. A callback panic retains the native multiplier.
	///
	/// The caller owns worker expiry, identity and lifecycle cleanup. Observe
	/// native callbacks before admitting workers, particularly under KHook.
	///
	/// # Safety
	///
	/// `targets` and `binding` must describe the same live TF2 module. The
	/// stock CObjectDispenser primary vtable must hold the generated
	/// no-argument float GetConstructionMultiplier method at its slot and
	/// remain loaded until plugin unload. RTTI and a generated signature
	/// alone do not establish the running binary's ABI. Callbacks must
	/// defer entity deletion as `Server::new` requires.
	pub unsafe fn hook_dispenser_construction_workers(
		self,
		targets: &ClassTargets<'_>,
		binding: ServerBinding,
		callback: ConstructionWorkerFn,
	) -> Result<DispenserConstructionHook, DispenserConstructionHookError> {
		if binding.game() != Game::TeamFortress2 {
			return Err(BuildingVtableError::WrongGame.into());
		}
		if ROUTE
			.state
			.get()
			.is_some_and(|route| self.has_hook(route.hook))
		{
			return Err(HookError::AlreadyInstalled.into());
		}
		let vtable = targets
			.find_all(
				&[BuildingClass::Dispenser.name()],
				GET_CONSTRUCTION_MULTIPLIER_SLOT,
			)
			.pop()
			.flatten()
			.ok_or(BuildingVtableError::NotFound(BuildingClass::Dispenser))?;
		// SAFETY: The caller supplies the exact generated ABI and live module.
		unsafe { self.install_dispenser_construction_workers(vtable.as_ptr(), binding, callback) }
	}

	/// # Safety
	/// The table stays live and holds GetConstructionMultiplierFn at the
	/// generated slot on stock dispensers whose entity base is primary.
	unsafe fn install_dispenser_construction_workers(
		self,
		vtable: NonNull<*mut c_void>,
		binding: ServerBinding,
		callback: ConstructionWorkerFn,
	) -> Result<DispenserConstructionHook, DispenserConstructionHookError> {
		if binding.game() != Game::TeamFortress2 {
			return Err(BuildingVtableError::WrongGame.into());
		}
		if ROUTE
			.state
			.get()
			.is_some_and(|route| self.has_hook(route.hook))
		{
			return Err(HookError::AlreadyInstalled.into());
		}
		// SAFETY: As the caller promises for this generated stock-class slot.
		let hook = unsafe {
			self.add_hook(
				CONSTRUCTION_MULTIPLIER,
				HookTarget::vtable(vtable),
				HookTiming::Post,
				&ROUTE,
			)
		}?;
		ROUTE.state.set(Some(RoutedConstruction {
			binding,
			callback,
			hook,
		}));
		Ok(DispenserConstructionHook {
			hook,
			_not_thread_safe: PhantomData,
		})
	}
}

fn added_worker(multiplier: f32, worker: Option<f32>) -> Option<f32> {
	let worker = worker?;
	if !multiplier.is_finite() || multiplier < 0.0 || !worker.is_finite() || worker < 0.0 {
		return None;
	}
	let combined = multiplier + worker;
	combined.is_finite().then_some(combined)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::test_support::harness::{Harness, on_both};
	use crate::test_support::server::{no_interfaces, tf2_binding};
	use source_sdk_2013::sys;

	thread_local! {
		static WORKER: Cell<Option<f32>> = const { Cell::new(Some(1.5)) };
		static PANIC: Cell<bool> = const { Cell::new(false) };
		static SEEN: Cell<Option<f32>> = const { Cell::new(None) };
	}

	#[repr(C)]
	struct NativeDispenser {
		vtable: *mut *mut c_void,
		multiplier: f32,
		calls: usize,
	}

	impl NativeDispenser {
		fn ptr(&mut self) -> *mut sys::CBaseEntity {
			std::ptr::from_mut(self).cast()
		}
	}

	#[test]
	fn a_callback_panic_retains_native_construction() {
		on_both(|harness| {
			let table = table();
			let hook = install(harness, table);
			let mut dispenser = NativeDispenser {
				vtable: table.as_ptr(),
				multiplier: 2.5,
				calls: 0,
			};
			PANIC.set(true);
			assert_eq!(multiplier(harness, &mut dispenser), 2.5);
			assert_eq!(dispenser.calls, 1);
			PANIC.set(false);
			hook.remove(harness.api());
		});
	}

	fn callback(_: Server<'_>, _: Entity<'_>, native: f32) -> Option<f32> {
		SEEN.set(Some(native));
		assert!(!PANIC.get(), "fixture construction callback panic");
		WORKER.get()
	}

	fn install(harness: &Harness, table: NonNull<*mut c_void>) -> DispenserConstructionHook {
		WORKER.set(Some(1.5));
		PANIC.set(false);
		SEEN.set(None);
		// SAFETY: The leaked fixture table contains exactly the tested float
		// signature; only its named generated slot is invoked.
		unsafe {
			harness.api().install_dispenser_construction_workers(
				table,
				tf2_binding(no_interfaces),
				callback,
			)
		}
		.unwrap()
	}

	fn multiplier(harness: &Harness, dispenser: &mut NativeDispenser) -> f32 {
		harness.call::<GetConstructionMultiplierFn>(
			dispenser.ptr(),
			GET_CONSTRUCTION_MULTIPLIER_SLOT,
			(),
		)
	}

	#[test]
	fn native_engineer_and_builder_terms_are_retained_for_both_consumers() {
		on_both(|harness| {
			let table = table();
			let hook = install(harness, table);
			// Native 1 + two 1.5 Engineer workers + a 0.25 builder term.
			let mut dispenser = NativeDispenser {
				vtable: table.as_ptr(),
				multiplier: 4.25,
				calls: 0,
			};
			let playback = multiplier(harness, &mut dispenser) * 0.5;
			let health_work = multiplier(harness, &mut dispenser) * 0.7495;
			assert_eq!(SEEN.get(), Some(4.25));
			assert_eq!(playback, 2.875);
			assert!((health_work - 4.309625).abs() < 0.000001);
			assert_eq!(dispenser.calls, 2);
			hook.remove(harness.api());
		});
	}

	unsafe extern "C" fn native_multiplier(this: *mut sys::CBaseEntity) -> f32 {
		// SAFETY: Fixture receivers are NativeDispenser instances.
		let dispenser = unsafe { &mut *this.cast::<NativeDispenser>() };
		dispenser.calls += 1;
		dispenser.multiplier
	}

	#[test]
	fn pause_removal_and_reinstallation_preserve_the_original_method() {
		on_both(|harness| {
			let table = table();
			let hook = install(harness, table);
			let mut dispenser = NativeDispenser {
				vtable: table.as_ptr(),
				multiplier: 1.0,
				calls: 0,
			};
			assert_eq!(multiplier(harness, &mut dispenser), 2.5);
			harness.set_status(true, true, harness.generation);
			SEEN.set(None);
			assert_eq!(multiplier(harness, &mut dispenser), 1.0);
			assert_eq!(SEEN.get(), None);
			harness.set_status(true, false, harness.generation);
			assert_eq!(multiplier(harness, &mut dispenser), 2.5);
			hook.remove(harness.api());
			SEEN.set(None);
			assert_eq!(multiplier(harness, &mut dispenser), 1.0);
			assert_eq!(SEEN.get(), None);
			let again = install(harness, table);
			assert_eq!(multiplier(harness, &mut dispenser), 2.5);
			again.remove(harness.api());
		});
	}

	#[test]
	fn prior_post_overrides_are_combined_instead_of_replaced() {
		on_both(|harness| {
			let table = table();
			let earlier = Box::leak(Box::new(
				|call: &HookCall<'_, GetConstructionMultiplierFn>| {
					HookAction::Override(call.return_value().unwrap() + 0.25)
				},
			));
			// SAFETY: The fixture's generated slot holds the declared signature.
			let earlier_id = unsafe {
				harness.api().add_hook(
					CONSTRUCTION_MULTIPLIER,
					HookTarget::vtable(table),
					HookTiming::Post,
					earlier,
				)
			}
			.unwrap();
			let hook = install(harness, table);
			let mut dispenser = NativeDispenser {
				vtable: table.as_ptr(),
				multiplier: 1.0,
				calls: 0,
			};
			assert_eq!(multiplier(harness, &mut dispenser), 2.75);
			assert_eq!(SEEN.get(), Some(1.25));
			hook.remove(harness.api());
			harness.api().remove_hook(earlier_id);
		});
	}

	#[test]
	fn reverse_construction_and_invalid_or_absent_workers_keep_native_results() {
		on_both(|harness| {
			let table = table();
			let hook = install(harness, table);
			let mut dispenser = NativeDispenser {
				vtable: table.as_ptr(),
				multiplier: -1.0,
				calls: 0,
			};
			assert_eq!(multiplier(harness, &mut dispenser), -1.0);
			dispenser.multiplier = 2.5;
			for worker in [None, Some(f32::NAN), Some(f32::INFINITY), Some(-1.0)] {
				WORKER.set(worker);
				assert_eq!(multiplier(harness, &mut dispenser), 2.5);
			}
			dispenser.multiplier = f32::MAX;
			WORKER.set(Some(f32::MAX));
			assert_eq!(multiplier(harness, &mut dispenser), f32::MAX);
			hook.remove(harness.api());
		});
	}

	fn table() -> NonNull<*mut c_void> {
		let slots = vec![native_multiplier as *mut c_void; GET_CONSTRUCTION_MULTIPLIER_SLOT + 1];
		NonNull::new(Vec::leak(slots).as_mut_ptr()).unwrap()
	}
}
