//! Tests of reading the engine's console registry (`ICvar`): its list of
//! commands and variables, and variables as other modules declare them.

use source_sdk_2013::commands::{CommandBaseKind, CommandFlags};
use source_sdk_2013::interfaces::Cvar;
use source_sdk_2013::interfaces::cvar::ConVar;
use source_sdk_2013::test_support::interfaces::cvar::{mock_command, mock_cvar, mock_var};
use source_sdk_2013::{Game, InterfaceFactory, Server};
use std::cell::Cell;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr::null_mut;

/// The most entries a listing reads, which keeps a corrupted registry that
/// links in a loop from hanging the server.
const MAX_LISTED: usize = 65_536;

thread_local! {
	/// The registry [`engine_factory`] exports.
	static REGISTRY: Cell<*mut sys::ICvar> = const { Cell::new(null_mut()) };
}

/// A registry as the wrappers see it, from the leaked mock `registry`.
fn cvar(registry: *mut sys::ICvar) -> Cvar<'static> {
	REGISTRY.set(registry);

	let factory = InterfaceFactory::new(engine_factory);

	// SAFETY: The factory exports only the leaked registry, which stays alive
	// for the rest of the test, on this thread.
	let server = unsafe { Server::new(factory, factory, Game::TeamFortress2, &()) };

	server.cvar().unwrap()
}

/// An interface factory that exports the registry [`cvar`] last set, and
/// nothing else.
unsafe extern "C" fn engine_factory(name: *const c_char, _: *mut c_int) -> *mut c_void {
	// SAFETY: Factories are called with NUL-terminated names.
	if unsafe { CStr::from_ptr(name) } == Cvar::VERSION {
		REGISTRY.get().cast()
	} else {
		null_mut()
	}
}

#[test]
fn listing_ends_at_the_last_entry_or_the_limit() {
	assert_eq!(
		cvar(mock_cvar(null_mut(), Vec::new()))
			.command_bases()
			.len(),
		0
	);
	assert_eq!(cvar(mock_cvar(null_mut(), Vec::new())).vars().count(), 0);

	// A corrupted registry that links an entry to itself.
	let looped = mock_command(c"sb_loop", null_mut());

	// SAFETY: The command is leaked, and nothing else refers to it yet.
	unsafe { (*looped).m_pNext = looped };

	let mut listed = cvar(mock_cvar(looped, Vec::new())).command_bases();

	assert_eq!(listed.len(), MAX_LISTED);
	assert!(listed.all(|base| base.as_ptr() == looped));
}

#[test]
fn the_registry_lists_variables_and_commands_in_order() {
	let gravity = mock_var(
		c"sv_gravity",
		c"800",
		c"800",
		CommandFlags::REPLICATED | CommandFlags::NOTIFY,
		null_mut(),
	);
	let status = mock_command(c"status", gravity.cast());
	let timelimit = mock_var(c"mp_timelimit", c"0", c"30", CommandFlags::NOTIFY, status);
	let cvar = cvar(mock_cvar(timelimit.cast(), Vec::new()));
	let mut listed = cvar.command_bases();

	assert_eq!(listed.len(), 3);
	assert_eq!(
		listed
			.clone()
			.map(|base| (base.name(), base.kind(), base.flags()))
			.collect::<Vec<_>>(),
		[
			(
				c"mp_timelimit",
				CommandBaseKind::Variable,
				CommandFlags::NOTIFY
			),
			(c"status", CommandBaseKind::Command, CommandFlags::NONE),
			(
				c"sv_gravity",
				CommandBaseKind::Variable,
				CommandFlags::REPLICATED | CommandFlags::NOTIFY
			),
		]
	);
	assert_eq!(listed.next_back().unwrap().as_ptr(), gravity.cast());
	assert_eq!(
		cvar.vars()
			.map(|var| (var.as_ptr(), var.is_default()))
			.collect::<Vec<_>>(),
		[(timelimit, false), (gravity, true)]
	);
}

#[test]
fn variables_report_the_flags_and_default_of_their_parent() {
	// A bit iconvar.h leaves unassigned.
	let unknown = 1 << 27;
	let flags =
		CommandFlags::from_bits_retain(unknown) | CommandFlags::REPLICATED | CommandFlags::CHEAT;
	let parent = mock_var(c"sv_cheats", c"0", c"1", flags, null_mut());

	// Another module's variable of the same name, which the engine pointed at
	// the first.
	let child = mock_var(c"sv_cheats", c"1", c"1", CommandFlags::NONE, null_mut());

	// SAFETY: The variable is leaked, and nothing else refers to it yet.
	unsafe { (*child).m_pParent = parent };

	// Each registry's `FindVar` finds one of the two.
	let find = |var| -> ConVar<'static> {
		cvar(mock_cvar(null_mut(), vec![var]))
			.find_var(c"sv_cheats")
			.unwrap()
	};
	let (parent, child) = (find(parent), find(child));

	for var in [parent, child] {
		assert_eq!(var.flags(), flags);
		assert_eq!(var.flags().bits() & unknown, unknown);
		assert!(
			var.flags()
				.contains(CommandFlags::REPLICATED | CommandFlags::CHEAT)
		);
		assert!(!var.flags().contains(CommandFlags::SERVER_CANNOT_QUERY));
		assert_eq!(var.default_string().as_c_str(), c"0");
		assert!(!var.is_default());
	}

	// SAFETY: The parent is leaked, and the engine writes this field too.
	let string = unsafe { &raw mut (*parent.as_ptr()).m_pszString };

	// SAFETY: As above, and each string is static.
	unsafe { string.write(c"0".as_ptr().cast_mut()) };
	assert!(child.is_default());

	// Compared byte for byte, not by number.
	// SAFETY: As above.
	unsafe { string.write(c"0.0".as_ptr().cast_mut()) };
	assert!(!child.is_default());

	// A missing string reads as empty, as `GetString` returns it.
	// SAFETY: As above.
	unsafe { string.write(null_mut()) };
	assert_eq!(child.string().as_c_str(), c"");
	assert!(!child.is_default());

	// SAFETY: As above.
	unsafe { (&raw mut (*parent.as_ptr()).m_pszDefaultValue).write(c"".as_ptr()) };
	assert!(child.is_default());
}
