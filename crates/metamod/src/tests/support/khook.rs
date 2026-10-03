//! A mock of KHook, Metamod 2.0's hooking library, which runs each detour's
//! loop as KHook does.

use crate::hook::Signature;
use crate::sys::khook::{Action, HookId as KHookId, HookRemovalFn, IKHook, IKHookVtable};
use std::cell::RefCell;
use std::ffi::{c_char, c_int, c_uint, c_void};
use std::mem;
use std::ptr::{self, NonNull};

static MOCK_KHOOK_VTABLE: IKHookVtable = IKHookVtable {
	setup_hook,
	setup_virtual_hook,
	remove_hook,
	get_context_ptr,
	get_original_function,
	get_original_value_ptr,
	get_override_value_ptr,
	get_current_value_ptr,
	destroy_return_value,
	find_original,
	find_original_virtual,
	do_recall,
	save_return_value,
	lookup_signature,
	was_original_function_skipped,
};

/// A hook KHook installed on a vtable slot.
#[derive(Debug, Clone, Copy)]
pub(crate) struct KhHook {
	pub(crate) context: *mut c_void,
	pub(crate) pre: *mut c_void,
	pub(crate) post: *mut c_void,
	pub(crate) make_return: *mut c_void,
	pub(crate) call_original: *mut c_void,
	pub(crate) stack_size: c_uint,
}

/// What a KHook detour keeps for a call.
#[derive(Default)]
struct KhLoop {
	original: Option<NonNull<c_void>>,
	action: u8,
	original_value: Option<KhValue>,
	override_value: Option<KhValue>,
	in_progress: bool,
	skipped: bool,
}

/// A vtable slot KHook hooked, with its hooks in KHook's order.
pub(crate) struct KhSlot {
	vtable: *mut *mut c_void,
	index: c_int,
	original: *mut c_void,
	pub(crate) hooks: Vec<KhHook>,
}

impl KhSlot {
	/// Inserts a hook where KHook's `DetourCapsule::InsertHook` does.
	fn insert(&mut self, hook: KhHook) {
		if hook.post.is_null() {
			self.hooks.insert(0, hook);
		} else if hook.pre.is_null() {
			self.hooks.push(hook);
		} else {
			let position = self
				.hooks
				.iter()
				.position(|other| !other.post.is_null())
				.unwrap_or(self.hooks.len());

			self.hooks.insert(position, hook);
		}
	}
}

#[derive(Default)]
pub(crate) struct KhState {
	/// The hooked slots, in the order KHook first hooked them.
	pub(crate) slots: Vec<KhSlot>,
	next_id: KHookId,
	loops: Vec<KhLoop>,
	last: Option<KhLoop>,
	contexts: Vec<*mut c_void>,
	/// How many times `RemoveHook` was called.
	pub(crate) removed: u32,
}

impl KhState {
	fn slot(&mut self, vtable: *mut *mut c_void, index: c_int) -> &mut KhSlot {
		let position = self
			.slots
			.iter()
			.position(|slot| slot.vtable == vtable && slot.index == index);

		match position {
			Some(position) => &mut self.slots[position],

			None => {
				self.slots.push(KhSlot {
					vtable,
					index,
					// SAFETY: KHook is given live vtables with the slot.
					original: unsafe { vtable.offset(index as isize).read() },
					hooks: Vec::new(),
				});

				self.slots.last_mut().unwrap()
			}
		}
	}
}

/// A value KHook copied with a hook's `init_op`.
struct KhValue {
	storage: Vec<u64>,
	destroy: *mut c_void,
}

impl KhValue {
	/// # Safety
	///
	/// `init` must copy a value of `size` bytes from `value`.
	unsafe fn new(
		value: *mut c_void,
		size: usize,
		init: *mut c_void,
		destroy: *mut c_void,
	) -> Self {
		let mut storage = vec![0; size.div_ceil(8)];

		// SAFETY: As the caller promises.
		unsafe {
			let init =
				mem::transmute::<*mut c_void, unsafe extern "C" fn(*mut c_void, *mut c_void)>(init);

			init(storage.as_mut_ptr().cast(), value);
		}

		Self { storage, destroy }
	}

	fn pointer(&mut self) -> *mut c_void {
		self.storage.as_mut_ptr().cast()
	}
}

impl Drop for KhValue {
	fn drop(&mut self) {
		// SAFETY: The value's `deinit_op`, given with its `init_op`.
		unsafe {
			let destroy =
				mem::transmute::<*mut c_void, unsafe extern "C" fn(*mut c_void)>(self.destroy);

			destroy(self.storage.as_mut_ptr().cast());
		}
	}
}

