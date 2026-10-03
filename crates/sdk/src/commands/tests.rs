//! Commands against mocks that behave like the engine's registry (`CCvar`)
//! and dispatch.

use super::*;
use crate::interfaces::{Cvar, ValveEngine};
use crate::server::Module;
use crate::test_support::edicts::edict_table;
use crate::test_support::interfaces::cvar::mock_base;
use crate::test_support::server::{export, mock_binding, mock_server};
use sdk_raw::commands::ConVarObject;
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use sdk_raw::vcall;
use std::cell::{Cell, RefCell};
use std::ffi::c_char;
use std::pin::Pin;
use std::ptr::{self, NonNull, null_mut};

thread_local! {
	/// Listed commands and variables, most recent first, as `CCvar` keeps them.
	static REGISTRY: RefCell<Vec<*mut sys::ConCommandBase>> = const { RefCell::new(Vec::new()) };

	/// Makes `RegisterConCommand` do nothing.
	static REFUSE_LINKS: Cell<bool> = const { Cell::new(false) };

	static NEXT_IDENTIFIER: Cell<c_int> = const { Cell::new(7) };
	static CHEATS: Cell<*mut sys::ConVar> = const { Cell::new(null_mut()) };
	/// What tier0's `Msg` printed, which reaches a dedicated server's console.
	static CONSOLE: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };

	/// What `ICvar::ConsolePrintf` printed, which a dedicated server discards.
	static DISPLAY_FUNCS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
	static CLIENT_PRINTS: RefCell<Vec<(c_int, String)>> = const { RefCell::new(Vec::new()) };
	static SEEN: RefCell<Vec<Seen>> = const { RefCell::new(Vec::new()) };

	/// What the engine's global change callbacks saw.
	static CHANGES: RefCell<Vec<Change>> = const { RefCell::new(Vec::new()) };

	/// How many changes the engine's global change callbacks would announce.
	static ANNOUNCED: Cell<usize> = const { Cell::new(0) };
}

/// A change the engine's global change callbacks saw.
#[derive(Debug, PartialEq)]
struct Change {
	name: String,
	old: String,
	old_float: f32,
	new: String,
}

/// A registrar standing in for Metamod's, which unlinks commands on unload.
struct Host<'s>(Cvar<'s>);

// SAFETY: Forwards to the engine's registry.
unsafe impl CommandRegistrar for Host<'_> {
	unsafe fn link(&self, command: NonNull<sys::ConCommandBase>) {
		unsafe { self.0.link(command) };
	}

	unsafe fn unlink(&self, command: NonNull<sys::ConCommandBase>) {
		unsafe { self.0.unlink(command) };
	}
}

// SAFETY: Test code is never unloaded.
unsafe impl UnlinksBeforeUnload for Host<'_> {}

/// One invocation a handler saw.
#[derive(Debug, PartialEq, Eq)]
struct Seen {
	/// The client's slot, or `None` for the server.
	slot: Option<c_int>,
	typed: String,
	args: Vec<String>,
}

unsafe extern "C" fn allocate_dll_identifier(_: *mut sys::ICvar) -> c_int {
	NEXT_IDENTIFIER.replace(NEXT_IDENTIFIER.get() + 1)
}

fn base_of<H>(command: &ConsoleCommand<H>) -> *mut sys::ConCommandBase {
	ptr::from_ref(command).cast_mut().cast()
}

/// Records a change as the engine's own global callback would see it, reading
/// the new value through the variable.
unsafe extern "C" fn call_global_change_callbacks(
	_: *mut sys::ICvar,
	var: *mut sys::ConVar,
	old: *const c_char,
	old_float: f32,
) {
	let text = |pointer| {
		unsafe { CStr::from_ptr(pointer) }
			.to_str()
			.unwrap()
			.to_owned()
	};
	let change = Change {
		name: text(unsafe { name_of(var.cast()) }.as_ptr()),
		old: text(old),
		old_float,
		new: text(unsafe { (&raw const (*var).m_pszString).read() }),
	};

	CHANGES.with_borrow_mut(|changes| changes.push(change));

	if unsafe {
		vcall!(var.cast::<sys::ConCommandBase>() => ConCommandBase_IsFlagSet(CommandFlags::NOTIFY.bits()))
	} {
		ANNOUNCED.set(ANNOUNCED.get() + 1);
	}
}

fn changes() -> Vec<Change> {
	CHANGES.take()
}

#[test]
fn client_only_commands_refuse_the_server() {
	mock_engine();

	let command = leak(ConsoleCommand::new(c"sb_whoami", record).access(CommandAccess::Clients));

	register(command).unwrap();
	engine_dispatch(command.get_ref(), &*tokenized("sb_whoami"));

	assert!(seen().is_empty());
	assert_eq!(
		console(),
		["sb_whoami: only players can use this command\n"]
	);
}

unsafe extern "C" fn client_printf(
	_: *mut sys::IVEngineServer,
	edict: *mut sys::edict_t,
	message: *const c_char,
) {
	let index = c_int::from(unsafe { (*edict)._base.m_EdictIndex });
	let message = unsafe { CStr::from_ptr(message) }
		.to_str()
		.unwrap()
		.to_owned();

	CLIENT_PRINTS.with_borrow_mut(|prints| prints.push((index, message)));
}

