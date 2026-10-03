//! Hooks through mock SourceHook and KHook implementations, which follow the
//! libraries' protocols closely enough to run the hooks as they would.
//!
//! Handlers' panics are caught, so handlers note what went wrong instead.

use super::*;
use crate::api::{MetamodApiBinding, MetamodVersion};
use crate::sys::api::ISmmApi;
use crate::sys::khook::{Action, HookId as KHookId, HookRemovalFn, IKHook, IKHookVtable};
use crate::sys::plugin::PluginStatus;

use crate::sys::sourcehook::{
	AddHookMode, HookManagerPubFunc, IHookContext, IHookContextVtable, IHookManagerInfo,
	IHookManagerInfoVtable, IShDelegate, ISourceHook, ISourceHookVtable, MMIFACE_SOURCEHOOK,
	Plugin, ProtoInfo, SH_HOOKMAN_VERSION,
};

use std::cell::RefCell;
use std::ffi::{CStr, c_char, c_uint};
use std::mem::{self, MaybeUninit};
use std::ptr;
use std::sync::{Mutex, MutexGuard, PoisonError};

const ADD: VirtualFunction<Add> = VirtualFunction::new(1);
const MIXED: VirtualFunction<Mixed> = VirtualFunction::new(2);
const POKE: VirtualFunction<Poke> = VirtualFunction::new(0);

/// The size of every [`Object`]'s vtable: [`poke`], [`add`], [`mixed`], then
/// more [`add`]s.
const SLOTS: usize = 80;

/// Each test is a load of its own.
static GENERATIONS: AtomicU64 = AtomicU64::new(1);

static MOCK_CONTEXT_VTABLE: IHookContextVtable = IHookContextVtable {
	get_next: context_get_next,
	get_override_ret_ptr: context_get_override_ret_ptr,
	get_orig_ret_ptr: context_get_orig_ret_ptr,
	should_call_orig: context_should_call_orig,
};

static MOCK_INFO_VTABLE: IHookManagerInfoVtable = IHookManagerInfoVtable {
	set_info: info_set_info,
};

static MOCK_KHOOK_VTABLE: IKHookVtable = IKHookVtable {
	setup_hook: khook_setup_hook,
	setup_virtual_hook: khook_setup_virtual_hook,
	remove_hook: khook_remove_hook,
	get_context_ptr: khook_get_context_ptr,
	get_original_function: khook_get_original_function,
	get_original_value_ptr: khook_get_original_value_ptr,
	get_override_value_ptr: khook_get_override_value_ptr,
	get_current_value_ptr: khook_get_current_value_ptr,
	destroy_return_value: khook_destroy_return_value,
	find_original: khook_find_original,
	find_original_virtual: khook_find_original_virtual,
	do_recall: khook_do_recall,
	save_return_value: khook_save_return_value,
	lookup_signature: khook_lookup_signature,
	was_original_function_skipped: khook_was_original_function_skipped,
};

static MOCK_SOURCEHOOK_VTABLE: ISourceHookVtable = ISourceHookVtable {
	get_iface_version: sourcehook_iface_version,
	get_impl_version: sourcehook_impl_version,
	add_hook: sourcehook_add_hook,
	remove_hook: sourcehook_remove_hook,
	remove_hook_by_id: sourcehook_hook_by_id,
	pause_hook_by_id: sourcehook_hook_by_id,
	unpause_hook_by_id: sourcehook_hook_by_id,
	set_res: sourcehook_set_res,
	get_prev_res: sourcehook_get_prev_res,
	get_status: sourcehook_get_status,
	get_orig_ret: sourcehook_get_orig_ret,
	get_override_ret: sourcehook_get_override_ret,
	get_iface_ptr: sourcehook_get_iface_ptr,
	get_override_ret_ptr: sourcehook_get_override_ret_ptr,
	remove_hook_manager: sourcehook_remove_hook_manager,
	set_ignore_hooks: sourcehook_ignore_hooks,
	reset_ignore_hooks: sourcehook_ignore_hooks,
	get_orig_vfn_ptr_entry: sourcehook_get_orig_vfn_ptr_entry,
	do_recall: sourcehook_do_recall,
	setup_hook_loop: sourcehook_setup_hook_loop,
	end_context: sourcehook_end_context,
	// SAFETY: Never called. A variadic function cannot be defined on stable.
	log_debug: unsafe {
		mem::transmute::<
			unsafe extern "C" fn(*mut ISourceHook, *const c_char),
			unsafe extern "C" fn(*mut ISourceHook, *const c_char, ...),
		>(sourcehook_log_debug)
	},
};

/// The hooks' state is the library's, so tests take turns.
static SERIAL: Mutex<()> = Mutex::new(());

thread_local! {
	/// What handlers noticed going wrong.
	static FAILURES: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };

	/// The mock KHook, for the hooks of another plugin's to call.
	pub(crate) static FOREIGN_KHOOK: Cell<*mut IKHook> = const { Cell::new(ptr::null_mut()) };

	/// How many times handlers noted a call.
	static HANDLER_CALLS: Cell<u32> = const { Cell::new(0) };

	/// How many times the hooked functions' originals ran.
	static ORIGINAL_CALLS: Cell<u32> = const { Cell::new(0) };

	/// What the shell reports in place of the C++ shell's own status.
	static PLUGIN_STATUS: Cell<Option<PluginStatus>> = const { Cell::new(None) };

	/// What handlers saw the calls return so far.
	static SEEN_RETURNS: RefCell<Vec<Option<i32>>> = const { RefCell::new(Vec::new()) };

	/// What the last handler to note a call saw of `superseded`.
	static SEEN_SUPERSEDED: Cell<Option<Option<bool>>> = const { Cell::new(None) };
}

type Add = unsafe extern "C" fn(*mut Object, i32) -> i32;

type Mixed =
	unsafe extern "C" fn(*mut Object, i8, f32, u64, f64, *const u8, i16, f32, bool, u32) -> f64;

type Poke = unsafe extern "C" fn(*mut Object);

/// Another plugin's SourceHook delegate on `Add`.
#[repr(C)]
pub(crate) struct ForeignDelegate {
	pub(crate) vtable: *const *mut c_void,
	pub(crate) sourcehook: *mut ISourceHook,
	pub(crate) result: Cell<MetaRes>,
	pub(crate) value: i32,
}

/// A running Metamod of one version, through mocks of its API and hooking
/// library.
pub(crate) struct Harness {
	version: MetamodVersion,
	smm: Box<MockSmm>,
	_smm_vtable: Box<[MaybeUninit<*const ()>; 34]>,
	pub(crate) sourcehook: Box<MockSourceHook>,
	pub(crate) khook: Box<MockKHook>,
	pub(crate) generation: u64,
	_serial: MutexGuard<'static, ()>,
}

