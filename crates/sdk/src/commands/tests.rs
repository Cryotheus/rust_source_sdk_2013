//! Commands against mocks that behave like the engine's registry (`CCvar`)
//! and dispatch.

use super::args::CommandLine;
use super::*;
use crate::edicts::test_support::mock_edict;
use crate::ffi::test_support::{mock_vtable, unexpected_call};
use crate::ffi::vcall;
use crate::interfaces::{Cvar, ValveEngine};
use crate::server::Module;
use crate::server::test_support::{export, mock_binding, mock_server};
use std::cell::{Cell, RefCell};
use std::ffi::c_char;
use std::pin::Pin;
use std::ptr::{self, NonNull, null_mut};

const FCVAR_GAMEDLL: c_int = 1 << 2;

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
}

/// One invocation a handler saw.
#[derive(Debug, PartialEq, Eq)]
struct Seen {
	/// The client's slot, or `None` for the server.
	slot: Option<c_int>,
	typed: String,
	args: Vec<String>,
}

unsafe fn name_of(base: *mut sys::ConCommandBase) -> &'static CStr {
	unsafe { CStr::from_ptr(vcall!(base => ConCommandBase_GetName())) }
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

unsafe extern "C" fn allocate_dll_identifier(_: *mut sys::ICvar) -> c_int {
	NEXT_IDENTIFIER.replace(NEXT_IDENTIFIER.get() + 1)
}

unsafe extern "C" fn find_var(_: *mut sys::ICvar, name: *const c_char) -> *mut sys::ConVar {
	if unsafe { CStr::from_ptr(name) } == c"sv_cheats" {
		CHEATS.get()
	} else {
		null_mut()
	}
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

unsafe extern "C" fn msg(format: *const c_char, mut arguments: ...) {
	let message = unsafe { printed(format, arguments.next_arg::<*const c_char>()) };

	CONSOLE.with_borrow_mut(|console| console.push(message));
}

unsafe extern "C" fn console_printf(
	_: *const sys::ICvar,
	format: *const c_char,
	mut arguments: ...
) {
	let message = unsafe { printed(format, arguments.next_arg::<*const c_char>()) };

	DISPLAY_FUNCS.with_borrow_mut(|display| display.push(message));
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

/// Exports a mock `ICvar` and `IVEngineServer` on this thread.
fn mock_engine() {
	let cvar_vtable = unsafe {
		mock_vtable::<sys::ICvar__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).ICvar_FindCommandBase).write(find_command_base);
			(&raw mut (*vtable).ICvar_RegisterConCommand).write(register_con_command);
			(&raw mut (*vtable).ICvar_UnregisterConCommand).write(unregister_con_command);
			(&raw mut (*vtable).ICvar_AllocateDLLIdentifier).write(allocate_dll_identifier);
			(&raw mut (*vtable).ICvar_FindVar).write(find_var);
			(&raw mut (*vtable).ICvar_ConsolePrintf).write(console_printf);
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
	crate::tier0::TEST_MSG.set(Some(msg));
	export(Module::Engine, Cvar::VERSION, cvar);
	export(Module::Engine, ValveEngine::VERSION, engine);
}

fn set_cheats(value: c_int) {
	unsafe { (&raw mut (*CHEATS.get()).m_nValue).write(value) };
}

unsafe extern "C" fn foreign_name(this: *const sys::ConCommandBase) -> *const c_char {
	unsafe { (&raw const (*this).m_pszName).read() }
}

unsafe extern "C" fn yes(_: *const sys::ConCommandBase) -> bool {
	true
}

unsafe extern "C" fn no(_: *const sys::ConCommandBase) -> bool {
	false
}

/// Lists a command or variable another module declared.
fn list_foreign(name: &'static CStr, is_command: bool) -> *mut sys::ConCommandBase {
	let vtable = unsafe {
		mock_vtable::<sys::ConCommandBase__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).ConCommandBase_GetName).write(foreign_name);
			(&raw mut (*vtable).ConCommandBase_IsCommand).write(if is_command { yes } else { no });
			(&raw mut (*vtable).ConCommandBase_IsRegistered).write(yes);
		})
	};
	let base = Box::leak(Box::new(sys::ConCommandBase {
		vtable_: Box::leak(vtable),
		m_pNext: null_mut(),
		m_bRegistered: true,
		m_pszName: name.as_ptr(),
		m_pszHelpString: c"".as_ptr(),
		m_nFlags: 0,
	}));

	REGISTRY.with_borrow_mut(|registry| registry.insert(0, &raw mut *base));
	base
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

