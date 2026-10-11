//! Preserving an owned stock dispenser disable reason at the native update
//! boundary. Plasma expiry, sapper removal and Show/Enable call the inherited
//! virtual UpdateDisabledState; skipping it retains the flag before any caller
//! continues to StartTouch, DispenseThink, RefillThink or OnGoActive.
//!
//! This hook does not itself establish suspension. Callers must first set the
//! disabled flag, retain ownership across callbacks, and witness interception
//! before admitting a preview: KHook can activate registration asynchronously.
//! The stock Show input is suitable for that witness because it calls the
//! updater without calling OnGoActive. The caller must restore its visibility.
//! Dropping the handle does not remove the hook. Pause/unload disable routing;
//! owners must end their suspensions before those transitions.

use crate::MetamodApi;

use crate::hook::{
	Handler, HookAction, HookCall, HookError, HookId, HookTarget, HookTiming, VirtualFunction,
};

use source_sdk_2013::entities::Entity;

use source_sdk_2013::raw::tf2::dispenser_hooks::{
	UPDATE_DISABLED_STATE_SLOT, UpdateDisabledStateFn,
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

/// A callback-scoped stock dispenser whose native disable state will update.
pub type DispenserDisabledFn = for<'s> fn(Server<'s>, Entity<'s>) -> DispenserDisabledAction;

const UPDATE_DISABLED: VirtualFunction<UpdateDisabledStateFn> =
	VirtualFunction::new(UPDATE_DISABLED_STATE_SLOT);

static ROUTE: DisableRoute = DisableRoute {
	state: Cell::new(None),
};

struct DisableRoute {
	state: Cell<Option<RoutedDisable>>,
}

impl Handler<UpdateDisabledStateFn> for DisableRoute {
	fn call(&self, call: &HookCall<'_, UpdateDisabledStateFn>) -> HookAction<()> {
		let Some(route) = self.state.get() else {
			return HookAction::Ignore;
		};
		let Some(entity) = NonNull::new(call.this()) else {
			return HookAction::Ignore;
		};
		let scope = ();
		// SAFETY: The hook is invoked during a live stock-dispenser method on
		// the main thread; registration bound it to this server's generation.
		let server = unsafe { route.binding.server(&scope) };
		// SAFETY: The native method's receiver is the live stock dispenser.
		let entity = unsafe { Entity::from_live(server, entity) };
		// Do not skip our guard merely because an earlier hook superseded the
		// update. Ownership still needs an interception witness in that case.
		match catch_unwind(AssertUnwindSafe(|| (route.callback)(server, entity))) {
			Ok(DispenserDisabledAction::Update) => HookAction::Ignore,
			Ok(DispenserDisabledAction::Preserve) | Err(_) => HookAction::Supersede(()),
		}
	}
}

// SAFETY: Installation and dispatch are restricted to the server's main thread.
// The route is copied before calling user code; no Cell borrow crosses it.
unsafe impl Sync for DisableRoute {}

/// Whether TF2 may recalculate the current disabled flag.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum DispenserDisabledAction {
	/// Retain ordinary native behavior for an unowned building.
	#[default]
	Update,
	/// Keep the current flag by skipping the native update. The caller must
	/// already have established the disabled flag for an owned suspension.
	Preserve,
}

/// The registered stock-dispenser disable-state guard.
#[must_use = "retain the hook to support explicitly removing it"]
#[derive(Debug)]
pub struct DispenserDisabledHook {
	hook: HookId,
	_not_thread_safe: PhantomData<Rc<()>>,
}

impl DispenserDisabledHook {
	/// Removes this generation's hook, without clearing a newer route.
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
pub enum DispenserDisabledHookError {
	#[error(transparent)]
	Hook(#[from] HookError),
	#[error(transparent)]
	Target(#[from] BuildingVtableError),
}

#[derive(Clone, Copy)]
struct RoutedDisable {
	binding: ServerBinding,
	callback: DispenserDisabledFn,
	hook: HookId,
}

impl MetamodApi<'_> {
	/// Guards stock dispensers' inherited UpdateDisabledState before TF2 can
	/// clear a caller-owned disable reason. Derived cart/robot dispensers are
	/// deliberately not registered. A callback panic preserves the current
	/// flag by skipping the update. Observe a callback before admitting work;
	/// successful registration alone is not an activation witness on KHook.
	///
	/// # Safety
	///
	/// `targets` and `binding` must describe the same live TF2 module. Its
	/// CObjectDispenser primary vtable must have the generated no-argument void
	/// UpdateDisabledState method and remain loaded until plugin unload. RTTI
	/// and the generated signature do not independently prove binary identity.
	pub unsafe fn hook_dispenser_disabled_state(
		self,
		targets: &ClassTargets<'_>,
		binding: ServerBinding,
		callback: DispenserDisabledFn,
	) -> Result<DispenserDisabledHook, DispenserDisabledHookError> {
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
				UPDATE_DISABLED_STATE_SLOT,
			)
			.pop()
			.flatten()
			.ok_or(BuildingVtableError::NotFound(BuildingClass::Dispenser))?;
		// SAFETY: The caller supplies the live module and generated ABI above.
		unsafe { self.install_dispenser_disabled_state(vtable.as_ptr(), binding, callback) }
	}

