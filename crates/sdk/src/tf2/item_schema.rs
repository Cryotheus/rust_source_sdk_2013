//! TF2's item schema: what an item definition says of the items made from
//! it, read through the game's own lookup, `CEconItemSchema::GetItemDefinition`,
//! which [`ItemGeneration`] finds, and the qualities and levels items are made
//! with.
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
//! The definitions' names and item classes are checked apart, on the
//! definitions whose strings the shipped schema fixes: the Rocket Launcher
//! (18), the Football Helmet and the Pet Balloonicorn. Should their strings
//! read wrong, or the schema lack one of them, only
//! [`ItemDefinition::name`], [`ItemDefinition::item_class`] and
//! [`ItemSchema::definition_by_name`] fail. The definitions' classes,
//! loadout positions, quality, levels and holiday restriction are checked
//! apart in the same way, on the Rocket Launcher, the Football Helmet, the
//! Mildly Disturbing Halloween Mask (115) and the Pet Balloonicorn, and only
//! the methods reading them fail with them.
//!
//! [`ItemSchema::definitions`] walks the schema's sorted map of its
//! definitions, whose tree the bindings leave opaque, and checks it on each
//! walk: as [`sdk_raw::tf2::item_schema`] describes, and against the game's
//! own lookup, which must find the first, middle and last of the definitions
//! the walk finds.
//!
//! The schema the server runs can differ from the one it shipped with, as
//! the Game Coordinator can send a newer one, which TF2 applies at the next
//! level change. Should it lack a checked definition, or change its values,
//! [`ItemSchema::new`] fails too.

#[cfg(test)]
#[path = "../tests/tf2/item_schema.rs"]
mod tests;

use crate::tf2::PlayerClass;
use crate::tf2::weapons::{ItemDefinitionIndex, ItemGenerationError};
use crate::{Game, NotThreadSafe, Server};
use sdk_raw::tf2::item_generation::ItemGeneration;
use sdk_raw::tf2::item_schema;
use std::ffi::{CStr, c_char, c_int};
use std::marker::PhantomData;
use std::ops::RangeInclusive;
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

/// The definitions whose classes, loadout positions, quality, levels and
/// holiday restriction the shipped schema fixes, to check their layout with.
const CHECKED_DETAILS: [Details; 4] = [
	// The Rocket Launcher.
	Details {
		index: 18,
		class: Some(PlayerClass::Soldier),
		position: LoadoutPosition::Primary,
		quality: ItemQuality::Normal,
		levels: Some((1, 1)),
		holiday: None,
	},
	// The Football Helmet, whose `head` slot the game reads as `misc`.
	Details {
		index: 49,
		class: Some(PlayerClass::Heavy),
		position: LoadoutPosition::Misc,
		quality: ItemQuality::Unique,
		levels: None,
		holiday: None,
	},
	// The Mildly Disturbing Halloween Mask.
	Details {
		index: 115,
		class: None,
		position: LoadoutPosition::Misc,
		quality: ItemQuality::Unique,
		levels: Some((10, 10)),
		holiday: Some(c"halloween_or_fullmoon"),
	},
	// The Pet Balloonicorn.
	Details {
		index: 738,
		class: None,
		position: LoadoutPosition::Misc,
		quality: ItemQuality::Unique,
		levels: Some((20, 20)),
		holiday: None,
	},
];

/// Whether the layout of the definitions' details passed its checks, once it
/// did or failed them.
static DETAILS: OnceLock<bool> = OnceLock::new();

/// What the shipped schema says of a [checked definition](CHECKED_DETAILS).
struct Details {
	/// The definition's index.
	index: u16,

	/// The one class that uses it, or `None` for all nine.
	class: Option<PlayerClass>,

	/// Its loadout position, for every class that uses it.
	position: LoadoutPosition,

	/// Its quality.
	quality: ItemQuality,

	/// Its lowest and highest levels, unless the schema's defaults.
	levels: Option<(u8, u8)>,

	/// Its holiday restriction.
	holiday: Option<&'static CStr>,
}

/// The fields of a definition its details are read from, as they are read,
/// before their layout is known to be checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RawDetails {
	/// `m_vbClassUsability`, a bit per class number.
	classes: u32,

	/// `m_iDefaultLoadoutSlot`.
	default_position: c_int,

	/// `m_unMaxItemLevel`.
	max_level: u8,

	/// `m_unMinItemLevel`.
	min_level: u8,

	/// `m_iLoadoutSlots`, by class number.
	positions: [c_int; 11],

	/// `m_nItemQuality`.
	quality: u8,
}