impl Harness {
	fn new(version: MetamodVersion) -> Self {
		let serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
		let generation = GENERATIONS.fetch_add(1, Ordering::Relaxed);

		FAILURES.take();
		HANDLER_CALLS.set(0);
		ORIGINAL_CALLS.set(0);
		SEEN_RETURNS.take();
		SEEN_SUPERSEDED.set(None);

		let smm_vtable = smm_vtable(version);

		let mut harness = Self {
			version,
			smm: Box::new(MockSmm {
				api: ISmmApi {
					vtable: smm_vtable.as_ptr().cast(),
				},
				sourcehook: ptr::null_mut(),
				khook: ptr::null_mut(),
			}),
			_smm_vtable: smm_vtable,
			sourcehook: Box::new(MockSourceHook {
				sourcehook: ISourceHook {
					vtable: &MOCK_SOURCEHOOK_VTABLE,
				},
				state: RefCell::default(),
			}),
			khook: Box::new(MockKHook {
				khook: IKHook {
					vtable: &MOCK_KHOOK_VTABLE,
				},
				state: RefCell::default(),
			}),
			generation,
			_serial: serial,
		};

		harness.smm.sourcehook = harness.sourcehook_ptr().cast();
		harness.smm.khook = harness.khook_ptr().cast();
		FOREIGN_KHOOK.set(harness.khook_ptr());
		harness.set_status(true, false, generation);

		harness
	}

	pub(crate) fn api(&self) -> MetamodApi<'_> {
		// SAFETY: The mock lives as long as the harness, and has the methods the
		// detection calls.
		let binding = unsafe { MetamodApiBinding::detect(NonNull::from(&*self.smm).cast()) }
			.expect("the mock is of a supported version");

		// SAFETY: As above.
		unsafe { binding.for_callback(self) }
	}

	/// Calls the function at `index` of the object's vtable, as the engine
	/// would through a hooked vtable.
	pub(crate) fn call<S: Signature>(
		&self,
		object: *mut S::This,
		index: usize,
		args: S::Args,
	) -> S::Output {
		// SAFETY: Every object of these tests starts with its vtable, which has
		// a function of signature `S` at `index`.
		let vtable = unsafe { object.cast::<*mut *mut c_void>().read() };

		match self.version {
			MetamodVersion::Stable1226 => {
				// SAFETY: As above, SourceHook patched the slot with a hook function
				// of the same signature, if any.
				let function = unsafe { vtable.add(index).read() };

				// SAFETY: As above.
				unsafe {
					S::invoke(
						S::from_address(NonNull::new(function).unwrap()),
						object,
						args,
					)
				}
			}

			MetamodVersion::Dev1469 => self.khook.call::<S>(vtable, index, object, args),
		}
	}

	fn khook_ptr(&self) -> *mut IKHook {
		ptr::from_ref(&*self.khook).cast::<IKHook>().cast_mut()
	}

	pub(crate) fn set_status(&self, loaded: bool, paused: bool, generation: u64) {
		PLUGIN_STATUS.set(Some(PluginStatus {
			generation,
			id: 7,
			loaded,
			paused,
		}));
	}

	pub(crate) fn sourcehook_ptr(&self) -> *mut ISourceHook {
		ptr::from_ref(&*self.sourcehook)
			.cast::<ISourceHook>()
			.cast_mut()
	}
}

impl Drop for Harness {
	fn drop(&mut self) {
		PLUGIN_STATUS.set(None);
		FOREIGN_KHOOK.set(ptr::null_mut());
	}
}

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
struct KhSlot {
	vtable: *mut *mut c_void,
	index: c_int,
	original: *mut c_void,
	hooks: Vec<KhHook>,
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
struct KhState {
	slots: Vec<KhSlot>,
	next_id: KHookId,
	loops: Vec<KhLoop>,
	last: Option<KhLoop>,
	contexts: Vec<*mut c_void>,
	removed: u32,
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

/// A hook function's loop, as SourceHook keeps it.
#[repr(C)]
struct MockContext {
	context: IHookContext,
	pre: Vec<*mut IShDelegate>,
	post: Vec<*mut IShDelegate>,
	phase: Cell<u8>,
	position: Cell<usize>,
	status: *mut MetaRes,
	current: *mut MetaRes,
	orig_ret: *const c_void,
	override_ret: *mut c_void,
	this: *mut c_void,
}

/// SourceHook's record of a hook manager, for its public function to report
/// through.
#[repr(C)]
struct MockInfo {
	info: IHookManagerInfo,
	set: Cell<bool>,
	index: Cell<c_int>,
	proto: Cell<*mut ProtoInfo>,
	hook_function: Cell<*mut c_void>,
}

impl MockInfo {
	fn new() -> Box<Self> {
		Box::new(Self {
			info: IHookManagerInfo {
				vtable: &MOCK_INFO_VTABLE,
			},
			set: Cell::new(false),
			index: Cell::new(-1),
			proto: Cell::new(ptr::null_mut()),
			hook_function: Cell::new(ptr::null_mut()),
		})
	}

	fn ptr(&self) -> *mut IHookManagerInfo {
		ptr::from_ref(self).cast::<IHookManagerInfo>().cast_mut()
	}
}

/// KHook, emulating each detour's loop.
#[repr(C)]
pub(crate) struct MockKHook {
	khook: IKHook,
	state: RefCell<KhState>,
}

impl MockKHook {
	/// Adds another plugin's hook, as KHook would.
	pub(crate) fn add_foreign(&self, vtable: *mut *mut c_void, index: usize, hook: KhHook) {
		self.state
			.borrow_mut()
			.slot(vtable, index as c_int)
			.insert(hook);
	}

	/// A call through a vtable slot KHook may have detoured, as its detour
	/// makes it.
	fn call<S: Signature>(
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

	fn with_context<R>(&self, context: *mut c_void, f: impl FnOnce() -> R) -> R {
		self.state.borrow_mut().contexts.push(context);

		let result = f();

		self.state.borrow_mut().contexts.pop();
		result
	}
}

/// Metamod's `ISmmAPI`, with the hooking library of its version.
#[repr(C)]
struct MockSmm {
	api: ISmmApi,
	sourcehook: *mut c_void,
	khook: *mut c_void,
}

/// SourceHook, with the hook loops it runs.
#[repr(C)]
pub(crate) struct MockSourceHook {
	sourcehook: ISourceHook,
	state: RefCell<ShState>,
}

impl MockSourceHook {
	/// Adds another plugin's delegate, to run before the others on the slot.
	pub(crate) fn add_foreign(&self, vfnptr: *mut *mut c_void, delegate: *mut IShDelegate) {
		self.state.borrow_mut().hooks.insert(
			0,
			ShHook {
				vfnptr,
				delegate,
				post: false,
			},
		);
	}

