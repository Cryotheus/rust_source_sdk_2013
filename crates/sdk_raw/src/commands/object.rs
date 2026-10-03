//! The object the engine sees as a `ConCommand`, with its vtable and the
//! thunks in it.

use super::FCVAR_GAMEDLL;
use crate::abi::VtablePage;
use crate::vtable_slot;
use std::cell::{Cell, UnsafeCell};
use std::ffi::{CStr, c_char, c_int};

#[cfg(target_os = "windows")]
use std::ffi::{c_uint, c_void};

use std::marker::{PhantomData, PhantomPinned};
use std::mem::{MaybeUninit, offset_of};
use std::ptr::{self, NonNull};

const _: () = {
	use sys::ConCommand__bindgen_vtable as Vtable;
	use sys::ConCommandBase__bindgen_vtable as Base;

	assert!(offset_of!(ConCommandObject, raw) == 0);
	assert!(offset_of!(sys::ConCommand, _base) == 0);

	// The engine calls commands through `ConCommandBase`'s slots, which start
	// `ConCommand`'s table.
	assert!(
		vtable_slot!(Vtable, ConCommand_IsCommand) == vtable_slot!(Base, ConCommandBase_IsCommand)
	);
	assert!(vtable_slot!(Vtable, ConCommand_Init) == vtable_slot!(Base, ConCommandBase_Init));
};

/// The vtable every prepared [`ConCommandObject`] points at, alone on its
/// page, since other plugins hook a command by overwriting entries of its
/// vtable in place. Rust never reads the table; the engine only gets its
/// address.
static VTABLE: VtablePage<sys::ConCommand__bindgen_vtable> =
	VtablePage::new(sys::ConCommand__bindgen_vtable {
		#[cfg(target_os = "windows")]
		ConCommand_destructor: destructor,
		#[cfg(not(target_os = "windows"))]
		ConCommand_complete_destructor: destructor,
		#[cfg(not(target_os = "windows"))]
		ConCommand_deleting_destructor: destructor,
		ConCommand_IsCommand: is_command,
		ConCommand_IsFlagSet: is_flag_set,
		ConCommand_AddFlags: add_flags,
		ConCommand_GetName: get_name,
		ConCommand_GetHelpText: get_help_text,
		ConCommand_IsRegistered: is_registered,
		ConCommand_GetDLLIdentifier: get_dll_identifier,
		ConCommand_CreateBase: create_base,
		ConCommand_Init: init,
		ConCommand_AutoCompleteSuggest: auto_complete_suggest,
		ConCommand_CanAutoComplete: can_auto_complete,
		ConCommand_Dispatch: dispatch,
	});

/// What the engine's calls to a [`ConCommandObject`] run, chosen by its
/// owner.
///
/// The hooks' address identifies the owner's commands, for
/// [`ConCommandObject::from_registered`], so an owner keeps them in a
/// `static` of its own.
#[derive(Debug)]
pub struct ConCommandHooks {
	/// Runs the command for the engine's call to its `Dispatch`, which the
	/// engine makes for every invocation that is not a client's string
	/// command.
	///
	/// It is called on the thread the engine calls commands on, with the
	/// pointer to the command the engine was handed and the `CCommand` the
	/// engine is running, which is live for the call. It must not unwind: a
	/// panic aborts the process.
	pub dispatch: unsafe fn(command: NonNull<ConCommandObject>, args: NonNull<sys::CCommand>),
}

/// A console command Rust implements, laid out so the engine sees a
/// `ConCommand` once [prepared](Self::prepare).
///
/// It is the first field of its owner's `#[repr(C)]` container, and runs the
/// owner's [`ConCommandHooks`] for the engine's calls (see the
/// [module documentation](super)). The engine, its host, and other plugins
/// keep a registered command's address and write its C++ fields through their
/// own pointers at any time, so Rust only reaches those fields through raw
/// pointers.
///
/// The command never carries or reports [`FCVAR_GAMEDLL`], even if another
/// plugin writes it into the flags, so the engine never dispatches a client's
/// invocation directly: it hands it to `IServerGameClients::ClientCommand`,
/// which tells who ran it, instead.
#[doc(alias("ConCommand"))]
#[repr(C)]
pub struct ConCommandObject {
	/// The engine-visible `ConCommand`. C++ writes its list link, registered
	/// flag, and flags at any time, so Rust never forms a reference into it.
	raw: UnsafeCell<sys::ConCommand>,

	/// What the engine's calls run. Its address identifies the owner.
	hooks: &'static ConCommandHooks,

	/// Returned from `GetDLLIdentifier`; given by `prepare`.
	dll_identifier: Cell<sys::CVarDLLIdentifier_t>,

	_pinned: PhantomPinned,
	_not_thread_safe: PhantomData<*mut ()>,
}