/// A position in a player's loadout (`loadout_positions_t`), which an item is
/// equipped in, such as a weapon slot or a cosmetic one.
///
/// Its numbers are those of TF2's loadouts, not of [weapon
/// slots](crate::tf2::weapons::WeaponSlot): the Engineer's construction PDA
/// is in [`Self::Pda`] (5), but in weapon slot 3.
#[doc(alias("loadout_positions_t", "LOADOUT_POSITION"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum LoadoutPosition {
	/// `LOADOUT_POSITION_PRIMARY`: 0.
	#[doc(alias("LOADOUT_POSITION_PRIMARY"))]
	Primary,

	/// `LOADOUT_POSITION_SECONDARY`: 1.
	#[doc(alias("LOADOUT_POSITION_SECONDARY"))]
	Secondary,

	/// `LOADOUT_POSITION_MELEE`: 2.
	#[doc(alias("LOADOUT_POSITION_MELEE"))]
	Melee,

	/// `LOADOUT_POSITION_UTILITY`: 3, the PASS Time gun's.
	#[doc(alias("LOADOUT_POSITION_UTILITY"))]
	Utility,

	/// `LOADOUT_POSITION_BUILDING`: 4, the Engineer's builder and the Spy's
	/// sappers.
	#[doc(alias("LOADOUT_POSITION_BUILDING"))]
	Building,

	/// `LOADOUT_POSITION_PDA`: 5, such as the Engineer's construction PDA or
	/// the Spy's disguise kit.
	#[doc(alias("LOADOUT_POSITION_PDA"))]
	Pda,

	/// `LOADOUT_POSITION_PDA2`: 6, such as the Engineer's destruction PDA or
	/// the Spy's watch.
	#[doc(alias("LOADOUT_POSITION_PDA2"))]
	Pda2,

	/// `LOADOUT_POSITION_HEAD`: 7, which the game no longer gives items: it
	/// reads an item's `head` slot as [`Self::Misc`].
	#[doc(alias("LOADOUT_POSITION_HEAD"))]
	Head,

	/// `LOADOUT_POSITION_MISC`: 8, the cosmetics.
	#[doc(alias("LOADOUT_POSITION_MISC"))]
	Misc,

	/// `LOADOUT_POSITION_ACTION`: 9, such as spellbooks and noise makers.
	#[doc(alias("LOADOUT_POSITION_ACTION"))]
	Action,

	/// `LOADOUT_POSITION_MISC2`: 10.
	#[doc(alias("LOADOUT_POSITION_MISC2"))]
	Misc2,

	/// `LOADOUT_POSITION_TAUNT`: 11, the first taunt.
	#[doc(alias("LOADOUT_POSITION_TAUNT"))]
	Taunt,

	/// `LOADOUT_POSITION_TAUNT2`: 12.
	#[doc(alias("LOADOUT_POSITION_TAUNT2"))]
	Taunt2,

	/// `LOADOUT_POSITION_TAUNT3`: 13.
	#[doc(alias("LOADOUT_POSITION_TAUNT3"))]
	Taunt3,

	/// `LOADOUT_POSITION_TAUNT4`: 14.
	#[doc(alias("LOADOUT_POSITION_TAUNT4"))]
	Taunt4,

	/// `LOADOUT_POSITION_TAUNT5`: 15.
	#[doc(alias("LOADOUT_POSITION_TAUNT5"))]
	Taunt5,

	/// `LOADOUT_POSITION_TAUNT6`: 16.
	#[doc(alias("LOADOUT_POSITION_TAUNT6"))]
	Taunt6,

	/// `LOADOUT_POSITION_TAUNT7`: 17.
	#[doc(alias("LOADOUT_POSITION_TAUNT7"))]
	Taunt7,

	/// `LOADOUT_POSITION_TAUNT8`: 18, the last taunt.
	#[doc(alias("LOADOUT_POSITION_TAUNT8"))]
	Taunt8,
}