fn client_prints() -> Vec<(c_int, String)> {
	CLIENT_PRINTS.take()
}

#[test]
fn clients_need_cheats_for_cheat_commands() {
	mock_engine();

	let command = leak(
		ConsoleCommand::new(c"sb_noclip", record)
			.access(CommandAccess::Everyone)
			.flags(CommandFlags::CHEAT | CommandFlags::HIDDEN),
	);

	register(command).unwrap();

	let mut table = edict_table(2, |_| false);
	let binding = mock_binding();
	let mut route = || unsafe {
		route_client_command(
			&binding,
			NonNull::from(&mut table[1]),
			NonNull::from(&*tokenized("sb_noclip")),
		)
	};

	assert_eq!(route(), ClientRoute::Handled);
	assert!(seen().is_empty());
	assert_eq!(
		client_prints(),
		[(
			1,
			"Can't use cheat command sb_noclip in multiplayer, unless the server has sv_cheats set to 1.\n"
				.to_owned()
		)]
	);

	set_cheats(1);
	assert_eq!(route(), ClientRoute::Handled);
	assert_eq!(seen().len(), 1);
}

#[test]
fn clients_reach_only_commands_they_may_run() {
	mock_engine();
	list_foreign(c"status", CommandBaseKind::Command);

	let everyone = leak(ConsoleCommand::new(c"sb_ping", record).access(CommandAccess::Everyone));
	let clients = leak(ConsoleCommand::new(c"sb_whoami", record).access(CommandAccess::Clients));
	let server_only = leak(ConsoleCommand::new(c"sb_kick_all", record));

	register(everyone).unwrap();
	register(clients).unwrap();
	register(server_only).unwrap();

	let mut table = edict_table(4, |slot| slot == 3);
	let binding = mock_binding();
	let mut route = |slot: usize, line: &str| unsafe {
		route_client_command(
			&binding,
			NonNull::from(&mut table[slot]),
			NonNull::from(&*tokenized(line)),
		)
	};

	assert_eq!(route(2, "SB_Ping x"), ClientRoute::Handled);
	assert_eq!(route(1, "sb_whoami"), ClientRoute::Handled);
	assert_eq!(
		seen(),
		[
			Seen {
				slot: Some(1),
				typed: "SB_Ping".to_owned(),
				args: vec!["x".to_owned()],
			},
			Seen {
				slot: Some(0),
				typed: "sb_whoami".to_owned(),
				args: vec![],
			},
		]
	);
	assert_eq!(
		client_prints(),
		[
			(2, "ran sb_ping\n".to_owned()),
			(1, "ran sb_whoami\n".to_owned())
		]
	);

	// Server-only, foreign, and unknown commands are the game's, as are
	// commands from the world or a free slot.
	assert_eq!(route(1, "sb_kick_all"), ClientRoute::NotRouted);
	assert_eq!(route(1, "status"), ClientRoute::NotRouted);
	assert_eq!(route(1, "sb_unknown"), ClientRoute::NotRouted);
	assert_eq!(route(0, "sb_ping"), ClientRoute::NotRouted);
	assert_eq!(route(3, "sb_ping"), ClientRoute::NotRouted);

	// So is a command unregistered since.
	let scope = ();
	let server = mock_server(&scope);

	everyone
		.unregister(server, &server.cvar().unwrap())
		.unwrap();
	assert_eq!(route(1, "sb_ping"), ClientRoute::NotRouted);

	assert!(seen().is_empty());
	assert!(client_prints().is_empty());
	assert!(console().is_empty());
}

fn console() -> Vec<String> {
	CONSOLE.take()
}

unsafe extern "C" fn console_printf(
	_: *const sys::ICvar,
	format: *const c_char,
	mut arguments: ...
) {
	let message = unsafe { printed(format, arguments.next_arg::<*const c_char>()) };

	DISPLAY_FUNCS.with_borrow_mut(|display| display.push(message));
}

/// Runs a command as the engine does for server-side invokers.
fn engine_dispatch<H>(command: &ConsoleCommand<H>, raw: *const sys::CCommand) {
	let dispatch = engine_vtable(command).ConCommand_Dispatch;

	unsafe { dispatch(base_of(command).cast(), raw) };
}

fn engine_vtable<H>(command: &ConsoleCommand<H>) -> &'static sys::ConCommand__bindgen_vtable {
	let vtable = unsafe { (&raw const (*base_of(command)).vtable_).read() };

	unsafe { &*vtable.cast::<sys::ConCommand__bindgen_vtable>() }
}

#[test]
fn errors_and_panics_are_reported_to_the_invoker() {
	mock_engine();

	let failing = leak(ConsoleCommand::new(
		c"sb_fail",
		|command: &CommandContext<'_>| {
			command.args().parse::<u8>(0)?;
			Err(CommandError::usage("sb_fail <count>"))
		},
	));
	let panicking = leak(
		ConsoleCommand::new(c"sb_panic", |_: &CommandContext<'_>| -> CommandResult {
			panic!("boom")
		})
		.access(CommandAccess::Everyone),
	);

	register(failing).unwrap();
	register(panicking).unwrap();

	engine_dispatch(failing.get_ref(), &*tokenized("sb_fail x"));
	engine_dispatch(failing.get_ref(), &*tokenized("sb_fail 3"));
	engine_dispatch(panicking.get_ref(), &*tokenized("sb_panic"));

	assert_eq!(
		console(),
		[
			"sb_fail: argument 1 (`x`) is not a valid u8\n",
			"sb_fail: usage: sb_fail <count>\n",
			"sb_panic: the command panicked: boom\n",
		]
	);

	// A panic triggered by a client is logged, and the client is told.
	let mut table = edict_table(2, |_| false);
	let route = unsafe {
		route_client_command(
			&mock_binding(),
			NonNull::from(&mut table[1]),
			NonNull::from(&*tokenized("sb_panic")),
		)
	};

	assert_eq!(route, ClientRoute::Handled);
	assert_eq!(console(), ["sb_panic: the command panicked: boom\n"]);
	assert_eq!(
		client_prints(),
		[(1, "sb_panic: the command failed\n".to_owned())]
	);
}

