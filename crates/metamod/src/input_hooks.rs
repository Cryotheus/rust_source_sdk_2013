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
use source_sdk_2013::raw::entities::datamap::{DataMaps, FTYPEDESC_INPUT};
use source_sdk_2013::raw::entities::{self as raw_entities, ACCEPT_INPUT_SLOT};
use source_sdk_2013::raw::inputs::Variant;
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

/// An absent input used only while a controlled readiness call is armed.
static PROBE_INPUT: &CStr = c"__rust_source_sdk_2013_input_hook_ready__";

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

/// The logical hook installed by `MetamodApi::hook_inputs`.
/// Clones refer to the same registration; removing any clone invalidates them all.
/// Native hook removal remains owned by Metamod during plugin unloading.
#[must_use = "retain input hooks to explicitly remove them"]
#[derive(Debug, Clone)]
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
		let acknowledged = PROBE.with(|probe| {
			let Some(mut state) = probe.get() else {
				return false;
			};
			if state.hook != route.hook
				|| state.receiver != entity.as_ptr()
				|| state.input != input
				|| input != PROBE_INPUT.as_ptr()
			{
				return false;
			}
			state.acknowledged = true;
			probe.set(Some(state));
			true
		});
		if acknowledged {
			return HookAction::Supersede(false);
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

	fn blocks_probe_input(_server: Server<'_>, _entity: Entity<'_>, input: &CStr) -> InputAction {
		CALLBACK_CALLS.set(CALLBACK_CALLS.get() + 1);
		if input.to_bytes() == PROBE_INPUT.to_bytes() {
			InputAction::Block
		} else {
			InputAction::Allow
		}
	}

	unsafe extern "C" fn game_accepting_input(
		this: *mut sys::CBaseEntity,
		input: *const c_char,
		activator: *mut sys::CBaseEntity,
		caller: *mut sys::CBaseEntity,
		value: *mut sys::variant_t,
		output: c_int,
	) -> bool {
		// SAFETY: This test function shares game_input's exact native signature.
		unsafe { game_input(this, input, activator, caller, value, output) };
		true
	}

	/// Uses the same production token and route through the mock's real dispatcher.
	fn probe(hooks: &InputHooks, harness: &Harness, entity: &mut Mock) -> Result<bool, HookError> {
		let receiver = NonNull::new(entity.ptr()).unwrap();
		// SAFETY: The mock receiver and its exact-signature vtable are live.
		unsafe {
			hooks.probe_with(harness.api(), receiver, |receiver, input, value| {
				assert_eq!((*value).fieldType, sys::_fieldtypes_FIELD_VOID);
				harness.call::<AcceptInput>(
					receiver,
					ACCEPT_INPUT_SLOT,
					(input.as_ptr(), ptr::null_mut(), ptr::null_mut(), value, 0),
				)
			})
		}
	}

	#[test]
	fn readiness_does_not_acknowledge_same_bytes_or_another_receiver() {
		on_both(|harness| {
			GAME_CALLS.set(0);
			CALLBACK_CALLS.set(0);
			let api = harness.api();
			let mut door = Mock::new();
			let mut other = Mock::new();
			// SAFETY: Both leaked tables have the exact configured signature.
			let hooks =
				unsafe { api.install_inputs(door.vtable(), tf2_binding(no_interfaces), on_input) }
					.unwrap();
			let other_hooks =
				unsafe { api.install_inputs(other.vtable(), tf2_binding(no_interfaces), on_input) }
					.unwrap();
			let copied = std::ffi::CString::new(PROBE_INPUT.to_bytes()).unwrap();
			assert!(!call(harness, &mut door, PROBE_INPUT));
			assert!(!call(harness, &mut door, &copied));
			assert_eq!((GAME_CALLS.get(), CALLBACK_CALLS.get()), (2, 2));
			let receiver = NonNull::new(door.ptr()).unwrap();
			// SAFETY: These controlled test calls use live mock receivers and the dispatcher.
			assert_eq!(
				unsafe {
					hooks.probe_with(api, receiver, |receiver, _, value| {
						harness.call::<AcceptInput>(
							receiver,
							ACCEPT_INPUT_SLOT,
							(copied.as_ptr(), ptr::null_mut(), ptr::null_mut(), value, 0),
						)
					})
				},
				Ok(false)
			);
			assert_eq!(
				unsafe {
					hooks.probe_with(api, receiver, |_, input, value| {
						harness.call::<AcceptInput>(
							other.ptr(),
							ACCEPT_INPUT_SLOT,
							(input.as_ptr(), ptr::null_mut(), ptr::null_mut(), value, 0),
						)
					})
				},
				Ok(false)
			);
			assert_eq!((GAME_CALLS.get(), CALLBACK_CALLS.get()), (4, 4));
			assert_eq!(probe(&hooks, harness, &mut door), Ok(true));
			assert_eq!((GAME_CALLS.get(), CALLBACK_CALLS.get()), (4, 4));
			hooks.remove(api);
			other_hooks.remove(api);
		});
	}

	#[test]
	fn readiness_is_revoked_if_removed_during_the_native_call() {
		on_both(|harness| {
			let api = harness.api();
			let mut door = Mock::new();
			// SAFETY: The leaked table has the configured native signature.
			let hooks =
				unsafe { api.install_inputs(door.vtable(), tf2_binding(no_interfaces), on_input) }
					.unwrap();
			let copy = hooks.clone();
			let receiver = NonNull::new(door.ptr()).unwrap();
			// SAFETY: Receiver and call arguments remain live throughout dispatcher re-entry.
			let result = unsafe {
				hooks.probe_with(api, receiver, |receiver, input, value| {
					let result = harness.call::<AcceptInput>(
						receiver,
						ACCEPT_INPUT_SLOT,
						(input.as_ptr(), ptr::null_mut(), ptr::null_mut(), value, 0),
					);
					copy.remove(api);
					result
				})
			};
			assert_eq!(result, Ok(false));
		});
	}

	#[test]
	fn readiness_is_scoped_to_receiver_registration_and_active_load() {
		on_both(|harness| {
			GAME_CALLS.set(0);
			CALLBACK_CALLS.set(0);
			let api = harness.api();
			let mut door = Mock::new();
			let mut other = Mock::new();
			// SAFETY: Both leaked tables have the exact configured signature.
			let hooks =
				unsafe { api.install_inputs(door.vtable(), tf2_binding(no_interfaces), on_input) }
					.unwrap();
			let other_hooks =
				unsafe { api.install_inputs(other.vtable(), tf2_binding(no_interfaces), on_input) }
					.unwrap();
			assert_eq!(
				probe(&hooks, harness, &mut other),
				Err(HookError::InvalidArgument)
			);
			assert_eq!(GAME_CALLS.get(), 0);
			assert_eq!(probe(&hooks, harness, &mut door), Ok(true));
			assert_eq!(probe(&other_hooks, harness, &mut other), Ok(true));
			assert_eq!(CALLBACK_CALLS.get(), 0);
			harness.set_status(true, true, harness.generation);
			assert_eq!(probe(&hooks, harness, &mut door), Ok(false));
			harness.set_status(true, false, harness.generation);
			assert_eq!(probe(&hooks, harness, &mut door), Ok(true));
			harness.set_status(true, false, harness.generation + 1);
			assert_eq!(probe(&hooks, harness, &mut door), Err(HookError::NotBound));
			harness.set_status(true, false, harness.generation);
			let copy = hooks.clone();
			hooks.remove(api);
			let calls = GAME_CALLS.get();
			assert_eq!(probe(&copy, harness, &mut door), Err(HookError::NotBound));
			assert_eq!(GAME_CALLS.get(), calls);
			other_hooks.remove(api);
		});
	}

	#[test]
	fn readiness_refuses_declared_or_uninspectable_probe_inputs() {
		// SAFETY: Native datamaps and field descriptions accept all-zero defaults.
		let mut field =
			unsafe { std::mem::MaybeUninit::<sys::typedescription_t>::zeroed().assume_init() };
		let mut base = unsafe { std::mem::MaybeUninit::<sys::datamap_t>::zeroed().assume_init() };
		let mut child = unsafe { std::mem::MaybeUninit::<sys::datamap_t>::zeroed().assume_init() };
		let copied = std::ffi::CString::new(PROBE_INPUT.to_bytes().to_ascii_uppercase()).unwrap();
		field.flags = FTYPEDESC_INPUT;
		field.externalName = copied.as_ptr();
		base.dataDesc = ptr::from_mut(&mut field);
		base.dataNumFields = 1;
		child.baseMap = ptr::from_mut(&mut base);
		// SAFETY: Each map/field/string stays live and unchanged during each check.
		assert!(!probe_input_is_absent(unsafe {
			DataMaps::new(ptr::from_ref(&child))
		}));
		// SAFETY: The description is owned, aligned, and unborrowed between checks.
		unsafe {
			(&raw mut field.flags).write(source_sdk_2013::raw::entities::datamap::FTYPEDESC_KEY)
		};
		assert!(probe_input_is_absent(unsafe {
			DataMaps::new(ptr::from_ref(&child))
		}));
		// SAFETY: As above, update the same native fixture through its live pointers.
		unsafe {
			(&raw mut field.flags).write(FTYPEDESC_INPUT);
			(&raw mut field.externalName).write(ptr::null());
		}
		assert!(!probe_input_is_absent(unsafe {
			DataMaps::new(ptr::from_ref(&child))
		}));
		// SAFETY: The base description is owned and unborrowed between checks.
		unsafe { (&raw mut base.dataNumFields).write(4097) };
		assert!(!probe_input_is_absent(unsafe {
			DataMaps::new(ptr::from_ref(&child))
		}));
		assert!(!probe_input_is_absent(unsafe {
			DataMaps::new(ptr::null())
		}));
		child.baseMap = ptr::from_mut(&mut child);
		assert!(!probe_input_is_absent(unsafe {
			DataMaps::new(ptr::from_ref(&child))
		}));
	}

	#[test]
	fn readiness_requires_its_own_registration_on_the_same_vtable() {
		on_both(|harness| {
			let api = harness.api();
			let mut door = Mock::new();
			GAME_CALLS.set(0);
			CALLBACK_CALLS.set(0);
			// SAFETY: Both registrations target the same live exact-signature table.
			let foreign = unsafe {
				api.install_inputs(
					door.vtable(),
					tf2_binding(no_interfaces),
					blocks_probe_input,
				)
			}
			.unwrap();
			let owner =
				unsafe { api.install_inputs(door.vtable(), tf2_binding(no_interfaces), on_input) }
					.unwrap();
			assert!(api.has_hook(owner.hook));
			assert!(api.has_hook(foreign.hook));
			// The foreign route supersedes with true, but cannot acknowledge the
			// owner's exact HookId token; the real site dispatcher skips the owner.
			assert_eq!(probe(&owner, harness, &mut door), Ok(false));
			assert_eq!((GAME_CALLS.get(), CALLBACK_CALLS.get()), (0, 1));
			foreign.remove(api);
			assert_eq!(probe(&owner, harness, &mut door), Ok(true));
			assert_eq!((GAME_CALLS.get(), CALLBACK_CALLS.get()), (0, 1));
			owner.remove(api);
		});
	}

	#[test]
	fn readiness_token_clears_on_unwind_and_rejects_nested_probes() {
		on_both(|harness| {
			let api = harness.api();
			let mut door = Mock::new();
			// SAFETY: The leaked table has the configured native signature.
			let hooks =
				unsafe { api.install_inputs(door.vtable(), tf2_binding(no_interfaces), on_input) }
					.unwrap();
			let receiver = NonNull::new(door.ptr()).unwrap();
			let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
				// SAFETY: The mock receiver remains live throughout this controlled unwind.
				let _ = unsafe {
					hooks.probe_with(api, receiver, |_, _, _| panic!("probe call unwind"))
				};
			}));
			assert!(panicked.is_err());
			assert!(PROBE.with(|probe| probe.get().is_none()));
			// SAFETY: The mock receiver remains live; only the outer call dispatches.
			assert_eq!(
				unsafe {
					hooks.probe_with(api, receiver, |receiver_ptr, input, value| {
						let nested = hooks.probe_with(api, receiver, |_, _, _| {
							panic!("nested call must not run")
						});
						assert_eq!(nested, Err(HookError::NotBound));
						harness.call::<AcceptInput>(
							receiver_ptr,
							ACCEPT_INPUT_SLOT,
							(input.as_ptr(), ptr::null_mut(), ptr::null_mut(), value, 0),
						)
					})
				},
				Ok(true)
			);
			assert!(PROBE.with(|probe| probe.get().is_none()));
			hooks.remove(api);
		});
	}

	#[test]
	fn readiness_waits_for_delayed_callback_even_when_native_returns_true() {
		let harness = Harness::new(crate::api::MetamodVersion::Dev1469);
		let api = harness.api();
		GAME_CALLS.set(0);
		CALLBACK_CALLS.set(0);
		let mut door = Mock::new();
		// SAFETY: The mock's leaked table is writable and the function has the same signature.
		unsafe {
			door.vtable()
				.as_ptr()
				.add(ACCEPT_INPUT_SLOT)
				.write(game_accepting_input as *mut c_void)
		};
		harness.khook.state.borrow_mut().defer_virtual_hooks = true;
		// SAFETY: The leaked table has the exact signature at the configured slot.
		let hooks =
			unsafe { api.install_inputs(door.vtable(), tf2_binding(no_interfaces), on_input) }
				.unwrap();
		assert!(api.has_hook(hooks.hook));
		assert_eq!(probe(&hooks, &harness, &mut door), Ok(false));
		assert_eq!(probe(&hooks, &harness, &mut door), Ok(false));
		assert_eq!((GAME_CALLS.get(), CALLBACK_CALLS.get()), (2, 0));
		assert_eq!(harness.khook.state.borrow().removed, 0);
		harness.khook.activate_pending_hooks();
		assert_eq!(probe(&hooks, &harness, &mut door), Ok(true));
		assert_eq!((GAME_CALLS.get(), CALLBACK_CALLS.get()), (2, 0));
		assert!(call(&harness, &mut door, c"Close"));
		assert_eq!((GAME_CALLS.get(), CALLBACK_CALLS.get()), (2, 1));
		hooks.remove(api);
		assert_eq!(harness.khook.state.borrow().removed, 0);
	}
}