impl LoadoutPosition {
	/// Every position, in the game's order.
	pub const ALL: [Self; 19] = [
		Self::Primary,
		Self::Secondary,
		Self::Melee,
		Self::Utility,
		Self::Building,
		Self::Pda,
		Self::Pda2,
		Self::Head,
		Self::Misc,
		Self::Action,
		Self::Misc2,
		Self::Taunt,
		Self::Taunt2,
		Self::Taunt3,
		Self::Taunt4,
		Self::Taunt5,
		Self::Taunt6,
		Self::Taunt7,
		Self::Taunt8,
	];

	/// The position with this `loadout_positions_t` number, or `None` for any
	/// other number, including `LOADOUT_POSITION_INVALID` (-1).
	pub const fn from_raw(raw: c_int) -> Option<Self> {
		if raw >= 0 && raw < Self::ALL.len() as c_int {
			Some(Self::ALL[raw as usize])
		} else {
			None
		}
	}

	/// The position's `loadout_positions_t` number.
	pub const fn to_raw(self) -> c_int {
		self as c_int
	}
}

/// The definitions whose name and item class the shipped schema fixes, to
/// check the strings' layout with.
const CHECKED_STRINGS: [(u16, &CStr, &CStr); 3] = [
	(18, c"TF_WEAPON_ROCKETLAUNCHER", c"tf_weapon_rocketlauncher"),
	(49, c"Football Helmet", c"tf_wearable"),
	(738, c"Pet Balloonicorn", c"tf_wearable"),
];

/// Whether the layout of the definitions' strings passed its checks, once it
/// did or failed them.
static STRINGS: OnceLock<bool> = OnceLock::new();

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

/// An item's level (`m_iEntityLevel`), which clients show in its description,
/// such as "Level 10 Rocket Launcher".
///
/// Clients receive the level as a signed 8-bit number, so it runs from 0 to
/// [`Self::MAX`].
#[doc(alias("m_iEntityLevel"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ItemLevel(u8);

impl ItemLevel {
	/// Level 1, which the game gives the items it creates for players.
	pub const DEFAULT: Self = Self(1);

	/// The highest level clients receive unchanged: 127.
	pub const MAX: Self = Self(i8::MAX as u8);

	/// The level, or `None` above [`Self::MAX`].
	pub const fn new(level: u8) -> Option<Self> {
		if level <= Self::MAX.0 {
			Some(Self(level))
		} else {
			None
		}
	}

	/// The level as a number.
	pub const fn get(self) -> u8 {
		self.0
	}
}

impl Default for ItemLevel {
	fn default() -> Self {
		Self::DEFAULT
	}
}

/// An item's quality (`EEconItemQuality`), which clients color its name by,
/// and show in it, such as a Strange or Vintage weapon.
///
/// A quality changes only how clients show the item: a Strange item counts
/// nothing without a kill-counting attribute (`kill eater`), and an Unusual
/// one has no effect without an effect attribute.
///
/// Clients receive the quality as a signed 5-bit number, so only the
/// qualities up to [`Self::DecoratedWeapon`] reach them unchanged. The
/// schema's unused qualities and its rarity grades are left out.
#[doc(alias("EEconItemQuality", "entityquality_t", "m_iEntityQuality"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ItemQuality {
	/// Normal (`AE_NORMAL`): the stock items.
	#[doc(alias("AE_NORMAL"))]
	Normal,

	/// Genuine (`AE_RARITY1`), such as items from promotions.
	#[doc(alias("AE_RARITY1"))]
	Genuine,

	/// Vintage (`AE_VINTAGE`), the items found before the Mann-Conomy update.
	#[doc(alias("AE_VINTAGE"))]
	Vintage,

	/// Unusual (`AE_UNUSUAL`).
	#[doc(alias("AE_UNUSUAL"))]
	Unusual,

	/// Unique (`AE_UNIQUE`), which the game gives the items it creates for
	/// players.
	#[doc(alias("AE_UNIQUE"))]
	Unique,

	/// Community (`AE_COMMUNITY`).
	#[doc(alias("AE_COMMUNITY"))]
	Community,

	/// Valve (`AE_DEVELOPER`).
	#[doc(alias("AE_DEVELOPER"))]
	Valve,

	/// Self-Made (`AE_SELFMADE`).
	#[doc(alias("AE_SELFMADE"))]
	SelfMade,

	/// Strange (`AE_STRANGE`).
	#[doc(alias("AE_STRANGE"))]
	Strange,

	/// Haunted (`AE_HAUNTED`).
	#[doc(alias("AE_HAUNTED"))]
	Haunted,

	/// Collector's (`AE_COLLECTORS`).
	#[doc(alias("AE_COLLECTORS"))]
	Collectors,

	/// Decorated Weapon (`AE_PAINTKITWEAPON`).
	#[doc(alias("AE_PAINTKITWEAPON"))]
	DecoratedWeapon,
}

