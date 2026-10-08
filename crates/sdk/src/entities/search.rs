//! Finding entities as the game's entity list does: by class, by model, and
//! near a point. [`Entity::name_matches`] matches names as the game's
//! searches by name do.

#[cfg(test)]
#[path = "../tests/entities/search.rs"]
mod tests;

use crate::entities::Entity;
use crate::interfaces::ServerTools;
use crate::math::Vector;
use sdk_raw::vcall;
use std::ffi::CStr;
use std::iter;
use std::ptr::{self, NonNull};

impl<'s> Entity<'s> {
	/// Whether the entity's name answers to `query`, as the game decides for
	/// the inputs and outputs it sends by name (`CBaseEntity::NameMatches`):
	/// ignoring ASCII case, with a `*` in `query` matching the rest of any name
	/// whose start matches what comes before it, and `!player`, in any case,
	/// matching every player.
	///
	/// An entity without a name answers to an empty query, and to one that
	/// starts with `*`. As the game compares characters as `int`s, a character
	/// of the name below `a` also answers to the one 32 below it, and a
	/// character below `A` to the one 32 above it, such as `1` to `Q`.
	#[doc(alias("NameMatches", "NamesMatch"))]
	pub fn name_matches(self, query: &CStr) -> bool {
		if query.to_bytes().eq_ignore_ascii_case(b"!player") {
			return self.is_a(c"CBasePlayer");
		}

		let Some(field) = self.name_field() else {
			return false;
		};

		// SAFETY: The field is the entity's `m_iName`, read without forming a
		// reference, whose string is pooled, and read at once.
		let name = unsafe { field.read() }.pszValue;

		if name.is_null() {
			return matches!(query.to_bytes(), [] | [b'*', ..]);
		}

		// SAFETY: Pooled strings live until the level ends.
		names_match(query.to_bytes(), unsafe { CStr::from_ptr(name) }.to_bytes())
	}
}

impl<'s> ServerTools<'s> {
	/// Iterates over the entities whose class name is `class_name`, ignoring
	/// ASCII case, or starts with what comes before a `*` that ends it, in the
	/// order of the entity list, as [`Self::find_by_class_name`] finds them.
	#[doc(alias("FindEntityByClassname"))]
	pub fn entities_by_class(
		self,
		class_name: &CStr,
	) -> impl Iterator<Item = Entity<'s>> + use<'s, '_> {
		iter::successors(self.find_by_class_name(None, class_name), move |&entity| {
			self.find_by_class_name(Some(entity), class_name)
		})
	}

	/// Iterates over the networked entities whose model is `model`, such as
	/// `models/props_gameplay/resupply_locker.mdl`, or `*1` for a map's first
	/// brush model, ignoring ASCII case, in the order of the entity list.
	#[doc(alias("FindEntityByModel"))]
	pub fn entities_by_model(self, model: &CStr) -> impl Iterator<Item = Entity<'s>> + use<'s, '_> {
		let find = move |after: Option<Entity<'s>>| {
			let after = after.map_or(ptr::null_mut(), Entity::as_ptr);

			// SAFETY: `Server::new` guarantees the interface is live, and `after`
			// is live or null. The game compares each entity's model name with
			// this one.
			let entity = unsafe {
				vcall!(self.as_ptr() => IServerTools_FindEntityByModel(after, model.as_ptr()))
			};

			// SAFETY: Entities are not freed immediately during `'s`.
			NonNull::new(entity).map(|entity| unsafe { Entity::from_raw(entity) })
		};

		iter::successors(find(None), move |&entity| find(Some(entity)))
	}

	/// Iterates over the networked entities whose collision bounds reach
	/// within `radius` units of `center`, in the order of the entity list.
	#[doc(alias("FindEntityInSphere"))]
	pub fn entities_in_sphere(
		self,
		center: Vector,
		radius: f32,
	) -> impl Iterator<Item = Entity<'s>> + use<'s> {
		let center = sys::Vector::from(center);

		let find = move |after: Option<Entity<'s>>| {
			let after = after.map_or(ptr::null_mut(), Entity::as_ptr);

			// SAFETY: As for `entities_by_model`. The game only reads the center
			// during the call.
			let entity = unsafe {
				vcall!(self.as_ptr() => IServerTools_FindEntityInSphere(after, &raw const center, radius))
			};

			// SAFETY: As for `entities_by_model`.
			NonNull::new(entity).map(|entity| unsafe { Entity::from_raw(entity) })
		};

		iter::successors(find(None), move |&entity| find(Some(entity)))
	}
}

/// Whether a character of a name answers to one of a query, as `NamesMatch`
/// compares them: equal, or 32 apart where the name's, promoted to an `int`,
/// lies at most 25 above `A` or `a`, which every character below them does.
fn characters_match(name: u8, query: u8) -> bool {
	let (name, query) = (i32::from(name), i32::from(query));
	let upper = i32::from(b'A');
	let lower = i32::from(b'a');

	name == query
		|| (name - upper <= 25 && name - upper + lower == query)
		|| (name - lower <= 25 && name - lower + upper == query)
}

/// Whether `name` answers to `query`, as the game's `NamesMatch` compares
/// them, characters promoted to `int`s.
fn names_match(query: &[u8], name: &[u8]) -> bool {
	let common = query
		.iter()
		.zip(name)
		.take_while(|&(&query, &name)| characters_match(name, query))
		.count();

	matches!(
		(&query[common..], &name[common..]),
		([], []) | ([b'*', ..], _)
	)
}