unsafe extern "C" fn find_command_base(
	_: *mut sys::ICvar,
	name: *const c_char,
) -> *mut sys::ConCommandBase {
	let name = unsafe { CStr::from_ptr(name) }.to_bytes();

	REGISTRY.with_borrow(|registry| {
		registry
			.iter()
			.copied()
			.find(|&base| {
				unsafe { name_of(base) }
					.to_bytes()
					.eq_ignore_ascii_case(name)
			})
			.unwrap_or(null_mut())
	})
}

unsafe extern "C" fn find_var(_: *mut sys::ICvar, name: *const c_char) -> *mut sys::ConVar {
	let name = unsafe { CStr::from_ptr(name) };

	if name == c"sv_cheats" {
		return CHEATS.get();
	}

	REGISTRY.with_borrow(|registry| {
		registry
			.iter()
			.copied()
			.find(|&base| {
				!unsafe { vcall!(base => ConCommandBase_IsCommand()) }
					&& unsafe { name_of(base) }
						.to_bytes()
						.eq_ignore_ascii_case(name.to_bytes())
			})
			.map_or(null_mut(), |base| base.cast())
	})
}

/// The most recently listed entry, whose `m_pNext` leads through the rest.
unsafe extern "C" fn get_commands(_: *mut sys::ICvar) -> *mut sys::ConCommandBase {
	REGISTRY.with_borrow(|registry| registry.first().copied().unwrap_or(null_mut()))
}

fn leak<H: CommandHandler>(command: ConsoleCommand<H>) -> Pin<&'static ConsoleCommand<H>> {
	Pin::static_ref(Box::leak(Box::new(command)))
}

/// Lists a command or variable another module declared.
fn list_foreign(name: &'static CStr, kind: CommandBaseKind) -> *mut sys::ConCommandBase {
	let next = REGISTRY.with_borrow(|registry| registry.first().copied().unwrap_or(null_mut()));
	let base = Box::leak(Box::new(mock_base(name, kind, CommandFlags::NONE, next)));

	REGISTRY.with_borrow_mut(|registry| registry.insert(0, &raw mut *base));
	base
}

fn listed() -> Vec<String> {
	REGISTRY.with_borrow(|registry| {
		registry
			.iter()
			.map(|&base| unsafe { name_of(base) }.to_str().unwrap().to_owned())
			.collect()
	})
}

/// Exports a mock `ICvar` and `IVEngineServer` on this thread.
fn mock_engine() {
	let cvar_vtable = unsafe {
		mock_vtable::<sys::ICvar__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).ICvar_FindCommandBase).write(find_command_base);
			(&raw mut (*vtable).ICvar_RegisterConCommand).write(register_con_command);
			(&raw mut (*vtable).ICvar_UnregisterConCommand).write(unregister_con_command);
			(&raw mut (*vtable).ICvar_AllocateDLLIdentifier).write(allocate_dll_identifier);
			(&raw mut (*vtable).ICvar_FindVar).write(find_var);
			(&raw mut (*vtable).ICvar_GetCommands).write(get_commands);
			(&raw mut (*vtable).ICvar_ConsolePrintf).write(console_printf);
			(&raw mut (*vtable).ICvar_CallGlobalChangeCallbacks)
				.write(call_global_change_callbacks);
		})
	};
	let engine_vtable = unsafe {
		mock_vtable::<sys::IVEngineServer__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).IVEngineServer_ClientPrintf).write(client_printf);
		})
	};

	let cvar = Box::leak(Box::new(sys::ICvar {
		vtable_: Box::leak(cvar_vtable),
	}));
	let engine = Box::leak(Box::new(sys::IVEngineServer {
		vtable_: Box::leak(engine_vtable),
	}));

	// SAFETY: Zero is valid for every field of `ConVar`.
	let cheats = Box::leak(Box::new(unsafe { std::mem::zeroed::<sys::ConVar>() }));

	CHEATS.set(cheats);
	crate::server::TEST_MSG.set(Some(msg));
	export(Module::Engine, Cvar::VERSION, cvar);
	export(Module::Engine, ValveEngine::VERSION, engine);
}

unsafe extern "C" fn msg(format: *const c_char, mut arguments: ...) {
	let message = unsafe { printed(format, arguments.next_arg::<*const c_char>()) };

	CONSOLE.with_borrow_mut(|console| console.push(message));
}

unsafe fn name_of(base: *mut sys::ConCommandBase) -> &'static CStr {
	unsafe { CStr::from_ptr(vcall!(base => ConCommandBase_GetName())) }
}

