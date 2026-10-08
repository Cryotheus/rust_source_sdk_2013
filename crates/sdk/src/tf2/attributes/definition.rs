//! Typed schema attribute definitions and the values they accept.

use crate::tf2::attributes::AttributeError;
use sdk_raw::tf2::attributes::INVALID_ATTRIB_DEF_INDEX;
use std::ffi::CStr;
use std::fmt::{self, Debug, Display, Formatter};
use std::marker::PhantomData;

/// An additive amount, such as health points: any finite float.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Amount(f32);

/// A [catalog](super::catalog) definition of any value type, as
/// [`catalog::ALL`](super::catalog::ALL) lists them, for attributes chosen
/// while the plugin runs, such as by name from a configuration file. Its
/// values are plain floats, which [`Self::stored`] checks as the definition's
/// value type and bounds would.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AnyAttributeDef {
	/// A definition of an additive amount.
	Amount(AttributeDef<Amount>),

	/// A definition of an on or off switch.
	Flag(AttributeDef<Flag>),

	/// A definition of a multiplier.
	Multiplier(AttributeDef<Multiplier>),

	/// A definition of a duration.
	Seconds(AttributeDef<Seconds>),
}

impl AnyAttributeDef {
	/// The attribute class, as for [`AttributeDef::class`].
	pub const fn class(&self) -> &'static CStr {
		self.raw().class
	}

	/// How the schema describes the value, as for [`AttributeDef::format`].
	pub const fn format(&self) -> DescriptionFormat {
		self.raw().format
	}

	/// The definition index, as for [`AttributeDef::index`].
	pub const fn index(&self) -> AttributeIndex {
		self.raw().index
	}

	/// The largest stored value this crate writes.
	pub const fn max(&self) -> f32 {
		self.raw().max
	}

	/// The smallest stored value this crate writes.
	pub const fn min(&self) -> f32 {
		self.raw().min
	}

	/// The schema name, as for [`AttributeDef::name`].
	pub const fn name(&self) -> &'static CStr {
		self.raw().name
	}

	/// The definition without its value type.
	pub(super) const fn raw(&self) -> RawDef {
		match self {
			Self::Amount(def) => def.raw(),
			Self::Flag(def) => def.raw(),
			Self::Multiplier(def) => def.raw(),
			Self::Seconds(def) => def.raw(),
		}
	}

	/// `value` as the game stores it, once checked as the definition's value
	/// type: [`AttributeError::InvalidValue`] for a value the type has no
	/// equal of, such as a [`Flag`] other than 0 or 1 or a negative
	/// [`Multiplier`], and [`AttributeError::OutOfDomain`] outside the
	/// definition's bounds.
	pub fn stored(&self, value: f32) -> Result<f32, AttributeError> {
		fn typed<V: AttributeValue>(
			def: &AttributeDef<V>,
			value: f32,
		) -> Result<f32, AttributeError> {
			def.stored(V::from_stored(value).ok_or(AttributeError::InvalidValue)?)
		}

		match self {
			Self::Amount(def) => typed(def, value),
			Self::Flag(def) => typed(def, value),
			Self::Multiplier(def) => typed(def, value),
			Self::Seconds(def) => typed(def, value),
		}
	}

	/// What the definition's values are: `amount`, `flag`, `multiplier` or
	/// `seconds`.
	pub const fn value_kind(&self) -> &'static str {
		match self {
			Self::Amount(_) => "amount",
			Self::Flag(_) => "flag",
			Self::Multiplier(_) => "multiplier",
			Self::Seconds(_) => "seconds",
		}
	}
}

impl Amount {
	/// No change.
	pub const ZERO: Self = Self(0.0);

	/// A finite amount, or `None` for NaN and infinities. Negative zero
	/// becomes zero.
	pub const fn new(amount: f32) -> Option<Self> {
		if amount.is_finite() {
			Some(Self(amount + 0.0))
		} else {
			None
		}
	}

	/// The amount.
	pub const fn get(self) -> f32 {
		self.0
	}
}

impl AttributeValue for Amount {
	fn from_stored(stored: f32) -> Option<Self> {
		Self::new(stored)
	}

	fn to_stored(self) -> f32 {
		self.0
	}
}

impl sealed::Sealed for Amount {}

