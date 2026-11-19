//! `IPhysicsSurfaceProps`, the physics library's table of surface properties:
//! what each kind of surface, such as `concrete` or `metal`, sounds like as
//! players step on it.

use sdk_raw::vcall;
use std::ffi::{CStr, c_int, c_ushort};
use std::ptr::NonNull;

interface! {
	/// The physics library's table of surface properties
	/// (`IPhysicsSurfaceProps`), which the game's `physprops` is. A trace
	/// tells the index of the properties of the surface it hit
	/// ([`Trace::surface_props`](crate::interfaces::engine_trace::Trace::surface_props)).
	///
	/// # Unverified
	///
	/// It is requested from the engine's factory, which reaches the libraries
	/// the engine loads besides its own, as it reaches the filesystem's for
	/// [`BaseFileSystem`](crate::interfaces::BaseFileSystem). That it reaches
	/// the physics library's on TF2's servers has not been tested.
	#[doc(alias("IPhysicsSurfaceProps", "physprops"))]
	pub struct PhysicsSurfaceProps(sys::IPhysicsSurfaceProps) = Engine sdk_raw::interfaces::physics_surface_props::VERSION;
}

/// The properties of a kind of surface, from [`PhysicsSurfaceProps::surface`].
#[derive(Debug, Clone, Copy)]
pub struct Surface<'s> {
	props: PhysicsSurfaceProps<'s>,
	data: NonNull<sys::surfacedata_t>,
}

impl<'s> Surface<'s> {
	/// The [game material](Self::game_material) of glass (`CHAR_TEX_GLASS`).
	pub const GLASS: c_ushort = b'Y' as c_ushort;

	/// The [game material](Self::game_material) of grates and chain-link
	/// fences (`CHAR_TEX_GRATE`).
	pub const GRATE: c_ushort = b'G' as c_ushort;

	/// The game's kind of material the surface is (`game.material`), one of
	/// the `CHAR_TEX_*` letters the game's decals and impacts go by, such as
	/// [`Self::GRATE`] for a grate or chain-link fence.
	#[doc(alias("material", "CHAR_TEX"))]
	pub fn game_material(self) -> c_ushort {
		// SAFETY: As for `step_sounds`.
		unsafe { self.data.as_ref() }.game.material
	}

	/// The sound script entries a player's left and right footsteps on the
	/// surface play, such as `Concrete.StepLeft` (`sounds.stepleft` and
	/// `sounds.stepright`), each `None` if the surface has none.
	#[doc(alias("stepleft", "stepright"))]
	pub fn step_sounds(self) -> (Option<&'s CStr>, Option<&'s CStr>) {
		// SAFETY: The library keeps its surfaces for as long as it is loaded,
		// and changes them only as it parses more, which no caller does.
		let sounds = unsafe { self.data.as_ref() }.sounds;

		(
			self.props.string(sounds.stepleft),
			self.props.string(sounds.stepright),
		)
	}
}

impl<'s> PhysicsSurfaceProps<'s> {
	/// How many surfaces the table holds (`SurfacePropCount`): their indices
	/// run from 0 to one less.
	#[doc(alias("SurfacePropCount"))]
	pub fn count(self) -> c_int {
		// SAFETY: As for `surface`.
		unsafe { vcall!(self.as_ptr() => IPhysicsSurfaceProps_SurfacePropCount()) }
	}

	/// The name of the surface of index `index`, such as `chainlink`, or
	/// `None` if the table gives none (`GetPropName`).
	#[doc(alias("GetPropName"))]
	pub fn name(self, index: c_int) -> Option<&'s CStr> {
		// SAFETY: As for `surface`. The library keeps its names, which end in
		// a null, for as long as it is loaded, beyond `'s`.
		unsafe {
			let name = vcall!(self.as_ptr() => IPhysicsSurfaceProps_GetPropName(index));

			(!name.is_null()).then(|| CStr::from_ptr(name))
		}
	}

	/// The string of index `index` in the table's own strings, or `None` for
	/// 0, which stands for none (`GetString`).
	fn string(self, index: c_ushort) -> Option<&'s CStr> {
		if index == 0 {
			return None;
		}

		// SAFETY: As for `surface`. The library keeps its strings, which end in
		// a null, for as long as it is loaded, beyond `'s`.
		unsafe {
			let string = vcall!(self.as_ptr() => IPhysicsSurfaceProps_GetString(index));

			(!string.is_null()).then(|| CStr::from_ptr(string))
		}
	}

	/// The properties of the surface of index `index`, as a trace reports it,
	/// or the default surface's for an index the table lacks, as the physics
	/// library gives them (`GetSurfaceData`). `None` if it gives none.
	#[doc(alias("GetSurfaceData"))]
	pub fn surface(self, index: c_int) -> Option<Surface<'s>> {
		// SAFETY: `Server::new` guarantees the interface is live. The library
		// looks the index up in its own table.
		let data = unsafe { vcall!(self.as_ptr() => IPhysicsSurfaceProps_GetSurfaceData(index)) };

		NonNull::new(data).map(|data| Surface { props: self, data })
	}

	/// The index of the surface named `name`, such as `water`, or `None` if
	/// the table has none of the name (`GetSurfaceIndex`).
	#[doc(alias("GetSurfaceIndex"))]
	pub fn surface_index(self, name: &CStr) -> Option<c_int> {
		// SAFETY: As for `surface`. The library reads the name during the call.
		let index =
			unsafe { vcall!(self.as_ptr() => IPhysicsSurfaceProps_GetSurfaceIndex(name.as_ptr())) };

		(index >= 0).then_some(index)
	}
}
