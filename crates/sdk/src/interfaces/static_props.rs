//! `IStaticPropMgrServer`, the engine's manager of the running level's static
//! props, and the props it hands out, each of which
//! [`EngineTrace::clip_to_static_prop`] clips a ray to alone.
//!
//! A static prop is a model the map places that never moves. The engine keeps
//! no entity for it: [`EngineTrace::trace`] finds it along the ray as it finds
//! entities, and reports it as the world's [entity][`Trace::entity`] with its
//! index in the [hitbox][`Trace::hitbox`] (see [`Trace::static_prop`]).
//! [`EngineTrace::clip_to_entity`] cannot reach one, as it has no entity.
//!
//! [`StaticProps::of`] lists the running level's props for one call into the
//! plugin. Nothing of a level's props is kept from one call to the next: each
//! call that wants them lists them again, which asks the engine for every
//! prop's collideable. So a prop is never reached after the level it belongs
//! to ended, and an index is only ever looked up among the props the engine
//! listed in the same call.
//!
//! # What the interfaces promise, and what was only seen
//!
//! `public/engine/IStaticPropMgr.h` promises that `GetAllStaticProps` lists
//! the collideable of each static prop, and that `GetStaticPropByIndex` gives
//! the collideable of the prop at an index. It does not say that the list is
//! in the order of the indices, nor what the second gives past the last prop.
//! The header also calls `GetAllStaticProps` a testing function that may be
//! removed. Removed without a new version of the interface, its slot would
//! hold `GetAllStaticPropsInAABB`, which takes other arguments, so this crate
//! calls it only on the interface asked for by its version string,
//! `StaticPropMgrServer002`.
//! `public/engine/IEngineTrace.h` promises that `ClipRayToCollideable` clips
//! a ray to the collideable it is given alone, without saying what it fills
//! the trace's entity and hitbox with.
//!
//! On the Linux dedicated server of TF2, as of November 2026, this was seen:
//!
//! - The list holds each of the level's static props once, in the order of
//!   their indices, which are those of the map's list of static props, so
//!   that the list's place of each prop is the index `GetStaticPropByIndex`
//!   gives it ([`StaticProps::agrees`] tells, of one prop).
//! - Clipped to a static prop's collideable, a ray reports the world as its
//!   entity and 1 more than the prop's index as its hitbox, as a whole trace
//!   that stops at the prop does. Where both are handed the same ray, every
//!   number of the two traces is the same to the bit.
//! - A prop's collideable gives the prop's origin as its map places it, and
//!   zero for its angles, however the map turns it. On one map, its bounds
//!   (`WorldSpaceSurroundingBounds`) held the corners of its collision model,
//!   placed and turned as the map has it, for all but three of 974 props
//!   solid by their collision model. Two of those three were looked at, and
//!   reached past them by 0.57 units.
//! - A whole trace was seen to stop at a static prop along a ray that passed
//!   an eighth of a unit outside the prop's bounds, and a clip of a prop's
//!   collideable, along a ray toward the prop's middle, to meet the prop 18
//!   units outside them. So the bounds hold neither every ray along which a
//!   whole trace may find a prop nor every point at which a clip may meet it.
//! - `IEngineTrace::EnumerateEntities`, given a box, hands no static prop to
//!   its enumerator, only entities, so it cannot find them.
//! - `TraceRayAgainstStaticProp` leaves the trace it is given as it was, so
//!   this crate does not call it.
//!
//! [`EngineTrace::clip_to_static_prop`]: crate::interfaces::EngineTrace::clip_to_static_prop
//! [`EngineTrace::trace`]: crate::interfaces::EngineTrace::trace
//! [`EngineTrace::clip_to_entity`]: crate::interfaces::EngineTrace::clip_to_entity
//! [`Trace::static_prop`]: crate::interfaces::engine_trace::Trace::static_prop
//! [`Trace::entity`]: crate::interfaces::engine_trace::Trace::entity
//! [`Trace::hitbox`]: crate::interfaces::engine_trace::Trace::hitbox

#[cfg(test)]
#[path = "../tests/interfaces/static_props.rs"]
mod tests;

use crate::InterfaceError;
use crate::Server;
use crate::entities::Entity;
use crate::entities::solid::{SolidFlags, SolidType};
use crate::math::{QAngle, Vector};
use sdk_raw::interfaces::static_prop_manager::{CollideableList, IStaticPropMgrServer, VERSION};
use sdk_raw::vcall;
use std::ffi::c_int;
use std::marker::PhantomData;
use std::ptr::{self, NonNull};
use std::sync::{Mutex, MutexGuard, TryLockError};

/// The list the engine fills with the level's static props each time they are
/// listed, kept so that its memory is grown once and reused (see [`List`]).
static LIST: Mutex<List> = Mutex::new(List(CollideableList {
	_phantom_0: PhantomData,
	_phantom_1: PhantomData,
	m_Memory: sys::CUtlMemory {
		_phantom_0: PhantomData,
		m_pMemory: ptr::null_mut(),
		m_nAllocationCount: 0,
		m_nGrowSize: 0,
	},
	m_Size: 0,
	m_pElements: ptr::null_mut(),
}));

