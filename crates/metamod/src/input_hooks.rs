//! Before-input hooks for TF2 entity classes.
//!
//! These class hooks intercept `CBaseEntity::AcceptInput`, allowing a plugin
//! to refuse selected inputs before map logic changes an entity. They hold
//! no entity, survive level changes, and stop calling handlers while paused.
//! A class hook does not cover the distinct vtables of derived classes.

use crate::MetamodApi;

use crate::hook::{
	Handler, HookAction, HookCall, HookError, HookId, HookTarget, HookTiming, VirtualFunction,
};

use source_sdk_2013::entities::Entity;
use source_sdk_2013::raw::entities::ACCEPT_INPUT_SLOT;
use source_sdk_2013::tf2::entity_inputs::InputTarget;
use source_sdk_2013::{Server, ServerBinding, sys};
use std::cell::Cell;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::marker::PhantomData;
use std::ptr::{self, NonNull};
use std::rc::Rc;

/// The native ABI. `variant_t` has a nontrivial copy constructor, so both
/// supported ABIs pass its caller-owned copy by address, as documented by
/// `source_sdk_2013::raw::entities::accept_input`.
type AcceptInput = unsafe extern "C" fn(
	*mut sys::CBaseEntity,
	*const c_char,
	*mut sys::CBaseEntity,
	*mut sys::CBaseEntity,
	*mut sys::variant_t,
	c_int,
) -> bool;

/// A callback-scoped server, the live receiver, and its input name.
/// A panic is contained by the dispatcher and lets the input run. The
/// callback must obey `Server::new`'s deferred-deletion contract.
pub type InputFn = for<'s> fn(Server<'s>, Entity<'s>, &CStr) -> InputAction;

const ACCEPT_INPUT: VirtualFunction<AcceptInput> = VirtualFunction::new(ACCEPT_INPUT_SLOT);

static ROUTES: [InputRoute; 64] = [const { InputRoute::new() }; 64];

/// Whether the game's input handler runs.
#[must_use]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum InputAction {
	/// Let the game's input handler run.
	#[default]
	Allow,
	/// Skip the handler and report the input as accepted. Later pre-hooks
	/// are superseded; the hooking library may still run post-hooks.
	Block,
}

/// The hook installed by `MetamodApi::hook_inputs`.
#[must_use = "retain input hooks to explicitly remove them"]
#[derive(Debug)]
pub struct InputHooks {
	hook: HookId,
	_not_thread_safe: PhantomData<Rc<()>>,
}

impl InputHooks {
	/// Removes the hook before normal plugin unloading.
	pub fn remove(self, api: MetamodApi<'_>) {
		api.remove_hook(self.hook);
		for route in &ROUTES {
			if route
				.state
				.get()
				.is_some_and(|state| state.hook == self.hook)
			{
				route.state.set(None);
			}
		}
	}
}

struct InputRoute {
	state: Cell<Option<RoutedInput>>,
}

impl InputRoute {
	const fn new() -> Self {
		Self {
			state: Cell::new(None),
		}
	}
}

impl Handler<AcceptInput> for InputRoute {
	fn call(&self, call: &HookCall<'_, AcceptInput>) -> HookAction<bool> {
		if call.timing() != HookTiming::Pre || call.superseded() == Some(true) {
			return HookAction::Ignore;
		}
		let Some(route) = self.state.get() else {
			return HookAction::Ignore;
		};
		let (input, _, _, _, _) = call.args();
		let Some(entity) = NonNull::new(call.this()) else {
			return HookAction::Ignore;
		};
		if input.is_null() {
			return HookAction::Ignore;
		}
		let scope = ();
		// SAFETY: The dispatcher invokes this on the main thread during a
		// live class method. The receiver and input name live through the
		// call; deferred deletion keeps the entity allocated.
		let (server, input) = unsafe { (route.binding.server(&scope), CStr::from_ptr(input)) };
		// SAFETY: Installation requires an entity class primary vtable.
		let entity = unsafe { Entity::from_live(server, entity) };
		match (route.callback)(server, entity, input) {
			InputAction::Allow => HookAction::Ignore,
			InputAction::Block => HookAction::Supersede(true),
		}
	}
}