#[test]
fn names_in_use_are_refused() {
	mock_engine();
	list_foreign(c"changelevel", CommandBaseKind::Command);
	list_foreign(c"sv_cheats", CommandBaseKind::Variable);

	let command = |name| leak(ConsoleCommand::new(name, record));
	let kind = |result: Result<(), RegisterCommandError>| result.unwrap_err().kind().clone();

	assert_eq!(
		kind(register(command(c"changelevel"))),
		RegisterCommandErrorKind::NameTaken(CommandBaseKind::Command)
	);
	assert_eq!(
		kind(register(command(c"SV_CHEATS"))),
		RegisterCommandErrorKind::NameTaken(CommandBaseKind::Variable)
	);

	register(command(c"sb_ping")).unwrap();
	assert_eq!(
		kind(register(command(c"SB_Ping"))),
		RegisterCommandErrorKind::NameTaken(CommandBaseKind::Command)
	);
	assert_eq!(listed(), ["sb_ping", "sv_cheats", "changelevel"]);
}

/// Reads a `printf("%s", message)` call, or reports another format, which
/// would interpret the message.
unsafe fn printed(format: *const c_char, message: *const c_char) -> String {
	if unsafe { CStr::from_ptr(format) } != c"%s" {
		return "<format>".to_owned();
	}

	unsafe { CStr::from_ptr(message) }
		.to_str()
		.unwrap_or("<invalid UTF-8>")
		.to_owned()
}

fn record(command: &CommandContext<'_>) -> CommandResult {
	let slot = match command.invoker() {
		Invoker::Server => None,
		Invoker::Client(client) => Some(client.slot()),
	};
	let args = command.args();

	SEEN.with_borrow_mut(|seen| {
		seen.push(Seen {
			slot,
			typed: args.name().to_str().unwrap().to_owned(),
			args: args
				.iter()
				.map(|arg| arg.to_str().unwrap().to_owned())
				.collect(),
		})
	});

	command.reply(format_args!("ran {}", command.name().to_str().unwrap()))?;
	Ok(())
}

fn register<H: CommandHandler>(
	command: Pin<&'static ConsoleCommand<H>>,
) -> Result<(), RegisterCommandError> {
	let scope = ();
	let server = mock_server(&scope);

	command.register(server, mock_binding(), &Host(server.cvar().unwrap()))
}

unsafe extern "C" fn register_con_command(_: *mut sys::ICvar, base: *mut sys::ConCommandBase) {
	if REFUSE_LINKS.get() || unsafe { vcall!(base => ConCommandBase_IsRegistered()) } {
		return;
	}

	unsafe { (&raw mut (*base).m_bRegistered).write(true) };

	REGISTRY.with_borrow_mut(|registry| {
		let next = registry.first().copied().unwrap_or(null_mut());

		unsafe { (&raw mut (*base).m_pNext).write(next) };
		registry.insert(0, base);
	});
}

#[test]
fn registration_links_once_and_unregisters() {
	mock_engine();

	let other = leak(ConsoleCommand::new(c"sb_other", record));
	let command = leak(ConsoleCommand::new(c"sb_ping", record).help(c"Replies."));

	register(other).unwrap();
	register(command).unwrap();
	assert_eq!(listed(), ["sb_ping", "sb_other"]);

	// Registering again would relink the command in place.
	assert_eq!(
		register(command).unwrap_err().to_string(),
		"could not register console command `sb_ping`: it is already registered"
	);
	assert_eq!(listed(), ["sb_ping", "sb_other"]);

	// The engine sees the command through its vtable.
	let base = base_of(command.get_ref());

	unsafe {
		assert!(vcall!(base => ConCommandBase_IsCommand()));
		assert!(vcall!(base => ConCommandBase_IsRegistered()));
		assert_eq!(name_of(base), c"sb_ping");
		assert_eq!(
			CStr::from_ptr(vcall!(base => ConCommandBase_GetHelpText())),
			c"Replies."
		);
		assert_eq!(vcall!(base => ConCommandBase_GetDLLIdentifier()), 8);
	}

	let scope = ();
	let server = mock_server(&scope);
	let cvar = server.cvar().unwrap();

	command.unregister(server, &cvar).unwrap();
	assert_eq!(listed(), ["sb_other"]);
	assert_eq!(
		command.unregister(server, &cvar).unwrap_err().kind(),
		&UnregisterCommandErrorKind::NotRegistered
	);

	register(command).unwrap();
	assert_eq!(listed(), ["sb_ping", "sb_other"]);
	assert_eq!(
		unsafe { vcall!(base => ConCommandBase_GetDLLIdentifier()) },
		9
	);
}

fn seen() -> Vec<Seen> {
	SEEN.take()
}

fn set_cheats(value: c_int) {
	unsafe { (&raw mut (*CHEATS.get()).m_nValue).write(value) };
}

#[test]
fn the_engine_dispatches_server_invocations() {
	mock_engine();

	let command = leak(ConsoleCommand::new(c"sb_ping", record).access(CommandAccess::Everyone));

	register(command).unwrap();
	engine_dispatch(command.get_ref(), &*tokenized("SB_PING a b"));

	assert_eq!(
		seen(),
		[Seen {
			slot: None,
			typed: "SB_PING".to_owned(),
			args: vec!["a".to_owned(), "b".to_owned()],
		}]
	);
	assert_eq!(console(), ["ran sb_ping\n"]);

	// A null or malformed command never reaches the handler.
	engine_dispatch(command.get_ref(), ptr::null());

	let mut malformed = tokenized("sb_ping");

	malformed.m_nArgc = 0;
	engine_dispatch(command.get_ref(), &*malformed);

	assert!(seen().is_empty());
	assert_eq!(
		console(),
		["sb_ping: the command has 0 arguments, outside 1 to 64\n"]
	);
}