impl ConCommandObject {
	/// Creates an unprepared command that runs `hooks`.
	pub const fn new(hooks: &'static ConCommandHooks) -> Self {
		Self {
			// SAFETY: Zero is valid for every field of `ConCommand`: null
			// pointers, absent callbacks, `false`, and 0. The engine only sees
			// the object once `prepare` has filled it in.
			raw: UnsafeCell::new(unsafe { MaybeUninit::zeroed().assume_init() }),
			hooks,
			dll_identifier: Cell::new(0),
			_pinned: PhantomPinned,
			_not_thread_safe: PhantomData,
		}
	}

	/// The pointer to hand the engine, which it passes back to every slot.
	///
	/// `this` should point to the owner's whole container, so the hooks can
	/// reach the rest of it through the pointer.
	pub const fn as_base(this: NonNull<Self>) -> NonNull<sys::ConCommandBase> {
		this.cast()
	}

	/// Recognizes a prepared command that was created with `hooks`, by its
	/// vtable and its hooks' address, returning a pointer to it with the
	/// provenance of `base`.
	///
	/// Commands of other owners, of other copies of this crate, and of C++
	/// are not recognized. An owner that only creates commands with its
	/// `hooks` in its own container may therefore cast the result to that
	/// container, if `base` is the pointer it handed the engine.
	///
	/// # Safety
	///
	/// `base` must point to a live `ConCommandBase`, such as one the engine's
	/// registry returned.
	pub unsafe fn from_registered(
		base: NonNull<sys::ConCommandBase>,
		hooks: &'static ConCommandHooks,
	) -> Option<NonNull<Self>> {
		// SAFETY: The caller guarantees the object is live.
		let vtable = unsafe { (&raw const (*base.as_ptr()).vtable_).read() };

		// Only `prepare` stores this vtable, in a `ConCommandObject`.
		if vtable.cast::<sys::ConCommand__bindgen_vtable>() != VTABLE.get().cast_const() {
			return None;
		}

		let object = base.cast::<Self>();

		// SAFETY: The object is a live `ConCommandObject`, whose hooks are read
		// without forming a reference to it.
		let own = unsafe { (&raw const (*object.as_ptr()).hooks).read() };

		ptr::eq(own, hooks).then_some(object)
	}

	/// The flags the engine currently sees, which other plugins may change.
	#[doc(alias("m_nFlags"))]
	pub fn flags(&self) -> c_int {
		// SAFETY: The field is read through the cell without forming a
		// reference. `prepare`'s contract keeps C++ from writing it on another
		// thread.
		unsafe { (&raw const (*self.raw.get())._base.m_nFlags).read() }
	}

	/// Fills in the engine-visible fields before the command is linked into
	/// the engine's registry: its vtable, a cleared list link, its name, its
	/// help text, its flags without [`FCVAR_GAMEDLL`], and the DLL identifier
	/// the registry allocated.
	///
	/// # Safety
	///
	/// The command must not be registered, so nothing else accesses its C++
	/// fields. Once prepared, it may be handed to the engine through
	/// [`Self::as_base`], after which, until the engine no longer lists it, it
	/// must stay where it is, the module containing its hooks' code must stay
	/// loaded, and it must only be accessed on the thread the engine calls
	/// commands on.
	pub unsafe fn prepare(
		&self,
		name: &'static CStr,
		help: &'static CStr,
		flags: c_int,
		dll_identifier: sys::CVarDLLIdentifier_t,
	) {
		let base = self.raw.get().cast::<sys::ConCommandBase>();

		// SAFETY: The command is not registered, so nothing else accesses these
		// fields, and they are written through the cell without forming
		// references. The callbacks and completion fields stay zero; only
		// tier1's own `ConCommand::Dispatch` reads them. The engine also reads
		// the flags directly, not only through `IsFlagSet`.
		unsafe {
			(&raw mut (*base).vtable_).write(VTABLE.get().cast_const().cast());
			(&raw mut (*base).m_pNext).write(ptr::null_mut());
			(&raw mut (*base).m_pszName).write(name.as_ptr());
			(&raw mut (*base).m_pszHelpString).write(help.as_ptr());
			(&raw mut (*base).m_nFlags).write(flags & !FCVAR_GAMEDLL);
		}

		self.dll_identifier.set(dll_identifier);
	}
}

// The slots below are only called by the engine and other plugins, on the
// thread the engine calls commands on, with `this` pointing to a prepared
// command. They read the C++ fields without forming references, since C++
// writes them too.

unsafe extern "C" fn add_flags(this: *mut sys::ConCommand, flags: c_int) {
	// SAFETY: See above.
	unsafe {
		let field = &raw mut (*this)._base.m_nFlags;

		field.write(field.read() | (flags & !FCVAR_GAMEDLL));
	}
}

