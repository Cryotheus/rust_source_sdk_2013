//! The command object the engine sees as a `ConCommand`.

use super::error::{
	CommandBaseKind, InvalidCommandName, RegisterCommandError, RegisterCommandErrorKind,
	UnregisterCommandError, UnregisterCommandErrorKind, validate_name,
};
use super::registrar::{CommandRegistrar, UnlinksBeforeUnload};
use super::{CommandAccess, CommandContext, CommandFlags, CommandHandler, CommandResult, route};
use crate::abi::CppDestructors;
use crate::ffi::{NotThreadSafe, vcall};
use crate::server::{Server, ServerBinding};
use std::cell::{Cell, UnsafeCell};
use std::ffi::{CStr, c_char, c_int, c_void};
use std::marker::{PhantomData, PhantomPinned};
use std::mem::{MaybeUninit, offset_of, size_of};
use std::pin::Pin;
use std::ptr::{self, NonNull};

/// A console command implemented in Rust, laid out so the engine sees a
/// `ConCommand`.
///
/// The engine, its host, and other plugins keep a registered command's
/// address and write its C++ fields through their own pointers, so a command
/// is registered pinned and `'static`, and Rust only reaches those fields
/// through raw pointers.
///
/// Commands are usually `static`s:
///
/// ```
/// use source_sdk_2013::commands::{CommandAccess, CommandContext, CommandFn, CommandResult, ConsoleCommand};
///
/// static PING: ConsoleCommand<CommandFn> = ConsoleCommand::new(c"sb_ping", ping as CommandFn)
///     .help(c"Replies with pong.")
///     .access(CommandAccess::Everyone);
///
/// fn ping(command: &CommandContext<'_>) -> CommandResult {
///     command.reply("pong")?;
///     Ok(())
/// }
/// ```
///
/// A command is `Sync` when its handler is: everything the engine or Rust
/// writes after construction is only touched on the server's main thread,
/// either by the engine or through a [`Server`], which only exists there.
#[repr(C)]
pub struct ConsoleCommand<H> {
	header: CommandHeader,
	handler: H,
}

// SAFETY: The interior-mutable header (the C++ fields and the `Cell`s) is only
// accessed by the engine, which calls commands on its main thread, and by
// methods taking a `Server`, whose contract confines it to that thread. Other
// threads can only reach the immutable fields and `&H`, which `H: Sync` covers.
unsafe impl<H: Sync> Sync for ConsoleCommand<H> {}

/// The type-erased start of every [`ConsoleCommand`]. `ConCommand *`,
/// `CommandHeader *`, and `ConsoleCommand<H> *` share an address.
#[repr(C)]
pub(super) struct CommandHeader {
	/// The engine-visible `ConCommand`. C++ writes its list link, registered
	/// flag, and flags at any time, including while Rust holds a reference to
	/// the command, so Rust never forms a reference into it.
	raw: UnsafeCell<sys::ConCommand>,

	/// Runs the handler of the `ConsoleCommand<H>` this header starts.
	dispatch: DispatchFn,

	name: &'static CStr,
	help: &'static CStr,
	access: CommandAccess,
	flags: CommandFlags,

	/// The server the command was last registered with, for calls from the
	/// engine.
	binding: Cell<Option<ServerBinding>>,

	/// Returned from `GetDLLIdentifier`; allocated at registration.
	dll_identifier: Cell<sys::CVarDLLIdentifier_t>,

	_pinned: PhantomPinned,
	_not_thread_safe: NotThreadSafe,
}

