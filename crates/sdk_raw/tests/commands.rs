//! Tests of console commands and variables: copies of the engine's `CCommand`,
//! the `ConCommand` object's vtable, and tier1's value parsing and formatting.

use source_sdk_2013_raw::commands::{
	CommandLine, CommandLineError, ConCommandHooks, ConCommandObject, FCVAR_DONTRECORD,
	FCVAR_GAMEDLL, FCVAR_HIDDEN, format_float, format_int, parse_float, parse_int,
};
use source_sdk_2013_raw::test_support::commands::tokenized;
use std::cell::RefCell;
use std::ffi::{CStr, c_char, c_int};
use std::ptr::{self, NonNull, null_mut};

#[test]
fn copies_are_indexed_from_the_name() {
	let raw = tokenized(
		"sb_give scout \"1 2\" 3",
		&["sb_give", "scout", "1 2", "3"],
		8,
	);
	let line = copy(&raw).unwrap();

	assert_eq!(line.argc(), 4);
	assert_eq!(line.arg(0), Some(c"sb_give"));
	assert_eq!(line.arg(2), Some(c"1 2"));
	assert_eq!(line.arg(4), None);
	assert_eq!(line.line(), c"sb_give scout \"1 2\" 3");
	assert_eq!(line.raw_args(), c"scout \"1 2\" 3");
}

/// Copies a command the tests built, which stays unchanged for the call.
fn copy(raw: &sys::CCommand) -> Result<CommandLine, CommandLineError> {
	// SAFETY: `raw` is live and unmodified for the call.
	unsafe { CommandLine::copy(NonNull::from(raw)) }
}

#[test]
fn malformed_commands_are_refused() {
	let mut raw = tokenized("a", &["a"], 0);

	raw.m_nArgc = 0;
	assert_eq!(copy(&raw).err(), Some(CommandLineError::ArgCount(0)));

	raw.m_nArgc = 1;
	raw.m_nArgv0Size = 2;
	assert_eq!(copy(&raw).err(), Some(CommandLineError::ArgsOffset(2)));

	raw.m_nArgv0Size = 1;
	raw.m_ppArgv[0] = c"elsewhere".as_ptr();
	assert_eq!(
		copy(&raw).err(),
		Some(CommandLineError::ArgOutsideBuffer { index: 0 })
	);

	raw.m_pArgSBuffer.fill(b'x' as c_char);
	assert_eq!(copy(&raw).err(), Some(CommandLineError::Unterminated));
}

#[test]
fn raw_arguments_match_args() {
	for line in ["sb_say", "sb_say   "] {
		let raw = tokenized(line, &["sb_say"], 0);
		let line = copy(&raw).unwrap();

		assert_eq!(line.argc(), 1);
		assert_eq!(line.raw_args(), c"");
	}

	let raw = tokenized(
		"sb_echo \"This is cryotheum\"",
		&["sb_echo", "This is cryotheum"],
		8,
	);
	let line = copy(&raw).unwrap();

	assert_eq!(line.arg(1), Some(c"This is cryotheum"));
	assert_eq!(line.raw_args(), c"\"This is cryotheum\"");
}

/// The hooks of the tests' commands, which record what they dispatch.
static HOOKS: ConCommandHooks = ConCommandHooks { dispatch: record };

/// Hooks of another owner's commands.
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

/// Records the command and its argument count in [`DISPATCHED`].
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

	// SAFETY: The slots are called as the engine calls them, on a prepared
	// command, whose flags are written without forming a reference.
	unsafe {
		((*vtable).ConCommand_AddFlags)(this, FCVAR_GAMEDLL | FCVAR_HIDDEN);
		assert!(((*vtable).ConCommand_IsFlagSet)(this, FCVAR_HIDDEN));
		assert!(!((*vtable).ConCommand_IsFlagSet)(this, FCVAR_GAMEDLL));

		// Another plugin may write the flags directly.
		(&raw mut (*base).m_nFlags).write(FCVAR_GAMEDLL);
		assert!(!((*vtable).ConCommand_IsFlagSet)(this, -1));
	}
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

#[test]
fn values_format_as_tier1_does() {
	let long = format_float(1.0e30);

	assert_eq!(long.as_bytes().len(), 31);
	assert!(
		long.to_str()
			.unwrap()
			.starts_with("1000000015047466219876688855040")
	);
	assert_eq!(format_float(2.5).as_c_str(), c"2.500000");
	assert_eq!(format_float(f32::NEG_INFINITY).as_c_str(), c"-inf");
	assert_eq!(format_float(-f32::NAN).as_c_str(), c"-nan");
	assert_eq!(format_int(-7).as_c_str(), c"-7");
}

#[test]
fn values_parse_as_c_does() {
	assert_eq!(parse_float(b"3.0"), 3.0);
	assert_eq!(parse_float(b" \t-1.5e1x"), -15.0);
	assert_eq!(parse_float(b"+2E-1"), 0.2);
	assert_eq!(parse_float(b".5"), 0.5);
	assert_eq!(parse_float(b"5."), 5.0);
	assert_eq!(parse_float(b"1e"), 1.0);
	assert_eq!(parse_float(b"1e+"), 1.0);
	assert_eq!(parse_float(b"e5"), 0.0);
	assert_eq!(parse_float(b"."), 0.0);
	assert_eq!(parse_float(b""), 0.0);
	assert_eq!(parse_float(b"1e999"), f64::INFINITY);
	assert_eq!(parse_float(b"0.01e-99999999999999999999"), 0.0);
	assert_eq!(
		parse_float(b"100000000000000000000e99999999999999999999"),
		f64::INFINITY
	);
	assert_eq!(parse_float(b"0e309"), 0.0);
	assert!(parse_float(b"-0e400").is_sign_negative());
	assert_eq!(
		parse_float(b"123456789012345678901234") as f32,
		1.234_567_9e23
	);

	assert_eq!(parse_int(b" 42 bots"), 42);
	assert_eq!(parse_int(b"-7"), -7);
	assert_eq!(parse_int(b"3.9"), 3);
	assert_eq!(parse_int(b"x"), 0);
	assert_eq!(parse_int(b"99999999999"), c_int::MAX);
	assert_eq!(parse_int(b"-99999999999"), c_int::MIN);
}
