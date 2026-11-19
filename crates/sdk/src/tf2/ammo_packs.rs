//! The ammo packs TF2 drops (`tf_ammo_pack`, `CTFAmmoPack`): by a player as
//! they die, or by a building as it falls apart.
//!
//! On a Halloween map, a death's pack is sometimes a pumpkin that grants
//! critical hits, and at Christmas a gift: the game picks the pack's
//! [kind](AmmoPackKind) as it drops it, giving it the holiday's model and
//! flying it by gravity instead of physics. [`AmmoPack::set_kind`] changes
//! only the kind, which decides what a pickup grants beside the ammo.

use crate::datatables::{NetProp, NetPropError};
use crate::entities::Entity;
use crate::{Game, InterfaceError, Server};
use sdk_raw::tf2::ammo_packs as raw;
use std::ffi::{CStr, c_int};

const AMMO_PACK: &CStr = c"CTFAmmoPack";

/// A dropped ammo pack (`CTFAmmoPack`, `tf_ammo_pack`).
#[derive(Debug, Clone, Copy)]
pub struct AmmoPack<'s> {
	entity: Entity<'s>,

	/// The pack's networked initial velocity, which its kind precedes.
	initial_velocity: NetProp<'s>,
}

impl<'s> AmmoPack<'s> {
	/// Wraps a dropped ammo pack. Fails with [`AmmoPackError::WrongGame`]
	/// unless the server runs TF2, and with [`AmmoPackError::NotAmmoPack`]
	/// unless `entity`'s data descriptions include `CTFAmmoPack`'s, as they do
	/// still once a plugin renamed its class.
	pub fn new(server: Server<'s>, entity: Entity<'s>) -> Result<Self, AmmoPackError> {
		if server.game() != Game::TeamFortress2 {
			return Err(AmmoPackError::WrongGame);
		}

		if !entity.has_data_map_class(AMMO_PACK) {
			return Err(AmmoPackError::NotAmmoPack);
		}

		let initial_velocity = server
			.server_game_dll()?
			.entity_net_prop(entity, c"m_vecInitialVelocity")?;

		if initial_velocity.offset() < raw::PACK_TYPE_BEFORE_INITIAL_VELOCITY {
			return Err(AmmoPackError::NotAmmoPack);
		}

		Ok(Self {
			entity,
			initial_velocity,
		})
	}

	/// The pack's entity.
	pub fn entity(self) -> Entity<'s> {
		self.entity
	}

	/// What the pack grants beside its ammo (`m_PackType`). Fails with
	/// [`AmmoPackError::UnknownKind`] if the field holds a number the game
	/// does not use, as it would if the game's layout changed.
	#[doc(alias("m_PackType"))]
	pub fn kind(self) -> Result<AmmoPackKind, AmmoPackError> {
		let raw = self.read_kind();

		AmmoPackKind::from_raw(raw).ok_or(AmmoPackError::UnknownKind(raw))
	}

	fn kind_ptr(self) -> *mut c_int {
		// SAFETY: `new` checked the offset is past the kind's, inside the pack.
		unsafe {
			self.entity
				.as_ptr()
				.cast::<u8>()
				.add(self.initial_velocity.offset() - raw::PACK_TYPE_BEFORE_INITIAL_VELOCITY)
				.cast::<c_int>()
		}
	}

	fn read_kind(self) -> c_int {
		// SAFETY: As for `set_kind`, only reading.
		unsafe { self.kind_ptr().read_unaligned() }
	}

	/// Sets what the pack grants beside its ammo (`m_PackType`), which clients
	/// never see. The model, the sound and the flight the game gave a holiday
	/// pack stay. Fails as [`Self::kind`] does, before writing anything.
	#[doc(alias("m_PackType"))]
	pub fn set_kind(self, kind: AmmoPackKind) -> Result<(), AmmoPackError> {
		self.kind()?;

		// SAFETY: The pack is a live `CTFAmmoPack` during `'s` on the main
		// thread, whose class declares its 4-byte kind right before the
		// networked initial velocity, and that kind held a known value.
		unsafe { self.kind_ptr().write_unaligned(kind.to_raw()) };
		Ok(())
	}
}

/// Why an [`AmmoPack`] operation failed.
#[derive(Debug, thiserror::Error)]
pub enum AmmoPackError {
	/// A required engine interface is unavailable.
	#[error(transparent)]
	Interface(#[from] InterfaceError),

	/// The pack's networked variables could not be found.
	#[error(transparent)]
	NetProp(#[from] NetPropError),

	/// The entity is not a dropped ammo pack.
	#[error("the entity is not a dropped ammo pack")]
	NotAmmoPack,

	/// The pack's kind holds a number the game does not use.
	#[error("the ammo pack's kind reads {0}, which TF2 does not use")]
	UnknownKind(c_int),

	/// The server does not run Team Fortress 2.
	#[error("ammo pack operations require Team Fortress 2")]
	WrongGame,
}

/// What an [`AmmoPack`] grants beside its ammo (`AmmoPackType_t`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[doc(alias("AmmoPackType_t", "m_PackType"))]
pub enum AmmoPackKind {
	/// Ammo only (`AP_NORMAL`).
	Normal,

	/// A pumpkin (`AP_HALLOWEEN`): the first player to pick it up is also
	/// critically boosted for a few seconds (`TF_COND_CRITBOOSTED_PUMPKIN`).
	/// It turns [`Normal`](Self::Normal) once picked up.
	Halloween,

	/// A gift (`AP_CHRISTMAS`), which also counts toward an achievement. It
	/// turns [`Normal`](Self::Normal) once picked up.
	Christmas,
}

impl AmmoPackKind {
	/// The kind of `raw`, or `None` for a number the game does not use.
	pub const fn from_raw(raw: c_int) -> Option<Self> {
		match raw {
			raw::AP_NORMAL => Some(Self::Normal),
			raw::AP_HALLOWEEN => Some(Self::Halloween),
			raw::AP_CHRISTMAS => Some(Self::Christmas),
			_ => None,
		}
	}

	/// The game's number for the kind.
	pub const fn to_raw(self) -> c_int {
		match self {
			Self::Normal => raw::AP_NORMAL,
			Self::Halloween => raw::AP_HALLOWEEN,
			Self::Christmas => raw::AP_CHRISTMAS,
		}
	}
}