type DispatchFn = unsafe fn(NonNull<CommandHeader>, &CommandContext<'_>) -> CommandResult;

const _: () = {
	assert!(offset_of!(CommandHeader, raw) == 0);
	assert!(offset_of!(ConsoleCommand<()>, header) == 0);
	assert!(offset_of!(sys::ConCommand, _base) == 0);
};

impl<H: CommandHandler> ConsoleCommand<H> {
	/// Creates an unregistered command that server-side invokers may run.
	///
	/// # Panics
	///
	/// If [`validate_name`] rejects `name`. For a `static`, this is a
	/// compile-time error.
	pub const fn new(name: &'static CStr, handler: H) -> Self {
		match validate_name(name) {
			Ok(()) => {}
			Err(InvalidCommandName::Empty) => panic!("console command names cannot be empty"),
			Err(InvalidCommandName::TooLong) => {
				panic!("console command names cannot be longer than 63 bytes")
			}
			Err(InvalidCommandName::InvalidByte { .. }) => {
				panic!("console command names may only contain ASCII letters, digits, and `_`")
			}
			Err(InvalidCommandName::Reserved) => {
				panic!("the engine reserves this name for its own client commands")
			}
		}

		Self {
			header: CommandHeader {
				// SAFETY: Zero is valid for every field of `ConCommand`: null
				// pointers, absent callbacks, `false`, and 0. The engine only sees
				// the object once registration has filled it in.
				raw: UnsafeCell::new(unsafe { MaybeUninit::zeroed().assume_init() }),
				dispatch: dispatch_erased::<H>,
				name,
				help: c"",
				access: CommandAccess::Server,
				flags: CommandFlags::NONE,
				binding: Cell::new(None),
				dll_identifier: Cell::new(0),
				_pinned: PhantomPinned,
				_not_thread_safe: PhantomData,
			},
			handler,
		}
	}

	/// Sets the text `help <name>` shows.
	pub const fn help(mut self, help: &'static CStr) -> Self {
		self.header.help = help;
		self
	}

	pub const fn access(mut self, access: CommandAccess) -> Self {
		self.header.access = access;
		self
	}

	pub const fn flags(mut self, flags: CommandFlags) -> Self {
		self.header.flags = flags;
		self
	}

	/// Registers the command with the engine through a registrar whose host
	/// unlinks it before the plugin is unloaded, such as Metamod:Source's.
	///
	/// Server-side invokers can run the command from now on. Clients can run
	/// it once the plugin passes `IServerGameClients::ClientCommand` calls to
	/// [`route_client_command`](super::route_client_command).
	///
	/// Later calls from the engine turn `binding` into a [`Server`] for the
	/// handler.
	pub fn register(
		self: Pin<&'static Self>,
		server: Server<'_>,
		binding: ServerBinding,
		registrar: &impl UnlinksBeforeUnload,
	) -> Result<(), RegisterCommandError> {
		// SAFETY: The registrar's host unlinks the command before the plugin's
		// code is unloaded.
		unsafe { self.register_unmanaged(server, binding, registrar) }
	}

	/// Registers the command through any registrar.
	///
	/// Before registering, this checks that the command is not registered
	/// already and that no other command or variable uses its name. Afterwards,
	/// it checks that the engine lists the command under its name, since
	/// neither the engine nor Metamod reports failure.
	///
	/// # Safety
	///
	/// The command must be unregistered before the module containing this
	/// crate's code or the handler's is unloaded.
	pub unsafe fn register_unmanaged(
		self: Pin<&'static Self>,
		server: Server<'_>,
		binding: ServerBinding,
		registrar: &impl CommandRegistrar,
	) -> Result<(), RegisterCommandError> {
		let header = &self.get_ref().header;
		let error = |kind| RegisterCommandError::new(header.name, kind);
		let cvar = server.cvar().map_err(|interface| error(interface.into()))?;

		// Metamod clears the list link before linking, so linking a listed
		// command again would cut off every command after it.
		if header.is_registered() {
			return Err(error(RegisterCommandErrorKind::AlreadyRegistered));
		}

		// The engine does not refuse a duplicate command name: it lists the new
		// command first, which hides the existing one.
		if let Some(existing) = cvar.find_command_base(header.name) {
			// SAFETY: The engine's registry holds live commands and variables.
			let kind = if unsafe { vcall!(existing.as_ptr() => ConCommandBase_IsCommand()) } {
				CommandBaseKind::Command
			} else {
				CommandBaseKind::Variable
			};

			return Err(error(RegisterCommandErrorKind::NameTaken(kind)));
		}

		header.prepare(binding, cvar.allocate_dll_identifier());

		let command = self.get_ref().as_base();

		// SAFETY: The command is pinned, `'static`, and prepared, and the
		// server's existence confines this call to the main thread.
		unsafe { registrar.link(command) };

		if header.is_registered() && cvar.find_command_base(header.name) == Some(command) {
			return Ok(());
		}

		// Unlinking also stops a host like Metamod from tracking the command.
		// SAFETY: As for `link`.
		unsafe { registrar.unlink(command) };

		Err(error(RegisterCommandErrorKind::NotLinked))
	}

	/// Unregisters the command, which then no longer runs.
	///
	/// A handler may unregister its own command.
	pub fn unregister(
		&self,
		server: Server<'_>,
		registrar: &impl CommandRegistrar,
	) -> Result<(), UnregisterCommandError> {
		let header = &self.header;
		let error = |kind| UnregisterCommandError::new(header.name, kind);

		if !header.is_registered() {
			return Err(error(UnregisterCommandErrorKind::NotRegistered));
		}

		let cvar = server.cvar().map_err(|interface| error(interface.into()))?;
		let command = self.as_base();

		// SAFETY: Only `register` makes the engine list a command, and it takes
		// the command pinned and `'static`. The server confines this call to the
		// main thread.
		unsafe { registrar.unlink(command) };

		if header.is_registered() || cvar.find_command_base(header.name) == Some(command) {
			return Err(error(UnregisterCommandErrorKind::StillLinked));
		}

		Ok(())
	}
}

impl<H> ConsoleCommand<H> {
	/// The name the command is registered under.
	pub const fn name(&self) -> &'static CStr {
		self.header.name
	}

	pub const fn handler(&self) -> &H {
		&self.handler
	}

	/// A pointer to the whole object, which the engine passes back to every
	/// vtable slot.
	fn as_base(&self) -> NonNull<sys::ConCommandBase> {
		NonNull::from(self).cast()
	}
}

