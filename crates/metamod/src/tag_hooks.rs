//! A hook on the game's list of the console variables that tag the server,
//! which can keep variables from tagging it.
//!
//! The engine recalculates the server's tags (`sv_tags`) as levels start and
//! as variables change. It asks the game which variables tag the server
//! through `IServerGameTags::GetTaggedConVarList`, and tags the server with
//! each one's tag while the variable is not at its default. The hook runs
//! after the game listed them, so the callback sees each entry of the list,
//! and can [exclude] the variables whose tags would
//! misdescribe the server, such as the respawn times a plugin changed.
//!
//! An excluded tag stays as `sv_tags` has it: one already there stays until
//! the plugin removes it, and once the plugin stops excluding it, the engine's
//! next recalculation sets it from the variable again.
//!
//! [exclude]: source_sdk_2013::interfaces::server_game_tags::TaggedConVar::exclude
//!
//! # When to install
//!
//! The interface lives as long as the game library, so install while loading.
//! The hook stops calling back while the plugin is paused and when it unloads,
//! and Metamod removes it after unloading the plugin.

#[cfg(test)]
#[path = "tests/tag_hooks.rs"]
mod tests;

use crate::MetamodApi;

use crate::hook::{
	Handler, HookAction, HookCall, HookError, HookId, HookTarget, HookTiming, VirtualFunction,
};

use source_sdk_2013::interfaces::ServerGameTags;
use source_sdk_2013::interfaces::server_game_tags::TaggedConVars;

use source_sdk_2013::raw::interfaces::server_game_tags::{
	GET_TAGGED_CON_VAR_LIST_SLOT, GetTaggedConVarListFn as GetTaggedConVarList, IServerGameTags,
};

use source_sdk_2013::{Server, ServerBinding};
use std::cell::Cell;
use std::ptr::NonNull;

/// Sees the console variables that tag the server, as the game just listed
/// them, and may [exclude] some. A panic is contained
/// by the hook dispatcher.
///
/// [exclude]: source_sdk_2013::interfaces::server_game_tags::TaggedConVar::exclude
pub type TaggedConVarsFn = for<'s> fn(Server<'s>, TaggedConVars<'s>);

/// `IServerGameTags::GetTaggedConVarList`.
const GET_TAGGED_CON_VAR_LIST: VirtualFunction<GetTaggedConVarList> =
	VirtualFunction::new(GET_TAGGED_CON_VAR_LIST_SLOT);

static ROUTE: TagsRoute = TagsRoute(Cell::new(None));

#[derive(Clone, Copy)]
struct RoutedTags {
	binding: ServerBinding,
	callback: TaggedConVarsFn,
	hook: HookId,
}

/// The callback the hook runs, with the hook.
struct TagsRoute(Cell<Option<RoutedTags>>);

impl Handler<GetTaggedConVarList> for TagsRoute {
	fn call(&self, call: &HookCall<'_, GetTaggedConVarList>) -> HookAction<()> {
		let (list,) = call.args();

		let (Some(routed), Some(list)) = (self.0.get(), NonNull::new(list)) else {
			return HookAction::Ignore;
		};

		let scope = ();

		// SAFETY: The hook dispatcher runs on the main thread, during one call
		// from the engine. The plugin supplied the binding with the hook.
		let server = unsafe { routed.binding.server(&scope) };

		// SAFETY: The engine made the list, laid out as TF2's, and frees it only
		// after the call; the game is done adding to it, and nothing else reads
		// it until the callback returns.
		let list = unsafe { TaggedConVars::from_raw(list) };

		(routed.callback)(server, list);
		HookAction::Ignore
	}
}

// SAFETY: Installation requires a main-thread MetamodApi; the hook dispatcher
// calls handlers only on that thread.
unsafe impl Sync for TagsRoute {}

impl MetamodApi<'_> {
	/// Runs `callback` after the game lists the console variables that tag
	/// the server, each time the engine recalculates its tags; see the
	/// [module documentation](crate::tag_hooks).
	///
	/// This hooks `IServerGameTags::GetTaggedConVarList` after the call.
	/// Installing again while the hook is installed returns
	/// [`HookError::AlreadyInstalled`].
	pub fn hook_tagged_convars(
		self,
		tags: ServerGameTags<'_>,
		binding: ServerBinding,
		callback: TaggedConVarsFn,
	) -> Result<(), HookError> {
		let tags = NonNull::new(tags.as_ptr()).ok_or(HookError::InvalidArgument)?;

		// SAFETY: `tags` is the game's interface, which outlives the plugin, and
		// has `GetTaggedConVarList` at the slot.
		unsafe { self.install_tagged_convars(tags, binding, callback) }
	}

	/// Hooks `GetTaggedConVarList` on `tags`.
	///
	/// # Safety
	///
	/// `tags` must be live, and its vtable must hold a function of the
	/// signature [`GetTaggedConVarList`] at [`GET_TAGGED_CON_VAR_LIST_SLOT`],
	/// until Metamod unloads the plugin.
	unsafe fn install_tagged_convars(
		self,
		tags: NonNull<IServerGameTags>,
		binding: ServerBinding,
		callback: TaggedConVarsFn,
	) -> Result<(), HookError> {
		if ROUTE
			.0
			.get()
			.is_some_and(|routed| self.has_hook(routed.hook))
		{
			return Err(HookError::AlreadyInstalled);
		}

		// SAFETY: As the caller promises; a `MetamodApi` only exists on the main
		// thread.
		let hook = unsafe {
			self.add_hook(
				GET_TAGGED_CON_VAR_LIST,
				HookTarget::instance(tags),
				HookTiming::Post,
				&ROUTE,
			)
		}?;

		ROUTE.0.set(Some(RoutedTags {
			binding,
			callback,
			hook,
		}));

		Ok(())
	}
}