interface! {
	/// The engine's manager of the running level's static props
	/// (`IStaticPropMgrServer`).
	#[doc(alias("IStaticPropMgrServer", "IStaticPropMgr"))]
	pub struct StaticPropManager(IStaticPropMgrServer) = Engine VERSION;
}

/// The list of collideables the engine fills, as [`LIST`] keeps it.
///
/// It starts empty, with no memory. The engine grows its memory with its own
/// allocator as it adds to it, and this crate cannot free memory the engine
/// allocated, so the memory is never freed: it is reused by every listing,
/// grown only by one that lists more props than any before it, and left
/// allocated as the plugin is unloaded, once. Each listing empties it first,
/// so that it holds the props of that listing alone; what it held before
/// is never read.
struct List(CollideableList);

// SAFETY: The list holds plain data: a pointer to memory of the engine's heap,
// which no one but the engine's code that grows it reaches, and pointers that
// are compared and read only during the call that listed them. It is only
// filled through `StaticProps::of`, which runs on the server's main thread,
// as `Server` requires.
unsafe impl Send for List {}

/// A static prop of the running level, as the engine has it, for one call
/// into the plugin.
///
/// Its collideable stays the live prop's for all of `'s`, also after the
/// [`StaticProps`] it came from is dropped: the engine keeps a level's static
/// props for the whole level, and `'s` lies within one call into the plugin,
/// during which no level change completes (condition 3 of [`Server::new`]).
/// The calls on the collideable rest on that.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaticProp<'s> {
	collideable: NonNull<sys::ICollideable>,
	index: u32,
	world: Entity<'s>,
	_not_thread_safe: crate::NotThreadSafe,
}

impl<'s> StaticProp<'s> {
	/// The angles of the prop's collision (`GetCollisionAngles`). They were
	/// seen to be zero for every static prop, however its map turns it (see
	/// the [module documentation](self)), so they do not tell how the prop is
	/// turned.
	#[doc(alias("GetCollisionAngles"))]
	pub fn angles(self) -> QAngle {
		// SAFETY: The collideable is the live prop's for `'s`, for the
		// reason the documentation of `StaticProp` gives. Its angles are a member of
		// it, copied at once.
		unsafe { (*vcall!(self.collideable.as_ptr() => ICollideable_GetCollisionAngles())).into() }
	}

	/// The box round the prop in the level, its low and high corners: the
	/// box the engine files the prop by (`WorldSpaceSurroundingBounds`).
	#[doc(alias("WorldSpaceSurroundingBounds"))]
	pub fn bounds(self) -> (Vector, Vector) {
		let mut mins = sys::Vector {
			x: 0.0,
			y: 0.0,
			z: 0.0,
		};
		let mut maxs = mins;

		// SAFETY: As for `angles`; the call only writes the two vectors, which
		// outlive it.
		unsafe {
			vcall!(self.collideable.as_ptr() => ICollideable_WorldSpaceSurroundingBounds(&raw mut mins, &raw mut maxs));
		}

		(mins.into(), maxs.into())
	}

	/// The prop's collideable, for calls this crate does not wrap. It is
	/// the live prop's during `'s` alone.
	pub const fn collideable(self) -> NonNull<sys::ICollideable> {
		self.collideable
	}

	/// The prop's index among the level's static props, as
	/// [`Trace::static_prop`] reports it.
	///
	/// [`Trace::static_prop`]: crate::interfaces::engine_trace::Trace::static_prop
	pub const fn index(self) -> u32 {
		self.index
	}

	/// The box round the prop's model, its low and high corners, in the
	/// model's own space (`OBBMins`, `OBBMaxs`).
	#[doc(alias("OBBMins", "OBBMaxs"))]
	pub fn local_bounds(self) -> (Vector, Vector) {
		// SAFETY: As for `angles`.
		unsafe {
			(
				(*vcall!(self.collideable.as_ptr() => ICollideable_OBBMins())).into(),
				(*vcall!(self.collideable.as_ptr() => ICollideable_OBBMaxs())).into(),
			)
		}
	}

	/// Where the prop is placed (`GetCollisionOrigin`).
	#[doc(alias("GetCollisionOrigin"))]
	pub fn origin(self) -> Vector {
		// SAFETY: As for `angles`.
		unsafe { (*vcall!(self.collideable.as_ptr() => ICollideable_GetCollisionOrigin())).into() }
	}

	/// The prop's solid flags (`GetSolidFlags`).
	#[doc(alias("GetSolidFlags"))]
	pub fn solid_flags(self) -> SolidFlags {
		// SAFETY: As for `angles`.
		let flags = unsafe { vcall!(self.collideable.as_ptr() => ICollideable_GetSolidFlags()) };

		// The flags are 16 bits wide, as the game keeps them.
		SolidFlags::from_bits_retain(flags as u16)
	}