/// A schema attribute vetted against the item schema TF2 ships: its name,
/// definition index, attribute class, description format, and the values
/// this crate writes to it.
///
/// Every definition in the [catalog](super::catalog) has TF2's legacy default
/// type, is stored as a float, and is read by gameplay hooks on the server.
/// The bounds are conservative policy rather than game limits: they keep
/// values away from the integer conversions and loops in game code that
/// extreme values overflow or stall.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AttributeDef<V: AttributeValue> {
	raw: RawDef,
	_value: PhantomData<fn() -> V>,
}

impl<V: AttributeValue> AttributeDef<V> {
	/// A definition the catalog vets. `min` and `max` bound the stored value,
	/// and must differ, so that [`ItemAttributes::set`] can confirm the
	/// running schema's index with another value.
	///
	/// [`ItemAttributes::set`]: super::ItemAttributes::set
	pub(super) const fn new(
		index: u16,
		name: &'static CStr,
		class: &'static CStr,
		format: DescriptionFormat,
		min: f32,
		max: f32,
	) -> Self {
		let Some(index) = AttributeIndex::new(index) else {
			panic!("catalog attributes need a valid definition index");
		};

		assert!(min.is_finite() && max.is_finite() && min < max);

		Self {
			raw: RawDef {
				name,
				index,
				class,
				format,
				min,
				max,
			},
			_value: PhantomData,
		}
	}

	/// The attribute class, the name of the gameplay hook that reads the
	/// attribute, such as `mult_dmg`.
	#[doc(alias("attribute_class"))]
	pub const fn class(&self) -> &'static CStr {
		self.raw.class
	}

	/// Whether the value lies within this definition's bounds.
	pub fn contains(&self, value: V) -> bool {
		self.raw.contains(value.to_stored())
	}

	/// How the schema describes the value, which decides how the game folds
	/// it with the same attribute class from other providers.
	#[doc(alias("description_format"))]
	pub const fn format(&self) -> DescriptionFormat {
		self.raw.format
	}

	/// The definition index the shipped schema gives the attribute. Writes
	/// check that the running schema maps [`Self::name`] to it.
	#[doc(alias("defindex"))]
	pub const fn index(&self) -> AttributeIndex {
		self.raw.index
	}

	/// The largest stored value this crate writes.
	pub const fn max(&self) -> f32 {
		self.raw.max
	}

	/// The smallest stored value this crate writes.
	pub const fn min(&self) -> f32 {
		self.raw.min
	}

	/// The schema name the game looks the attribute up by, such as
	/// `damage bonus`.
	pub const fn name(&self) -> &'static CStr {
		self.raw.name
	}

	/// The definition without its value type.
	pub(super) const fn raw(&self) -> RawDef {
		self.raw
	}

	/// The value as the game stores it, or [`AttributeError::OutOfDomain`]
	/// outside this definition's bounds.
	pub fn stored(&self, value: V) -> Result<f32, AttributeError> {
		let stored = value.to_stored();

		if self.raw.contains(stored) {
			Ok(stored)
		} else {
			Err(AttributeError::OutOfDomain)
		}
	}
}

/// An attribute definition index in TF2's item schema
/// (`attrib_definition_index_t`), never the invalid sentinel 65535.
#[doc(alias("attrib_definition_index_t"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AttributeIndex(u16);

impl AttributeIndex {
	/// Excludes [`INVALID_ATTRIB_DEF_INDEX`], 65535.
	pub const fn new(index: u16) -> Option<Self> {
		if index == INVALID_ATTRIB_DEF_INDEX {
			None
		} else {
			Some(Self(index))
		}
	}

	/// The raw definition index.
	pub const fn get(self) -> u16 {
		self.0
	}
}

impl Display for AttributeIndex {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		Display::fmt(&self.0, f)
	}
}

/// A value one of TF2's float-stored default-type attributes holds.
///
/// Sealed: the [catalog](super::catalog) pairs each definition with the value
/// type its gameplay hooks expect, and values reach the game only as the
/// float the schema stores.
pub trait AttributeValue: sealed::Sealed + Copy + Debug {
	/// Converts a stored float back, or returns `None` if it is not a valid
	/// value of this type.
	fn from_stored(stored: f32) -> Option<Self>;

	/// The value as the game stores it.
	fn to_stored(self) -> f32;
}

/// How the game folds values of one attribute class from several providers,
/// such as a player and each of their weapons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Combine {
	/// Values are added together.
	Add,

	/// Values are combined with a bitwise or.
	BitOr,

	/// Values are multiplied together.
	Multiply,
}

