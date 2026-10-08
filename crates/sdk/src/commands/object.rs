//! The command object the engine sees as a `ConCommand`.

use super::error::{
	CommandBaseKind, InvalidCommandName, RegisterCommandError, RegisterCommandErrorKind,
	UnregisterCommandError, UnregisterCommandErrorKind, validate_name,
};

use super::registrar::{CommandRegistrar, UnlinksBeforeUnload};

use super::{
	Client, ClientFilter, CommandAccess, CommandContext, CommandFlags, CommandHandler,
	CommandResult, Invoker, route,
};

use crate::server::{Server, ServerBinding};
use sdk_raw::commands::{ConCommandHooks, ConCommandObject, is_registered};
use sdk_raw::vcall;
use std::cell::Cell;
use std::ffi::CStr;
use std::mem::offset_of;
use std::pin::Pin;
use std::ptr::NonNull;

/// Runs the handler of the command a type-erased header starts: a
/// [`dispatch_erased`] for the command's handler type.
type DispatchFn = unsafe fn(NonNull<CommandHeader>, &CommandContext<'_>) -> CommandResult;

const _: () = {
	assert!(offset_of!(CommandHeader, object) == 0);
	assert!(offset_of!(ConsoleCommand<()>, header) == 0);
};

/// What the engine's calls to every [`ConsoleCommand`] run. Only
/// [`ConsoleCommand::new`] creates objects with these hooks, so their address
/// tells this crate's commands apart.
static HOOKS: ConCommandHooks = ConCommandHooks {
	dispatch: engine_dispatch,
};

/// The type-erased start of every [`ConsoleCommand`]. `ConCommand *`,
/// `CommandHeader *`, and `ConsoleCommand<H> *` share an address.
#[repr(C)]
pub(super) struct CommandHeader {
	/// The engine-visible `ConCommand`, created with [`HOOKS`]. It makes the
	/// command neither `Send`, `Sync`, nor `Unpin`.
	object: ConCommandObject,

	/// Runs the handler of the `ConsoleCommand<H>` this header starts.
	dispatch: DispatchFn,

	name: &'static CStr,
	help: &'static CStr,
	access: CommandAccess,
	client_filter: Option<ClientFilter>,
	flags: CommandFlags,

	/// The server the command was last registered with, for calls from the
	/// engine.
	binding: Cell<Option<ServerBinding>>,
}

impl CommandHeader {
	/// Recognizes a command this copy of the crate registered.
	///
	/// # Safety
	///
	/// `base` must point to a live `ConCommandBase`, such as one the engine's
	/// registry returned.
	pub(super) unsafe fn from_registered(
		base: NonNull<sys::ConCommandBase>,
	) -> Option<RegisteredCommand> {
		// SAFETY: The caller guarantees the object is live.
		let object = unsafe { ConCommandObject::from_registered(base, &HOOKS) }?;

		// Only `ConsoleCommand::new` creates objects with `HOOKS`, each the start
		// of a header, and only `register` prepares one, taking the command
		// pinned and `'static`. The pointer came from the engine, which was
		// given one to the whole command.
		Some(RegisteredCommand(object.cast()))
	}

	pub(super) const fn access(&self) -> CommandAccess {
		self.access
	}

	/// Whether `client` may run the command: its access lets clients run it,
	/// and its filter, if any, accepts them.
	pub(super) fn accepts_client(&self, server: Server<'_>, client: Client<'_>) -> bool {
		self.access.allows_clients()
			&& self
				.client_filter
				.is_none_or(|filter| (filter.accepts)(server, client))
	}

	pub(super) fn binding(&self) -> Option<ServerBinding> {
		self.binding.get()
	}

	/// The flags the engine currently sees, which other plugins may change.
	pub(super) fn current_flags(&self) -> CommandFlags {
		CommandFlags::from_bits_retain(self.object.flags())
	}

	pub(super) const fn name(&self) -> &'static CStr {
		self.name
	}

	/// Fills in the engine-visible fields before linking.
	///
	/// # Safety
	///
	/// The command must be pinned, `'static`, and not registered.
	unsafe fn prepare(&self, binding: ServerBinding, dll_identifier: sys::CVarDLLIdentifier_t) {
		// SAFETY: The caller guarantees the command is not registered and stays
		// where it is, and it is only linked through a registrar, on the main
		// thread, and unlinked before its code is unloaded.
		unsafe {
			self.object
				.prepare(self.name, self.help, self.flags.bits(), dll_identifier)
		};

		self.binding.set(Some(binding));
	}
}

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
#[doc(alias("ConCommand"))]
#[repr(C)]
pub struct ConsoleCommand<H> {
	header: CommandHeader,
	handler: H,
}

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
				object: ConCommandObject::new(&HOOKS),
				dispatch: dispatch_erased::<H>,
				name,
				help: c"",
				access: CommandAccess::Server,
				client_filter: None,
				flags: CommandFlags::NONE,
				binding: Cell::new(None),
			},
			handler,
		}
	}

	/// Sets who may run the command, [`CommandAccess::Server`] by default.
	pub const fn access(mut self, access: CommandAccess) -> Self {
		self.header.access = access;
		self
	}

	/// Narrows the clients [`access`](Self::access) lets run the command to
	/// those `filter` accepts, such as a plugin's admins. Every client may run
	/// it by default.
	///
	/// Clients the filter refuses are told the command is unknown, as for a
	/// [`CommandAccess::Server`] command. Server-side invokers are not
	/// filtered.
	pub const fn clients(mut self, filter: ClientFilter) -> Self {
		self.header.client_filter = Some(filter);
		self
	}

	/// Sets the flags the engine sees once the command is registered, none by
	/// default. [`CommandFlags::GAME_DLL`] is left out.
	pub const fn flags(mut self, flags: CommandFlags) -> Self {
		self.header.flags = flags;
		self
	}

	/// Sets the text `help <name>` shows.
	pub const fn help(mut self, help: &'static CStr) -> Self {
		self.header.help = help;
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

		// SAFETY: `register_base` only prepares the command once it found it
		// unregistered, and the command is pinned and `'static`.
		let prepare = |dll_identifier| unsafe { header.prepare(binding, dll_identifier) };

		// SAFETY: The command is pinned and `'static`, `prepare` fills in every
		// field the engine reads, and the caller unregisters it in time.
		unsafe {
			register_base(
				self.get_ref().as_base(),
				header.name,
				CommandBaseKind::Command,
				server,
				registrar,
				prepare,
			)
		}
	}

	/// Unregisters the command, which then no longer runs.
	///
	/// A handler may unregister its own command.
	pub fn unregister(
		&self,
		server: Server<'_>,
		registrar: &impl CommandRegistrar,
	) -> Result<(), UnregisterCommandError> {
		// SAFETY: Only `register` makes the engine list a command, and it takes
		// the command pinned and `'static`.
		unsafe {
			unregister_base(
				self.as_base(),
				self.header.name,
				CommandBaseKind::Command,
				server,
				registrar,
			)
		}
	}
}

