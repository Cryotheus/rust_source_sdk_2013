//! Hand-written ABI of `IPhysicsSurfaceProps` that the generated bindings do
//! not describe: the version string it is requested by, from
//! `public/vphysics_interface.h`.

use std::ffi::CStr;

/// The version string `IPhysicsSurfaceProps` is exported and requested under.
///
/// This is `VPHYSICS_SURFACEPROPS_INTERFACE_VERSION` from
/// `public/vphysics_interface.h`.
#[doc(alias("VPHYSICS_SURFACEPROPS_INTERFACE_VERSION"))]
pub const VERSION: &CStr = c"VPhysicsSurfaceProps001";