#[test]
fn the_engine_never_sees_the_game_dll_flag() {
	mock_engine();

	// Flags copied from a variable the game declared carry it.
	let flags = CommandFlags::GAME_DLL | CommandFlags::DONT_RECORD;
	let command = leak(ConsoleCommand::new(c"sb_ping", record).flags(flags));

	register(command).unwrap();

	let base = base_of(command.get_ref());
	let added = (CommandFlags::GAME_DLL | CommandFlags::HIDDEN).bits();

	unsafe {
		// The engine also reads the field directly.
		assert_eq!(
			(&raw const (*base).m_nFlags).read(),
			CommandFlags::DONT_RECORD.bits()
		);
		assert!(vcall!(base => ConCommandBase_IsFlagSet(CommandFlags::DONT_RECORD.bits())));
		assert!(!vcall!(base => ConCommandBase_IsFlagSet(CommandFlags::HIDDEN.bits())));

		vcall!(base => ConCommandBase_AddFlags(added));
		assert!(vcall!(base => ConCommandBase_IsFlagSet(CommandFlags::HIDDEN.bits())));
		assert!(!vcall!(base => ConCommandBase_IsFlagSet(CommandFlags::GAME_DLL.bits())));

		// Another plugin may write the flags directly.
		(&raw mut (*base).m_nFlags).write(CommandFlags::GAME_DLL.bits());
		assert!(!vcall!(base => ConCommandBase_IsFlagSet(CommandFlags::GAME_DLL.bits())));
		assert!(!vcall!(base => ConCommandBase_IsFlagSet(-1)));
	}
}

/// Tokenizes a line of unquoted arguments separated by single spaces.
fn tokenized(line: &str) -> Box<sys::CCommand> {
	let args = line.split(' ').collect::<Vec<_>>();
	let args_start = if args.len() > 1 { args[0].len() + 1 } else { 0 };

	sdk_raw::test_support::commands::tokenized(line, &args, args_start)
}

#[test]
fn unlisted_commands_are_reported() {
	mock_engine();
	REFUSE_LINKS.set(true);

	let command = leak(ConsoleCommand::new(c"sb_ping", record));
	let error = register(command).unwrap_err();

	assert_eq!(error.kind(), &RegisterCommandErrorKind::NotLinked);
	assert_eq!(error.name(), c"sb_ping");
	assert!(listed().is_empty());

	REFUSE_LINKS.set(false);
	register(command).unwrap();
}

unsafe extern "C" fn unregister_con_command(_: *mut sys::ICvar, base: *mut sys::ConCommandBase) {
	if !unsafe { vcall!(base => ConCommandBase_IsRegistered()) } {
		return;
	}

	unsafe { (&raw mut (*base).m_bRegistered).write(false) };

	REGISTRY.with_borrow_mut(|registry| {
		if let Some(position) = registry.iter().position(|&entry| entry == base) {
			registry.remove(position);

			if let Some(&previous) = position
				.checked_sub(1)
				.and_then(|index| registry.get(index))
			{
				let next = registry.get(position).copied().unwrap_or(null_mut());

				unsafe { (&raw mut (*previous).m_pNext).write(next) };
			}
		}

		unsafe { (&raw mut (*base).m_pNext).write(null_mut()) };
	});
}

thread_local! {
	static RAW_COMMAND: Cell<*mut sys::CCommand> = const { Cell::new(null_mut()) };
	static CLIENT_EDICT: Cell<*mut sys::edict_t> = const { Cell::new(null_mut()) };
}

static SELF_REMOVING: ConsoleCommand<CommandFn> =
	ConsoleCommand::new(c"sb_once", remove_self as CommandFn).access(CommandAccess::Everyone);

#[test]
fn change_callbacks_get_the_interface_and_the_old_value() {
	thread_local! {
		static SEEN_BY_CALLBACK: RefCell<Vec<(usize, String, f32)>> = const { RefCell::new(Vec::new()) };
	}

	unsafe extern "C" fn callback(var: *mut sys::IConVar, old: *const c_char, old_float: f32) {
		let old = unsafe { CStr::from_ptr(old) }.to_str().unwrap().to_owned();

		SEEN_BY_CALLBACK.with_borrow_mut(|seen| seen.push((var.addr(), old, old_float)));
	}

	mock_engine();

	let variable = leak_variable(ConsoleVariable::new(c"sb_speed", c"1.5"));

	register_variable(variable).unwrap();

	// The engine installs a callback when another module registers a variable
	// of the same name.
	let raw = variable_base(variable.get_ref()).cast::<sys::ConVar>();

	unsafe { (&raw mut (*raw).m_fnChangeCallback).write(Some(callback)) };

	let scope = ();
	let server = mock_server(&scope);

	variable.set_float(server, 2.0);
	assert_eq!(
		SEEN_BY_CALLBACK.take(),
		[(
			interface_of(variable.get_ref()).addr(),
			"1.5".to_owned(),
			1.5
		)]
	);
	assert_eq!(changes().len(), 1);
	assert_eq!(variable.string(server).as_c_str(), c"2.000000");
}