thread_local! {
	static PROBE: Cell<Option<ProbeState>> = const { Cell::new(None) };
}

/// Clears a probe token before its receiver can leave the callback scope.
struct ProbeGuard;

impl ProbeGuard {
	fn begin(hook: HookId, receiver: *mut sys::CBaseEntity) -> Result<Self, HookError> {
		PROBE.with(|probe| {
			if probe.get().is_some() {
				return Err(HookError::NotBound);
			}
			probe.set(Some(ProbeState {
				hook,
				receiver,
				input: PROBE_INPUT.as_ptr(),
				acknowledged: false,
			}));
			Ok(Self)
		})
	}

	fn acknowledged(&self) -> bool {
		PROBE.with(|probe| probe.get().is_some_and(|state| state.acknowledged))
	}
}

impl Drop for ProbeGuard {
	fn drop(&mut self) {
		PROBE.with(|probe| probe.set(None));
	}
}

#[derive(Clone, Copy)]
struct ProbeState {
	hook: HookId,
	receiver: *mut sys::CBaseEntity,
	input: *const c_char,
	acknowledged: bool,
}

impl InputHooks {
	/// Proves that this registration dispatches a call on `receiver` now.
	///
	/// Sends a reserved, absent input through the actual `AcceptInput` virtual
	/// slot. This route acknowledges the exact armed call without invoking the
	/// user callback and supersedes it. The native return value is ignored.
	/// `Ok(false)` means no acknowledgment: retry from a later engine callback
	/// before changing protected state. There is no fixed activation delay.
	///
	/// Wrong vtables, marked receivers, declared probe inputs, or incomplete
	/// datamaps give `InvalidArgument`; removed or obsolete registrations give
	/// `NotBound`. A paused registration does not acknowledge. No readiness is
	/// cached across calls, maps, pauses, removals, or plugin loads.
	///
	/// # Safety
	///
	/// The receiver must belong to this registration's running server. Its
	/// native `AcceptInput` implementation and any other hooks must treat the
	/// reserved absent name as benign when this hook is not active. Stock
	/// `CBaseDoor` and `CRotDoor` use `CBaseEntity::AcceptInput`, whose unknown
	/// input fallback only returns false and can emit developer diagnostics.
	/// Other classes may override that fallback. All code the probe calls must
	/// obey `Server::new`'s deferred-deletion contract.
	///
	/// # Panics
	///
	/// If `Entity::is_marked_for_deletion` cannot validate its native field.
	pub unsafe fn probe_ready(
		&self,
		api: MetamodApi<'_>,
		receiver: Entity<'_>,
	) -> Result<bool, HookError> {
		let receiver_ptr = NonNull::new(receiver.as_ptr()).ok_or(HookError::InvalidArgument)?;
		// SAFETY: Entity is callback-scoped and the caller establishes its server.
		unsafe { self.probe_route(api, receiver_ptr) }?;
		if receiver.is_marked_for_deletion() {
			return Err(HookError::InvalidArgument);
		}
		// SAFETY: The receiver is live, and its game DLL's maps are immutable.
		let maps = unsafe { DataMaps::new(raw_entities::data_desc_map(receiver.as_ptr())) };
		if !probe_input_is_absent(maps) {
			return Err(HookError::InvalidArgument);
		}
		// SAFETY: The caller establishes a benign unknown-input fallback and
		// deferred deletion; probe_with supplies a valid void variant.
		unsafe {
			self.probe_with(api, receiver_ptr, |receiver, input, value| {
				raw_entities::accept_input(
					receiver,
					input,
					ptr::null_mut(),
					ptr::null_mut(),
					value,
					0,
				)
			})
		}
	}