/// KHook, emulating each detour's loop.
#[repr(C)]
pub(crate) struct MockKHook {
	khook: IKHook,
	pub(crate) state: RefCell<KhState>,
}

impl MockKHook {
	pub(crate) fn new() -> Box<Self> {
		Box::new(Self {
			khook: IKHook {
				vtable: &MOCK_KHOOK_VTABLE,
			},
			state: RefCell::default(),
		})
	}

	/// Adds another plugin's hook, as KHook would.
	pub(crate) fn add_foreign(&self, vtable: *mut *mut c_void, index: usize, hook: KhHook) {
		self.state
			.borrow_mut()
			.slot(vtable, index as c_int)
			.insert(hook);
	}

	/// A call through a vtable slot KHook may have detoured, as its detour
	/// makes it.
	pub(crate) fn call<S: Signature>(
		&self,
		vtable: *mut *mut c_void,
		index: usize,
		this: *mut S::This,
		args: S::Args,
	) -> S::Output {
		let found = self
			.state
			.borrow()
			.slots
			.iter()
			.find(|slot| slot.vtable == vtable && slot.index == index as c_int)
			.map(|slot| (slot.original, slot.hooks.clone()));

		// SAFETY: The detour calls each function with the call's arguments, as
		// the hooked function, whose signature they share.
		let invoke = |function: *mut c_void| unsafe {
			S::invoke(S::from_address(NonNull::new(function).unwrap()), this, args)
		};

		let (original, hooks) = match found {
			Some((original, hooks)) if !hooks.is_empty() => (original, hooks),

			// SAFETY: The slot holds the original function.
			_ => return invoke(unsafe { vtable.add(index).read() }),
		};

		self.state.borrow_mut().loops.push(KhLoop {
			original: NonNull::new(original),
			..KhLoop::default()
		});

		for hook in &hooks {
			if !hook.pre.is_null() {
				self.with_context(hook.context, || invoke(hook.pre));
			}
		}

		let superseded = self.state.borrow().loops.last().unwrap().action == Action::SUPERSEDE.0;

		if superseded {
			self.state.borrow_mut().loops.last_mut().unwrap().skipped = true;
		} else {
			self.state
				.borrow_mut()
				.loops
				.last_mut()
				.unwrap()
				.in_progress = true;
			invoke(hooks[0].call_original);
			self.state
				.borrow_mut()
				.loops
				.last_mut()
				.unwrap()
				.in_progress = false;
		}

		for hook in hooks.iter().rev() {
			if !hook.post.is_null() {
				self.with_context(hook.context, || invoke(hook.post));
			}
		}

		let finished = self.state.borrow_mut().loops.pop();

		self.state.borrow_mut().last = finished;
		invoke(hooks[0].make_return)
	}

	pub(crate) fn ptr(&self) -> *mut IKHook {
		ptr::from_ref(self).cast::<IKHook>().cast_mut()
	}

	fn with_context<R>(&self, context: *mut c_void, f: impl FnOnce() -> R) -> R {
		self.state.borrow_mut().contexts.push(context);

		let result = f();

		self.state.borrow_mut().contexts.pop();
		result
	}
}

unsafe extern "C" fn destroy_return_value(this: *mut IKHook) {
	// The values' `deinit_op`s run as they drop.
	let last = mock_khook(this).state.borrow_mut().last.take();

	drop(last);
}

unsafe extern "C" fn do_recall(
	_this: *mut IKHook,
	_action: Action,
	_value: *mut c_void,
	_size: usize,
	_init: *mut c_void,
	_destroy: *mut c_void,
) -> *mut c_void {
	ptr::null_mut()
}

unsafe extern "C" fn find_original(_this: *mut IKHook, function: *mut c_void) -> *mut c_void {
	function
}

unsafe extern "C" fn find_original_virtual(
	this: *mut IKHook,
	vtable: *mut *mut c_void,
	index: c_int,
) -> *mut c_void {
	let state = mock_khook(this).state.borrow();

	match state
		.slots
		.iter()
		.find(|slot| slot.vtable == vtable && slot.index == index)
	{
		Some(slot) => slot.original,

		// SAFETY: KHook is given live vtables with the slot.
		None => unsafe { vtable.offset(index as isize).read() },
	}
}

unsafe extern "C" fn get_context_ptr(this: *mut IKHook) -> *mut c_void {
	*mock_khook(this).state.borrow().contexts.last().unwrap()
}