#[test]
fn handlers_can_leave_client_invocations_to_the_game() {
	mock_engine();

	let command = leak(
		ConsoleCommand::new(c"jointeam", |command: &CommandContext<'_>| {
			let Invoker::Client(client) = command.invoker() else {
				return Err(CommandError::Unhandled);
			};

			// Takes the invocation for the blue team only.
			if command.args().get(0) == Some(c"blue") {
				return record(command);
			}

			SEEN.with_borrow_mut(|seen| {
				seen.push(Seen {
					slot: Some(client.slot()),
					typed: "left to the game".to_owned(),
					args: vec![],
				})
			});

			Err(CommandError::Unhandled)
		})
		.access(CommandAccess::Everyone),
	);

	register(command).unwrap();

	let mut table = edict_table(2, |_| false);
	let binding = mock_binding();
	let mut route = |line: &str| unsafe {
		route_client_command(
			&binding,
			NonNull::from(&mut table[1]),
			NonNull::from(&*tokenized(line)),
		)
	};

	assert_eq!(route("jointeam red"), ClientRoute::NotRouted);
	assert_eq!(route("jointeam blue"), ClientRoute::Handled);
	assert_eq!(seen().len(), 2);
	assert_eq!(client_prints(), [(1, "ran jointeam\n".to_owned())]);

	// Nothing else handles a server-side invocation, and nothing is printed.
	engine_dispatch(command.get_ref(), &*tokenized("jointeam red"));
	assert!(seen().is_empty());
	assert!(console().is_empty());
}

#[test]
fn handlers_may_unregister_their_own_command() {
	mock_engine();
	register(Pin::static_ref(&SELF_REMOVING)).unwrap();

	engine_dispatch(&SELF_REMOVING, &*tokenized("sb_once"));

	assert!(listed().is_empty());
	assert_eq!(console(), ["removed\n"]);
}

#[test]
fn handlers_with_state_dispatch_through_the_whole_command() {
	mock_engine();

	// A handler with fields lies past the header, so it is only reachable
	// through a pointer to the whole command.
	let greeting = String::from("hello");
	let command = leak(
		ConsoleCommand::new(
			c"sb_greet",
			move |command: &CommandContext<'_>| -> CommandResult {
				command.reply(&greeting)?;
				Ok(())
			},
		)
		.access(CommandAccess::Everyone),
	);

	register(command).unwrap();
	engine_dispatch(command.get_ref(), &*tokenized("sb_greet"));
	assert_eq!(console(), ["hello\n"]);

	let mut table = edict_table(2, |_| false);
	let route = unsafe {
		route_client_command(
			&mock_binding(),
			NonNull::from(&mut table[1]),
			NonNull::from(&*tokenized("sb_greet")),
		)
	};

	assert_eq!(route, ClientRoute::Handled);
	assert_eq!(client_prints(), [(1, "hello\n".to_owned())]);
}

/// Where the engine sees a variable's `IConVar`.
fn interface_of(variable: &ConsoleVariable) -> *mut sys::IConVar {
	unsafe { ConVarObject::interface(NonNull::from(variable).cast()) }.as_ptr()
}

#[test]
#[should_panic = "console command names may only contain ASCII letters, digits, and `_`"]
fn invalid_names_panic() {
	let _ = ConsoleCommand::new(c"sb;quit", record);
}

#[test]
#[should_panic = "console variable names may only contain ASCII letters, digits, and `_`"]
fn invalid_variable_names_panic() {
	let _ = ConsoleVariable::new(c"sb rounds", c"3");
}

fn leak_variable(variable: ConsoleVariable) -> Pin<&'static ConsoleVariable> {
	Pin::static_ref(Box::leak(Box::new(variable)))
}

#[test]
fn names_are_validated() {
	assert_eq!(validate_name(c"sb_ping_2"), Ok(()));
	assert_eq!(validate_name(c""), Err(InvalidCommandName::Empty));
	assert_eq!(validate_name(c"RPT"), Err(InvalidCommandName::Reserved));
	assert_eq!(
		validate_name(c"rpt_connect"),
		Err(InvalidCommandName::Reserved)
	);
	assert_eq!(validate_name(c"rpt_"), Ok(()));
	assert_eq!(
		validate_name(c"sb ping"),
		Err(InvalidCommandName::InvalidByte {
			index: 2,
			byte: b' '
		})
	);
	assert_eq!(
		validate_name(c"+attack").unwrap_err().to_string(),
		"byte 0 (0x2b) is not an ASCII letter, digit, or `_`"
	);
	assert_eq!(
		validate_name(&CString::new("a".repeat(63)).unwrap()),
		Ok(())
	);
	assert_eq!(
		validate_name(&CString::new("a".repeat(64)).unwrap()),
		Err(InvalidCommandName::TooLong)
	);
}