fn leak<H: CommandHandler>(command: ConsoleCommand<H>) -> Pin<&'static ConsoleCommand<H>> {
	Pin::static_ref(Box::leak(Box::new(command)))
}

fn register<H: CommandHandler>(
	command: Pin<&'static ConsoleCommand<H>>,
) -> Result<(), RegisterCommandError> {
	let scope = ();
	let server = mock_server(&scope);

	command.register(server, mock_binding(), &Host(server.cvar().unwrap()))
}

fn listed() -> Vec<String> {
	REGISTRY.with_borrow(|registry| {
		registry
			.iter()
			.map(|&base| unsafe { name_of(base) }.to_str().unwrap().to_owned())
			.collect()
	})
}

fn base_of<H>(command: &ConsoleCommand<H>) -> *mut sys::ConCommandBase {
	ptr::from_ref(command).cast_mut().cast()
}

fn engine_vtable<H>(command: &ConsoleCommand<H>) -> &'static sys::ConCommand__bindgen_vtable {
	let vtable = unsafe { (&raw const (*base_of(command)).vtable_).read() };

	unsafe { &*vtable.cast::<sys::ConCommand__bindgen_vtable>() }
}

/// Runs a command as the engine does for server-side invokers.
fn engine_dispatch<H>(command: &ConsoleCommand<H>, raw: *const sys::CCommand) {
	let dispatch = engine_vtable(command).ConCommand_Dispatch;

	unsafe { dispatch(base_of(command).cast(), raw) };
}

