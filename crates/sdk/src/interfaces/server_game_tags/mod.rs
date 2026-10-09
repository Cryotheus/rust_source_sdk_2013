//! `IServerGameTags`, the game's list of the console variables that tag the
//! server, and that list, which hooks of the interface can change.
//!
//! The engine keeps the server's tags in `sv_tags`, which the server browser
//! shows and filters by. As it recalculates them, it asks the game for the
//! variables that tag the server, through
//! `IServerGameTags::GetTaggedConVarList`, and tags the server with each
//! variable's tag while the variable is not at its default. TF2 lists, among
//! others, `mp_respawnwavetime` as `respawntimes`, and the `tf_gamemode_*`
//! variables it sets for a level's mode as `cp`, `ctf` or `payload`
//! (`game/shared/tf/tf_gamerules.cpp`).
//!
//! The engine is inferred to recalculate the tags as levels start and as
//! variables marked `FCVAR_NOTIFY` change, since it is not public, and
//! [`EngineReplay::recalculate_tags`] has it recalculate them at once.
//!
//! [`EngineReplay::recalculate_tags`]: super::EngineReplay::recalculate_tags

#[cfg(test)]
#[path = "../../tests/interfaces/server_game_tags.rs"]
mod tests;

use crate::key_values::{KeyValues, SubKeys};
use sdk_raw::interfaces::server_game_tags::{IServerGameTags, VERSION};
use std::ffi::CStr;
use std::marker::PhantomData;
use std::ptr::NonNull;

interface! {
	/// The game's list of the console variables that tag the server
	/// (`IServerGameTags`), which the engine asks for as it recalculates the
	/// server's tags. Hooking its `GetTaggedConVarList` gives the list as a
	/// [`TaggedConVars`].
	#[doc(alias("IServerGameTags", "CServerGameTags"))]
	pub struct ServerGameTags(IServerGameTags) = GameServer VERSION;
}

/// An entry of [`TaggedConVars`]: a console variable that tags the server
/// while it is not at its default, and its tag.
#[derive(Debug)]
pub struct TaggedConVar<'a> {
	entry: KeyValues<'a>,
	_list: PhantomData<&'a mut ()>,
}

impl TaggedConVar<'_> {
	/// The console variable's name, as the entry's `convar` holds it, or
	/// `None` if it holds none or its name cannot be found (see
	/// [`KeyValues::name`]). An excluded entry names none.
	pub fn convar(&self) -> Option<&CStr> {
		self.string(c"convar").filter(|name| !name.is_empty())
	}

	/// Keeps the engine from finding the variable, so that it neither adds
	/// nor removes the tag as it recalculates the server's tags. The tag stays
	/// as `sv_tags` has it, which the plugin can then change itself.
	///
	/// The entry's `convar` is emptied, and no variable has an empty name.
	pub fn exclude(self) {
		let Some(convar) = self.entry.find_key(c"convar") else {
			return;
		};

		let Some(convar) = NonNull::new(convar.as_ptr()) else {
			return;
		};

		// SAFETY: The entry's key values are live, and laid out as TF2's.
		let string = unsafe { sdk_raw::key_values::string_value(convar) };

		if string.is_null() {
			return;
		}

		// SAFETY: The string is the key values' own, NUL-terminated, so at least
		// a byte long, and `TaggedConVars::from_raw`'s caller lets it change.
		// Taking `self` ends every borrow of it.
		unsafe { string.write(0) };
	}

	/// The tag, as the entry's `tag` holds it, or `None` if it holds none or
	/// its name cannot be found (see [`KeyValues::name`]).
	pub fn tag(&self) -> Option<&CStr> {
		self.string(c"tag")
	}

	/// The string value of the entry's key values named `name`.
	fn string(&self, name: &CStr) -> Option<&CStr> {
		self.entry.find_key(name)?.string()
	}
}

/// The console variables that tag the server, as the game just listed them for
/// the engine (`IServerGameTags::GetTaggedConVarList`), with one entry for
/// each variable and its tag. The engine owns the list, and frees it after the
/// call.
///
/// [Excluding](TaggedConVar::exclude) an entry keeps the engine from finding
/// its variable, so the tag stays as it is in `sv_tags`, whatever the
/// variable's value: neither added nor removed.
#[derive(Debug)]
pub struct TaggedConVars<'k> {
	list: KeyValues<'k>,
}

impl<'k> TaggedConVars<'k> {
	/// Wraps the list the engine passed to `GetTaggedConVarList`.
	///
	/// # Safety
	///
	/// `raw` must point to the list, as for [`KeyValues::from_raw`], except
	/// that [`TaggedConVar::exclude`] may change its entries' strings: nothing
	/// else may read or change the list's key values during `'k`.
	pub const unsafe fn from_raw(raw: NonNull<sys::KeyValues>) -> Self {
		Self {
			// SAFETY: The caller upholds the same contract, and the list's
			// strings are only read through this handle's entries.
			list: unsafe { KeyValues::from_raw(raw) },
		}
	}

	/// Returns the list's pointer, for calls this crate does not wrap.
	pub const fn as_ptr(&self) -> *mut sys::KeyValues {
		self.list.as_ptr()
	}

	/// The list's entries, in order.
	pub fn iter_mut(&mut self) -> TaggedConVarsIter<'_> {
		TaggedConVarsIter {
			entries: self.list.sub_keys(),
			_list: PhantomData,
		}
	}
}

/// The entries of [`TaggedConVars`], in order, from
/// [`TaggedConVars::iter_mut`].
#[derive(Debug)]
pub struct TaggedConVarsIter<'a> {
	entries: SubKeys<'a>,
	_list: PhantomData<&'a mut ()>,
}

impl<'a> Iterator for TaggedConVarsIter<'a> {
	type Item = TaggedConVar<'a>;

	fn next(&mut self) -> Option<Self::Item> {
		self.entries.next().map(|entry| TaggedConVar {
			entry,
			_list: PhantomData,
		})
	}
}
