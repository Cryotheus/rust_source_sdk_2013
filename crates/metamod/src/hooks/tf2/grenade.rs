//! Observes native grenade explosions, including jars, without inferring an
//! explosion from projectile removal or a player's input.
#[cfg(test)]
#[path = "../../tests/hooks/tf2/grenade.rs"]
mod tests;

use crate::{
	MetamodApi,
	hook::{HookAction, HookTiming, VirtualFunction},
	hooks::tf2::class::ClassHooks,
};

use source_sdk_2013::{
	Server, ServerBinding, entities::Entity, raw::vtable_slot, sys, tf2::class_targets::BaseEntity,
};

use std::ffi::c_int;

/// Callback-scoped grenade before/after the actual native `Explode` call.
pub type ExplodeFn = for<'s> fn(Server<'s>, HookTiming, Entity<'s>);

type NativeExplode = unsafe extern "C" fn(*mut sys::CBaseEntity, *mut sys::trace_t, c_int);

const EXPLODE: VirtualFunction<NativeExplode> = VirtualFunction::new(vtable_slot!(
	sys::CTFGrenadePipebombProjectile__bindgen_vtable,
	CTFGrenadePipebombProjectile_Explode
));

impl MetamodApi<'_> {
	/// Observes actual explosions. Hooks are dormant while paused and retain
	/// callback/registration lifetime guarantees of other class hooks.
	///
	/// # Safety
	/// Every covered class must derive from `CTFGrenadePipebombProjectile` and
	/// preserve its primary `Explode(trace_t*, int)` slot. In particular, do not
	/// cover arbitrary `BaseEntity` classes despite the erased marker type.
	/// The callback must obey the server's no-immediate-deletion contract.
	pub unsafe fn hook_grenade_explosions(
		self,
		binding: ServerBinding,
		callback: ExplodeFn,
	) -> ClassHooks<BaseEntity> {
		ClassHooks::new(
			binding,
			callback,
			EXPLODE,
			&[HookTiming::Post, HookTiming::Pre],
			|server, callback, grenade, call| {
				callback(server, call.timing(), grenade);
				HookAction::Ignore
			},
		)
	}
}