// SAFETY: Installation and hook dispatch run only on the main thread; Cell
// borrows are never held across engine calls.
unsafe impl Sync for InputRoute {}

#[derive(Clone, Copy)]
struct RoutedInput {
	binding: ServerBinding,
	callback: InputFn,
	hook: HookId,
	vtable: usize,
}

impl MetamodApi<'_> {
	/// Intercepts input names before the game handles them for the target
	/// class. `target` and `binding` must belong to the same running server.
	/// A registration may activate asynchronously on Metamod 2.0; a
	/// successful result does not promise interception of the next call.
	///
	/// # Safety
	///
	/// The target must derive from `CBaseEntity` through primary bases,
	/// with its `AcceptInput` at `ACCEPT_INPUT_SLOT`. Its game module must
	/// remain loaded through plugin unloading.
	pub unsafe fn hook_inputs(
		self,
		target: InputTarget<'_>,
		binding: ServerBinding,
		callback: InputFn,
	) -> Result<InputHooks, HookError> {
		// SAFETY: The caller establishes the class and lifetime contract.
		unsafe { self.install_inputs(target.as_ptr(), binding, callback) }
	}

	/// # Safety
	/// The vtable must meet `hook_inputs`'s entity-class and lifetime contract.
	unsafe fn install_inputs(
		self,
		vtable: NonNull<*mut c_void>,
		binding: ServerBinding,
		callback: InputFn,
	) -> Result<InputHooks, HookError> {
		let address = vtable.addr().get();
		let installed = |state: RoutedInput| self.has_hook(state.hook);
		if ROUTES.iter().any(|route| {
			route.state.get().is_some_and(|state| {
				state.vtable == address
					&& ptr::fn_addr_eq(state.callback, callback)
					&& installed(state)
			})
		}) {
			return Err(HookError::AlreadyInstalled);
		}
		let route = ROUTES
			.iter()
			.find(|route| route.state.get().is_none_or(|state| !installed(state)))
			.ok_or(HookError::TooManyFunctions)?;
		// SAFETY: The caller supplies a live primary vtable of this ABI.
		let hook = unsafe {
			self.add_hook(
				ACCEPT_INPUT,
				HookTarget::vtable(vtable),
				HookTiming::Pre,
				route,
			)
		}?;
		route.state.set(Some(RoutedInput {
			binding,
			callback,
			hook,
			vtable: address,
		}));
		Ok(InputHooks {
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

	thread_local! {
		static GAME_CALLS: Cell<usize> = const { Cell::new(0) };
		static CALLBACK_CALLS: Cell<usize> = const { Cell::new(0) };
	}

	#[repr(C)]
	struct Mock {
		vtable: *mut *mut c_void,
	}

	impl Mock {
		fn new() -> Box<Self> {
			let slots = Vec::leak(vec![game_input as *mut c_void; ACCEPT_INPUT_SLOT + 1]);
			Box::new(Self {
				vtable: slots.as_mut_ptr(),
			})
		}

		fn ptr(&mut self) -> *mut sys::CBaseEntity {
			ptr::from_mut(self).cast()
		}

		fn vtable(&self) -> NonNull<*mut c_void> {
			NonNull::new(self.vtable).unwrap()
		}
	}

	fn call(harness: &Harness, entity: &mut Mock, input: &CStr) -> bool {
		harness.call::<AcceptInput>(
			entity.ptr(),
			ACCEPT_INPUT_SLOT,
			(
				input.as_ptr(),
				ptr::null_mut(),
				ptr::null_mut(),
				ptr::null_mut(),
				0,
			),
		)
	}

	unsafe extern "C" fn game_input(
		this: *mut sys::CBaseEntity,
		input: *const c_char,
		activator: *mut sys::CBaseEntity,
		caller: *mut sys::CBaseEntity,
		value: *mut sys::variant_t,
		output: c_int,
	) -> bool {
		GAME_CALLS.set(GAME_CALLS.get() + 1);
		NATIVE_ARGS.set([
			this.addr(),
			input.addr(),
			activator.addr(),
			caller.addr(),
			value.addr(),
			output as usize,
		]);
		false
	}

	#[test]
	fn inputs_filter_names_and_restore_the_game_handler() {
		on_both(|harness| {
			GAME_CALLS.set(0);
			CALLBACK_CALLS.set(0);
			let api = harness.api();
			let mut door = Mock::new();
			let mut other = Mock::new();
			// SAFETY: The leaked mock vtable has the exact signature at the slot.
			let hooks =
				unsafe { api.install_inputs(door.vtable(), tf2_binding(no_interfaces), on_input) }
					.unwrap();
			assert!(matches!(
				unsafe { api.install_inputs(door.vtable(), tf2_binding(no_interfaces), on_input) },
				Err(HookError::AlreadyInstalled)
			));
			assert!(!call(harness, &mut door, c"Open"));
			assert!(call(harness, &mut door, c"cLoSe"));
			assert!(!call(harness, &mut other, c"Close"));
			assert_eq!((GAME_CALLS.get(), CALLBACK_CALLS.get()), (2, 2));
			hooks.remove(api);
			assert!(!call(harness, &mut door, c"Close"));
			assert_eq!((GAME_CALLS.get(), CALLBACK_CALLS.get()), (3, 2));
		});
	}

	fn on_input(_server: Server<'_>, _entity: Entity<'_>, input: &CStr) -> InputAction {
		CALLBACK_CALLS.set(CALLBACK_CALLS.get() + 1);
		if input.to_bytes().eq_ignore_ascii_case(b"Close") {
			InputAction::Block
		} else {
			InputAction::Allow
		}
	}

	fn panics(_server: Server<'_>, _entity: Entity<'_>, _input: &CStr) -> InputAction {
		panic!("input callback panic")
	}

	#[test]
	fn paused_and_panicking_callbacks_let_the_game_run() {
		on_both(|harness| {
			GAME_CALLS.set(0);
			CALLBACK_CALLS.set(0);
			let api = harness.api();
			let mut door = Mock::new();
			// SAFETY: The leaked mock vtable has the exact signature at the slot.
			let hooks =
				unsafe { api.install_inputs(door.vtable(), tf2_binding(no_interfaces), on_input) }
					.unwrap();
			harness.set_status(true, true, harness.generation);
			assert!(!call(harness, &mut door, c"Close"));
			harness.set_status(true, false, harness.generation);
			assert!(call(harness, &mut door, c"Close"));
			hooks.remove(api);
			let hooks =
				unsafe { api.install_inputs(door.vtable(), tf2_binding(no_interfaces), panics) }
					.unwrap();
			assert!(!call(harness, &mut door, c"Close"));
			hooks.remove(api);
			assert_eq!((GAME_CALLS.get(), CALLBACK_CALLS.get()), (2, 1));
		});
	}

	thread_local! {
		static NATIVE_ARGS: Cell<[usize; 6]> = const { Cell::new([0; 6]) };
	}

	#[test]
	fn allowing_inputs_preserves_all_native_arguments() {
		on_both(|harness| {
			let api = harness.api();
			let mut door = Mock::new();
			let mut activator = Mock::new();
			let mut caller = Mock::new();
			let mut value = std::mem::MaybeUninit::<sys::variant_t>::zeroed();
			let (this, activator, caller, value) = (
				door.ptr(),
				activator.ptr(),
				caller.ptr(),
				value.as_mut_ptr(),
			);
			// SAFETY: The mock ignores variant contents; all passed addresses
			// remain live, and the leaked vtable has the exact native signature.
			let hook =
				unsafe { api.install_inputs(door.vtable(), tf2_binding(no_interfaces), on_input) }
					.unwrap();
			let input = c"Open".as_ptr();
			let result = harness.call::<AcceptInput>(
				this,
				ACCEPT_INPUT_SLOT,
				(input, activator, caller, value, 73),
			);
			assert!(!result);
			assert_eq!(
				NATIVE_ARGS.get(),
				[
					this.addr(),
					input.addr(),
					activator.addr(),
					caller.addr(),
					value.addr(),
					73
				]
			);
			hook.remove(api);
		});
	}
}