impl ItemQuality {
	/// Every quality, in the game's order.
	pub const ALL: [Self; 12] = [
		Self::Normal,
		Self::Genuine,
		Self::Vintage,
		Self::Unusual,
		Self::Unique,
		Self::Community,
		Self::Valve,
		Self::SelfMade,
		Self::Strange,
		Self::Haunted,
		Self::Collectors,
		Self::DecoratedWeapon,
	];

	/// The quality with this `EEconItemQuality` number, or `None` for one
	/// left out.
	pub const fn from_raw(raw: sys::entityquality_t) -> Option<Self> {
		Some(match raw {
			sys::EEconItemQuality_AE_NORMAL => Self::Normal,
			sys::EEconItemQuality_AE_RARITY1 => Self::Genuine,
			sys::EEconItemQuality_AE_VINTAGE => Self::Vintage,
			sys::EEconItemQuality_AE_UNUSUAL => Self::Unusual,
			sys::EEconItemQuality_AE_UNIQUE => Self::Unique,
			sys::EEconItemQuality_AE_COMMUNITY => Self::Community,
			sys::EEconItemQuality_AE_DEVELOPER => Self::Valve,
			sys::EEconItemQuality_AE_SELFMADE => Self::SelfMade,
			sys::EEconItemQuality_AE_STRANGE => Self::Strange,
			sys::EEconItemQuality_AE_HAUNTED => Self::Haunted,
			sys::EEconItemQuality_AE_COLLECTORS => Self::Collectors,
			sys::EEconItemQuality_AE_PAINTKITWEAPON => Self::DecoratedWeapon,
			_ => return None,
		})
	}

	/// The quality's `EEconItemQuality` number.
	pub const fn to_raw(self) -> sys::entityquality_t {
		match self {
			Self::Normal => sys::EEconItemQuality_AE_NORMAL,
			Self::Genuine => sys::EEconItemQuality_AE_RARITY1,
			Self::Vintage => sys::EEconItemQuality_AE_VINTAGE,
			Self::Unusual => sys::EEconItemQuality_AE_UNUSUAL,
			Self::Unique => sys::EEconItemQuality_AE_UNIQUE,
			Self::Community => sys::EEconItemQuality_AE_COMMUNITY,
			Self::Valve => sys::EEconItemQuality_AE_DEVELOPER,
			Self::SelfMade => sys::EEconItemQuality_AE_SELFMADE,
			Self::Strange => sys::EEconItemQuality_AE_STRANGE,
			Self::Haunted => sys::EEconItemQuality_AE_HAUNTED,
			Self::Collectors => sys::EEconItemQuality_AE_COLLECTORS,
			Self::DecoratedWeapon => sys::EEconItemQuality_AE_PAINTKITWEAPON,
		}
	}
}

impl<'s> ItemDefinition<'s> {
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

	/// The entity class the game creates the definition's items as
	/// (`m_pszItemClassname`, the `item_class` of `items_game.txt`), such as
	/// `tf_wearable` or `tf_weapon_rocketlauncher`, or `None` for a definition
	/// without one. A weapon shared by several player classes can have a class
	/// the game translates for the player's, such as `tf_weapon_shotgun`.
	///
	/// Fails with [`ItemSchemaError::UnsupportedLayout`] unless the definitions'
	/// strings passed their check, as the
	/// [module documentation](crate::tf2::item_schema#layout) describes.
	#[doc(alias("m_pszItemClassname", "GetItemClass"))]
	pub fn item_class(self) -> Result<Option<&'s CStr>, ItemSchemaError> {
		strings_checked()?;

