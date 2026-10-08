//! Tests of the engine's calls that set a `ConsoleVariable`, on prepared but
//! unregistered variables, whose changes run no callbacks.

use super::*;
use crate::server::TEST_MSG;
use crate::test_support::server::{mock_binding, mock_server};
use sdk_raw::vcall;
use std::cell::RefCell;
use std::ffi::c_char;

thread_local! {
	/// What `Msg`'s stand-in printed, which reaches a dedicated server's
	/// console.
	static PRINTED: RefCell<Vec<CString>> = const { RefCell::new(Vec::new()) };
}

/// A stand-in for tier0's `Msg`, which records the string its `%s` reads.
unsafe extern "C" fn msg(_format: *const c_char, mut arguments: ...) {
	// SAFETY: `print_through` passes a NUL-terminated string for the `%s`.
	let printed = unsafe { CStr::from_ptr(arguments.next_arg::<*const c_char>()) }.to_owned();

	PRINTED.with_borrow_mut(|lines| lines.push(printed));
}

/// Prepares `variable` as registering it would, without linking it.
fn prepare(variable: &'static ConsoleVariable) {
	// SAFETY: The variable is a `static`, so it never moves, and is never
	// registered.
	unsafe { variable.prepare(mock_binding(), 0) };
}

/// Sets `variable` through its `IConVar`, as the console, configs and other
/// plugins do, from a string, a float and an integer in turn.
fn set_as_engine(variable: &ConsoleVariable, text: &CStr, float: f32, int: c_int) {
	// SAFETY: The variable is prepared, so its vtables are filled in, and the
	// calls run on this thread, as the engine's would on its main thread.
	unsafe {
		let interface = ConVarObject::interface(variable.object_ptr()).as_ptr();

		vcall!(interface => IConVar_SetValue(text.as_ptr()));
		vcall!(interface => IConVar_SetValue1(float));
		vcall!(interface => IConVar_SetValue2(int));
	}
}

#[test]
fn the_engine_changes_a_variable() {
	static VARIABLE: ConsoleVariable = ConsoleVariable::new(c"sb_test_changeable", c"idle");

	let scope = ();
	let server = mock_server(&scope);

	prepare(&VARIABLE);
	TEST_MSG.set(Some(msg));

	set_as_engine(&VARIABLE, c"calm", 2.5, 3);
	assert_eq!(VARIABLE.string(server), c"3");
	assert_eq!(VARIABLE.int(server), 3);
	assert!(PRINTED.take().is_empty());

	TEST_MSG.set(None);
}

#[test]
fn the_engine_cannot_change_a_read_only_variable() {
	static VARIABLE: ConsoleVariable =
		ConsoleVariable::new(c"sb_test_read_only", c"idle").read_only();

	let scope = ();
	let server = mock_server(&scope);

	prepare(&VARIABLE);
	TEST_MSG.set(Some(msg));

	// Each of the three calls is refused, and says so.
	set_as_engine(&VARIABLE, c"calm", 2.5, 3);
	assert_eq!(VARIABLE.string(server), c"idle");
	assert_eq!(VARIABLE.float(server), 0.0);
	assert_eq!(PRINTED.take(), [c"sb_test_read_only is read-only.\n"; 3]);

	// Its own setters still change it.
	VARIABLE.set_string(server, c"pregame");
	assert_eq!(VARIABLE.string(server), c"pregame");

	VARIABLE.revert(server);
	assert_eq!(VARIABLE.string(server), c"idle");
	assert!(PRINTED.take().is_empty());

	TEST_MSG.set(None);
}