impl<H: std::fmt::Debug> std::fmt::Debug for ConsoleCommand<H> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("ConsoleCommand")
			.field("name", &self.header.name)
			.field("help", &self.header.help)
			.field("access", &self.header.access)
			.field("flags", &self.header.flags)
			.field("handler", &self.handler)
			.finish_non_exhaustive()
	}
}

/// Runs the handler of the `ConsoleCommand<H>` that `header` starts.
///
/// # Safety
///
/// `header` must be the header of a live `ConsoleCommand<H>`.
unsafe fn dispatch_erased<H: CommandHandler>(
	header: NonNull<CommandHeader>,
	context: &CommandContext<'_>,
) -> CommandResult {
	// SAFETY: The caller guarantees `header` starts a `ConsoleCommand<H>`,
	// which is `repr(C)` with the header first.
	let command = unsafe { header.cast::<ConsoleCommand<H>>().as_ref() };

	command.handler.dispatch(context)
}

impl CommandHeader {
	pub(super) const fn name(&self) -> &'static CStr {
		self.name
	}

	pub(super) const fn access(&self) -> CommandAccess {
		self.access
	}

	pub(super) fn binding(&self) -> Option<ServerBinding> {
		self.binding.get()
	}

	/// The flags the engine currently sees, which other plugins may change.
	pub(super) fn current_flags(&self) -> c_int {
		// SAFETY: Fields are read through the cell without forming references.
		unsafe { (&raw const (*self.raw.get())._base.m_nFlags).read() }
	}

	/// Whether the engine has marked the command registered.
	fn is_registered(&self) -> bool {
		// SAFETY: As for `current_flags`.
		unsafe { (&raw const (*self.raw.get())._base.m_bRegistered).read() }
	}

	/// Fills in the engine-visible fields before linking.
	fn prepare(&self, binding: ServerBinding, dll_identifier: sys::CVarDLLIdentifier_t) {
		let base = self.raw.get().cast::<sys::ConCommandBase>();

		// SAFETY: The command is not registered, so nothing else accesses these
		// fields, and they are written through the cell without forming
		// references. The callbacks and completion fields stay zero; only
		// tier1's own `ConCommand::Dispatch` reads them.
		unsafe {
			(&raw mut (*base).vtable_).write(vtable().cast());
			(&raw mut (*base).m_pNext).write(ptr::null_mut());
			(&raw mut (*base).m_pszName).write(self.name.as_ptr());
			(&raw mut (*base).m_pszHelpString).write(self.help.as_ptr());
			(&raw mut (*base).m_nFlags).write(self.flags.bits());
		}

		self.binding.set(Some(binding));
		self.dll_identifier.set(dll_identifier);
	}

	/// Recognizes a command this copy of the crate registered, by its vtable.
	///
	/// # Safety
	///
	/// `base` must point to a live `ConCommandBase`, such as one the engine's
	/// registry returned.
	pub(super) unsafe fn from_registered(
		base: NonNull<sys::ConCommandBase>,
	) -> Option<RegisteredCommand> {
		// SAFETY: The caller guarantees the object is live.
		let vtable_pointer = unsafe { (&raw const (*base.as_ptr()).vtable_).read() };

		// Only `prepare` stores this vtable, for a command `register` took
		// pinned and `'static`. The pointer came from the engine, which was
		// given one to the whole command.
		(vtable_pointer.cast::<c_void>() == vtable().cast()).then(|| RegisteredCommand(base.cast()))
	}

	/// # Safety
	///
	/// `this` must be a pointer the engine passes to a slot of [`VTABLE`].
	unsafe fn from_this(this: *const sys::ConCommand) -> RegisteredCommand {
		// SAFETY: Only prepared commands use the vtable, and the engine passes
		// back the pointer it was given, to the whole command.
		RegisteredCommand(unsafe { NonNull::new_unchecked(this.cast_mut().cast()) })
	}
}

