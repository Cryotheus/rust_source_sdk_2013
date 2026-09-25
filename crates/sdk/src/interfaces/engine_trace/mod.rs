//! `IEngineTrace`, which traces rays and queries what the world contains.

use crate::ffi::vcall;
use crate::math::Vector;
use std::ffi::c_int;
use std::ptr;

interface! {
	/// Traces rays and queries what the world contains (`IEngineTrace`).
	#[doc(alias = "IEngineTrace")]
	pub struct EngineTrace(sys::IEngineTrace) = Engine c"EngineTraceServer003";
}

impl<'s> EngineTrace<'s> {
	/// The `CONTENTS_*` flags of the world and entities at a point, as defined
	/// in `public/bspflags.h`.
	#[doc(alias = "GetPointContents")]
	pub fn point_contents(self, position: Vector) -> c_int {
		let position = sys::Vector::from(position);

		// SAFETY: `Server::new` guarantees the interface is live, the position
		// is a local, and no entity is requested.
		unsafe {
			vcall!(self.as_ptr() => IEngineTrace_GetPointContents(&position, ptr::null_mut()))
		}
	}
}
