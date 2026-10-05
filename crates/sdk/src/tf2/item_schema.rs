//! TF2's item schema: what an item definition says of the items made from
//! it, read through the game's own lookup, `CEconItemSchema::GetItemDefinition`,
//! which [`ItemGeneration`] finds.
//!
//! # Layout
//!
//! A definition is read at the offsets of the generated
//! `CEconItemDefinition`, from the Source SDK 2013 headers. TF2's own build
//! can lay it out otherwise, and exports nothing to check the offsets
//! against, so [`ItemSchema::new`] checks them on definitions whose values
//! the shipped schema fixes: each must hold its own index, the Pet
//! Balloonicorn (738) must be filtered for Pyrovision, the first of the
//! Romevision items (30143) for Romevision, and the Football Helmet (49) for
//! none. A layout that fails fails every later call, until the plugin is
//! reloaded.
//!
//! The schema the server runs can differ from the one it shipped with, as
//! the Game Coordinator can send a newer one, which TF2 applies at the next
//! level change. Should it lack a checked definition, or change its values,
//! [`ItemSchema::new`] fails too.

use crate::tf2::weapons::{ItemDefinitionIndex, ItemGenerationError};
use crate::{Game, NotThreadSafe, Server};
use sdk_raw::tf2::item_generation::ItemGeneration;
use std::ffi::c_int;
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::sync::OnceLock;

/// The definitions whose index, and a vision filter flag, the shipped schema
/// fixes, to check the layout with, with the flag. A definition without a flag
/// must have none.
const CHECKED: [(u16, VisionFilter); 3] = [
	(738, VisionFilter::PYRO),
	(30143, VisionFilter::ROME),
	(49, VisionFilter::empty()),
];

/// Whether the layout passed its checks, once it did or failed them.
static LAYOUT: OnceLock<bool> = OnceLock::new();

/// One of the item schema's definitions, scoped to one engine callback: the
/// schema frees its definitions when the game applies a newer one.
#[doc(alias("CEconItemDefinition"))]
#[derive(Debug, Clone, Copy)]
pub struct ItemDefinition<'s> {
	raw: NonNull<sys::CEconItemDefinition>,
	_scope: PhantomData<&'s ()>,
	_not_thread_safe: NotThreadSafe,
}

impl ItemDefinition<'_> {
	/// The definition's index (`m_nDefIndex`).
	#[doc(alias("m_nDefIndex", "GetDefinitionIndex"))]
	pub fn index(self) -> u16 {
		// SAFETY: The schema keeps its definition through the callback, and the
		// layout was checked.
		unsafe { (&raw const (*self.raw.as_ptr()).m_nDefIndex).read() }
	}

	/// The visions in which clients draw the items of the definition
	/// (`vision_filter_flags`), or none for items every viewer sees. A client
	/// draws an item with any flag only for viewers with one of the visions,
	/// such as a Pyro with Pyrovision goggles.
	#[doc(alias("m_nVisionFilterFlags", "GetVisionFilterFlags", "vision_filter_flags"))]
	pub fn vision_filter(self) -> VisionFilter {
		// SAFETY: As for `index`.
		VisionFilter::from_bits_retain(unsafe {
			(&raw const (*self.raw.as_ptr()).m_nVisionFilterFlags).read()
		})
	}
}

/// TF2's item schema, scoped to one engine callback.
#[doc(alias("CEconItemSchema"))]
#[derive(Debug, Clone, Copy)]
pub struct ItemSchema<'s> {
	generation: ItemGeneration,
	_scope: PhantomData<&'s ()>,
	_not_thread_safe: NotThreadSafe,
}

