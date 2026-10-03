//! Tests of console commands and variables against mocks that behave like the
//! engine's registry (`CCvar`) and dispatch: registration, what the engine
//! sees of the objects through their vtables, the routing of clients'
//! invocations, and re-entrant handlers.
//!
//! Test processes load no tier0, so the server console's lines reach the
//! registry's `ConsolePrintf`, which the mock records.

use sdk_raw::test_support::{mock_vtable, unexpected_call};
use sdk_raw::util::cstr::cstring_from_buffer;
use sdk_raw::util::printf::vsnprintf;
use sdk_raw::vcall;
use source_sdk_2013::Module;
use source_sdk_2013::commands::{
	ClientRoute, CommandAccess, CommandBaseKind, CommandContext, CommandError, CommandFlags,
	CommandFn, CommandHandler, CommandRegistrar, CommandResult, ConsoleCommand, ConsoleVariable,
	Invoker, RegisterCommandError, RegisterCommandErrorKind, UnlinksBeforeUnload,
	UnregisterCommandErrorKind, route_client_command,
};
use source_sdk_2013::interfaces::{Cvar, ValveEngine};
use source_sdk_2013::test_support::edicts::edict_table;
use source_sdk_2013::test_support::interfaces::cvar::mock_base;
use source_sdk_2013::test_support::leak;
use source_sdk_2013::test_support::server::{export, mock_binding, mock_server};
use std::cell::{Cell, RefCell};
use std::ffi::{CStr, c_char, c_int};
use std::pin::Pin;
use std::ptr::{self, NonNull, null_mut};

thread_local! {
	/// Listed commands and variables, most recent first, as `CCvar` keeps them.
	static REGISTRY: RefCell<Vec<*mut sys::ConCommandBase>> = const { RefCell::new(Vec::new()) };

	/// Makes `RegisterConCommand` do nothing.
	static REFUSE_LINKS: Cell<bool> = const { Cell::new(false) };

	/// The DLL identifier `AllocateDLLIdentifier` returns next.
	static NEXT_IDENTIFIER: Cell<c_int> = const { Cell::new(7) };

	/// The variable `FindVar` returns for `sv_cheats`.
	static CHEATS: Cell<*mut sys::ConVar> = const { Cell::new(null_mut()) };

	/// What `ICvar::ConsolePrintf` printed, which is the server console's
	/// output without tier0.
	static CONSOLE: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };

	/// The edict index and message of every `IVEngineServer::ClientPrintf`.
	static CLIENT_PRINTS: RefCell<Vec<(c_int, String)>> = const { RefCell::new(Vec::new()) };

	/// The invocations [`record`] saw.
	static SEEN: RefCell<Vec<Seen>> = const { RefCell::new(Vec::new()) };

	/// What the engine's global change callbacks saw.
	static CHANGES: RefCell<Vec<Change>> = const { RefCell::new(Vec::new()) };

	/// How many changes the engine's global change callbacks would announce.
	static ANNOUNCED: Cell<usize> = const { Cell::new(0) };

	/// The command the engine is running, which it re-tokenizes in place.
	static RAW_COMMAND: Cell<*mut sys::CCommand> = const { Cell::new(null_mut()) };

	/// The edict of the client a handler makes run a command.
	static CLIENT_EDICT: Cell<*mut sys::edict_t> = const { Cell::new(null_mut()) };
}