/// A registered command, reached through the pointer the engine was given.
///
/// That pointer covers the whole `ConsoleCommand<H>`, which the handler needs;
/// one derived from a `&CommandHeader` would only cover the header.
#[derive(Clone, Copy)]
pub(super) struct RegisteredCommand(NonNull<CommandHeader>);

impl RegisteredCommand {
	/// Runs the handler.
	pub(super) fn dispatch(self, context: &CommandContext<'_>) -> CommandResult {
		// SAFETY: `dispatch` was chosen for the handler type of the command
		// this header starts, and the pointer covers the whole command, which
		// is `'static`.
		unsafe { (self.dispatch)(self.0, context) }
	}
}

impl std::ops::Deref for RegisteredCommand {
	type Target = CommandHeader;

	fn deref(&self) -> &CommandHeader {
		// SAFETY: Registered commands are `'static` and never mutably borrowed.
		unsafe { self.0.as_ref() }
	}
}

/// The `ConCommand` vtable, in the order of `public/tier1/convar.h`.
#[repr(C)]
struct ConCommandVtable {
	destructors: CppDestructors,
	is_command: unsafe extern "C" fn(this: *const sys::ConCommand) -> bool,
	is_flag_set: unsafe extern "C" fn(this: *const sys::ConCommand, flag: c_int) -> bool,
	add_flags: unsafe extern "C" fn(this: *mut sys::ConCommand, flags: c_int),
	get_name: unsafe extern "C" fn(this: *const sys::ConCommand) -> *const c_char,
	get_help_text: unsafe extern "C" fn(this: *const sys::ConCommand) -> *const c_char,
	is_registered: unsafe extern "C" fn(this: *const sys::ConCommand) -> bool,
	get_dll_identifier:
		unsafe extern "C" fn(this: *const sys::ConCommand) -> sys::CVarDLLIdentifier_t,
	create_base: unsafe extern "C" fn(
		this: *mut sys::ConCommand,
		name: *const c_char,
		help: *const c_char,
		flags: c_int,
	),
	init: unsafe extern "C" fn(this: *mut sys::ConCommand),
	auto_complete_suggest: unsafe extern "C" fn(
		this: *mut sys::ConCommand,
		partial: *const c_char,
		commands: *mut c_void,
	) -> c_int,
	can_auto_complete: unsafe extern "C" fn(this: *mut sys::ConCommand) -> bool,
	dispatch: unsafe extern "C" fn(this: *mut sys::ConCommand, command: *const sys::CCommand),
}

// Every slot must line up with the engine's declaration under each ABI.
const _: () = {
	use sys::ConCommand__bindgen_vtable as Engine;

	assert!(size_of::<ConCommandVtable>() == size_of::<Engine>());
	assert!(offset_of!(ConCommandVtable, is_command) == offset_of!(Engine, ConCommand_IsCommand));
	assert!(offset_of!(ConCommandVtable, is_flag_set) == offset_of!(Engine, ConCommand_IsFlagSet));
	assert!(offset_of!(ConCommandVtable, add_flags) == offset_of!(Engine, ConCommand_AddFlags));
	assert!(offset_of!(ConCommandVtable, get_name) == offset_of!(Engine, ConCommand_GetName));
	assert!(
		offset_of!(ConCommandVtable, get_help_text) == offset_of!(Engine, ConCommand_GetHelpText)
	);
	assert!(
		offset_of!(ConCommandVtable, is_registered) == offset_of!(Engine, ConCommand_IsRegistered)
	);
	assert!(
		offset_of!(ConCommandVtable, get_dll_identifier)
			== offset_of!(Engine, ConCommand_GetDLLIdentifier)
	);
	assert!(offset_of!(ConCommandVtable, create_base) == offset_of!(Engine, ConCommand_CreateBase));
	assert!(offset_of!(ConCommandVtable, init) == offset_of!(Engine, ConCommand_Init));
	assert!(
		offset_of!(ConCommandVtable, auto_complete_suggest)
			== offset_of!(Engine, ConCommand_AutoCompleteSuggest)
	);
	assert!(
		offset_of!(ConCommandVtable, can_auto_complete)
			== offset_of!(Engine, ConCommand_CanAutoComplete)
	);
	assert!(offset_of!(ConCommandVtable, dispatch) == offset_of!(Engine, ConCommand_Dispatch));
};

