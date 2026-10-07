//! Hand-written ABI of `IServerGameTags`, which the generated bindings do not
//! describe: the game's list of the console variables that tag the server,
//! from `public/eiface.h`.

use crate::vtable_slot;
use std::ffi::CStr;

/// `void IServerGameTags::GetTaggedConVarList(KeyValues *)`, which the engine
/// calls as it recalculates the server's tags (`sv_tags`), and where the game
/// adds an entry to the list for each console variable that tags the server:
/// key values whose `convar` names the variable and whose `tag` is the tag.
/// The engine tags the server with each tag while its variable is not at its
/// default. The engine owns the list, and frees it after the call.
#[doc(alias("GetTaggedConVarList"))]
pub type GetTaggedConVarListFn =
	unsafe extern "C" fn(this: *mut IServerGameTags, list: *mut sys::KeyValues);

const _: () = assert!(
	vtable_slot!(IServerGameTagsVtable, get_tagged_con_var_list) == GET_TAGGED_CON_VAR_LIST_SLOT
);

/// The vtable slot of `IServerGameTags::GetTaggedConVarList`, its only
/// method.
///
/// The interface declares no virtual destructor, so the slot is the same under
/// the MSVC and Itanium ABIs.
#[doc(alias("GetTaggedConVarList"))]
pub const GET_TAGGED_CON_VAR_LIST_SLOT: usize = 0;

/// The version string `IServerGameTags` is exported and requested under.
///
/// This is `INTERFACEVERSION_SERVERGAMETAGS` from `public/eiface.h`.
#[doc(alias("INTERFACEVERSION_SERVERGAMETAGS"))]
pub const VERSION: &CStr = c"ServerGameTags001";

/// `IServerGameTags`, the game's list of the console variables that tag the
/// server, which `CServerGameTags` implements by asking the game rules.
#[doc(alias("CServerGameTags"))]
#[repr(C)]
pub struct IServerGameTags {
	/// The pointer to the object's vtable.
	pub vtable_: *const IServerGameTagsVtable,
}

/// The vtable of `IServerGameTags`.
#[repr(C)]
pub struct IServerGameTagsVtable {
	/// `GetTaggedConVarList`.
	pub get_tagged_con_var_list: GetTaggedConVarListFn,
}