impl<H> ConsoleCommand<H> {
	/// Who may run the command, as set with [`access`](Self::access).
	pub const fn access_level(&self) -> CommandAccess {
		self.header.access
	}

	/// Whether `invoker` may run the command, as its
	/// [`access`](Self::access) and [client filter](Self::clients) decide.
	///
	/// A [cheat](CommandFlags::CHEAT) command also needs `sv_cheats` set, which
	/// this does not check, so a listing can show it as one.
	pub fn allows(&self, server: Server<'_>, invoker: Invoker<'_>) -> bool {
		match invoker {
			Invoker::Server => self.header.access.allows_server(),
			Invoker::Client(client) => self.header.accepts_client(server, client),
		}
	}

	/// A pointer to the whole object, which the engine passes back to every
	/// vtable slot.
	fn as_base(&self) -> NonNull<sys::ConCommandBase> {
		ConCommandObject::as_base(NonNull::from(self).cast())
	}

	/// The filter set with [`clients`](Self::clients), if any.
	pub const fn client_filter(&self) -> Option<ClientFilter> {
		self.header.client_filter
	}

	/// The flags the engine sees: those set with [`flags`](Self::flags) once
	/// the command is registered, which other plugins may have changed since,
	/// and none before.
	pub fn current_flags(&self, _server: Server<'_>) -> CommandFlags {
		self.header.current_flags()
	}

	/// The handler that runs each invocation.
	pub const fn handler(&self) -> &H {
		&self.handler
	}

	/// The text `help <name>` shows, as set with [`help`](Self::help).
	#[doc(alias("GetHelpText"))]
	pub const fn help_text(&self) -> &'static CStr {
		self.header.help
	}

	/// The name the command is registered under.
	#[doc(alias("GetName"))]
	pub const fn name(&self) -> &'static CStr {
		self.header.name
	}
}

impl<H: std::fmt::Debug> std::fmt::Debug for ConsoleCommand<H> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("ConsoleCommand")
			.field("name", &self.header.name)
			.field("help", &self.header.help)
			.field("access", &self.header.access)
			.field("client_filter", &self.header.client_filter)
			.field("flags", &self.header.flags)
			.field("handler", &self.handler)
			.finish_non_exhaustive()
	}
}