	/// How the prop collides (`GetSolid`), or `None` for a type past
	/// [`SolidType::Vphysics`].
	#[doc(alias("GetSolid"))]
	pub fn solid_type(self) -> Option<SolidType> {
		// SAFETY: As for `angles`.
		let solid = unsafe { vcall!(self.collideable.as_ptr() => ICollideable_GetSolid()) };

		u8::try_from(solid).ok().and_then(SolidType::from_raw)
	}

	/// The world entity, which a ray that stops at the prop names.
	pub(crate) const fn world(self) -> Entity<'s> {
		self.world
	}
}

/// The running level's static props, as the engine listed them for one call
/// into the plugin (see the [module documentation](self)).
///
/// While it lives, the props cannot be listed again: [`Self::of`] refuses.
pub struct StaticProps<'s> {
	list: MutexGuard<'static, List>,
	manager: StaticPropManager<'s>,
	world: Entity<'s>,
}

impl<'s> StaticProps<'s> {
	/// Lists the props of the running level, as the manager `manager` has
	/// them, the world being `world`.
	pub(crate) fn from_manager(
		manager: StaticPropManager<'s>,
		world: Entity<'s>,
	) -> Result<Self, StaticPropsError> {
		let mut list = match LIST.try_lock() {
			Ok(list) => list,
			Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
			Err(TryLockError::WouldBlock) => return Err(StaticPropsError::Listed),
		};

		list.0.m_Size = 0;

		// SAFETY: The interface is live for `'s`. The list is a well-formed empty
		// list whose memory is null or the engine's own, grown by an earlier
		// listing, which the engine may grow again with its allocator.
		unsafe { vcall!(manager.as_ptr() => get_all_static_props(&raw mut list.0)) };

		let (length, allocated) = (list.0.m_Size, list.0.m_Memory.m_nAllocationCount);

		if length < 0 || length > allocated || length > 0 && list.0.m_Memory.m_pMemory.is_null() {
			list.0.m_Size = 0;
			return Err(StaticPropsError::Malformed { length, allocated });
		}

		Ok(Self {
			list,
			manager,
			world,
		})
	}

	/// Lists the static props of the running level, for this call into the
	/// plugin. Fails if they are listed already and that listing still lives,
	/// so drop each listing before the next.
	#[doc(alias("GetAllStaticProps"))]
	pub fn of(server: &Server<'s>) -> Result<Self, StaticPropsError> {
		let manager = server.static_prop_manager()?;
		let world = server
			.server_tools()?
			.entity_by_index(0)
			.ok_or(StaticPropsError::NoWorld)?;

		Self::from_manager(manager, world)
	}

	/// Whether the engine's lookup of the prop at `index` by its index
	/// (`GetStaticPropByIndex`) gives the prop at that place of its list.
	/// `None` past the last prop, for which nothing is asked.
	#[doc(alias("GetStaticPropByIndex"))]
	pub fn agrees(&self, index: u32) -> Option<bool> {
		let listed = self.get(index)?;
		let index = c_int::try_from(index).ok()?;

		// SAFETY: The interface is live for `'s`, and the index is that of one of
		// the props the engine listed in this call, during which the level, and
		// with it its props, last.
		let found = unsafe { vcall!(self.manager.as_ptr() => get_static_prop_by_index(index)) };

		Some(ptr::eq(found, listed.collideable.as_ptr()))
	}

	/// The prop at `index`, or `None` past the last, or if the engine listed
	/// none there.
	pub fn get(&self, index: u32) -> Option<StaticProp<'s>> {
		let slot = usize::try_from(index)
			.ok()
			.filter(|&slot| slot < self.len())?;

		// SAFETY: The slot is within the length the engine left, which is within
		// the list's memory, as `from_manager` checked.
		let collideable = unsafe { self.list.0.m_Memory.m_pMemory.add(slot).read() };

		NonNull::new(collideable).map(|collideable| StaticProp {
			collideable,
			index,
			world: self.world,
			_not_thread_safe: PhantomData,
		})
	}

	/// Whether the level has no static prop.
	pub fn is_empty(&self) -> bool {
		self.len() == 0
	}

	/// How many static props the level has, as the engine listed them.
	pub fn len(&self) -> usize {
		usize::try_from(self.list.0.m_Size).unwrap_or(0)
	}
}

/// Why the level's static props could not be listed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StaticPropsError {
	/// The engine does not export its manager of static props.
	#[error(transparent)]
	Interface(#[from] InterfaceError),

	/// The world, which a ray that stops at a static prop names, is not found.
	#[error("the world entity is not found")]
	NoWorld,

	/// The props are listed already, and that listing has not been dropped.
	#[error("the static props are listed already")]
	Listed,

	/// The engine left its list in a state no list can be in: a negative
	/// length, or one longer than its memory.
	#[error("the engine's list of static props holds {length} of {allocated}")]
	Malformed {
		/// The length the engine left.
		length: c_int,

		/// How many the memory of the list holds.
		allocated: c_int,
	},
}
