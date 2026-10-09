//! A mock of SourceHook, Metamod 1.12's hooking library, which runs hook loops
//! as SourceHook does.

use crate::sys::sourcehook::{
	AddHookMode, HookManagerPubFunc, IHookContext, IHookContextVtable, IHookManagerInfo,
	IHookManagerInfoVtable, IShDelegate, ISourceHook, ISourceHookVtable, MetaRes, Plugin,
	ProtoInfo, SH_HOOKMAN_VERSION,
};

use std::cell::{Cell, RefCell};
use std::ffi::{c_char, c_int, c_void};
use std::mem;
use std::ptr;

static MOCK_CONTEXT_VTABLE: IHookContextVtable = IHookContextVtable {
	get_next: context_get_next,
	get_override_ret_ptr: context_get_override_ret_ptr,
	get_orig_ret_ptr: context_get_orig_ret_ptr,
	should_call_orig: context_should_call_orig,
};

static MOCK_INFO_VTABLE: IHookManagerInfoVtable = IHookManagerInfoVtable {
	set_info: info_set_info,
};

static MOCK_SOURCEHOOK_VTABLE: ISourceHookVtable = ISourceHookVtable {
	get_iface_version: iface_version,
	get_impl_version: impl_version,
	add_hook,
	remove_hook,
	remove_hook_by_id: hook_by_id,
	pause_hook_by_id: hook_by_id,
	unpause_hook_by_id: hook_by_id,
	set_res,
	get_prev_res,
	get_status,
	get_orig_ret,
	get_override_ret,
	get_iface_ptr,
	get_override_ret_ptr,
	remove_hook_manager,
	set_ignore_hooks: ignore_hooks,
	reset_ignore_hooks: ignore_hooks,
	get_orig_vfn_ptr_entry,
	do_recall,
	setup_hook_loop,
	end_context,
	// SAFETY: Never called. A variadic function cannot be defined on stable.
	log_debug: unsafe {
		mem::transmute::<
			unsafe extern "C" fn(*mut ISourceHook, *const c_char),
			unsafe extern "C" fn(*mut ISourceHook, *const c_char, ...),
		>(log_debug)
	},
};

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
pub(crate) struct MockInfo {
	info: IHookManagerInfo,
	set: Cell<bool>,
	/// The vtable slot of the hooked function.
	pub(crate) index: Cell<c_int>,
	/// The hooked function's prototype.
	pub(crate) proto: Cell<*mut ProtoInfo>,
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

/// SourceHook, with the hook loops it runs.
#[repr(C)]
pub(crate) struct MockSourceHook {
	sourcehook: ISourceHook,
	pub(crate) state: RefCell<ShState>,
}

impl MockSourceHook {
	pub(crate) fn new() -> Box<Self> {
		Box::new(Self {
			sourcehook: ISourceHook {
				vtable: &MOCK_SOURCEHOOK_VTABLE,
			},
			state: RefCell::default(),
		})
	}

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

	pub(crate) fn ptr(&self) -> *mut ISourceHook {
		ptr::from_ref(self).cast::<ISourceHook>().cast_mut()
	}

	fn top_context(&self) -> &MockContext {
		let context = *self.state.borrow().contexts.last().unwrap();

		// SAFETY: Contexts live until `end_context`.
		unsafe { &*context }
	}
}

/// A delegate SourceHook runs for a hooked vtable slot.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ShHook {
	vfnptr: *mut *mut c_void,
	delegate: *mut IShDelegate,
	pub(crate) post: bool,
}

/// A hook manager a plugin registered.
pub(crate) struct ShManager {
	plugin: Plugin,
	public_function: usize,
	pub(crate) info: Box<MockInfo>,
}

#[derive(Default)]
pub(crate) struct ShState {
	/// The hook managers, in the order they were registered.
	pub(crate) managers: Vec<ShManager>,
	/// Each hooked slot, and what it held before.
	slots: Vec<(*mut *mut c_void, *mut c_void)>,
	/// The delegates, in the order they run.
	pub(crate) hooks: Vec<ShHook>,
	contexts: Vec<*mut MockContext>,
	/// How many hook loops hook functions set up.
	pub(crate) loops: usize,
	next_id: c_int,
}

unsafe extern "C" fn add_hook(
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

unsafe extern "C" fn do_recall(_this: *mut ISourceHook) {}

unsafe extern "C" fn end_context(this: *mut ISourceHook, context: *mut IHookContext) {
	let popped = mock_sourcehook(this).state.borrow_mut().contexts.pop();

	assert_eq!(
		popped.map(|popped| popped.cast::<IHookContext>()),
		Some(context)
	);

	// SAFETY: `setup_hook_loop` allocated it.
	drop(unsafe { Box::from_raw(context.cast::<MockContext>()) });
}

unsafe extern "C" fn get_iface_ptr(this: *mut ISourceHook) -> *mut c_void {
	mock_sourcehook(this).top_context().this
}

unsafe extern "C" fn get_orig_ret(this: *mut ISourceHook) -> *const c_void {
	mock_sourcehook(this).top_context().orig_ret
}

unsafe extern "C" fn get_orig_vfn_ptr_entry(
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

unsafe extern "C" fn get_override_ret(this: *mut ISourceHook) -> *const c_void {
	let context = mock_sourcehook(this).top_context();

	// SAFETY: The hook function's status outlives its loop.
	match unsafe { context.status.read() } >= MetaRes::OVERRIDE {
		true => context.override_ret,
		false => ptr::null(),
	}
}

unsafe extern "C" fn get_override_ret_ptr(this: *mut ISourceHook) -> *mut c_void {
	mock_sourcehook(this).top_context().override_ret
}

unsafe extern "C" fn get_prev_res(_this: *mut ISourceHook) -> MetaRes {
	MetaRes::IGNORED
}

unsafe extern "C" fn get_status(this: *mut ISourceHook) -> MetaRes {
	// SAFETY: As for `get_override_ret`.
	unsafe { mock_sourcehook(this).top_context().status.read() }
}

unsafe extern "C" fn hook_by_id(_this: *mut ISourceHook, _id: c_int) -> bool {
	false
}

unsafe extern "C" fn iface_version(_this: *mut ISourceHook) -> c_int {
	5
}

unsafe extern "C" fn ignore_hooks(_this: *mut ISourceHook, _vfnptr: *mut c_void) {}

unsafe extern "C" fn impl_version(_this: *mut ISourceHook) -> c_int {
	5
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

unsafe extern "C" fn log_debug(_this: *mut ISourceHook, _format: *const c_char) {}

fn mock_sourcehook<'a>(this: *mut ISourceHook) -> &'a MockSourceHook {
	// SAFETY: The tests' `ISourceHook` is a mock one, which outlives its calls.
	unsafe { &*this.cast::<MockSourceHook>() }
}

unsafe extern "C" fn remove_hook(
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

unsafe extern "C" fn remove_hook_manager(
	_this: *mut ISourceHook,
	_plugin: Plugin,
	_hook_manager: HookManagerPubFunc,
) {
}

unsafe extern "C" fn set_res(this: *mut ISourceHook, res: MetaRes) {
	// SAFETY: As for `get_override_ret`.
	unsafe { mock_sourcehook(this).top_context().current.write(res) };
}

#[allow(
	clippy::too_many_arguments,
	reason = "It stands for a C++ method taking them"
)]
unsafe extern "C" fn setup_hook_loop(
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
	state.loops += 1;
	context.cast()
}