// SAFETY: The interior-mutable header (the C++ fields and the `Cell`s) is only
// accessed by the engine, which calls commands on its main thread, and by
// methods taking a `Server`, whose contract confines it to that thread. Other
// threads can only reach the immutable fields and `&H`, which `H: Sync` covers.
unsafe impl<H: Sync> Sync for ConsoleCommand<H> {}

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

/// Runs a command for the engine's call to its `Dispatch`, which the engine
/// makes for every invocation that is not a client's string command.
///
/// # Safety
///
/// As [`ConCommandHooks::dispatch`] promises: `object` is the pointer the
/// engine was given to a command created with [`HOOKS`], and `command` is the
/// command the engine runs, live for the call, on the main thread.
unsafe fn engine_dispatch(object: NonNull<ConCommandObject>, command: NonNull<sys::CCommand>) {
	// Only `ConsoleCommand::new` creates objects with `HOOKS`, each the start of
	// a header, and the engine passes back the pointer it was given, to the
	// whole command, which `register` took pinned and `'static`.
	let registered = RegisteredCommand(object.cast());

	// SAFETY: The engine passes the command being run for this call, on the
	// main thread.
	unsafe { route::dispatch_from_engine(registered, command) };
}

/// Links a command or variable, after checking that it is not registered
/// already and that nothing else uses its name, then checks that the engine
/// lists it under its name, since neither the engine nor Metamod reports
/// failure.
///
/// `prepare` fills in the engine-visible fields, given the DLL identifier the
/// registry allocated, just before linking.
///
/// # Safety
///
/// `base` must be a pinned, `'static` command or variable of this crate, whose
/// fields the engine reads are filled in once `prepare` returns, and which is
/// unregistered before the module containing its code is unloaded.
pub(super) unsafe fn register_base(
	base: NonNull<sys::ConCommandBase>,
	name: &'static CStr,
	kind: CommandBaseKind,
	server: Server<'_>,
	registrar: &impl CommandRegistrar,
	prepare: impl FnOnce(sys::CVarDLLIdentifier_t),
) -> Result<(), RegisterCommandError> {
	let error = |error| RegisterCommandError::new(name, kind, error);
	let cvar = server.cvar().map_err(|interface| error(interface.into()))?;

	// Metamod clears the list link before linking, so linking a listed
	// command again would cut off every command after it.
	// SAFETY: The caller guarantees the object is live.
	if unsafe { is_registered(base) } {
		return Err(error(RegisterCommandErrorKind::AlreadyRegistered));
	}

	// The engine does not refuse a duplicate name: it lists a new command
	// first, which hides the existing one, and links a new variable to the
	// existing one without listing it.
	if let Some(existing) = cvar.find_command_base(name) {
		// SAFETY: The engine's registry holds live commands and variables.
		let existing = if unsafe { vcall!(existing.as_ptr() => ConCommandBase_IsCommand()) } {
			CommandBaseKind::Command
		} else {
			CommandBaseKind::Variable
		};

		return Err(error(RegisterCommandErrorKind::NameTaken(existing)));
	}

	prepare(cvar.allocate_dll_identifier());

	// SAFETY: The object is pinned, `'static`, and prepared, and the server's
	// existence confines this call to the main thread.
	unsafe { registrar.link(base) };

	// SAFETY: As above.
	if unsafe { is_registered(base) } && cvar.find_command_base(name) == Some(base) {
		return Ok(());
	}

	// Unlinking also stops a host like Metamod from tracking the object.
	// SAFETY: As for `link`.
	unsafe { registrar.unlink(base) };

	Err(error(RegisterCommandErrorKind::NotLinked))
}

/// Unlinks a command or variable [`register_base`] linked.
///
/// # Safety
///
/// `base` must be a live command or variable of this crate, which only
/// `register_base` makes the engine list.
pub(super) unsafe fn unregister_base(
	base: NonNull<sys::ConCommandBase>,
	name: &'static CStr,
	kind: CommandBaseKind,
	server: Server<'_>,
	registrar: &impl CommandRegistrar,
) -> Result<(), UnregisterCommandError> {
	let error = |error| UnregisterCommandError::new(name, kind, error);

	// SAFETY: The caller guarantees the object is live.
	if !unsafe { is_registered(base) } {
		return Err(error(UnregisterCommandErrorKind::NotRegistered));
	}

	let cvar = server.cvar().map_err(|interface| error(interface.into()))?;

	// SAFETY: `register_base` only links pinned, `'static` objects. The server
	// confines this call to the main thread.
	unsafe { registrar.unlink(base) };

	// SAFETY: As above.
	if unsafe { is_registered(base) } || cvar.find_command_base(name) == Some(base) {
		return Err(error(UnregisterCommandErrorKind::StillLinked));
	}

	Ok(())
}