impl<'s> ItemSchema<'s> {
	/// The item schema the server runs.
	///
	/// Fails with [`ItemSchemaError::NotTf2`] on another game, with
	/// [`ItemSchemaError::Generation`] if the game's lookup is not found, or
	/// before the game has a schema, and with
	/// [`ItemSchemaError::UnsupportedLayout`] if the definitions are not laid
	/// out as the generated bindings say, as the
	/// [module documentation](crate::tf2::item_schema#layout) describes.
	pub fn new(server: Server<'s>) -> Result<Self, ItemSchemaError> {
		if server.game() != Game::TeamFortress2 {
			return Err(ItemSchemaError::NotTf2);
		}

		// SAFETY: `Server::new` guarantees that the game server module, whose
		// factory this is, stays loaded through the callback. A cached resolution
		// for the same factory and module base was made in this same image, as
		// for the item generation of `weapons`.
		let generation = unsafe { ItemGeneration::cached(server.game_server_factory().as_raw()) }?;

		let schema = Self {
			generation,
			_scope: PhantomData,
			_not_thread_safe: PhantomData,
		};

		match LAYOUT.get() {
			Some(true) => Ok(schema),
			Some(false) => Err(ItemSchemaError::UnsupportedLayout),

			None => {
				// A schema missing a checked definition says nothing of the layout,
				// so only a definition that reads wrong is remembered.
				let checked = schema.check_layout()?;

				LAYOUT.set(checked).ok();

				if checked {
					Ok(schema)
				} else {
					Err(ItemSchemaError::UnsupportedLayout)
				}
			}
		}
	}

	/// Whether the [checked definitions](CHECKED) read as the shipped schema
	/// has them.
	fn check_layout(self) -> Result<bool, ItemSchemaError> {
		for (index, flag) in CHECKED {
			let Some(definition) = self.find(index)? else {
				return Err(ItemSchemaError::Generation(
					ItemGenerationError::UnknownDefinition,
				));
			};

			let vision_filter = definition.vision_filter();

			let flagged = if flag.is_empty() {
				vision_filter.is_empty()
			} else {
				vision_filter.contains(flag)
			};

			if definition.index() != index || !flagged {
				return Ok(false);
			}
		}

		Ok(true)
	}

	/// The schema's definition with the index, or `None` if it has none.
	#[doc(alias("GetItemDefinition"))]
	pub fn definition(
		self,
		index: ItemDefinitionIndex,
	) -> Result<Option<ItemDefinition<'s>>, ItemSchemaError> {
		let definition = self.find(index.get())?;

		// The lookup is by index, so a definition holding another reads past
		// where the layout was checked.
		match definition {
			Some(definition) if definition.index() != index.get() => {
				Err(ItemSchemaError::UnsupportedLayout)
			}

			definition => Ok(definition),
		}
	}

	/// Looks the definition up, before the layout is known to be checked.
	fn find(self, index: u16) -> Result<Option<ItemDefinition<'s>>, ItemSchemaError> {
		// SAFETY: `Server::new` guarantees the game server module stays loaded
		// through the callback this schema is scoped to, on the main thread.
		match unsafe { self.generation.definition(index) } {
			Ok(raw) => Ok(Some(ItemDefinition {
				raw,
				_scope: PhantomData,
				_not_thread_safe: PhantomData,
			})),

			Err(ItemGenerationError::UnknownDefinition) => Ok(None),
			Err(error) => Err(error.into()),
		}
	}
}

/// Why the item schema could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
pub enum ItemSchemaError {
	/// The game's item schema lookup is not found, or the game has no schema
	/// yet.
	#[error(transparent)]
	Generation(#[from] ItemGenerationError),

	/// The server does not run TF2.
	#[error("the item schema is TF2's")]
	NotTf2,

	/// The definitions are not laid out as the generated bindings say.
	#[error("the item schema's definitions are not laid out as the bindings expect")]
	UnsupportedLayout,
}

bitflags::bitflags! {
	/// The visions in which clients draw an item (`TF_VISION_FILTER_*`).
	/// Unknown bits are preserved.
	#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
	pub struct VisionFilter: c_int {
		/// Pyrovision, such as the Pyro's goggles give.
		#[doc(alias("TF_VISION_FILTER_PYRO"))]
		const PYRO = 1 << 0;

		/// Halloween vision, during the Halloween event.
		#[doc(alias("TF_VISION_FILTER_HALLOWEEN"))]
		const HALLOWEEN = 1 << 1;

		/// Romevision, which dresses Mann vs. Machine's robots as Romans.
		#[doc(alias("TF_VISION_FILTER_ROME"))]
		const ROME = 1 << 2;
	}
}