#[test]
fn nested_invocations_keep_their_own_invoker_and_arguments() {
	mock_engine();

	let inner = leak(ConsoleCommand::new(c"sb_inner", record).access(CommandAccess::Clients));
	let outer = leak(ConsoleCommand::new(
		c"sb_outer",
		|command: &CommandContext<'_>| {
			// The engine re-tokenizes its buffer in place when it runs a command.
			let raw = RAW_COMMAND.get();

			unsafe {
				(&raw mut (*raw).m_nArgc).write(1);
				(&raw mut (*raw).m_pArgSBuffer).cast::<u8>().write(b'!');
			}

			// A handler making a client run a command re-enters the hook.
			let route = unsafe {
				route_client_command(
					&mock_binding(),
					NonNull::new(CLIENT_EDICT.get()).unwrap(),
					NonNull::from(&*tokenized("sb_inner nested")),
				)
			};

			assert_eq!(route, ClientRoute::Handled);
			record(command)
		},
	));

	register(inner).unwrap();
	register(outer).unwrap();

	let mut table = edict_table(2, |_| false);
	let raw = Box::into_raw(tokenized("sb_outer a b"));

	CLIENT_EDICT.set(&raw mut table[1]);
	RAW_COMMAND.set(raw);
	engine_dispatch(outer.get_ref(), raw);
	drop(unsafe { Box::from_raw(raw) });

	assert_eq!(
		seen(),
		[
			Seen {
				slot: Some(0),
				typed: "sb_inner".to_owned(),
				args: vec!["nested".to_owned()],
			},
			Seen {
				slot: None,
				typed: "sb_outer".to_owned(),
				args: vec!["a".to_owned(), "b".to_owned()],
			},
		]
	);
}

#[test]
fn quiet_changes_are_not_announced() {
	mock_engine();

	let variable =
		leak_variable(ConsoleVariable::new(c"sb_loud", c"0").flags(CommandFlags::NOTIFY));
	let scope = ();
	let server = mock_server(&scope);

	register_variable(variable).unwrap();

	let var = server.cvar().unwrap().find_var(c"sb_loud").unwrap();

	var.set_string(c"1");
	assert_eq!(ANNOUNCED.get(), 1);

	// Still changed, and the callbacks still run, but unannounced.
	var.set_string_quietly(c"2");
	assert_eq!(ANNOUNCED.get(), 1);
	assert_eq!(changes().len(), 2);
	assert_eq!(var.int(), 2);

	let base = variable_base(variable.get_ref());

	assert!(unsafe { vcall!(base => ConCommandBase_IsFlagSet(CommandFlags::NOTIFY.bits())) });
}

fn register_variable(variable: Pin<&'static ConsoleVariable>) -> Result<(), RegisterCommandError> {
	let scope = ();
	let server = mock_server(&scope);

	variable.register(server, mock_binding(), &Host(server.cvar().unwrap()))
}

#[test]
fn registered_commands_and_variables_are_listed() {
	mock_engine();

	let command = leak(ConsoleCommand::new(c"sb_ping", record).flags(CommandFlags::GAME_DLL));
	let variable = leak_variable(
		ConsoleVariable::new(c"sb_rounds", c"3")
			.flags(CommandFlags::GAME_DLL | CommandFlags::NOTIFY),
	);

	register(command).unwrap();
	register_variable(variable).unwrap();

	let scope = ();
	let server = mock_server(&scope);
	let cvar = server.cvar().unwrap();

	// Each reports its kind through its own vtable. Only the command drops
	// the flag.
	assert_eq!(
		cvar.command_bases()
			.map(|base| (base.name(), base.kind(), base.flags()))
			.collect::<Vec<_>>(),
		[
			(
				c"sb_rounds",
				CommandBaseKind::Variable,
				CommandFlags::GAME_DLL | CommandFlags::NOTIFY
			),
			(c"sb_ping", CommandBaseKind::Command, CommandFlags::NONE),
		]
	);

	let vars = cvar.vars().collect::<Vec<_>>();

	assert_eq!(vars.len(), 1);

	let var = vars[0];

	assert_eq!(var.as_ptr().cast(), variable_base(variable.get_ref()));
	assert_eq!(var.default_string().as_c_str(), c"3");
	assert!(var.is_default());

	var.set_string(c"5");
	assert!(!var.is_default());

	// The variable now holds its own copy of the string, not the default.
	var.set_string(c"3");
	assert_ne!(
		unsafe { (&raw const (*var.as_ptr()).m_pszString).read() }.cast_const(),
		variable.default_value().as_ptr()
	);
	assert!(var.is_default());
}

fn remove_self(command: &CommandContext<'_>) -> CommandResult {
	let server = command.server();

	SELF_REMOVING
		.unregister(server, &server.cvar()?)
		.map_err(CommandError::other)?;
	command.reply("removed")?;
	Ok(())
}

#[test]
fn replies_are_terminated_lines_without_nul() {
	assert_eq!(line_from("a\0b"), c"ab\n");
	assert_eq!(line_from(format_args!("{}", 3)), c"3\n");
}

#[test]
fn server_replies_use_the_console_display_functions_without_tier0() {
	mock_engine();
	crate::server::TEST_MSG.set(None);

	let command = leak(ConsoleCommand::new(c"sb_ping", record));

	register(command).unwrap();
	engine_dispatch(command.get_ref(), &*tokenized("sb_ping"));

	assert!(console().is_empty());
	assert_eq!(DISPLAY_FUNCS.take(), ["ran sb_ping\n"]);
}