/// Suggestions go into a `CUtlVector` that grows through tier0's allocator,
/// which Rust does not use, so commands offer none.
unsafe extern "C" fn auto_complete_suggest(
	_this: *mut sys::ConCommand,
	_partial: *const c_char,
	_commands: *mut sys::CUtlVector<sys::CUtlString, sys::CUtlMemory<sys::CUtlString>>,
) -> c_int {
	0
}

unsafe extern "C" fn can_auto_complete(_this: *mut sys::ConCommand) -> bool {
	false
}

/// Only the tier1 constructors of the module owning a command call this.
unsafe extern "C" fn create_base(
	_this: *mut sys::ConCommand,
	_name: *const c_char,
	_help: *const c_char,
	_flags: c_int,
) {
}

/// A command belongs to its Rust owner, so C++ destroying or deleting it
/// leaves it intact.
#[cfg(target_os = "windows")]
unsafe extern "C" fn destructor(this: *mut sys::ConCommand, _flags: c_uint) -> *mut c_void {
	this.cast()
}

/// As the Windows version, for the Itanium ABI's complete and deleting
/// destructors.
#[cfg(not(target_os = "windows"))]
unsafe extern "C" fn destructor(_this: *mut sys::ConCommand) {}

/// The engine calls this for every invocation that is not a client's string
/// command.
unsafe extern "C" fn dispatch(this: *mut sys::ConCommand, command: *const sys::CCommand) {
	// The engine always passes the command it runs.
	let (Some(object), Some(command)) = (NonNull::new(this), NonNull::new(command.cast_mut()))
	else {
		return;
	};

	let object = object.cast::<ConCommandObject>();

	// SAFETY: See above. The hooks are read without forming a reference.
	let hooks = unsafe { (&raw const (*object.as_ptr()).hooks).read() };

	// SAFETY: The engine passes back the pointer it was handed, and the
	// command it runs, live for the call, on its thread.
	unsafe { (hooks.dispatch)(object, command) };
}

unsafe extern "C" fn get_dll_identifier(this: *const sys::ConCommand) -> sys::CVarDLLIdentifier_t {
	// SAFETY: See above. The identifier lies outside the C++ fields.
	unsafe { (*this.cast::<ConCommandObject>()).dll_identifier.get() }
}

unsafe extern "C" fn get_help_text(this: *const sys::ConCommand) -> *const c_char {
	// SAFETY: See above.
	unsafe { (&raw const (*this)._base.m_pszHelpString).read() }
}

unsafe extern "C" fn get_name(this: *const sys::ConCommand) -> *const c_char {
	// SAFETY: See above.
	unsafe { (&raw const (*this)._base.m_pszName).read() }
}

/// Only tier1's registration of the module owning a command calls this.
unsafe extern "C" fn init(_this: *mut sys::ConCommand) {}

unsafe extern "C" fn is_command(_this: *const sys::ConCommand) -> bool {
	true
}

/// Never reports [`FCVAR_GAMEDLL`], even if another plugin writes it into the
/// flags.
unsafe extern "C" fn is_flag_set(this: *const sys::ConCommand, flag: c_int) -> bool {
	// SAFETY: See above.
	let flags = unsafe { (&raw const (*this)._base.m_nFlags).read() };

	flags & flag & !FCVAR_GAMEDLL != 0
}

