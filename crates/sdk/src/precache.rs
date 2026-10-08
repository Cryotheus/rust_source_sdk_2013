//! Precaching models, generic files and decals, which clients load as they
//! connect, through the engine's own precache tables.
//!
//! The engine stops the server through `Host_Error` when a table has no room
//! for a new name, or when a name starts with a space, a control character,
//! or a byte past ASCII, so [`Server::precache_model`] and its siblings check
//! for those first.
//!
//! Clients load what is precached as they connect, so precache while the
//! level loads, as the game precaches what its entities use as they spawn.
//! A name precached later reaches clients already connected only as the
//! table's update does, which they may not act on in time.

#[cfg(test)]
#[path = "tests/precache.rs"]
mod tests;

use crate::interfaces::network_string_tables::MODEL_PRECACHE;
use crate::{InterfaceError, Server};
use sdk_raw::vcall;
use std::ffi::CStr;

/// The name of the table listing the precached decals.
pub const DECAL_PRECACHE: &CStr = c"decalprecache";

/// The name of the table listing the precached generic files, such as
/// particle definitions and scripts clients read.
pub const GENERIC_PRECACHE: &CStr = c"genericprecache";

/// What a name is precached as, which picks its table and the engine's
/// method.
#[derive(Debug, Clone, Copy)]
enum Kind {
	Decal,
	Generic,
	Model,
}

impl Kind {
	/// The table names of this kind are precached into.
	const fn table(self) -> &'static CStr {
		match self {
			Self::Decal => DECAL_PRECACHE,
			Self::Generic => GENERIC_PRECACHE,
			Self::Model => MODEL_PRECACHE,
		}
	}
}

/// Why a name was not precached.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PrecacheError {
	/// The name is empty, or starts with a space, a control character, or a
	/// byte past ASCII, which the engine stops the server for.
	#[error("the name is empty, or starts with a character the engine refuses")]
	InvalidName,

	/// An interface the precache needs is missing.
	#[error(transparent)]
	Interface(#[from] InterfaceError),

	/// No level is loaded, so there is no table to precache into.
	#[error("no level is loaded, so the precache table is missing")]
	NoTable,

	/// The engine reported that it precached nothing.
	#[error("the engine precached nothing")]
	Refused,

	/// The table has no room for another name.
	#[error("the precache table is full")]
	TableFull,
}

impl<'s> Server<'s> {
	/// Adds `name` to its kind's precache table, once it is known not to stop
	/// the server.
	fn precache(&self, kind: Kind, name: &CStr, preload: bool) -> Result<usize, PrecacheError> {
		// `PR_CheckEmptyString` compares the first `char`, signed on every
		// platform the game ships on, with a space.
		if !matches!(name.to_bytes().first(), Some(0x21..=0x7f)) {
			return Err(PrecacheError::InvalidName);
		}

		let engine = self.valve_engine()?.as_ptr();

		let table = self
			.network_string_tables()?
			.find(kind.table())
			.ok_or(PrecacheError::NoTable)?;

		if table.find(name).is_none() && table.len() >= table.max_len() {
			return Err(PrecacheError::TableFull);
		}

		let name = name.as_ptr();

		// SAFETY: `Server::new` guarantees the engine is live, and the name is
		// one the engine takes, which its table has room for, so the engine
		// adds it, copying it, without an error.
		let index = unsafe {
			match kind {
				Kind::Decal => vcall!(engine => IVEngineServer_PrecacheDecal(name, preload)),
				Kind::Generic => vcall!(engine => IVEngineServer_PrecacheGeneric(name, preload)),
				Kind::Model => vcall!(engine => IVEngineServer_PrecacheModel(name, preload)),
			}
		};

		usize::try_from(index).map_err(|_| PrecacheError::Refused)
	}

	/// Precaches a decal, such as `decals/scorch1`, and returns its index in
	/// the [`DECAL_PRECACHE`] table, as the game's `UTIL_PrecacheDecal` does.
	/// A decal precached already keeps its index.
	#[doc(alias("PrecacheDecal"))]
	pub fn precache_decal(&self, decal: &CStr, preload: bool) -> Result<usize, PrecacheError> {
		self.precache(Kind::Decal, decal, preload)
	}

	/// Precaches a generic file, such as a particle definition, and returns
	/// its index in the [`GENERIC_PRECACHE`] table. A file precached already
	/// keeps its index.
	#[doc(alias("PrecacheGeneric"))]
	pub fn precache_generic(&self, path: &CStr, preload: bool) -> Result<usize, PrecacheError> {
		self.precache(Kind::Generic, path, preload)
	}

	/// Precaches a model, such as `models/props_farm/box.mdl`, and returns its
	/// index in the [`MODEL_PRECACHE`] table, which entities give as their
	/// model index. A model precached already keeps its index. With `preload`,
	/// the server loads the model at once, rather than as it is first used.
	#[doc(alias("PrecacheModel"))]
	pub fn precache_model(&self, model: &CStr, preload: bool) -> Result<usize, PrecacheError> {
		self.precache(Kind::Model, model, preload)
	}
}