unsafe extern "C" fn get_current_value_ptr(this: *mut IKHook, pop: bool) -> *mut c_void {
	let mut state = mock_khook(this).state.borrow_mut();
	let state = &mut *state;

	let current = match pop {
		true => state.last.as_mut(),
		false => state.loops.last_mut(),
	};

	let value = match current {
		Some(current) if current.action >= Action::OVERRIDE.0 => current.override_value.as_mut(),
		Some(current) => current.original_value.as_mut(),
		None => None,
	};

	value.map_or(ptr::null_mut(), KhValue::pointer)
}

unsafe extern "C" fn get_original_function(this: *mut IKHook) -> *mut c_void {
	let state = mock_khook(this).state.borrow();

	state
		.loops
		.last()
		.and_then(|current| current.original)
		.map_or(ptr::null_mut(), NonNull::as_ptr)
}

unsafe extern "C" fn get_original_value_ptr(this: *mut IKHook) -> *mut c_void {
	let mut state = mock_khook(this).state.borrow_mut();

	state
		.loops
		.last_mut()
		.and_then(|current| current.original_value.as_mut())
		.map_or(ptr::null_mut(), KhValue::pointer)
}

unsafe extern "C" fn get_override_value_ptr(this: *mut IKHook) -> *mut c_void {
	let mut state = mock_khook(this).state.borrow_mut();

	state
		.loops
		.last_mut()
		.and_then(|current| current.override_value.as_mut())
		.map_or(ptr::null_mut(), KhValue::pointer)
}

unsafe extern "C" fn lookup_signature(
	_this: *mut IKHook,
	_start: *mut c_void,
	_size: usize,
	_signature: *const c_char,
) -> *mut c_void {
	ptr::null_mut()
}

fn mock_khook<'a>(this: *mut IKHook) -> &'a MockKHook {
	// SAFETY: The tests' `IKHook` is a mock one, which outlives its calls.
	unsafe { &*this.cast::<MockKHook>() }
}

unsafe extern "C" fn remove_hook(
	this: *mut IKHook,
	_id: KHookId,
	_async: bool,
	_removal: Option<HookRemovalFn>,
	_context: *mut c_void,
) {
	mock_khook(this).state.borrow_mut().removed += 1;
}

unsafe extern "C" fn save_return_value(
	this: *mut IKHook,
	action: Action,
	value: *mut c_void,
	size: usize,
	init: *mut c_void,
	destroy: *mut c_void,
	original: bool,
) {
	let mut state = mock_khook(this).state.borrow_mut();
	let current = state.loops.last_mut().unwrap();

	// SAFETY: Hooks pass what their `init_op` copies.
	let saved = || unsafe { KhValue::new(value, size, init, destroy) };

	if original {
		// As KHook aborts on this, so does the test.
		assert!(
			current.in_progress,
			"the original value was saved outside its call"
		);

		if size != 0 {
			current.original_value = Some(saved());
		}
	}

	if action.0 > current.action {
		current.action = action.0;

		if size != 0 {
			current.override_value = Some(saved());
		}
	}
}

unsafe extern "C" fn setup_hook(
	_this: *mut IKHook,
	_function: *mut c_void,
	_context: *mut c_void,
	_removed: *mut c_void,
	_pre: *mut c_void,
	_post: *mut c_void,
	_make_return: *mut c_void,
	_call_original: *mut c_void,
	_stack_size: c_uint,
	_async: bool,
) -> KHookId {
	crate::sys::khook::INVALID_HOOK
}

/// Inserts the hook at once, ignoring `async`. KHook adds a hook from its
/// worker thread unless it created the slot's detour for it, so unlike in
/// these tests, a hook added to a detoured slot can miss the next calls.
unsafe extern "C" fn setup_virtual_hook(
	this: *mut IKHook,
	vtable: *mut *mut c_void,
	index: c_int,
	context: *mut c_void,
	_removed: *mut c_void,
	pre: *mut c_void,
	post: *mut c_void,
	make_return: *mut c_void,
	call_original: *mut c_void,
	stack_size: c_uint,
	_async: bool,
) -> KHookId {
	let mut state = mock_khook(this).state.borrow_mut();
	let id = state.next_id;

	state.next_id += 1;

	state.slot(vtable, index).insert(KhHook {
		context,
		pre,
		post,
		make_return,
		call_original,
		stack_size,
	});

	id
}

unsafe extern "C" fn was_original_function_skipped(this: *mut IKHook) -> bool {
	mock_khook(this)
		.state
		.borrow()
		.loops
		.last()
		.unwrap()
		.skipped
}