/// Tokenizes a line of unquoted arguments separated by single spaces.
fn tokenized(line: &str) -> Box<sys::CCommand> {
	let args = line.split(' ').collect::<Vec<_>>();
	let args_start = if args.len() > 1 { args[0].len() + 1 } else { 0 };

	CommandLine::tokenized(line, &args, args_start)
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

fn seen() -> Vec<Seen> {
	SEEN.take()
}

fn console() -> Vec<String> {
	CONSOLE.take()
}

fn client_prints() -> Vec<(c_int, String)> {
	CLIENT_PRINTS.take()
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
		"could not register console command `sb_ping`: the command is already registered"
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

#[test]
fn names_in_use_are_refused() {
	mock_engine();
	list_foreign(c"changelevel", true);
	list_foreign(c"sv_cheats", false);

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
	let mut table = [mock_edict(0, false), mock_edict(1, false)];
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

#[test]
fn clients_reach_only_commands_they_may_run() {
	mock_engine();
	list_foreign(c"status", true);

	let everyone = leak(ConsoleCommand::new(c"sb_ping", record).access(CommandAccess::Everyone));
	let clients = leak(ConsoleCommand::new(c"sb_whoami", record).access(CommandAccess::Clients));
	let server_only = leak(ConsoleCommand::new(c"sb_kick_all", record));

	register(everyone).unwrap();
	register(clients).unwrap();
	register(server_only).unwrap();

	let mut table = [
		mock_edict(0, false),
		mock_edict(1, false),
		mock_edict(2, false),
		mock_edict(3, true),
	];
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

#[test]
fn clients_need_cheats_for_cheat_commands() {
	mock_engine();

	let command = leak(
		ConsoleCommand::new(c"sb_noclip", record)
			.access(CommandAccess::Everyone)
			.flags(CommandFlags::CHEAT | CommandFlags::HIDDEN),
	);

	register(command).unwrap();

	let mut table = [mock_edict(0, false), mock_edict(1, false)];
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
fn the_engine_never_sees_the_game_dll_flag() {
	mock_engine();

	let command = leak(ConsoleCommand::new(c"sb_ping", record).flags(CommandFlags::DONT_RECORD));

	register(command).unwrap();

	let base = base_of(command.get_ref());

	unsafe {
		assert!(vcall!(base => ConCommandBase_IsFlagSet(CommandFlags::DONT_RECORD.bits())));
		assert!(!vcall!(base => ConCommandBase_IsFlagSet(CommandFlags::HIDDEN.bits())));

		vcall!(base => ConCommandBase_AddFlags(FCVAR_GAMEDLL | CommandFlags::HIDDEN.bits()));
		assert!(vcall!(base => ConCommandBase_IsFlagSet(CommandFlags::HIDDEN.bits())));
		assert!(!vcall!(base => ConCommandBase_IsFlagSet(FCVAR_GAMEDLL)));

		// Another plugin may write the flags directly.
		(&raw mut (*base).m_nFlags).write(FCVAR_GAMEDLL);
		assert!(!vcall!(base => ConCommandBase_IsFlagSet(FCVAR_GAMEDLL)));
		assert!(!vcall!(base => ConCommandBase_IsFlagSet(-1)));
	}
}

#[test]
fn destructors_and_completion_leave_the_command_intact() {
	mock_engine();

	let command = leak(ConsoleCommand::new(c"sb_ping", record));

	register(command).unwrap();

	let vtable = engine_vtable(command.get_ref());
	let this = base_of(command.get_ref()).cast::<sys::ConCommand>();

	unsafe {
		#[cfg(windows)]
		{
			(vtable.ConCommand_destructor)(this, 0);
			(vtable.ConCommand_destructor)(this, 1);
		}

		#[cfg(target_os = "linux")]
		{
			(vtable.ConCommand_complete_destructor)(this);
			(vtable.ConCommand_deleting_destructor)(this);
		}

		(vtable.ConCommand_CreateBase)(this, c"other".as_ptr(), c"".as_ptr(), FCVAR_GAMEDLL);
		(vtable.ConCommand_Init)(this);
		assert_eq!(
			(vtable.ConCommand_AutoCompleteSuggest)(this, c"sb".as_ptr(), null_mut()),
			0
		);
		assert!(!(vtable.ConCommand_CanAutoComplete)(this));
	}

	engine_dispatch(command.get_ref(), &*tokenized("sb_ping"));
	assert_eq!(seen().len(), 1);
	assert_eq!(listed(), ["sb_ping"]);
}

thread_local! {
	static RAW_COMMAND: Cell<*mut sys::CCommand> = const { Cell::new(null_mut()) };
	static CLIENT_EDICT: Cell<*mut sys::edict_t> = const { Cell::new(null_mut()) };
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

	let mut table = [mock_edict(0, false), mock_edict(1, false)];
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

static SELF_REMOVING: ConsoleCommand<CommandFn> =
	ConsoleCommand::new(c"sb_once", remove_self as CommandFn).access(CommandAccess::Everyone);

fn remove_self(command: &CommandContext<'_>) -> CommandResult {
	let server = command.server();

	SELF_REMOVING
		.unregister(server, &server.cvar()?)
		.map_err(CommandError::other)?;
	command.reply("removed")?;
	Ok(())
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
#[should_panic = "console command names may only contain ASCII letters, digits, and `_`"]
fn invalid_names_panic() {
	let _ = ConsoleCommand::new(c"sb;quit", record);
}

#[test]
fn the_vtable_fills_its_page() {
	mock_engine();

	let command = leak(ConsoleCommand::new(c"sb_ping", record));

	register(command).unwrap();
	assert_eq!(
		ptr::from_ref(engine_vtable(command.get_ref())).addr() % 4096,
		0
	);
}

#[test]
fn replies_are_terminated_lines_without_nul() {
	assert_eq!(line_from("a\0b"), c"ab\n");
	assert_eq!(line_from(format_args!("{}", 3)), c"3\n");
	assert!(
		CommandFlags::CHEAT
			.union(CommandFlags::HIDDEN)
			.contains(CommandFlags::HIDDEN)
	);
	assert!(!CommandFlags::HIDDEN.contains(CommandFlags::CHEAT));
}

#[test]
fn server_replies_use_the_console_display_functions_without_tier0() {
	mock_engine();
	crate::tier0::TEST_MSG.set(None);

	let command = leak(ConsoleCommand::new(c"sb_ping", record));

	register(command).unwrap();
	engine_dispatch(command.get_ref(), &*tokenized("sb_ping"));

	assert!(console().is_empty());
	assert_eq!(DISPLAY_FUNCS.take(), ["ran sb_ping\n"]);
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

	let mut table = [mock_edict(0, false), mock_edict(1, false)];
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