/// A command that unregisters itself when it runs.
static SELF_REMOVING: ConsoleCommand<CommandFn> =
	ConsoleCommand::new(c"sb_once", remove_self as CommandFn).access(CommandAccess::Everyone);

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
		// SAFETY: The caller upholds the registry's contract.
		unsafe { self.0.link(command) };
	}

	unsafe fn unlink(&self, command: NonNull<sys::ConCommandBase>) {
		// SAFETY: As for `link`.
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

/// `ICvar::AllocateDLLIdentifier`, which counts up from 7.
unsafe extern "C" fn allocate_dll_identifier(_: *mut sys::ICvar) -> c_int {
	NEXT_IDENTIFIER.replace(NEXT_IDENTIFIER.get() + 1)
}

/// Where the engine sees a command or variable, which is laid out as its
/// `ConCommand` or `ConVar`.
fn base_of<T>(object: &T) -> *mut sys::ConCommandBase {
	ptr::from_ref(object).cast_mut().cast()
}

/// `ICvar::CallGlobalChangeCallbacks`, which records a change as the engine's
/// own global callback would see it, reading the new value through the
/// variable.
unsafe extern "C" fn call_global_change_callbacks(
	_: *mut sys::ICvar,
	var: *mut sys::ConVar,
	old: *const c_char,
	old_float: f32,
) {
	// SAFETY: The wrappers pass a live variable, whose strings are
	// NUL-terminated, and a NUL-terminated old value.
	let change = unsafe {
		Change {
			name: text(name_of(var.cast()).as_ptr()),
			old: text(old),
			old_float,
			new: text((&raw const (*var).m_pszString).read()),
		}
	};

	CHANGES.with_borrow_mut(|changes| changes.push(change));

	// SAFETY: As above.
	if unsafe {
		vcall!(var.cast::<sys::ConCommandBase>() => ConCommandBase_IsFlagSet(CommandFlags::NOTIFY.bits()))
	} {
		ANNOUNCED.set(ANNOUNCED.get() + 1);
	}
}

#[test]
fn change_callbacks_get_the_interface_and_the_old_value() {
	thread_local! {
		static SEEN_BY_CALLBACK: RefCell<Vec<(*mut sys::IConVar, String, f32)>> =
			const { RefCell::new(Vec::new()) };
	}

	unsafe extern "C" fn callback(var: *mut sys::IConVar, old: *const c_char, old_float: f32) {
		// SAFETY: The variable passes its NUL-terminated old string.
		let old = unsafe { text(old) };

		SEEN_BY_CALLBACK.with_borrow_mut(|seen| seen.push((var, old, old_float)));
	}

	mock_engine();

	let variable = leak_pinned(ConsoleVariable::new(c"sb_speed", c"1.5"));

	register_variable(variable).unwrap();

	// The engine installs a callback when another module registers a variable
	// of the same name.
	let raw = base_of(variable.get_ref()).cast::<sys::ConVar>();

	// SAFETY: The variable is leaked, and the engine writes this field too.
	unsafe { (&raw mut (*raw).m_fnChangeCallback).write(Some(callback)) };

	let scope = ();
	let server = mock_server(&scope);

	variable.set_float(server, 2.0);

	// The callback gets the variable's `IConVar` subobject.
	// SAFETY: The variable is leaked, and the subobject lies within it.
	let interface = unsafe { &raw mut (*raw)._base_1 };

	assert_eq!(
		SEEN_BY_CALLBACK.take(),
		[(interface, "1.5".to_owned(), 1.5)]
	);
	assert_eq!(changes().len(), 1);
	assert_eq!(variable.string(server).as_c_str(), c"2.000000");
}

/// The changes the global change callbacks saw since the last call.
fn changes() -> Vec<Change> {
	CHANGES.take()
}

/// `IVEngineServer::ClientPrintf`, which records the edict's index and the
/// message.
unsafe extern "C" fn client_printf(
	_: *mut sys::IVEngineServer,
	edict: *mut sys::edict_t,
	message: *const c_char,
) {
	// SAFETY: The wrappers pass an edict of the mock table and a
	// NUL-terminated message.
	let (index, message) = unsafe { (c_int::from((*edict)._base.m_EdictIndex), text(message)) };

	CLIENT_PRINTS.with_borrow_mut(|prints| prints.push((index, message)));
}

/// What `ClientPrintf` printed since the last call.
fn client_prints() -> Vec<(c_int, String)> {
	CLIENT_PRINTS.take()
}

#[test]
fn clients_need_cheats_for_cheat_commands() {
	mock_engine();

	let command = leak_pinned(
		ConsoleCommand::new(c"sb_noclip", record)
			.access(CommandAccess::Everyone)
			.flags(CommandFlags::CHEAT | CommandFlags::HIDDEN),
	);

	register(command).unwrap();

	let mut table = edict_table(2, |_| false);

	assert_eq!(route(&raw mut table[1], "sb_noclip"), ClientRoute::Handled);
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
	assert_eq!(route(&raw mut table[1], "sb_noclip"), ClientRoute::Handled);
	assert_eq!(seen().len(), 1);
}

#[test]
fn clients_reach_only_commands_they_may_run() {
	mock_engine();
	list_foreign(c"status", CommandBaseKind::Command);

	let everyone =
		leak_pinned(ConsoleCommand::new(c"sb_ping", record).access(CommandAccess::Everyone));
	let clients =
		leak_pinned(ConsoleCommand::new(c"sb_whoami", record).access(CommandAccess::Clients));
	let server_only = leak_pinned(ConsoleCommand::new(c"sb_kick_all", record));

	register(everyone).unwrap();
	register(clients).unwrap();
	register(server_only).unwrap();

	let mut table = edict_table(4, |slot| slot == 3);
	let mut route = |slot: usize, line: &str| route(&raw mut table[slot], line);

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

/// What reached the server console since the last call.
fn console() -> Vec<String> {
	CONSOLE.take()
}

/// `ICvar::ConsolePrintf`, which records what it prints, or `<format>` for a
/// format other than `%s`, which would interpret the message.
unsafe extern "C" fn console_printf(_: *const sys::ICvar, format: *const c_char, arguments: ...) {
	// SAFETY: The wrappers pass a NUL-terminated format.
	let message = if unsafe { CStr::from_ptr(format) } == c"%s" {
		let mut buffer = [0; 1024];

		// SAFETY: The wrappers pass a NUL-terminated string for the `%s`.
		unsafe { vsnprintf(&mut buffer, format, arguments) };
		cstring_from_buffer(&buffer).into_string().unwrap()
	} else {
		"<format>".to_owned()
	};

	CONSOLE.with_borrow_mut(|console| console.push(message));
}

/// Runs a command as the engine does for server-side invokers.
fn engine_dispatch<H>(command: &ConsoleCommand<H>, raw: *const sys::CCommand) {
	let base = base_of(command);

	// SAFETY: The command is a live `ConCommand`, whose vtable the engine
	// reads as `ConCommand`'s, and the engine passes a tokenized command or
	// null.
	unsafe {
		let vtable = (&raw const (*base).vtable_)
			.read()
			.cast::<sys::ConCommand__bindgen_vtable>();

		((*vtable).ConCommand_Dispatch)(base.cast(), raw);
	}
}

#[test]
fn errors_and_panics_are_reported_to_the_invoker() {
	mock_engine();

	let failing = leak_pinned(ConsoleCommand::new(
		c"sb_fail",
		|command: &CommandContext<'_>| {
			command.args().parse::<u8>(0)?;
			Err(CommandError::usage("sb_fail <count>"))
		},
	));
	let panicking = leak_pinned(
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

	assert_eq!(route(&raw mut table[1], "sb_panic"), ClientRoute::Handled);
	assert_eq!(console(), ["sb_panic: the command panicked: boom\n"]);
	assert_eq!(
		client_prints(),
		[(1, "sb_panic: the command failed\n".to_owned())]
	);
}

/// `ICvar::FindCommandBase`, which finds a listed command or variable by name,
/// ignoring ASCII case, as `CCvar` does.
unsafe extern "C" fn find_command_base(
	_: *mut sys::ICvar,
	name: *const c_char,
) -> *mut sys::ConCommandBase {
	// SAFETY: The wrappers pass NUL-terminated names.
	let name = unsafe { CStr::from_ptr(name) }.to_bytes();

	REGISTRY.with_borrow(|registry| {
		registry
			.iter()
			.copied()
			// SAFETY: The registry lists live commands and variables.
			.find(|&base| {
				unsafe { name_of(base) }
					.to_bytes()
					.eq_ignore_ascii_case(name)
			})
			.unwrap_or(null_mut())
	})
}

/// `ICvar::FindVar`, which finds `sv_cheats`, or a listed variable by name,
/// ignoring ASCII case.
unsafe extern "C" fn find_var(_: *mut sys::ICvar, name: *const c_char) -> *mut sys::ConVar {
	// SAFETY: The wrappers pass NUL-terminated names.
	let name = unsafe { CStr::from_ptr(name) };

	if name == c"sv_cheats" {
		return CHEATS.get();
	}

	REGISTRY.with_borrow(|registry| {
		registry
			.iter()
			.copied()
			.find(|&base| {
				// SAFETY: The registry lists live commands and variables.
				unsafe {
					!vcall!(base => ConCommandBase_IsCommand())
						&& name_of(base)
							.to_bytes()
							.eq_ignore_ascii_case(name.to_bytes())
				}
			})
			.map_or(null_mut(), |base| base.cast())
	})
}

/// `ICvar::GetCommands`, which returns the most recently listed entry, whose
/// `m_pNext` leads through the rest.
unsafe extern "C" fn get_commands(_: *mut sys::ICvar) -> *mut sys::ConCommandBase {
	REGISTRY.with_borrow(|registry| registry.first().copied().unwrap_or(null_mut()))
}

#[test]
fn handlers_can_leave_client_invocations_to_the_game() {
	mock_engine();

	let command = leak_pinned(
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

	assert_eq!(
		route(&raw mut table[1], "jointeam red"),
		ClientRoute::NotRouted
	);
	assert_eq!(
		route(&raw mut table[1], "jointeam blue"),
		ClientRoute::Handled
	);
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
	let command = leak_pinned(
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

	assert_eq!(route(&raw mut table[1], "sb_greet"), ClientRoute::Handled);
	assert_eq!(client_prints(), [(1, "hello\n".to_owned())]);
}

/// Leaks a command or variable, pinned, as plugins keep theirs in statics.
fn leak_pinned<T>(object: T) -> Pin<&'static T> {
	Pin::static_ref(Box::leak(Box::new(object)))
}

/// Lists a command or variable another module declared.
fn list_foreign(name: &'static CStr, kind: CommandBaseKind) -> *mut sys::ConCommandBase {
	let next = REGISTRY.with_borrow(|registry| registry.first().copied().unwrap_or(null_mut()));
	let base = leak(mock_base(name, kind, CommandFlags::NONE, next));

	REGISTRY.with_borrow_mut(|registry| registry.insert(0, base));
	base
}

/// The names of the listed commands and variables, most recent first.
fn listed() -> Vec<String> {
	REGISTRY.with_borrow(|registry| {
		registry
			.iter()
			// SAFETY: The registry lists live commands and variables.
			.map(|&base| unsafe { name_of(base) }.to_str().unwrap().to_owned())
			.collect()
	})
}

/// Exports a mock `ICvar` and `IVEngineServer` on this thread.
fn mock_engine() {
	// SAFETY: The vtables hold only function pointers, `unexpected_call`
	// aborts whichever slot reaches it, and the patches only write slots of
	// the vtable being built.
	let (cvar_vtable, engine_vtable) = unsafe {
		(
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
			}),
			mock_vtable::<sys::IVEngineServer__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).IVEngineServer_ClientPrintf).write(client_printf);
				},
			),
		)
	};

	// SAFETY: Zero is valid for every field of `ConVar`.
	CHEATS.set(leak(unsafe { std::mem::zeroed::<sys::ConVar>() }));
	export(
		Module::Engine,
		Cvar::VERSION,
		leak(sys::ICvar {
			vtable_: Box::leak(cvar_vtable),
		}),
	);
	export(
		Module::Engine,
		ValveEngine::VERSION,
		leak(sys::IVEngineServer {
			vtable_: Box::leak(engine_vtable),
		}),
	);
}

