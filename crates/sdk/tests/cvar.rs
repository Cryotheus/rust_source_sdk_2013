//! Tests of reading the engine's console registry (`ICvar`): its list of
//! commands and variables, and variables as other modules declare them.

use sdk_raw::vcall;
use source_sdk_2013::commands::{CommandBaseKind, CommandFlags};
use source_sdk_2013::interfaces::Cvar;
use source_sdk_2013::interfaces::cvar::{ConVar, ConVarChange, ConVarWatchError};
use source_sdk_2013::test_support::interfaces::cvar::{mock_command, mock_cvar, mock_var};
use source_sdk_2013::{Game, InterfaceFactory, Server, ServerBinding};
use std::cell::{Cell, RefCell};
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::ptr::{null, null_mut};

/// The most entries a listing reads, which keeps a corrupted registry that
/// links in a loop from hanging the server.
const MAX_LISTED: usize = 65_536;

thread_local! {
	/// The registry [`engine_factory`] exports.
	static REGISTRY: Cell<*mut sys::ICvar> = const { Cell::new(null_mut()) };

	/// The changes [`record`] saw on this thread.
	static SEEN: RefCell<Vec<Seen>> = const { RefCell::new(Vec::new()) };

	/// How many times [`panic_on_change`] ran on this thread.
	static PANICKED: Cell<usize> = const { Cell::new(0) };
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

#[test]
fn entries_read_their_help_text_and_bounds() {
	let parent = mock_var(
		c"sv_gravity",
		c"800",
		c"800",
		CommandFlags::NOTIFY,
		null_mut(),
	);
	let child = mock_var(
		c"sv_gravity",
		c"800",
		c"800",
		CommandFlags::NONE,
		null_mut(),
	);
	let status = mock_command(c"status", parent.cast());

	// SAFETY: The entries are leaked, and nothing else refers to them yet.
	unsafe {
		(*child).m_pParent = parent;
		(*parent)._base.m_pszHelpString = c"World gravity.".as_ptr();
		(*child)._base.m_pszHelpString = c"Another module's text.".as_ptr();
		(*status).m_pszHelpString = c"Display map and connection status.".as_ptr();
		(*parent).m_bHasMin = true;
		(*parent).m_fMinVal = -4.5;
		(*child).m_bHasMax = true;
		(*child).m_fMaxVal = 10.0;
	}

	let cvar = cvar(mock_cvar(status, vec![child]));

	assert_eq!(
		cvar.command_bases()
			.map(|base| (base.name(), base.help_text()))
			.collect::<Vec<_>>(),
		[
			(c"status", c"Display map and connection status."),
			(c"sv_gravity", c"World gravity."),
		]
	);

	// The variable that holds the value has the text and the bounds.
	let child = cvar.find_var(c"sv_gravity").unwrap();

	assert_eq!(child.help_text(), c"World gravity.");
	assert_eq!(child.bounds(), (Some(-4.5), None));

	// A missing text reads as empty.
	// SAFETY: As above.
	unsafe { (*parent)._base.m_pszHelpString = std::ptr::null() };
	assert_eq!(child.help_text(), c"");
}

/// A change [`record`] saw.
#[derive(Debug, PartialEq)]
struct Seen {
	name: CString,
	var: Option<*mut sys::ConVar>,
	old: CString,
	old_float: f32,

	/// The variable's value as the callback read it, if it got the variable.
	new: Option<CString>,
}

/// A binding to the factories of the servers [`cvar`] makes, which export the
/// registry it last set.
fn binding() -> ServerBinding {
	let factory = InterfaceFactory::new(engine_factory);

	// SAFETY: As for `cvar`.
	unsafe { ServerBinding::new(factory, factory, Game::TeamFortress2) }
}

/// Tells the registry's global change callbacks that `var` changed from
/// `old`, as a variable does after each change of its string.
fn call_global_change_callbacks(
	registry: *mut sys::ICvar,
	var: *mut sys::ConVar,
	old: *const c_char,
	old_float: f32,
) {
	// SAFETY: The registry and the variable, if any, are leaked mocks, and the
	// old string, if any, is NUL-terminated.
	unsafe { vcall!(registry => ICvar_CallGlobalChangeCallbacks(var, old, old_float)) };
}

/// Counts its runs in [`PANICKED`], then panics.
fn panic_on_change(_: Server<'_>, _: ConVarChange<'_>) {
	PANICKED.set(PANICKED.get() + 1);
	panic!("the callback failed");
}

/// Records each change it gets in [`SEEN`].
fn record(_: Server<'_>, change: ConVarChange<'_>) {
	let seen = Seen {
		name: change.name().to_owned(),
		var: change.var().map(ConVar::as_ptr),
		old: change.old_string().to_owned(),
		old_float: change.old_float(),
		new: change.var().map(ConVar::string),
	};

	SEEN.with_borrow_mut(|changes| changes.push(seen));
}

/// The changes [`record`] saw on this thread since the last call.
fn seen() -> Vec<Seen> {
	SEEN.take()
}

#[test]
fn watches_pass_on_each_change_with_its_variable_and_old_value() {
	let gravity = mock_var(
		c"sv_gravity",
		c"800",
		c"600",
		CommandFlags::NOTIFY,
		null_mut(),
	);
	let registry = mock_cvar(gravity.cast(), vec![gravity]);
	let watch = cvar(registry).watch_changes(binding(), record).unwrap();
	let seen_gravity = |old: &CStr, old_float| Seen {
		name: c"sv_gravity".to_owned(),
		var: Some(gravity),
		old: old.to_owned(),
		old_float,
		new: Some(c"600".to_owned()),
	};

	call_global_change_callbacks(registry, gravity, c"800".as_ptr(), 800.0);
	assert_eq!(seen(), [seen_gravity(c"800", 800.0)]);

	// A missing old string reads as empty, and a missing variable is skipped.
	call_global_change_callbacks(registry, gravity, null(), 0.0);
	call_global_change_callbacks(registry, null_mut(), c"1".as_ptr(), 1.0);
	assert_eq!(seen(), [seen_gravity(c"", 0.0)]);

	drop(watch);
}

#[test]
fn one_watch_runs_at_a_time() {
	let gravity = mock_var(
		c"sv_gravity",
		c"800",
		c"600",
		CommandFlags::NONE,
		null_mut(),
	);
	let registry = mock_cvar(gravity.cast(), vec![gravity]);
	let cvar = cvar(registry);
	let change = || call_global_change_callbacks(registry, gravity, c"800".as_ptr(), 800.0);
	let watch = cvar.watch_changes(binding(), record).unwrap();

	assert_eq!(
		cvar.watch_changes(binding(), panic_on_change).unwrap_err(),
		ConVarWatchError::AlreadyWatching
	);
	assert_eq!(
		ConVarWatchError::AlreadyWatching.to_string(),
		"console variable changes are already being watched"
	);

	// The refused watch installed nothing.
	change();
	assert_eq!(seen().len(), 1);
	assert_eq!(PANICKED.take(), 0);

	// Once stopped, another may start, and the engine calls only that one, so
	// stopping removed the first.
	watch.stop();
	change();
	assert!(seen().is_empty());

	let watch = cvar.watch_changes(binding(), record).unwrap();

	change();
	assert_eq!(seen().len(), 1);

	// Dropping a watch stops it too.
	drop(watch);
	change();
	assert!(seen().is_empty());
	assert!(cvar.watch_changes(binding(), record).is_ok());
}

#[test]
fn unlisted_variables_are_passed_by_name() {
	let gravity = mock_var(
		c"sv_gravity",
		c"800",
		c"600",
		CommandFlags::NONE,
		null_mut(),
	);

	// A variable a module declared but has not registered, which the registry
	// does not list.
	let stray = mock_var(c"sb_stray", c"0", c"1", CommandFlags::NONE, null_mut());

	// Another variable of a listed name, which is not the one listed.
	let shadow = mock_var(
		c"SV_Gravity",
		c"800",
		c"100",
		CommandFlags::NONE,
		null_mut(),
	);
	let registry = mock_cvar(gravity.cast(), vec![gravity]);
	let watch = cvar(registry).watch_changes(binding(), record).unwrap();
	let unlisted = |name: &CStr, old: &CStr, old_float| Seen {
		name: name.to_owned(),
		var: None,
		old: old.to_owned(),
		old_float,
		new: None,
	};

	call_global_change_callbacks(registry, stray, c"0".as_ptr(), 0.0);
	call_global_change_callbacks(registry, shadow, c"800".as_ptr(), 800.0);
	assert_eq!(
		seen(),
		[
			unlisted(c"sb_stray", c"0", 0.0),
			unlisted(c"SV_Gravity", c"800", 800.0)
		]
	);

	drop(watch);
}

#[test]
fn panics_are_contained() {
	let gravity = mock_var(
		c"sv_gravity",
		c"800",
		c"600",
		CommandFlags::NONE,
		null_mut(),
	);
	let registry = mock_cvar(gravity.cast(), vec![gravity]);
	let watch = cvar(registry)
		.watch_changes(binding(), panic_on_change)
		.unwrap();

	// Each panic ends its call, and the next change is passed on.
	call_global_change_callbacks(registry, gravity, c"800".as_ptr(), 800.0);
	call_global_change_callbacks(registry, gravity, c"700".as_ptr(), 700.0);
	assert_eq!(PANICKED.take(), 2);

	drop(watch);
}

#[test]
fn changes_on_other_threads_are_skipped() {
	/// The mock registry and variable, which another thread changes.
	struct Engine {
		registry: *mut sys::ICvar,
		var: *mut sys::ConVar,
	}

	// SAFETY: The mocks are leaked, and the test's thread waits while the
	// other uses them.
	unsafe impl Send for Engine {}

	impl Engine {
		/// Changes the variable on this thread, and counts the changes
		/// [`record`] saw on it.
		fn change(&self) -> usize {
			call_global_change_callbacks(self.registry, self.var, c"800".as_ptr(), 800.0);
			seen().len()
		}
	}

	let gravity = mock_var(
		c"sv_gravity",
		c"800",
		c"600",
		CommandFlags::NONE,
		null_mut(),
	);
	let registry = mock_cvar(gravity.cast(), vec![gravity]);
	let watch = cvar(registry).watch_changes(binding(), record).unwrap();
	let engine = Engine {
		registry,
		var: gravity,
	};

	// Neither thread hears of the other thread's change.
	assert_eq!(
		std::thread::spawn(move || engine.change()).join().unwrap(),
		0
	);
	assert!(seen().is_empty());

	// This thread's changes are still passed on.
	call_global_change_callbacks(registry, gravity, c"800".as_ptr(), 800.0);
	assert_eq!(seen().len(), 1);

	drop(watch);
}