/// How the item schema describes an attribute's value (`description_format`),
/// which also decides how the game combines it (`ApplyAttribute` in
/// `attribute_manager.cpp`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DescriptionFormat {
	/// An amount added to the hooked value.
	#[doc(alias("value_is_additive"))]
	Additive,

	/// A fraction added to the hooked value, described as a percentage.
	#[doc(alias("value_is_additive_percentage"))]
	AdditivePercentage,

	/// A multiplier, described as a percentage of decrease.
	#[doc(alias("value_is_inverted_percentage"))]
	InvertedPercentage,

	/// Bits combined with a bitwise or.
	#[doc(alias("value_is_or"))]
	Or,

	/// A multiplier, described as a percentage of increase.
	#[doc(alias("value_is_percentage"))]
	Percentage,
}

impl DescriptionFormat {
	/// How the game combines values of this format.
	pub const fn combine(self) -> Combine {
		match self {
			Self::Additive | Self::AdditivePercentage => Combine::Add,
			Self::InvertedPercentage | Self::Percentage => Combine::Multiply,
			Self::Or => Combine::BitOr,
		}
	}

	/// The format's keyword in `items_game.txt`, such as `value_is_percentage`.
	pub const fn keyword(self) -> &'static str {
		match self {
			Self::Additive => "value_is_additive",
			Self::AdditivePercentage => "value_is_additive_percentage",
			Self::InvertedPercentage => "value_is_inverted_percentage",
			Self::Or => "value_is_or",
			Self::Percentage => "value_is_percentage",
		}
	}
}

/// An on or off switch, stored as 1 or 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Flag(pub bool);

impl AttributeValue for Flag {
	fn from_stored(stored: f32) -> Option<Self> {
		if stored == 1.0 {
			Some(Self(true))
		} else if stored == 0.0 {
			Some(Self(false))
		} else {
			None
		}
	}

	fn to_stored(self) -> f32 {
		if self.0 { 1.0 } else { 0.0 }
	}
}

impl sealed::Sealed for Flag {}

/// A factor the game multiplies the hooked value by: a finite, nonnegative
/// float.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Multiplier(f32);

impl Multiplier {
	/// No change.
	pub const ONE: Self = Self(1.0);

	/// A finite, nonnegative factor, or `None` otherwise. Negative zero
	/// becomes zero.
	pub const fn new(factor: f32) -> Option<Self> {
		if factor.is_finite() && factor >= 0.0 {
			Some(Self(factor + 0.0))
		} else {
			None
		}
	}

	/// The factor.
	pub const fn get(self) -> f32 {
		self.0
	}
}

impl AttributeValue for Multiplier {
	fn from_stored(stored: f32) -> Option<Self> {
		Self::new(stored)
	}

	fn to_stored(self) -> f32 {
		self.0
	}
}

impl sealed::Sealed for Multiplier {}

/// A definition without its value type: what an [`AttributeSet`] keeps for
/// each entry, and what the setters need.
///
/// [`AttributeSet`]: super::AttributeSet
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct RawDef {
	pub(super) name: &'static CStr,
	pub(super) index: AttributeIndex,
	pub(super) class: &'static CStr,
	pub(super) format: DescriptionFormat,
	pub(super) min: f32,
	pub(super) max: f32,
}

impl RawDef {
	/// Whether a stored value lies within the bounds. NaN never does.
	pub(super) fn contains(self, stored: f32) -> bool {
		stored >= self.min && stored <= self.max
	}
}

/// A duration in seconds: a finite, nonnegative float.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Seconds(f32);

impl Seconds {
	/// No time.
	pub const ZERO: Self = Self(0.0);

	/// A finite, nonnegative number of seconds, or `None` otherwise. Negative
	/// zero becomes zero.
	pub const fn new(seconds: f32) -> Option<Self> {
		if seconds.is_finite() && seconds >= 0.0 {
			Some(Self(seconds + 0.0))
		} else {
			None
		}
	}

	/// The number of seconds.
	pub const fn get(self) -> f32 {
		self.0
	}
}

impl AttributeValue for Seconds {
	fn from_stored(stored: f32) -> Option<Self> {
		Self::new(stored)
	}

	fn to_stored(self) -> f32 {
		self.0
	}
}

impl sealed::Sealed for Seconds {}

/// Keeps [`AttributeValue`] from being implemented outside this module.
mod sealed {
	pub trait Sealed {}
}
