//! Tests of `ConVar::quietly`: the flag it clears while its closure runs, and
//! sets back after, on a variable of a mock registry.

use super::*;
use crate::server::Module;
use crate::test_support::interfaces::cvar::{mock_cvar, mock_var};
use crate::test_support::server::{export, mock_server};
use std::ptr::null_mut;

/// `sv_gravity`, marked `flags`, as the registry of a mock server finds it,
/// with the variable itself.
fn gravity(scope: &(), flags: CommandFlags) -> (ConVar<'_>, *mut sys::ConVar) {
	let raw = mock_var(c"sv_gravity", c"800", c"800", flags, null_mut());

	export(
		Module::Engine,
		sdk_raw::interfaces::cvar::VERSION,
		mock_cvar(raw.cast(), vec![raw]),
	);

	let var = mock_server(scope)
		.cvar()
		.unwrap()
		.find_var(c"sv_gravity")
		.unwrap();

	(var, raw)
}

#[test]
fn a_variable_goes_unannounced_only_while_quietly_runs() {
	let scope = ();
	let (var, _) = gravity(&scope, CommandFlags::NOTIFY | CommandFlags::REPLICATED);

	let inside = var.quietly(|| var.flags());

	assert!(!inside.contains(CommandFlags::NOTIFY));
	assert!(inside.contains(CommandFlags::REPLICATED));
	assert!(
		var.flags()
			.contains(CommandFlags::NOTIFY | CommandFlags::REPLICATED)
	);
}

#[test]
fn quietly_keeps_the_flags_added_and_leaves_an_unannounced_variable_so() {
	let scope = ();
	let (var, raw) = gravity(&scope, CommandFlags::REPLICATED);

	// As a change callback adding a flag would.
	let returned = var.quietly(|| {
		// SAFETY: The variable is leaked, and nothing else reads its flags
		// meanwhile.
		unsafe { (*raw)._base.m_nFlags |= CommandFlags::CHEAT.bits() };
		7
	});

	assert_eq!(returned, 7);
	assert_eq!(var.flags(), CommandFlags::REPLICATED | CommandFlags::CHEAT);
}

#[test]
fn quietly_sets_the_flag_back_after_a_panic() {
	let scope = ();
	let (var, _) = gravity(&scope, CommandFlags::NOTIFY);

	let panicked = std::panic::catch_unwind(AssertUnwindSafe(|| {
		var.quietly(|| panic!("the closure panics"));
	}));

	assert!(panicked.is_err());
	assert_eq!(var.flags(), CommandFlags::NOTIFY);
}