#[test]
fn the_engine_sets_variables_through_their_interface() {
	mock_engine();

	let variable = leak_variable(ConsoleVariable::new(c"sb_bots", c"10").min(1.0).max(32.0));
	let scope = ();
	let server = mock_server(&scope);

	register_variable(variable).unwrap();

	let var = server.cvar().unwrap().find_var(c"sb_bots").unwrap();
	let change = |old: &str, old_float, new: &str| Change {
		name: "sb_bots".to_owned(),
		old: old.to_owned(),
		old_float,
		new: new.to_owned(),
	};

	var.set_string(c"20");
	assert_eq!((var.int(), var.float()), (20, 20.0));
	assert_eq!(changes(), [change("10", 10.0, "20")]);

	// Out-of-bounds values are clamped, and shown as the float they became.
	var.set_string(c"40");
	assert_eq!(var.string().as_c_str(), c"32.000000");
	assert_eq!(variable.int(server), 32);
	assert_eq!(changes(), [change("20", 20.0, "32.000000")]);

	var.set_float(0.5);
	assert_eq!(var.string().as_c_str(), c"1.000000");
	assert_eq!(var.int(), 1);

	var.set_int(7);
	assert_eq!(var.string().as_c_str(), c"7");
	assert_eq!(
		changes(),
		[
			change("32.000000", 32.0, "1.000000"),
			change("1.000000", 1.0, "7")
		]
	);

	// Unchanged values run no callbacks.
	var.set_int(7);
	var.set_string(c"7");
	var.set_float(7.0);
	assert!(changes().is_empty());

	// Strings that are not numbers read as 0, clamped to the minimum.
	var.set_string(c"many");
	assert_eq!(var.string().as_c_str(), c"1.000000");
	assert_eq!(changes(), [change("7", 7.0, "1.000000")]);

	// Setting from Rust behaves the same.
	variable.revert(server);
	assert_eq!(var.string().as_c_str(), c"10");
	assert_eq!(changes(), [change("1.000000", 1.0, "10")]);
}

fn variable_base(variable: &ConsoleVariable) -> *mut sys::ConCommandBase {
	ptr::from_ref(variable).cast_mut().cast()
}

#[test]
fn variables_hold_long_floats_as_tier1_does() {
	mock_engine();

	let variable = leak_variable(ConsoleVariable::new(c"sb_scale", c"0"));
	let scope = ();
	let server = mock_server(&scope);

	register_variable(variable).unwrap();

	// `%f`, in tier1's 32-byte buffer.
	variable.set_float(server, 1.0e30);

	let string = variable.string(server);

	assert_eq!(string.as_bytes().len(), 31);
	assert!(
		string
			.to_str()
			.unwrap()
			.starts_with("1000000015047466219876688855040")
	);

	variable.set_float(server, f32::NEG_INFINITY);
	assert_eq!(variable.string(server).as_c_str(), c"-inf");
	assert_eq!(variable.int(server), c_int::MIN);
}

#[test]
fn variables_hold_their_default_and_register_as_variables() {
	mock_engine();

	let variable =
		leak_variable(ConsoleVariable::new(c"sb_rounds", c"  12.5 rounds").help(c"Rounds."));
	let scope = ();
	let server = mock_server(&scope);

	// The default is parsed as tier1 parses it, before registration too.
	assert_eq!(variable.float(server), 12.5);
	assert_eq!(variable.int(server), 12);
	assert!(variable.bool(server));
	assert_eq!(variable.string(server).as_c_str(), c"  12.5 rounds");

	// Setting an unregistered variable runs no callbacks.
	variable.set_int(server, 3);
	assert_eq!(variable.string(server).as_c_str(), c"3");
	assert!(changes().is_empty());

	register_variable(variable).unwrap();
	assert_eq!(listed(), ["sb_rounds"]);

	let base = variable_base(variable.get_ref());

	unsafe {
		assert!(!vcall!(base => ConCommandBase_IsCommand()));
		assert!(vcall!(base => ConCommandBase_IsRegistered()));
		assert_eq!(name_of(base), c"sb_rounds");
		assert_eq!(
			CStr::from_ptr(vcall!(base => ConCommandBase_GetHelpText())),
			c"Rounds."
		);
		assert_eq!(vcall!(base => ConCommandBase_GetDLLIdentifier()), 7);
	}

	// The engine finds it as a variable, and reads it through its parent.
	let var = server.cvar().unwrap().find_var(c"SB_Rounds").unwrap();

	assert_eq!(var.name(), c"sb_rounds");
	assert_eq!(var.int(), 3);
	assert_eq!(var.string().as_c_str(), c"3");
	assert_eq!(var.default_string().as_c_str(), c"  12.5 rounds");

	// Registering again, or another object under the name, is refused.
	let error = register_variable(variable).unwrap_err();

	assert_eq!(error.base(), CommandBaseKind::Variable);
	assert_eq!(
		error.to_string(),
		"could not register console variable `sb_rounds`: it is already registered"
	);
	assert_eq!(
		register(leak(ConsoleCommand::new(c"sb_rounds", record)))
			.unwrap_err()
			.kind(),
		&RegisterCommandErrorKind::NameTaken(CommandBaseKind::Variable)
	);

	variable
		.unregister(server, &server.cvar().unwrap())
		.unwrap();
	assert!(listed().is_empty());
	assert!(server.cvar().unwrap().find_var(c"sb_rounds").is_none());
}
