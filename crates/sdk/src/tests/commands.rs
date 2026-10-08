//! Tests of where commands' replies to the server console go, with and
//! without the stand-in for tier0's `Msg` that only unit tests can install.

use super::*;
use crate::interfaces::Cvar;
use crate::server::{Module, TEST_MSG};
use crate::test_support::leak;
use crate::test_support::server::{export, mock_server};
use sdk_raw::commands::CommandLine;
use sdk_raw::test_support::commands::tokenized;
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use std::cell::RefCell;
use std::ffi::c_char;
use std::ptr::NonNull;

thread_local! {
	/// The format and first argument of each call to `Msg`'s stand-in, whose
	/// output reaches a dedicated server's console.
	static MSG: RefCell<Vec<(CString, CString)>> = const { RefCell::new(Vec::new()) };

	/// The format and first argument of each `ICvar::ConsolePrintf`, whose
	/// output a dedicated server discards.
	static DISPLAY_FUNCS: RefCell<Vec<(CString, CString)>> = const { RefCell::new(Vec::new()) };
}

/// `ICvar::ConsolePrintf`, which records its format and the string its `%s`
/// reads.
unsafe extern "C" fn console_printf(
	_: *const sys::ICvar,
	format: *const c_char,
	mut arguments: ...
) {
	// SAFETY: The wrapper passes a NUL-terminated format, and a NUL-terminated
	// string for its `%s`.
	let call = unsafe {
		(
			CStr::from_ptr(format).to_owned(),
			CStr::from_ptr(arguments.next_arg::<*const c_char>()).to_owned(),
		)
	};

	DISPLAY_FUNCS.with_borrow_mut(|calls| calls.push(call));
}

/// A stand-in for tier0's `Msg`, which records its format and the string its
/// `%s` reads.
unsafe extern "C" fn msg(format: *const c_char, mut arguments: ...) {
	// SAFETY: As for `console_printf`.
	let call = unsafe {
		(
			CStr::from_ptr(format).to_owned(),
			CStr::from_ptr(arguments.next_arg::<*const c_char>()).to_owned(),
		)
	};

	MSG.with_borrow_mut(|calls| calls.push(call));
}

#[test]
fn server_replies_use_the_console_display_functions_without_tier0() {
	// SAFETY: The vtable holds only function pointers, `unexpected_call` aborts
	// whichever slot reaches it, and the patch only writes a slot of the
	// vtable being built.
	let vtable = unsafe {
		mock_vtable::<sys::ICvar__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).ICvar_ConsolePrintf).write(console_printf);
		})
	};

	export(
		Module::Engine,
		Cvar::VERSION,
		leak(sys::ICvar {
			vtable_: Box::leak(vtable),
		}),
	);

	let scope = ();
	let raw = tokenized("sb_ping", &["sb_ping"], 0);

	// SAFETY: The command stays live and unchanged for the call.
	let line = unsafe { CommandLine::copy(NonNull::from(&*raw)) }.unwrap();
	let command = CommandContext::new(
		mock_server(&scope),
		CommandArgs::new(&line),
		Invoker::Server,
		c"sb_ping",
	);
	let printed = [(c"%s".to_owned(), c"ran sb_ping\n".to_owned())];

	// tier0's `Msg` reaches a dedicated server's console, so it comes first.
	TEST_MSG.set(Some(msg));
	command.reply("ran sb_ping").unwrap();
	assert_eq!(MSG.take(), printed);
	assert!(DISPLAY_FUNCS.take().is_empty());

	TEST_MSG.set(None);
	command.reply("ran sb_ping").unwrap();
	assert!(MSG.take().is_empty());
	assert_eq!(DISPLAY_FUNCS.take(), printed);
}