/// The name of a command or variable, read through its vtable.
///
/// # Safety
///
/// `base` must be a live command or variable, which stays alive for the rest
/// of the test.
unsafe fn name_of(base: *mut sys::ConCommandBase) -> &'static CStr {
	// SAFETY: The caller keeps the object alive, and its name with it.
	unsafe { CStr::from_ptr(vcall!(base => ConCommandBase_GetName())) }
}

#[test]
fn names_in_use_are_refused() {
	mock_engine();
	list_foreign(c"changelevel", CommandBaseKind::Command);
	list_foreign(c"sv_cheats", CommandBaseKind::Variable);

	let command = |name| leak_pinned(ConsoleCommand::new(name, record));
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
fn nested_invocations_keep_their_own_invoker_and_arguments() {
	mock_engine();

	let inner =
		leak_pinned(ConsoleCommand::new(c"sb_inner", record).access(CommandAccess::Clients));
	let outer = leak_pinned(ConsoleCommand::new(
		c"sb_outer",
		|command: &CommandContext<'_>| {
			// The engine re-tokenizes its buffer in place when it runs a command.
			let raw = RAW_COMMAND.get();

			// SAFETY: The test keeps the command the engine runs alive, and
			// nothing else accesses it during the handler.
			unsafe {
				(&raw mut (*raw).m_nArgc).write(1);
				(&raw mut (*raw).m_pArgSBuffer).cast::<u8>().write(b'!');
			}

			// A handler making a client run a command re-enters the hook.
			assert_eq!(
				route(CLIENT_EDICT.get(), "sb_inner nested"),
				ClientRoute::Handled
			);
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

	// SAFETY: `raw` came from `Box::into_raw`, and the dispatch is over.
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

	let variable = leak_pinned(ConsoleVariable::new(c"sb_loud", c"0").flags(CommandFlags::NOTIFY));
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

	let base = base_of(variable.get_ref());

	// SAFETY: The variable is leaked.
	assert!(unsafe { vcall!(base => ConCommandBase_IsFlagSet(CommandFlags::NOTIFY.bits())) });
}

/// A handler that records the invocation and replies to its invoker.
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

/// Registers a command with the mock engine, through a host that unlinks it on
/// unload.
fn register<H: CommandHandler>(
	command: Pin<&'static ConsoleCommand<H>>,
) -> Result<(), RegisterCommandError> {
	let scope = ();
	let server = mock_server(&scope);

	command.register(server, mock_binding(), &Host(server.cvar().unwrap()))
}

/// `ICvar::RegisterConCommand`, which lists an unregistered command or
/// variable first, unless links are refused.
unsafe extern "C" fn register_con_command(_: *mut sys::ICvar, base: *mut sys::ConCommandBase) {
	// SAFETY: The wrappers pass a live command or variable, whose fields the
	// registry writes as `CCvar` does.
	unsafe {
		if REFUSE_LINKS.get() || vcall!(base => ConCommandBase_IsRegistered()) {
			return;
		}

		(&raw mut (*base).m_bRegistered).write(true);
		(&raw mut (*base).m_pNext).write(get_commands(null_mut()));
	}

	REGISTRY.with_borrow_mut(|registry| registry.insert(0, base));
}

/// Registers a variable with the mock engine, through a host that unlinks it
/// on unload.
fn register_variable(variable: Pin<&'static ConsoleVariable>) -> Result<(), RegisterCommandError> {
	let scope = ();
	let server = mock_server(&scope);

	variable.register(server, mock_binding(), &Host(server.cvar().unwrap()))
}

#[test]
fn registered_commands_and_variables_are_listed() {
	mock_engine();

	let command =
		leak_pinned(ConsoleCommand::new(c"sb_ping", record).flags(CommandFlags::GAME_DLL));
	let variable = leak_pinned(
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

	assert_eq!(var.as_ptr().cast(), base_of(variable.get_ref()));
	assert_eq!(var.default_string().as_c_str(), c"3");
	assert!(var.is_default());

	var.set_string(c"5");
	assert!(!var.is_default());

	// The variable now holds its own copy of the string, not the default.
	var.set_string(c"3");

	// SAFETY: The variable is leaked.
	let string = unsafe { (&raw const (*var.as_ptr()).m_pszString).read() };

	assert_ne!(string.cast_const(), variable.default_value().as_ptr());
	assert!(var.is_default());
}

#[test]
fn registration_links_once_and_unregisters() {
	mock_engine();

	let other = leak_pinned(ConsoleCommand::new(c"sb_other", record));
	let command = leak_pinned(ConsoleCommand::new(c"sb_ping", record).help(c"Replies."));

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

	// SAFETY: The command is leaked, and its strings with it.
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

	// SAFETY: As above.
	let identifier = unsafe { vcall!(base => ConCommandBase_GetDLLIdentifier()) };

	assert_eq!(identifier, 9);
}

/// The handler of [`SELF_REMOVING`].
fn remove_self(command: &CommandContext<'_>) -> CommandResult {
	let server = command.server();

	SELF_REMOVING
		.unregister(server, &server.cvar()?)
		.map_err(CommandError::other)?;
	command.reply("removed")?;
	Ok(())
}

/// Runs `line` as the string command of the client whose edict is `edict`, a
/// slot of a mock edict table, as the game's `ClientCommand` hook would.
fn route(edict: *mut sys::edict_t, line: &str) -> ClientRoute {
	// SAFETY: The edict is a live slot of a mock table, and the command is
	// tokenized for the call, which runs on the test's thread, standing in
	// for the server's main thread.
	unsafe {
		route_client_command(
			&mock_binding(),
			NonNull::new(edict).unwrap(),
			NonNull::from(&*tokenized(line)),
		)
	}
}

/// The invocations [`record`] saw since the last call.
fn seen() -> Vec<Seen> {
	SEEN.take()
}

/// Sets `sv_cheats`.
fn set_cheats(value: c_int) {
	// SAFETY: `mock_engine` leaked the variable.
	unsafe { (&raw mut (*CHEATS.get()).m_nValue).write(value) };
}

/// Copies a NUL-terminated UTF-8 string.
///
/// # Safety
///
/// `pointer` must point to a NUL-terminated string.
unsafe fn text(pointer: *const c_char) -> String {
	// SAFETY: The caller passes a NUL-terminated string.
	unsafe { CStr::from_ptr(pointer) }
		.to_str()
		.unwrap()
		.to_owned()
}

#[test]
fn the_engine_dispatches_server_invocations() {
	mock_engine();

	let command =
		leak_pinned(ConsoleCommand::new(c"sb_ping", record).access(CommandAccess::Everyone));

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
fn the_engine_sets_variables_through_their_interface() {
	mock_engine();

	let variable = leak_pinned(ConsoleVariable::new(c"sb_bots", c"10").min(1.0).max(32.0));
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

	let command = leak_pinned(ConsoleCommand::new(c"sb_ping", record));
	let error = register(command).unwrap_err();

	assert_eq!(error.kind(), &RegisterCommandErrorKind::NotLinked);
	assert_eq!(error.name(), c"sb_ping");
	assert!(listed().is_empty());

	REFUSE_LINKS.set(false);
	register(command).unwrap();
}

/// `ICvar::UnregisterConCommand`, which unlists a registered command or
/// variable, relinking the entry before it.
unsafe extern "C" fn unregister_con_command(_: *mut sys::ICvar, base: *mut sys::ConCommandBase) {
	// SAFETY: The wrappers pass a live command or variable.
	if !unsafe { vcall!(base => ConCommandBase_IsRegistered()) } {
		return;
	}

	// SAFETY: As above, and the registry writes these fields as `CCvar` does.
	unsafe {
		(&raw mut (*base).m_bRegistered).write(false);
		(&raw mut (*base).m_pNext).write(null_mut());
	}

	REGISTRY.with_borrow_mut(|registry| {
		let Some(position) = registry.iter().position(|&entry| entry == base) else {
			return;
		};

		registry.remove(position);

		if let Some(&previous) = position
			.checked_sub(1)
			.and_then(|index| registry.get(index))
		{
			let next = registry.get(position).copied().unwrap_or(null_mut());

			// SAFETY: The registry lists live commands and variables.
			unsafe { (&raw mut (*previous).m_pNext).write(next) };
		}
	});
}

#[test]
fn variables_hold_their_default_and_register_as_variables() {
	mock_engine();

	let variable =
		leak_pinned(ConsoleVariable::new(c"sb_rounds", c"  12.5 rounds").help(c"Rounds."));
	let scope = ();
	let server = mock_server(&scope);

	// Floats become the integer value as tier1's cast makes them.
	variable.set_float(server, f32::NEG_INFINITY);
	assert_eq!(variable.int(server), c_int::MIN);

	// Setting an unregistered variable runs no callbacks.
	variable.set_int(server, 3);
	assert_eq!(variable.string(server).as_c_str(), c"3");
	assert!(changes().is_empty());

	register_variable(variable).unwrap();
	assert_eq!(listed(), ["sb_rounds"]);

	let base = base_of(variable.get_ref());

	// SAFETY: The variable is leaked, and its strings with it.
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
		register(leak_pinned(ConsoleCommand::new(c"sb_rounds", record)))
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