		// SAFETY: The schema keeps its definition through the callback, and the
		// layout of its strings was checked.
		Ok(unsafe { self.read_string(|raw| &raw const (*raw).m_pszItemClassname) })
	}

	/// The definition's name (`m_pszDefinitionName`, the `name` of
	/// `items_game.txt`), such as `Pet Balloonicorn` or
	/// `TF_WEAPON_ROCKETLAUNCHER`, which [`ItemSchema::definition_by_name`] finds
	/// it by, or `None` for a definition without one.
	///
	/// Fails as [`Self::item_class`] does.
	#[doc(alias("m_pszDefinitionName", "GetDefinitionName"))]
	pub fn name(self) -> Result<Option<&'s CStr>, ItemSchemaError> {
		strings_checked()?;

		// SAFETY: As for `item_class`.
		Ok(unsafe { self.read_string(|raw| &raw const (*raw).m_pszDefinitionName) })
	}

	/// Wraps one of the schema's definitions.
	fn new(raw: NonNull<sys::CEconItemDefinition>) -> Self {
		Self {
			raw,
			_scope: PhantomData,
			_not_thread_safe: PhantomData,
		}
	}

	/// Reads the string `field` places in the definition, or `None` if the
	/// definition has none there.
	///
	/// # Safety
	///
	/// `field` places a string pointer within the definition, which the schema
	/// keeps, with its strings, through the callback.
	unsafe fn read_string(
		self,
		field: impl FnOnce(*const sys::CEconItemDefinition) -> *const *const c_char,
	) -> Option<&'s CStr> {
		// SAFETY: The caller vouches for the field and the string it points to,
		// which lives as long as the schema's definition.
		unsafe {
			let string = field(self.raw.as_ptr()).read();

			(!string.is_null()).then(|| CStr::from_ptr(string))
		}
	}

	/// The loadout position the definition's items are equipped in
	/// (`m_iDefaultLoadoutSlot`, the `item_slot` of `items_game.txt`), unless a
	/// class equips them elsewhere, or `None` for a definition without one, such
	/// as a tool.
	///
	/// Fails with [`ItemSchemaError::UnsupportedLayout`] unless the definitions'
	/// details passed their check, as the
	/// [module documentation](crate::tf2::item_schema#layout) describes.
	#[doc(alias("m_iDefaultLoadoutSlot", "GetDefaultLoadoutSlot", "item_slot"))]
	pub fn default_loadout_position(self) -> Result<Option<LoadoutPosition>, ItemSchemaError> {
		Ok(LoadoutPosition::from_raw(
			self.checked_details()?.default_position,
		))
	}

	/// The holiday the definition's items are restricted to
	/// (`m_pszHolidayRestriction`), as the schema names it, such as
	/// `halloween_or_fullmoon`, or `None` for items of any day. The game refuses
	/// to equip a restricted item outside its holiday.
	///
	/// Fails as [`Self::default_loadout_position`] does.
	#[doc(alias("m_pszHolidayRestriction", "GetHolidayRestriction"))]
	pub fn holiday_restriction(self) -> Result<Option<&'s CStr>, ItemSchemaError> {
		self.checked_details()?;

		// SAFETY: The schema keeps its definition, and the string, through the
		// callback, and the layout of its details was checked.
		Ok(unsafe { self.read_string(|raw| &raw const (*raw).m_pszHolidayRestriction) })
	}

	/// Whether `class` can use the definition's items (`m_vbClassUsability`, the
	/// `used_by_classes` of `items_game.txt`).
	///
	/// Fails as [`Self::default_loadout_position`] does.
	#[doc(alias("m_vbClassUsability", "CanBeUsedByClass", "used_by_classes"))]
	pub fn is_used_by(self, class: PlayerClass) -> Result<bool, ItemSchemaError> {
		Ok(self.checked_details()?.classes & 1 << class.to_raw() != 0)
	}

	/// The levels the definition's items can have (`m_unMinItemLevel` and
	/// `m_unMaxItemLevel`), among which the game rolls one when asked for the
	/// definition's own level. The items the SDK gives are at the level they
	/// are given.
	///
	/// Fails as [`Self::default_loadout_position`] does.
	#[doc(alias("m_unMinItemLevel", "m_unMaxItemLevel", "GetMinLevel", "GetMaxLevel"))]
	pub fn levels(self) -> Result<RangeInclusive<u8>, ItemSchemaError> {
		let details = self.checked_details()?;

		Ok(details.min_level..=details.max_level)
	}

	/// The loadout position `class` equips the definition's items in
	/// (`m_iLoadoutSlots`), such as the Engineer's shotgun in
	/// [`LoadoutPosition::Primary`] where other classes have it in
	/// [`LoadoutPosition::Secondary`], or `None` if `class` does not use them.
	///
	/// Fails as [`Self::default_loadout_position`] does.
	#[doc(alias("m_iLoadoutSlots", "GetLoadoutSlot"))]
	pub fn loadout_position(
		self,
		class: PlayerClass,
	) -> Result<Option<LoadoutPosition>, ItemSchemaError> {
		let positions = self.checked_details()?.positions;

		Ok(positions
			.get(class.to_raw() as usize)
			.and_then(|&position| LoadoutPosition::from_raw(position)))
	}

	/// The quality the schema gives the definition's items (`m_nItemQuality`,
	/// the `item_quality` of `items_game.txt`), such as [`ItemQuality::Normal`]
	/// for the stock weapons, or `None` for a quality [`ItemQuality`] leaves out.
	/// The items the SDK gives have the quality they are given.
	///
	/// Fails as [`Self::default_loadout_position`] does.
	#[doc(alias("m_nItemQuality", "GetQuality", "item_quality"))]
	pub fn quality(self) -> Result<Option<ItemQuality>, ItemSchemaError> {
		Ok(ItemQuality::from_raw(c_int::from(
			self.checked_details()?.quality,
		)))
	}

	/// The fields the definition's details are read from, once their layout
	/// passed its checks.
	fn checked_details(self) -> Result<RawDetails, ItemSchemaError> {
		match DETAILS.get() {
			// SAFETY: The schema keeps its definition through the callback, and the
			// layout of its details was checked.
			Some(true) => Ok(unsafe { self.read_details() }),
			_ => Err(ItemSchemaError::UnsupportedLayout),
		}
	}

	/// Reads the fields the definition's details are read from.
	///
	/// # Safety
	///
	/// The schema keeps the definition, a `CTFItemDefinition` as all of TF2's
	/// are, through the callback.
	unsafe fn read_details(self) -> RawDetails {
		let base = self.raw.as_ptr();
		let tf = base.cast::<sys::CTFItemDefinition>();

		// SAFETY: The caller vouches for the definition. The fields are plain data
		// within it.
		unsafe {
			RawDetails {
				classes: (&raw const (*tf).m_vbClassUsability).read(),
				default_position: (&raw const (*tf).m_iDefaultLoadoutSlot).read(),
				max_level: (&raw const (*base).m_unMaxItemLevel).read(),
				min_level: (&raw const (*base).m_unMinItemLevel).read(),
				positions: (&raw const (*tf).m_iLoadoutSlots).read(),
				quality: (&raw const (*base).m_nItemQuality).read(),
			}
		}
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

		let checked = match LAYOUT.get() {
			Some(&checked) => checked,

			None => {
				// A schema missing a checked definition says nothing of the layout,
				// so only a definition that reads wrong is remembered.
				let checked = schema.check_layout()?;

				LAYOUT.set(checked).ok();
				checked
			}
		};

		if !checked {
			return Err(ItemSchemaError::UnsupportedLayout);
		}

		// The strings are checked apart, so that a layout placing them otherwise
		// fails only what reads them. As above, only a definition that reads wrong
		// is remembered.
		if STRINGS.get().is_none()
			&& let Ok(Some(checked)) = schema.check_strings()
		{
			STRINGS.set(checked).ok();
		}

		// As are the details.
		if DETAILS.get().is_none()
			&& let Ok(Some(checked)) = schema.check_details()
		{
			DETAILS.set(checked).ok();
		}

		Ok(schema)
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
			Ok(raw) => Ok(Some(ItemDefinition::new(raw))),
			Err(ItemGenerationError::UnknownDefinition) => Ok(None),
			Err(error) => Err(error.into()),
		}
	}

	/// Whether the [checked definitions' strings](CHECKED_STRINGS) read as the
	/// shipped schema has them, or `None` if the schema lacks one of them.
	fn check_strings(self) -> Result<Option<bool>, ItemSchemaError> {
		for (index, name, class) in CHECKED_STRINGS {
			let Some(definition) = self.find(index)? else {
				return Ok(None);
			};

			// SAFETY: The definition is the schema's, live through the callback, and
			// its index and vision filter, on either side of the strings, were
			// checked.
			let strings = unsafe {
				(
					definition.read_string(|raw| &raw const (*raw).m_pszDefinitionName),
					definition.read_string(|raw| &raw const (*raw).m_pszItemClassname),
				)
			};

			if strings != (Some(name), Some(class)) {
				return Ok(Some(false));
			}
		}

		Ok(Some(true))
	}

	/// Whether the [checked definitions' details](CHECKED_DETAILS) read as the
	/// shipped schema has them, or `None` if the schema lacks one of them.
	fn check_details(self) -> Result<Option<bool>, ItemSchemaError> {
		for details in CHECKED_DETAILS {
			let Some(definition) = self.find(details.index)? else {
				return Ok(None);
			};

			// SAFETY: The definition is the schema's, live through the callback, and
			// a `CTFItemDefinition`, as all of TF2's are.
			let raw = unsafe { definition.read_details() };

			let levels = details
				.levels
				.is_none_or(|levels| levels == (raw.min_level, raw.max_level));

			let mut classes = 0;
			let mut positions = [-1; 11];

			for class in PlayerClass::ALL {
				if details.class.is_none_or(|only| only == class) {
					classes |= 1 << class.to_raw();
					positions[class.to_raw() as usize] = details.position.to_raw();
				}
			}

			let checked = levels
				&& raw.classes == classes
				&& raw.positions == positions
				&& raw.default_position == details.position.to_raw()
				&& raw.quality == details.quality.to_raw() as u8;

			// The holiday restriction is a pointer, so it is read only once the rest
			// placed it.
			//
			// SAFETY: As above, and the definition keeps the string.
			if !checked
				|| unsafe {
					definition.read_string(|raw| &raw const (*raw).m_pszHolidayRestriction)
				} != details.holiday
			{
				return Ok(Some(false));
			}
		}

		Ok(Some(true))
	}

	/// The schema's definition with the name, as [`ItemDefinition::name`] gives
	/// it, compared ignoring ASCII case as the game's own lookup is, or `None` if
	/// it has none. Should several have the name, the one with the lowest index.
	///
	/// It walks every definition, as [`Self::definitions`] does, so a plugin
	/// looking a name up often keeps the index it finds instead. Fails as
	/// [`Self::definitions`] and [`ItemDefinition::name`] do.
	#[doc(alias("GetItemDefinitionByName"))]
	pub fn definition_by_name(
		self,
		name: &CStr,
	) -> Result<Option<ItemDefinition<'s>>, ItemSchemaError> {
		for definition in self.definitions()? {
			let named = definition
				.name()?
				.is_some_and(|own| own.to_bytes().eq_ignore_ascii_case(name.to_bytes()));

			if named {
				return Ok(Some(definition));
			}
		}

		Ok(None)
	}

	/// Every definition of the schema, in order of their index, read from its
	/// sorted map of them (`m_mapItemsSorted`).
	///
	/// The map is checked on each call, as the
	/// [module documentation](crate::tf2::item_schema#layout) describes: this
	/// fails with [`ItemSchemaError::UnsupportedLayout`] if it is not laid out as
	/// the SDK's headers say, and as [`Self::definition`] does otherwise. Each
	/// call reads the whole map, which holds over ten thousand definitions.
	#[doc(alias("m_mapItemsSorted", "GetSortedItemDefinitionMap"))]
	pub fn definitions(self) -> Result<Vec<ItemDefinition<'s>>, ItemSchemaError> {
		// SAFETY: `Server::new` guarantees the game server module stays loaded
		// through the callback this schema is scoped to, on the main thread.
		let (schema, default) = unsafe {
			(
				self.generation.schema()?,
				self.generation.default_definition()?,
			)
		};

		// SAFETY: The schema is the game's live one, which only the game changes,
		// when it applies a newer one at a level change. The default definition is
		// the one the game's lookup gives.
		let sorted = unsafe { item_schema::sorted_definitions(schema, default) }
			.ok_or(ItemSchemaError::UnsupportedLayout)?;

		// The game's own lookup, through another map, must find the first, middle
		// and last of them too.
		let samples = [sorted.first(), sorted.get(sorted.len() / 2), sorted.last()];

		for &(index, raw) in samples.into_iter().flatten() {
			if self.find(index)?.map(|definition| definition.raw) != Some(raw) {
				return Err(ItemSchemaError::UnsupportedLayout);
			}
		}

		Ok(sorted
			.into_iter()
			.map(|(_, raw)| ItemDefinition::new(raw))
			.collect())
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

/// Fails unless the layout of the definitions' strings passed its checks.
fn strings_checked() -> Result<(), ItemSchemaError> {
	match STRINGS.get() {
		Some(true) => Ok(()),
		_ => Err(ItemSchemaError::UnsupportedLayout),
	}
}