	/// # Safety
	/// The table remains live and contains UpdateDisabledStateFn at the named
	/// generated slot, on stock dispenser objects with a primary entity base.
	unsafe fn install_dispenser_disabled_state(
		self,
		vtable: NonNull<*mut c_void>,
		binding: ServerBinding,
		callback: DispenserDisabledFn,
	) -> Result<DispenserDisabledHook, DispenserDisabledHookError> {
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
		// SAFETY: As promised by the caller for this exact generated slot.
		let hook = unsafe {
			self.add_hook(
				UPDATE_DISABLED,
				HookTarget::vtable(vtable),
				HookTiming::Pre,
				&ROUTE,
			)
		}?;
		ROUTE.state.set(Some(RoutedDisable {
			binding,
			callback,
			hook,
		}));
		Ok(DispenserDisabledHook {
			hook,
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
		static OWNED: Cell<usize> = const { Cell::new(0) };
		static OBSERVED: Cell<bool> = const { Cell::new(false) };
		static PANIC: Cell<bool> = const { Cell::new(false) };
	}

	#[repr(C)]
	struct NativeDispenser {
		vtable: *mut *mut c_void,
		disabled: bool,
		supplied: usize,
	}

	impl NativeDispenser {
		fn new(vtable: NonNull<*mut c_void>) -> Self {
			Self {
				vtable: vtable.as_ptr(),
				disabled: true,
				supplied: 0,
			}
		}

		fn ptr(&mut self) -> *mut sys::CBaseEntity {
			std::ptr::from_mut(self).cast()
		}

		fn service(&mut self) {
			// StartTouch, DispenseThink and RefillThink all gate new service
			// on IsDisabled. Simulate the later call in the same native frame.
			if !self.disabled {
				self.supplied += 1;
			}
		}
	}

	fn callback(_: Server<'_>, entity: Entity<'_>) -> DispenserDisabledAction {
		OBSERVED.set(true);
		assert!(!PANIC.get(), "fixture callback panic");
		if entity.as_ptr().addr() == OWNED.get() {
			DispenserDisabledAction::Preserve
		} else {
			DispenserDisabledAction::Update
		}
	}

	#[test]
	fn callback_panic_cannot_enable_a_suspended_dispenser() {
		on_both(|harness| {
			let vtable = table();
			let hook = install(harness, vtable);
			let mut held = NativeDispenser::new(vtable);
			OWNED.set(held.ptr().addr());
			PANIC.set(true);
			assert!(update(harness, &mut held));
			held.service();
			assert!(held.disabled);
			assert_eq!(held.supplied, 0);
			PANIC.set(false);
			hook.remove(harness.api());
		});
	}

	#[test]
	fn inactive_or_removed_hooks_supply_no_admission_witness() {
		on_both(|harness| {
			let vtable = table();
			let hook = install(harness, vtable);
			let mut held = NativeDispenser::new(vtable);
			OWNED.set(held.ptr().addr());
			assert!(update(harness, &mut held));
			harness.set_status(true, true, harness.generation);
			assert!(!update(harness, &mut held));
			assert!(!held.disabled);
			harness.set_status(true, false, harness.generation);
			held.disabled = true;
			assert!(update(harness, &mut held));
			hook.remove(harness.api());
			assert!(!update(harness, &mut held));
			assert!(!held.disabled);
			let again = install(harness, vtable);
			held.disabled = true;
			assert!(update(harness, &mut held));
			assert!(held.disabled);
			again.remove(harness.api());
		});
	}

	fn install(harness: &Harness, vtable: NonNull<*mut c_void>) -> DispenserDisabledHook {
		PANIC.set(false);
		// SAFETY: The leaked mock table holds the generated void signature.
		unsafe {
			harness.api().install_dispenser_disabled_state(
				vtable,
				tf2_binding(no_interfaces),
				callback,
			)
		}
		.unwrap()
	}

	unsafe extern "C" fn native_update(this: *mut sys::CBaseEntity) {
		// SAFETY: Fixture receivers are NativeDispenser instances.
		unsafe {
			(*this.cast::<NativeDispenser>()).disabled = false;
		}
	}

	#[test]
	fn owned_disable_survives_native_updates_before_every_service_entry() {
		on_both(|harness| {
			let vtable = table();
			let hook = install(harness, vtable);
			let mut held = NativeDispenser::new(vtable);
			let mut ordinary = NativeDispenser::new(vtable);
			OWNED.set(held.ptr().addr());
			// Expiring plasma, removing a sapper and Show all reach this
			// updater before the remainder of the game frame runs.
			for _ in 0..3 {
				assert!(update(harness, &mut held));
				held.service();
				assert!(held.disabled);
				assert_eq!(held.supplied, 0);
			}
			assert!(update(harness, &mut ordinary));
			ordinary.service();
			assert!(!ordinary.disabled);
			assert_eq!(ordinary.supplied, 1);
			OWNED.set(0);
			assert!(update(harness, &mut held));
			held.service();
			assert_eq!(held.supplied, 1);
			hook.remove(harness.api());
		});
	}

	fn table() -> NonNull<*mut c_void> {
		let slots = vec![native_update as *mut c_void; UPDATE_DISABLED_STATE_SLOT + 1];
		NonNull::new(Vec::leak(slots).as_mut_ptr()).unwrap()
	}

	fn update(harness: &Harness, entity: &mut NativeDispenser) -> bool {
		OBSERVED.set(false);
		harness.call::<UpdateDisabledStateFn>(entity.ptr(), UPDATE_DISABLED_STATE_SLOT, ());
		OBSERVED.get()
	}
}