	fn top_context(&self) -> &MockContext {
		let context = *self.state.borrow().contexts.last().unwrap();

		// SAFETY: Contexts live until `end_context`.
		unsafe { &*context }
	}
}

/// An object of a C++ class, as far as hooks know it.
#[repr(C)]
struct Object {
	vtable: *mut *mut c_void,
	base: i32,
}

impl Object {
	fn new(vtable: *mut *mut c_void, base: i32) -> Box<Self> {
		Box::new(Self { vtable, base })
	}

	fn ptr(&mut self) -> NonNull<Self> {
		NonNull::from(self)
	}
}

/// A pointer to send to another thread.
#[derive(Clone, Copy)]
struct SendPtr<T>(*const T);

// SAFETY: The tests' values outlive the threads they send them to, and only one
// thread uses them at a time.
unsafe impl<T> Send for SendPtr<T> {}

#[derive(Debug, Clone, Copy)]
struct ShHook {
	vfnptr: *mut *mut c_void,
	delegate: *mut IShDelegate,
	post: bool,
}

struct ShManager {
	plugin: Plugin,
	public_function: usize,
	info: Box<MockInfo>,
}

#[derive(Default)]
struct ShState {
	managers: Vec<ShManager>,
	/// Each hooked slot, and what it held before.
	slots: Vec<(*mut *mut c_void, *mut c_void)>,
	hooks: Vec<ShHook>,
	contexts: Vec<*mut MockContext>,
	next_id: c_int,
}

#[test]
fn a_slot_keeps_its_signature() {
	fn handler(_call: &HookCall<'_, Add>) -> HookAction<i32> {
		HookAction::Ignore
	}

	fn other(_call: &HookCall<'_, Poke>) -> HookAction<()> {
		HookAction::Ignore
	}

	on_both(|harness| {
		let api = harness.api();
		let mut object = Object::new(class(), 5);
		let target = HookTarget::class_of(object.ptr());
		let mismatched = VirtualFunction::<Poke>::new(ADD.index());

		// SAFETY: The class has `Add` at the slot. The mismatched hook is refused
		// before anything is installed.
		unsafe {
			api.add_hook(ADD, target, HookTiming::Pre, &handler)
				.unwrap();

			assert_eq!(
				api.add_hook(mismatched, target, HookTiming::Pre, &other),
				Err(HookError::SignatureMismatch)
			);
		}
	});
}

unsafe extern "C" fn add(this: *mut Object, amount: i32) -> i32 {
	ORIGINAL_CALLS.set(ORIGINAL_CALLS.get() + 1);

	// SAFETY: The tests call it on their objects.
	unsafe { (*this).base + amount }
}

#[test]
fn calls_pass_every_kind_of_argument() {
	fn check(call: &HookCall<'_, Mixed>) -> HookAction<f64> {
		note_handler(call.superseded());

		expect(
			call.args()
				== (
					-3,
					1.5,
					1 << 40,
					-2.25,
					64 as *const u8,
					-300,
					0.25,
					true,
					7,
				),
			"the arguments changed",
		);

		match call.timing() {
			HookTiming::Pre => HookAction::Ignore,
			HookTiming::Post => HookAction::Override(call.return_value().unwrap_or(f64::NAN) + 0.5),
		}
	}

	on_both(|harness| {
		let api = harness.api();
		let mut object = Object::new(class(), 5);
		let target = HookTarget::class_of(object.ptr());
		let args = (
			-3,
			1.5,
			1 << 40,
			-2.25,
			64 as *const u8,
			-300,
			0.25,
			true,
			7,
		);

		// SAFETY: The class has `Mixed` at the slot.
		unsafe {
			api.add_hook(MIXED, target, HookTiming::Pre, &check)
				.unwrap();
			api.add_hook(MIXED, target, HookTiming::Post, &check)
				.unwrap();
		}

		let expected =
			5.0 - 3.0 + 1.5 + (1u64 << 40) as f64 - 2.25 + 64.0 - 300.0 + 0.25 + 1.0 + 7.0;

		assert_eq!(
			harness.call::<Mixed>(&raw mut *object, MIXED.index(), args),
			expected + 0.5
		);
		assert_eq!((HANDLER_CALLS.get(), ORIGINAL_CALLS.get()), (2, 1));
	});
}

fn class() -> *mut *mut c_void {
	let mut slots = vec![add as Add as *mut c_void; SLOTS];

	slots[POKE.index()] = poke as Poke as *mut c_void;
	slots[MIXED.index()] = mixed as Mixed as *mut c_void;
	Vec::leak(slots).as_mut_ptr()
}

unsafe extern "C" fn context_get_next(context: *mut IHookContext) -> *mut IShDelegate {
	// SAFETY: SourceHook's contexts are mock ones.
	let context = unsafe { &*context.cast::<MockContext>() };

	let delegates = match context.phase.get() {
		0 => &context.pre,
		1 => &context.post,
		_ => return ptr::null_mut(),
	};

	match delegates.get(context.position.get()) {
		Some(&delegate) => {
			context.position.set(context.position.get() + 1);
			delegate
		}

		None => {
			context.phase.set(context.phase.get() + 1);
			context.position.set(0);
			ptr::null_mut()
		}
	}
}

unsafe extern "C" fn context_get_orig_ret_ptr(context: *mut IHookContext) -> *const c_void {
	// SAFETY: As for `context_get_next`.
	unsafe { (*context.cast::<MockContext>()).orig_ret }
}

unsafe extern "C" fn context_get_override_ret_ptr(context: *mut IHookContext) -> *mut c_void {
	// SAFETY: As for `context_get_next`.
	unsafe { (*context.cast::<MockContext>()).override_ret }
}

unsafe extern "C" fn context_should_call_orig(_context: *mut IHookContext) -> bool {
	true
}

unsafe extern "C" fn detour_interface(this: *mut ISmmApi, _plugin: c_int) -> *mut c_void {
	// SAFETY: The harness's API is a `MockSmm`.
	unsafe { (*this.cast::<MockSmm>()).khook }
}

unsafe extern "C" fn dev_api_versions(
	_this: *mut ISmmApi,
	major: *mut c_int,
	minor: *mut c_int,
	plugin_current: *mut c_int,
	plugin_minimum: *mut c_int,
) {
	// SAFETY: Metamod's caller passes its variables.
	unsafe {
		major.write(2);
		minor.write(1);
		plugin_current.write(18);
		plugin_minimum.write(18);
	}
}

/// Notes what a handler found wrong.
fn expect(condition: bool, failure: &'static str) {
	if !condition {
		FAILURES.with_borrow_mut(|failures| failures.push(failure));
	}
}

unsafe extern "C" fn foreign_call(delegate: *mut ForeignDelegate, _amount: i32) -> i32 {
	// SAFETY: The tests' foreign delegates are live while hooked.
	let delegate = unsafe { &*delegate };

	// SAFETY: SourceHook is calling the delegate.
	unsafe {
		((*(*delegate.sourcehook).vtable).set_res)(delegate.sourcehook, delegate.result.get())
	};

	delegate.value
}

unsafe extern "C" fn foreign_copy(destination: *mut i32, value: *const i32) {
	// SAFETY: KHook copies the value it was given into its storage.
	unsafe { destination.write(value.read()) };
}

unsafe extern "C" fn foreign_destroy(_value: *mut i32) {}

pub(crate) unsafe extern "C" fn foreign_is_equal(
	this: *mut IShDelegate,
	other: *mut IShDelegate,
) -> bool {
	this == other
}

/// Another plugin's `make_call_original`, as `KHook::Virtual` makes it.
unsafe extern "C" fn foreign_khook_call_original(this: *mut Object, amount: i32) -> i32 {
	let khook = FOREIGN_KHOOK.get();

	// SAFETY: The mock KHook is running a detour of `Add`.
	unsafe {
		let functions = &*(*khook).vtable;
		let original = mem::transmute::<*mut c_void, Add>((functions.get_original_function)(khook));
		let mut value = original(this, amount);

		(functions.save_return_value)(
			khook,
			Action::IGNORE,
			ptr::from_mut(&mut value).cast(),
			size_of::<i32>(),
			foreign_copy as unsafe extern "C" fn(*mut i32, *const i32) as *mut c_void,
			foreign_destroy as unsafe extern "C" fn(*mut i32) as *mut c_void,
			true,
		);

		value
	}
}

/// Another plugin's `make_return`, as `KHook::Virtual` makes it.
unsafe extern "C" fn foreign_khook_make_return(_this: *mut Object, _amount: i32) -> i32 {
	let khook = FOREIGN_KHOOK.get();

	// SAFETY: As above.
	unsafe {
		let functions = &*(*khook).vtable;
		let value = (functions.get_current_value_ptr)(khook, true)
			.cast::<i32>()
			.read();

		(functions.destroy_return_value)(khook);
		value
	}
}

/// Another plugin's pre hook, superseding with 77.
unsafe extern "C" fn foreign_khook_supersede(_this: *mut Object, _amount: i32) -> i32 {
	let khook = FOREIGN_KHOOK.get();
	let mut value = 77;

	// SAFETY: As above.
	unsafe {
		((*(*khook).vtable).save_return_value)(
			khook,
			Action::SUPERSEDE,
			ptr::from_mut(&mut value).cast(),
			size_of::<i32>(),
			foreign_copy as unsafe extern "C" fn(*mut i32, *const i32) as *mut c_void,
			foreign_destroy as unsafe extern "C" fn(*mut i32) as *mut c_void,
			false,
		);
	}

	0
}

pub(crate) unsafe extern "C" fn foreign_noop(_this: *mut IShDelegate) {}

#[test]
fn functions_returning_nothing_can_be_superseded() {
	fn supersede(call: &HookCall<'_, Poke>) -> HookAction<()> {
		note_handler(call.superseded());
		expect(
			call.return_value().is_none(),
			"a value was returned before the function ran",
		);

		HookAction::Supersede(())
	}

	fn after(call: &HookCall<'_, Poke>) -> HookAction<()> {
		expect(
			call.return_value() == Some(()),
			"the call returned no value",
		);
		HookAction::Ignore
	}

	on_both(|harness| {
		let api = harness.api();
		let mut object = Object::new(class(), 5);
		let target = HookTarget::class_of(object.ptr());

		// SAFETY: The class has `Poke` at the slot.
		unsafe {
			api.add_hook(POKE, target, HookTiming::Pre, &supersede)
				.unwrap();
			api.add_hook(POKE, target, HookTiming::Post, &after)
				.unwrap();
		}

		harness.call::<Poke>(&raw mut *object, POKE.index(), ());
		assert_eq!((HANDLER_CALLS.get(), ORIGINAL_CALLS.get()), (1, 0));
	});
}

#[test]
fn handlers_decide_calls_before_and_after_the_function() {
	fn supersede_one(call: &HookCall<'_, Add>) -> HookAction<i32> {
		note_handler(call.superseded());

		match call.args() {
			(1,) => HookAction::Supersede(10),
			_ => HookAction::Ignore,
		}
	}

	fn override_two(call: &HookCall<'_, Add>) -> HookAction<i32> {
		expect(
			call.superseded().is_none(),
			"a post hook was told whether it was superseded",
		);
		SEEN_RETURNS.with_borrow_mut(|seen| seen.push(call.return_value()));

		match call.args() {
			(2,) => HookAction::Override(call.return_value().unwrap_or_default() + 100),
			_ => HookAction::Handled,
		}
	}

	on_both(|harness| {
		let api = harness.api();
		let mut object = Object::new(class(), 5);
		let target = HookTarget::class_of(object.ptr());

		// SAFETY: The class has `Add` at the slot.
		let pre = unsafe { api.add_hook(ADD, target, HookTiming::Pre, &supersede_one) }.unwrap();

		// SAFETY: As above.
		let post = unsafe { api.add_hook(ADD, target, HookTiming::Post, &override_two) }.unwrap();

		assert!(api.has_hook(pre) && api.has_hook(post));
		assert_eq!(harness.call::<Add>(&raw mut *object, ADD.index(), (1,)), 10);
		assert_eq!(ORIGINAL_CALLS.get(), 0);
		assert_eq!(
			harness.call::<Add>(&raw mut *object, ADD.index(), (2,)),
			107
		);
		assert_eq!(ORIGINAL_CALLS.get(), 1);
		assert_eq!(harness.call::<Add>(&raw mut *object, ADD.index(), (3,)), 8);
		assert_eq!(HANDLER_CALLS.get(), 3);

		// SAFETY: As above.
		let original = unsafe { api.original_function(ADD, target) }.unwrap();

		assert_eq!(original as usize, add as Add as usize);
		assert!(api.remove_hook(pre));
		assert!(!api.remove_hook(pre));
		assert!(!api.has_hook(pre) && api.has_hook(post));
		assert_eq!(harness.call::<Add>(&raw mut *object, ADD.index(), (1,)), 6);

		// After the superseding value, the function's own values.
		assert_eq!(SEEN_RETURNS.take(), [Some(10), Some(7), Some(8), Some(6)]);
	});
}

#[test]
fn handlers_ignore_other_threads() {
	fn supersede(_call: &HookCall<'_, Add>) -> HookAction<i32> {
		HookAction::Supersede(0)
	}

	on_both(|harness| {
		let api = harness.api();
		let mut object = Object::new(class(), 5);

		// SAFETY: The class has `Add` at the slot.
		unsafe {
			api.add_hook(
				ADD,
				HookTarget::class_of(object.ptr()),
				HookTiming::Pre,
				&supersede,
			)
			.unwrap();
		}

		let object = SendPtr(ptr::from_mut(&mut *object).cast_const());
		let shared = SendPtr(ptr::from_ref(harness));

		let returned = std::thread::spawn(move || {
			let (object, shared) = (object, shared);

			// SAFETY: The harness outlives the thread, which this thread awaits.
			let harness = unsafe { &*shared.0 };

			FOREIGN_KHOOK.set(harness.khook_ptr());
			harness.call::<Add>(object.0.cast_mut(), ADD.index(), (1,))
		})
		.join()
		.unwrap();

		assert_eq!(returned, 6);
	});
}

#[test]
fn handlers_see_earlier_hooks_only_through_sourcehook() {
	fn look(call: &HookCall<'_, Add>) -> HookAction<i32> {
		note_handler(call.superseded());
		HookAction::Ignore
	}

	on_both(|harness| {
		let api = harness.api();
		let vtable = class();
		let mut object = Object::new(vtable, 5);

		// SAFETY: The class has `Add` at the slot.
		unsafe {
			api.add_hook(
				ADD,
				HookTarget::class_of(object.ptr()),
				HookTiming::Pre,
				&look,
			)
			.unwrap();
		}

		let foreign_vtable = [
			foreign_is_equal as unsafe extern "C" fn(*mut IShDelegate, *mut IShDelegate) -> bool
				as *mut c_void,
			foreign_noop as unsafe extern "C" fn(*mut IShDelegate) as *mut c_void,
			foreign_call as unsafe extern "C" fn(*mut ForeignDelegate, i32) -> i32 as *mut c_void,
		];

		let foreign = ForeignDelegate {
			vtable: foreign_vtable.as_ptr(),
			sourcehook: harness.sourcehook_ptr(),
			result: Cell::new(MetaRes::SUPERCEDE),
			value: 77,
		};

		match harness.version {
			MetamodVersion::Stable1226 => harness.sourcehook.add_foreign(
				// SAFETY: The class has the slot.
				unsafe { vtable.add(ADD.index()) },
				ptr::from_ref(&foreign).cast::<IShDelegate>().cast_mut(),
			),

			// Another plugin's hook, added after this one's, which KHook runs first,
			// using its `make_return` and `make_call_original` for the call.
			MetamodVersion::Dev1469 => harness.khook.add_foreign(
				vtable,
				ADD.index(),
				KhHook {
					context: ptr::null_mut(),
					pre: foreign_khook_supersede as Add as *mut c_void,
					post: ptr::null_mut(),
					make_return: foreign_khook_make_return as Add as *mut c_void,
					call_original: foreign_khook_call_original as Add as *mut c_void,
					stack_size: 0,
				},
			),
		}

		assert_eq!(harness.call::<Add>(&raw mut *object, ADD.index(), (1,)), 77);
		assert_eq!(ORIGINAL_CALLS.get(), 0);

		let expected = match harness.version {
			MetamodVersion::Stable1226 => Some(true),
			MetamodVersion::Dev1469 => None,
		};

		assert_eq!(SEEN_SUPERSEDED.get(), Some(expected));

		// Then the other plugin lets calls through, which this plugin's
		// `make_call_original` and `make_return` complete under KHook.
		foreign.result.set(MetaRes::IGNORED);

		for slot in &mut harness.khook.state.borrow_mut().slots {
			slot.hooks.retain(|hook| !hook.context.is_null());
		}

		assert_eq!(harness.call::<Add>(&raw mut *object, ADD.index(), (1,)), 6);
	});
}

#[test]
fn hooks_apply_to_their_targets() {
	fn count(call: &HookCall<'_, Add>) -> HookAction<i32> {
		note_handler(call.superseded());
		HookAction::Override(-1)
	}

	on_both(|harness| {
		let api = harness.api();
		let vtable = class();
		let mut first = Object::new(vtable, 1);
		let mut second = Object::new(vtable, 2);

		// SAFETY: The class has `Add` at the slot.
		unsafe {
			api.add_hook(
				ADD,
				HookTarget::instance(first.ptr()),
				HookTiming::Pre,
				&count,
			)
			.unwrap();
		}

		assert_eq!(harness.call::<Add>(&raw mut *first, ADD.index(), (1,)), -1);
		assert_eq!(harness.call::<Add>(&raw mut *second, ADD.index(), (1,)), 3);
		assert_eq!((HANDLER_CALLS.get(), ORIGINAL_CALLS.get()), (1, 2));

		let vtable = HookTarget::vtable(NonNull::new(vtable).unwrap());

		// SAFETY: As above.
		unsafe { api.add_hook(ADD, vtable, HookTiming::Pre, &count) }.unwrap();

		assert_eq!(harness.call::<Add>(&raw mut *second, ADD.index(), (1,)), -1);
		assert_eq!(HANDLER_CALLS.get(), 2);
	});
}

#[test]
fn inactive_plugins_run_no_handlers() {
	fn supersede(call: &HookCall<'_, Add>) -> HookAction<i32> {
		note_handler(call.superseded());
		HookAction::Supersede(0)
	}

	on_both(|harness| {
		let api = harness.api();
		let mut object = Object::new(class(), 5);
		let target = HookTarget::class_of(object.ptr());

		// SAFETY: The class has `Add` at the slot.
		let hook = unsafe { api.add_hook(ADD, target, HookTiming::Pre, &supersede) }.unwrap();

		harness.set_status(true, true, harness.generation);
		assert_eq!(harness.call::<Add>(&raw mut *object, ADD.index(), (1,)), 6);

		harness.set_status(false, false, harness.generation);
		assert_eq!(harness.call::<Add>(&raw mut *object, ADD.index(), (1,)), 6);
		assert!(!api.has_hook(hook));

		// A later load of the library, which still has the hooks of this one.
		harness.set_status(true, false, harness.generation + 1);
		assert_eq!(harness.call::<Add>(&raw mut *object, ADD.index(), (1,)), 6);
		assert!(!api.has_hook(hook));

		harness.set_status(true, false, harness.generation);
		assert_eq!(harness.call::<Add>(&raw mut *object, ADD.index(), (1,)), 0);
		assert_eq!(HANDLER_CALLS.get(), 1);
	});
}

unsafe extern "C" fn info_set_info(
	info: *mut IHookManagerInfo,
	version: c_int,
	vtbl_offs: c_int,
	vtbl_idx: c_int,
	proto: *mut ProtoInfo,
	hook_function: *mut c_void,
) {
	// SAFETY: SourceHook passes its own records, which are mock ones.
	let info = unsafe { &*info.cast::<MockInfo>() };

	info.set
		.set(version == SH_HOOKMAN_VERSION && vtbl_offs == 0);
	info.index.set(vtbl_idx);
	info.proto.set(proto);
	info.hook_function.set(hook_function);
}

unsafe extern "C" fn khook_destroy_return_value(this: *mut IKHook) {
	// The values' `deinit_op`s run as they drop.
	let last = mock_khook(this).state.borrow_mut().last.take();

	drop(last);
}

unsafe extern "C" fn khook_do_recall(
	_this: *mut IKHook,
	_action: Action,
	_value: *mut c_void,
	_size: usize,
	_init: *mut c_void,
	_destroy: *mut c_void,
) -> *mut c_void {
	ptr::null_mut()
}

unsafe extern "C" fn khook_find_original(_this: *mut IKHook, function: *mut c_void) -> *mut c_void {
	function
}

unsafe extern "C" fn khook_find_original_virtual(
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

unsafe extern "C" fn khook_get_context_ptr(this: *mut IKHook) -> *mut c_void {
	*mock_khook(this).state.borrow().contexts.last().unwrap()
}

unsafe extern "C" fn khook_get_current_value_ptr(this: *mut IKHook, pop: bool) -> *mut c_void {
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

unsafe extern "C" fn khook_get_original_function(this: *mut IKHook) -> *mut c_void {
	let state = mock_khook(this).state.borrow();

	state
		.loops
		.last()
		.and_then(|current| current.original)
		.map_or(ptr::null_mut(), NonNull::as_ptr)
}

unsafe extern "C" fn khook_get_original_value_ptr(this: *mut IKHook) -> *mut c_void {
	let mut state = mock_khook(this).state.borrow_mut();

	state
		.loops
		.last_mut()
		.and_then(|current| current.original_value.as_mut())
		.map_or(ptr::null_mut(), KhValue::pointer)
}

unsafe extern "C" fn khook_get_override_value_ptr(this: *mut IKHook) -> *mut c_void {
	let mut state = mock_khook(this).state.borrow_mut();

	state
		.loops
		.last_mut()
		.and_then(|current| current.override_value.as_mut())
		.map_or(ptr::null_mut(), KhValue::pointer)
}

#[test]
fn khook_hooks_stay_installed_with_their_stack_sizes() {
	fn handler(_call: &HookCall<'_, Mixed>) -> HookAction<f64> {
		HookAction::Ignore
	}

	fn other(_call: &HookCall<'_, Poke>) -> HookAction<()> {
		HookAction::Ignore
	}

	let harness = Harness::new(MetamodVersion::Dev1469);
	let api = harness.api();
	let mut object = Object::new(class(), 5);
	let target = HookTarget::class_of(object.ptr());

	// SAFETY: The class has these functions at their slots.
	let hooks = unsafe {
		[
			api.add_hook(MIXED, target, HookTiming::Pre, &handler)
				.unwrap(),
			api.add_hook(MIXED, target, HookTiming::Pre, &handler)
				.unwrap(),
			api.add_hook(POKE, target, HookTiming::Post, &other)
				.unwrap(),
		]
	};

	for hook in hooks {
		assert!(api.remove_hook(hook));
	}

	let state = harness.khook.state.borrow();

	let stack_sizes = state
		.slots
		.iter()
		.flat_map(|slot| slot.hooks.iter().map(|hook| hook.stack_size))
		.collect::<Vec<_>>();

	// For `this`, nine parameters and the returned value, then just `this`.
	let expected = match cfg!(windows) {
		true => [32 + 8 * 7, 32],
		false => [8 + 8 + 9 * 8, 8],
	};

	assert_eq!(stack_sizes, expected);
	assert_eq!(state.removed, 0);
}

unsafe extern "C" fn khook_lookup_signature(
	_this: *mut IKHook,
	_start: *mut c_void,
	_size: usize,
	_signature: *const c_char,
) -> *mut c_void {
	ptr::null_mut()
}

unsafe extern "C" fn khook_remove_hook(
	this: *mut IKHook,
	_id: KHookId,
	_async: bool,
	_removal: Option<HookRemovalFn>,
	_context: *mut c_void,
) {
	mock_khook(this).state.borrow_mut().removed += 1;
}

unsafe extern "C" fn khook_save_return_value(
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

unsafe extern "C" fn khook_setup_hook(
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
unsafe extern "C" fn khook_setup_virtual_hook(
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

unsafe extern "C" fn khook_was_original_function_skipped(this: *mut IKHook) -> bool {
	mock_khook(this)
		.state
		.borrow()
		.loops
		.last()
		.unwrap()
		.skipped
}

unsafe extern "C" fn meta_factory(
	this: *mut ISmmApi,
	name: *const c_char,
	_return_code: *mut c_int,
	_plugin: *mut c_int,
) -> *mut c_void {
	// SAFETY: The harness's API is a `MockSmm`, and callers pass a name.
	unsafe {
		match CStr::from_ptr(name) == MMIFACE_SOURCEHOOK {
			true => (*this.cast::<MockSmm>()).sourcehook,
			false => ptr::null_mut(),
		}
	}
}

#[allow(
	clippy::too_many_arguments,
	reason = "It stands for a C++ method taking them"
)]
unsafe extern "C" fn mixed(
	this: *mut Object,
	a: i8,
	b: f32,
	c: u64,
	d: f64,
	e: *const u8,
	f: i16,
	g: f32,
	h: bool,
	i: u32,
) -> f64 {
	ORIGINAL_CALLS.set(ORIGINAL_CALLS.get() + 1);

	// SAFETY: The tests call it on their objects.
	let base = unsafe { (*this).base };

	f64::from(base)
		+ f64::from(a)
		+ f64::from(b)
		+ c as f64
		+ d
		+ e as usize as f64
		+ f64::from(f)
		+ f64::from(g)
		+ f64::from(u8::from(h))
		+ f64::from(i)
}

fn mock_khook<'a>(this: *mut IKHook) -> &'a MockKHook {
	// SAFETY: The tests' `IKHook` is a mock one, which outlives its calls.
	unsafe { &*this.cast::<MockKHook>() }
}

fn mock_sourcehook<'a>(this: *mut ISourceHook) -> &'a MockSourceHook {
	// SAFETY: The tests' `ISourceHook` is a mock one, which outlives its calls.
	unsafe { &*this.cast::<MockSourceHook>() }
}

/// Notes a handler's call, with what it saw of `superseded`.
fn note_handler(superseded: Option<bool>) {
	HANDLER_CALLS.set(HANDLER_CALLS.get() + 1);
	SEEN_SUPERSEDED.set(Some(superseded));
}

/// Runs `test` on a Metamod of each version.
pub(crate) fn on_both(test: impl Fn(&Harness)) {
	for version in [MetamodVersion::Stable1226, MetamodVersion::Dev1469] {
		let harness = Harness::new(version);

		test(&harness);
		assert_eq!(FAILURES.take(), Vec::<&str>::new(), "on {version:?}");
	}
}

#[test]
fn panicking_handlers_are_ignored() {
	fn panic(_call: &HookCall<'_, Add>) -> HookAction<i32> {
		panic!("a handler panicked, as this test means it to");
	}

	on_both(|harness| {
		let api = harness.api();
		let mut object = Object::new(class(), 5);

		// SAFETY: The class has `Add` at the slot.
		unsafe {
			api.add_hook(
				ADD,
				HookTarget::class_of(object.ptr()),
				HookTiming::Pre,
				&panic,
			)
			.unwrap();
		}

		assert_eq!(harness.call::<Add>(&raw mut *object, ADD.index(), (1,)), 6);
	});
}

pub(super) fn plugin_status() -> Option<PluginStatus> {
	PLUGIN_STATUS.get()
}

unsafe extern "C" fn poke(_this: *mut Object) {
	ORIGINAL_CALLS.set(ORIGINAL_CALLS.get() + 1);
}

fn smm_vtable(version: MetamodVersion) -> Box<[MaybeUninit<*const ()>; 34]> {
	let mut slots = Box::new([MaybeUninit::uninit(); 34]);

	match version {
		MetamodVersion::Stable1226 => {
			slots[10].write(stable_api_versions as *const ());
			slots[13].write(meta_factory as *const ());
		}

		MetamodVersion::Dev1469 => {
			slots[10].write(dev_api_versions as *const ());
			slots[12].write(meta_factory as *const ());
			slots[33].write(detour_interface as *const ());
		}
	}

	slots
}

unsafe extern "C" fn sourcehook_add_hook(
	this: *mut ISourceHook,
	plugin: Plugin,
	mode: AddHookMode,
	iface: *mut c_void,
	thisptr_offs: c_int,
	public_function: HookManagerPubFunc,
	delegate: *mut IShDelegate,
	post: bool,
) -> c_int {
	let sourcehook = mock_sourcehook(this);
	let info = MockInfo::new();

	// SAFETY: The hook manager's public function reports through the record.
	let refused = unsafe { public_function(false, info.ptr()) } != 0;

	if refused || !info.set.get() || thisptr_offs != 0 {
		return 0;
	}

	let index = info.index.get() as isize;

	// SAFETY: Callers pass a live vtable, or object, with the slot.
	let vfnptr = unsafe {
		match mode {
			AddHookMode::DVP => iface.cast::<*mut c_void>().offset(index),
			_ => iface.cast::<*mut *mut c_void>().read().offset(index),
		}
	};

	let (manager, patch) = {
		let mut state = sourcehook.state.borrow_mut();
		let key = public_function as usize;

		let manager = match state
			.managers
			.iter()
			.position(|manager| manager.plugin == plugin && manager.public_function == key)
		{
			Some(manager) => manager,

			None => {
				state.managers.push(ShManager {
					plugin,
					public_function: key,
					info,
				});

				state.managers.len() - 1
			}
		};

		let manager = ptr::from_ref(&*state.managers[manager].info);
		let patch = !state.slots.iter().any(|&(slot, _)| slot == vfnptr);

		if patch {
			// SAFETY: As above.
			state.slots.push((vfnptr, unsafe { vfnptr.read() }));
		}

		(manager, patch)
	};

	if patch {
		// SAFETY: SourceHook registers the hook manager it patches the slot with,
		// and reads its hook function through the pointer it reported. Records
		// live as long as the mock.
		unsafe {
			public_function(true, (*manager).ptr());
			vfnptr.write((*manager).hook_function.get().cast::<*mut c_void>().read());
		}
	}

	let mut state = sourcehook.state.borrow_mut();

	state.next_id += 1;

	state.hooks.push(ShHook {
		vfnptr,
		delegate,
		post,
	});

	state.next_id
}

unsafe extern "C" fn sourcehook_do_recall(_this: *mut ISourceHook) {}

unsafe extern "C" fn sourcehook_end_context(this: *mut ISourceHook, context: *mut IHookContext) {
	let popped = mock_sourcehook(this).state.borrow_mut().contexts.pop();

	assert_eq!(
		popped.map(|popped| popped.cast::<IHookContext>()),
		Some(context)
	);

	// SAFETY: `sourcehook_setup_hook_loop` allocated it.
	drop(unsafe { Box::from_raw(context.cast::<MockContext>()) });
}

unsafe extern "C" fn sourcehook_get_iface_ptr(this: *mut ISourceHook) -> *mut c_void {
	mock_sourcehook(this).top_context().this
}

unsafe extern "C" fn sourcehook_get_orig_ret(this: *mut ISourceHook) -> *const c_void {
	mock_sourcehook(this).top_context().orig_ret
}

unsafe extern "C" fn sourcehook_get_orig_vfn_ptr_entry(
	this: *mut ISourceHook,
	vfnptr: *mut c_void,
) -> *mut c_void {
	let state = mock_sourcehook(this).state.borrow();

	state
		.slots
		.iter()
		.find(|&&(slot, _)| slot == vfnptr.cast())
		.map_or(ptr::null_mut(), |&(_, original)| original)
}

unsafe extern "C" fn sourcehook_get_override_ret(this: *mut ISourceHook) -> *const c_void {
	let context = mock_sourcehook(this).top_context();

	// SAFETY: The hook function's status outlives its loop.
	match unsafe { context.status.read() } >= MetaRes::OVERRIDE {
		true => context.override_ret,
		false => ptr::null(),
	}
}

unsafe extern "C" fn sourcehook_get_override_ret_ptr(this: *mut ISourceHook) -> *mut c_void {
	mock_sourcehook(this).top_context().override_ret
}

unsafe extern "C" fn sourcehook_get_prev_res(_this: *mut ISourceHook) -> MetaRes {
	MetaRes::IGNORED
}

unsafe extern "C" fn sourcehook_get_status(this: *mut ISourceHook) -> MetaRes {
	// SAFETY: As for `sourcehook_get_override_ret`.
	unsafe { mock_sourcehook(this).top_context().status.read() }
}

unsafe extern "C" fn sourcehook_hook_by_id(_this: *mut ISourceHook, _id: c_int) -> bool {
	false
}

#[test]
fn sourcehook_hook_managers_describe_their_functions() {
	fn handler(_call: &HookCall<'_, Mixed>) -> HookAction<f64> {
		HookAction::Ignore
	}

	let harness = Harness::new(MetamodVersion::Stable1226);
	let api = harness.api();
	let mut object = Object::new(class(), 5);

	// SAFETY: The class has `Mixed` at the slot.
	unsafe {
		api.add_hook(
			MIXED,
			HookTarget::class_of(object.ptr()),
			HookTiming::Post,
			&handler,
		)
		.unwrap();
	}

	let state = harness.sourcehook.state.borrow();
	let info = &state.managers[0].info;

	assert_eq!(info.index.get(), MIXED.index() as c_int);

	// SAFETY: Hook managers' prototypes are leaked.
	let proto = unsafe { &*info.proto.get() };

	// SAFETY: As above, with an entry before the parameters'.
	let parameters = unsafe { std::slice::from_raw_parts(proto.params_pass_info, 10) };

	assert_eq!(proto.num_of_params, 9);
	assert_eq!(proto.ret_pass_info.size, size_of::<f64>());
	assert_eq!(parameters[0].size, 1);

	assert_eq!(
		parameters[1..]
			.iter()
			.map(|parameter| parameter.size)
			.collect::<Vec<_>>(),
		[1, 4, 8, 8, 8, 2, 4, 1, 4]
	);

	assert!(state.hooks[0].post);
}

#[test]
fn sourcehook_hook_managers_run_out() {
	fn handler(_call: &HookCall<'_, Add>) -> HookAction<i32> {
		HookAction::Ignore
	}

	let harness = Harness::new(MetamodVersion::Stable1226);
	let api = harness.api();
	let mut object = Object::new(class(), 5);
	let target = HookTarget::class_of(object.ptr());

	// Every slot holding `Add`, each needing a hook manager of its own.
	let slots = (0..SLOTS).filter(|&index| index != POKE.index() && index != MIXED.index());

	for (hooked, index) in slots.enumerate() {
		// SAFETY: The class has `Add` at the slot.
		let result = unsafe {
			api.add_hook(
				VirtualFunction::<Add>::new(index),
				target,
				HookTiming::Pre,
				&handler,
			)
		};

		if hooked < signature::HOOK_MANAGERS {
			assert!(result.is_ok());
		} else {
			assert_eq!(result, Err(HookError::TooManyFunctions));
			return;
		}
	}

	panic!("the hook managers did not run out");
}

unsafe extern "C" fn sourcehook_iface_version(_this: *mut ISourceHook) -> c_int {
	5
}

unsafe extern "C" fn sourcehook_ignore_hooks(_this: *mut ISourceHook, _vfnptr: *mut c_void) {}

unsafe extern "C" fn sourcehook_impl_version(_this: *mut ISourceHook) -> c_int {
	5
}

unsafe extern "C" fn sourcehook_log_debug(_this: *mut ISourceHook, _format: *const c_char) {}

unsafe extern "C" fn sourcehook_remove_hook(
	_this: *mut ISourceHook,
	_plugin: Plugin,
	_iface: *mut c_void,
	_thisptr_offs: c_int,
	_hook_manager: HookManagerPubFunc,
	_handler: *mut IShDelegate,
	_post: bool,
) -> bool {
	false
}

unsafe extern "C" fn sourcehook_remove_hook_manager(
	_this: *mut ISourceHook,
	_plugin: Plugin,
	_hook_manager: HookManagerPubFunc,
) {
}

unsafe extern "C" fn sourcehook_set_res(this: *mut ISourceHook, res: MetaRes) {
	// SAFETY: As for `sourcehook_get_override_ret`.
	unsafe { mock_sourcehook(this).top_context().current.write(res) };
}

#[allow(
	clippy::too_many_arguments,
	reason = "It stands for a C++ method taking them"
)]
unsafe extern "C" fn sourcehook_setup_hook_loop(
	this: *mut ISourceHook,
	_info: *mut IHookManagerInfo,
	vfnptr: *mut c_void,
	thisptr: *mut c_void,
	orig_call_addr: *mut *mut c_void,
	status: *mut MetaRes,
	_prev_res: *mut MetaRes,
	cur_res: *mut MetaRes,
	orig_ret: *const c_void,
	override_ret: *mut c_void,
) -> *mut IHookContext {
	let mut state = mock_sourcehook(this).state.borrow_mut();
	let vfnptr = vfnptr.cast::<*mut c_void>();

	if let Some(&(_, original)) = state.slots.iter().find(|&&(slot, _)| slot == vfnptr) {
		// SAFETY: The hook function passes its variable.
		unsafe { orig_call_addr.write(original) };
	}

	let delegates = |post: bool| {
		state
			.hooks
			.iter()
			.filter(|hook| hook.vfnptr == vfnptr && hook.post == post)
			.map(|hook| hook.delegate)
			.collect::<Vec<_>>()
	};

	let context = Box::into_raw(Box::new(MockContext {
		context: IHookContext {
			vtable: &MOCK_CONTEXT_VTABLE,
		},
		pre: delegates(false),
		post: delegates(true),
		phase: Cell::new(0),
		position: Cell::new(0),
		status,
		current: cur_res,
		orig_ret,
		override_ret,
		this: thisptr,
	}));

	state.contexts.push(context);
	context.cast()
}

unsafe extern "C" fn stable_api_versions(
	_this: *mut ISmmApi,
	major: *mut c_int,
	minor: *mut c_int,
	plugin_current: *mut c_int,
	plugin_minimum: *mut c_int,
) {
	// SAFETY: Metamod's caller passes its variables.
	unsafe {
		major.write(2);
		minor.write(0);
		plugin_current.write(16);
		plugin_minimum.write(14);
	}
}
