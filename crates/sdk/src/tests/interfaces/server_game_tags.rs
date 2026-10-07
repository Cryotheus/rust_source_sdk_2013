//! Tests of `TaggedConVars`: reading and excluding the entries of a mock list
//! laid out as TF2's game lists them.

use super::*;
use sdk_raw::test_support::key_values::{MockKey, MockTree};

#[test]
fn an_empty_list_has_no_entries() {
	let tree = MockTree::new(&MockKey::Section(c"GameTags", vec![]));

	// SAFETY: The tree lives through the test, laid out as TF2's, and only this
	// handle reads it.
	let mut tags = unsafe { TaggedConVars::from_raw(tree.root()) };

	assert_eq!(tags.iter_mut().count(), 0);
}

#[test]
fn entries_name_their_variable_and_tag_in_order() {
	let tree = list();

	// SAFETY: The tree lives through the test, laid out as TF2's, and only this
	// handle reads it.
	let mut tags = unsafe { TaggedConVars::from_raw(tree.root()) };

	assert_eq!(tags.as_ptr(), tree.root().as_ptr());

	let entries: Vec<_> = tags
		.iter_mut()
		.map(|entry| {
			(
				entry.convar().map(CStr::to_owned),
				entry.tag().map(CStr::to_owned),
			)
		})
		.collect();

	assert_eq!(
		entries,
		[
			(
				Some(c"mp_respawnwavetime".to_owned()),
				Some(c"respawntimes".to_owned())
			),
			(Some(c"tf_gamemode_cp".to_owned()), Some(c"cp".to_owned())),
			(None, Some(c"orphan".to_owned())),
			(Some(c"sv_gravity".to_owned()), Some(c"gravity".to_owned())),
		]
	);
}

/// An entry of the list, as TF2's game adds them.
fn entry<'a>(convar: &'a CStr, tag: &'a CStr) -> MockKey<'a> {
	MockKey::Section(
		c"tag",
		vec![
			MockKey::String(c"convar", convar),
			MockKey::String(c"tag", tag),
		],
	)
}

#[test]
fn excluded_entries_name_no_variable_and_keep_their_tag() {
	let tree = list();

	// SAFETY: The tree lives through the test, laid out as TF2's, and only this
	// handle reads it.
	let mut tags = unsafe { TaggedConVars::from_raw(tree.root()) };

	for entry in tags.iter_mut() {
		if matches!(entry.tag(), Some(tag) if tag == c"cp" || tag == c"orphan") {
			entry.exclude();
		}
	}

	let entries: Vec<_> = tags
		.iter_mut()
		.map(|entry| {
			(
				entry.convar().map(CStr::to_owned),
				entry.tag().map(CStr::to_owned),
			)
		})
		.collect();

	assert_eq!(
		entries,
		[
			(
				Some(c"mp_respawnwavetime".to_owned()),
				Some(c"respawntimes".to_owned())
			),
			(None, Some(c"cp".to_owned())),
			(None, Some(c"orphan".to_owned())),
			(Some(c"sv_gravity".to_owned()), Some(c"gravity".to_owned())),
		]
	);

	// The engine reads an excluded entry's variable as the empty string, which
	// names none. The string itself stays allocated, for the engine to free.
	let convars: Vec<_> = tags
		.iter_mut()
		.map(|entry| entry.string(c"convar").map(CStr::to_owned))
		.collect();

	assert_eq!(
		convars,
		[
			Some(c"mp_respawnwavetime".to_owned()),
			Some(c"".to_owned()),
			None,
			Some(c"sv_gravity".to_owned()),
		]
	);

	assert_eq!(tree.string(c"tag", c"convar"), Some(c"mp_respawnwavetime"));
}

/// A list with TF2's entries for `mp_respawnwavetime`, `tf_gamemode_cp`, an
/// entry without a variable, and one named in other case, as the multiplayer
/// rules name theirs after the variable.
fn list() -> MockTree {
	MockTree::new(&MockKey::Section(
		c"GameTags",
		vec![
			entry(c"mp_respawnwavetime", c"respawntimes"),
			entry(c"tf_gamemode_cp", c"cp"),
			MockKey::Section(c"tag", vec![MockKey::String(c"tag", c"orphan")]),
			MockKey::Section(
				c"sv_gravity",
				vec![
					MockKey::String(c"CONVAR", c"sv_gravity"),
					MockKey::String(c"Tag", c"gravity"),
				],
			),
		],
	))
}

#[test]
fn the_interface_is_requested_by_its_header_version() {
	assert_eq!(ServerGameTags::VERSION, c"ServerGameTags001");
}