unsafe extern "C" fn is_registered(this: *const sys::ConCommand) -> bool {
	// SAFETY: See above.
	unsafe { (&raw const (*this)._base.m_bRegistered).read() }
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::commands::{FCVAR_DONTRECORD, FCVAR_HIDDEN, tokenized};
	use std::cell::RefCell;
	use std::ptr::null_mut;

	static HOOKS: ConCommandHooks = ConCommandHooks { dispatch: record };
	static OTHER_HOOKS: ConCommandHooks = ConCommandHooks { dispatch: record };

	thread_local! {
		/// The commands and argument counts the hooks saw.
		static DISPATCHED: RefCell<Vec<(*mut ConCommandObject, c_int)>> = const { RefCell::new(Vec::new()) };
	}

	#[test]
	fn destructors_and_completion_leave_the_command_intact() {
		let command = prepared(c"sb_ping", 0);
		let base = ConCommandObject::as_base(NonNull::from(&*command)).as_ptr();
		let this = base.cast::<sys::ConCommand>();
		let vtable = vtable(base);

		// SAFETY: The slots are called as the engine calls them, on a prepared
		// command.
		unsafe {
			#[cfg(target_os = "windows")]
			{
				assert_eq!(((*vtable).ConCommand_destructor)(this, 0), this.cast());
				assert_eq!(((*vtable).ConCommand_destructor)(this, 1), this.cast());
			}

			#[cfg(not(target_os = "windows"))]
			{
				((*vtable).ConCommand_complete_destructor)(this);
				((*vtable).ConCommand_deleting_destructor)(this);
			}

			((*vtable).ConCommand_CreateBase)(this, c"other".as_ptr(), c"".as_ptr(), FCVAR_GAMEDLL);
			((*vtable).ConCommand_Init)(this);
			assert_eq!(
				((*vtable).ConCommand_AutoCompleteSuggest)(this, c"sb".as_ptr(), null_mut()),
				0
			);
			assert!(!((*vtable).ConCommand_CanAutoComplete)(this));

			let args = tokenized_ping();

			((*vtable).ConCommand_Dispatch)(this, &raw const *args);
			assert_eq!(
				CStr::from_ptr(((*vtable).ConCommand_GetName)(this)),
				c"sb_ping"
			);
			assert_eq!(
				CStr::from_ptr(((*vtable).ConCommand_GetHelpText)(this)),
				c"Help."
			);
			assert_eq!(((*vtable).ConCommand_GetDLLIdentifier)(this), 7);
			assert!(((*vtable).ConCommand_IsCommand)(this));
			assert!(!((*vtable).ConCommand_IsRegistered)(this));

			// A null command never reaches the hooks.
			((*vtable).ConCommand_Dispatch)(this, ptr::null());
		}

		assert_eq!(DISPATCHED.take(), [(base.cast(), 1)]);
	}

	/// A prepared command, as the engine would get it.
	fn prepared(name: &'static CStr, flags: c_int) -> Box<ConCommandObject> {
		let command = Box::new(ConCommandObject::new(&HOOKS));

		// SAFETY: The command is not registered, and the tests only access it on
		// this thread.
		unsafe { command.prepare(name, c"Help.", flags, 7) };
		command
	}

	unsafe fn record(command: NonNull<ConCommandObject>, args: NonNull<sys::CCommand>) {
		// SAFETY: The command being run is live for the call.
		let argc = unsafe { (&raw const (*args.as_ptr()).m_nArgc).read() };

		DISPATCHED.with_borrow_mut(|dispatched| dispatched.push((command.as_ptr(), argc)));
	}

	#[test]
	fn registered_commands_are_recognized_by_their_hooks() {
		let command = prepared(c"sb_ping", 0);
		let base = ConCommandObject::as_base(NonNull::from(&*command));
		let unprepared = ConCommandObject::new(&HOOKS);

		// SAFETY: Both objects are live `ConCommandBase`s.
		unsafe {
			assert_eq!(
				ConCommandObject::from_registered(base, &HOOKS),
				Some(base.cast())
			);
			assert_eq!(ConCommandObject::from_registered(base, &OTHER_HOOKS), None);
			assert_eq!(
				ConCommandObject::from_registered(NonNull::from(&unprepared).cast(), &HOOKS),
				None
			);
		}
	}

	#[test]
	fn the_engine_never_sees_the_game_dll_flag() {
		let command = prepared(c"sb_ping", FCVAR_GAMEDLL | FCVAR_DONTRECORD);
		let base = ConCommandObject::as_base(NonNull::from(&*command)).as_ptr();
		let this = base.cast::<sys::ConCommand>();
		let vtable = vtable(base);

		assert_eq!(command.flags(), FCVAR_DONTRECORD);

		// SAFETY: As above.
		unsafe {
			((*vtable).ConCommand_AddFlags)(this, FCVAR_GAMEDLL | FCVAR_HIDDEN);
			assert!(((*vtable).ConCommand_IsFlagSet)(this, FCVAR_HIDDEN));
			assert!(!((*vtable).ConCommand_IsFlagSet)(this, FCVAR_GAMEDLL));

			// Another plugin may write the flags directly.
			(&raw mut (*base).m_nFlags).write(FCVAR_GAMEDLL);
			assert!(!((*vtable).ConCommand_IsFlagSet)(this, -1));
		}
	}

	#[test]
	fn the_vtable_fills_its_page() {
		let command = prepared(c"sb_ping", 0);
		let base = ConCommandObject::as_base(NonNull::from(&*command)).as_ptr();

		assert_eq!(vtable(base).addr() % 4096, 0);
		assert_eq!(vtable(base), VTABLE.get().cast_const());
	}

	/// A command line of one argument, as the engine tokenizes it.
	fn tokenized_ping() -> Box<sys::CCommand> {
		tokenized("sb_ping", &["sb_ping"], 0)
	}

	/// The engine's view of a command's vtable.
	fn vtable(base: *mut sys::ConCommandBase) -> *const sys::ConCommand__bindgen_vtable {
		// SAFETY: The tests pass prepared commands.
		unsafe { (&raw const (*base).vtable_).read() }.cast()
	}
}