/// The vtable every [`ConsoleCommand`] points at, alone on its page.
///
/// Other plugins hook a command by overwriting entries of its vtable in
/// place, so the table is interior-mutable, which also places it in writable
/// memory. KHook sets a page it patched back to read-and-execute, which would
/// fault the next write to any other static on that page; the alignment makes
/// the table fill its page. Rust never reads the table; the engine only gets
/// its address.
#[repr(C, align(4096))]
struct VtablePage(UnsafeCell<ConCommandVtable>);

// SAFETY: Rust never accesses the table after initialization, except to take
// its address. Only the engine and hooking libraries read or write it, on the
// server's main thread.
unsafe impl Sync for VtablePage {}

static VTABLE: VtablePage = VtablePage(UnsafeCell::new(ConCommandVtable {
	destructors: CppDestructors::new_noop(),
	is_command,
	is_flag_set,
	add_flags,
	get_name,
	get_help_text,
	is_registered,
	get_dll_identifier,
	create_base,
	init,
	auto_complete_suggest,
	can_auto_complete,
	dispatch,
}));

const _: () = assert!(size_of::<VtablePage>() == 4096);

fn vtable() -> *const ConCommandVtable {
	VTABLE.0.get()
}

// The slots below are only called by the engine and other plugins, on the
// server's main thread, with `this` pointing to a prepared command. They read
// the C++ fields without forming references, since C++ writes them too.

unsafe extern "C" fn is_command(_this: *const sys::ConCommand) -> bool {
	true
}

/// Never reports `FCVAR_GAMEDLL`, even if another plugin writes it into the
/// flags, so the engine never dispatches a client's invocation directly.
unsafe extern "C" fn is_flag_set(this: *const sys::ConCommand, flag: c_int) -> bool {
	// SAFETY: See above.
	let flags = unsafe { (&raw const (*this)._base.m_nFlags).read() };

	flags & flag & !CommandFlags::GAME_DLL != 0
}

unsafe extern "C" fn add_flags(this: *mut sys::ConCommand, flags: c_int) {
	// SAFETY: See above.
	unsafe {
		let field = &raw mut (*this)._base.m_nFlags;

		field.write(field.read() | (flags & !CommandFlags::GAME_DLL));
	}
}

unsafe extern "C" fn get_name(this: *const sys::ConCommand) -> *const c_char {
	// SAFETY: See above.
	unsafe { (&raw const (*this)._base.m_pszName).read() }
}

unsafe extern "C" fn get_help_text(this: *const sys::ConCommand) -> *const c_char {
	// SAFETY: See above.
	unsafe { (&raw const (*this)._base.m_pszHelpString).read() }
}

unsafe extern "C" fn is_registered(this: *const sys::ConCommand) -> bool {
	// SAFETY: See above.
	unsafe { (&raw const (*this)._base.m_bRegistered).read() }
}

unsafe extern "C" fn get_dll_identifier(this: *const sys::ConCommand) -> sys::CVarDLLIdentifier_t {
	// SAFETY: See above.
	unsafe { CommandHeader::from_this(this) }
		.dll_identifier
		.get()
}

/// Only the tier1 constructors of the module owning a command call this.
unsafe extern "C" fn create_base(
	_this: *mut sys::ConCommand,
	_name: *const c_char,
	_help: *const c_char,
	_flags: c_int,
) {
}

/// Only tier1's registration of the module owning a command calls this.
unsafe extern "C" fn init(_this: *mut sys::ConCommand) {}

/// Suggestions go into a `CUtlVector` that grows through tier0's allocator,
/// which Rust does not use, so commands offer none.
unsafe extern "C" fn auto_complete_suggest(
	_this: *mut sys::ConCommand,
	_partial: *const c_char,
	_commands: *mut c_void,
) -> c_int {
	0
}

unsafe extern "C" fn can_auto_complete(_this: *mut sys::ConCommand) -> bool {
	false
}

/// The engine calls this for every invocation that is not a client's string
/// command.
unsafe extern "C" fn dispatch(this: *mut sys::ConCommand, command: *const sys::CCommand) {
	// SAFETY: See above.
	let registered = unsafe { CommandHeader::from_this(this) };

	// SAFETY: The engine passes the command being run for this call, on the
	// main thread.
	unsafe { route::dispatch_from_engine(registered, command) };
}