	/// # Safety
	/// The receiver must be a live entity of the running server.
	unsafe fn probe_route(
		&self,
		api: MetamodApi<'_>,
		receiver: NonNull<sys::CBaseEntity>,
	) -> Result<RoutedInput, HookError> {
		if !api.has_hook(self.hook) {
			return Err(HookError::NotBound);
		}
		let route = ROUTES
			.iter()
			.find_map(|route| route.state.get().filter(|state| state.hook == self.hook))
			.ok_or(HookError::NotBound)?;
		// SAFETY: A live primary CBaseEntity starts with its vtable pointer.
		let vtable = unsafe { receiver.as_ptr().cast::<*mut *mut c_void>().read() };
		if vtable.addr() != route.vtable {
			return Err(HookError::InvalidArgument);
		}
		Ok(route)
	}

	/// Arms the production token and runs either the native call or its test dispatcher.
	///
	/// # Safety
	/// The receiver is live and the call obeys probe_ready's fallback/lifetime contract.
	unsafe fn probe_with(
		&self,
		api: MetamodApi<'_>,
		receiver: NonNull<sys::CBaseEntity>,
		call: impl FnOnce(*mut sys::CBaseEntity, &CStr, *mut sys::variant_t) -> bool,
	) -> Result<bool, HookError> {
		// SAFETY: The caller establishes the receiver's lifetime.
		unsafe { self.probe_route(api, receiver) }?;
		let guard = ProbeGuard::begin(self.hook, receiver.as_ptr())?;
		let mut value = Variant::Void.to_raw();
		let _native_result = call(receiver.as_ptr(), PROBE_INPUT, ptr::from_mut(&mut value));
		Ok(guard.acknowledged()
			&& api.has_hook(self.hook)
			&& ROUTES.iter().any(|route| {
				route
					.state
					.get()
					.is_some_and(|state| state.hook == self.hook)
			}))
	}
}

/// Refuses a name collision or a map chain that cannot be inspected completely.
fn probe_input_is_absent(maps: DataMaps<'_>) -> bool {
	let mut any = false;
	for (index, map) in maps.enumerate() {
		any = true;
		if index + 1 == DataMaps::MAX_MAPS && map.base().is_some() {
			return false;
		}
		let fields = map.fields();
		// SAFETY: DataMaps keeps each native description allocated and unchanged.
		let count = unsafe { (&raw const (*map.as_ptr()).dataNumFields).read() };
		if usize::try_from(count).ok() != Some(fields.len()) {
			return false;
		}
		for field in fields {
			if field.flags & FTYPEDESC_INPUT != 0 {
				let Some(name) = field.external_name() else {
					return false;
				};
				if name.to_bytes().eq_ignore_ascii_case(PROBE_INPUT.to_bytes()) {
					return false;
				}
			}
		}
	}
	any
}
