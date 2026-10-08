//! Per-player screen overlays: a material TF2 draws over one player's view,
//! without `sv_cheats`.
//!
//! [`ScreenOverlay`] sets a player's `m_Local.m_szScriptOverlayMaterial`
//! (`game/server/playerlocaldata.h:90`) through `SetScriptOverlayMaterial`
//! (`game/server/player.h:1225-1234`), the native method behind VScript's
//! `CBasePlayer` function of that name. The engine sends the name only to that
//! player's own client, with the rest of its local player data
//! (`SendProxy_SendLocalDataTable`, `game/server/player.cpp:8199`).
//!
//! # Drawing
//!
//! The client looks the material up by name and draws it over its whole
//! screen, in a slot of its own that it draws after the one TF2's condition
//! overlays use, such as those of Jarate, burning and ÜberCharges
//! (`game/client/viewrender.cpp:1249-1297`). The overlay is therefore drawn on
//! top of them, and neither replaces the other.
//!
//! Observed on TF2's 64-bit Windows server with a retail client and
//! `sv_cheats` 0: the client draws [`OverlayMaterial::JARATE`] once
//! [`ScreenOverlay::set`] sets it and stops once [`ScreenOverlay::clear`]
//! clears it, and [`ScreenOverlay::get`] returns the networked name.
//!
//! A client draws the overlay of its own player, so a spectator sees their own
//! overlay rather than that of the player they observe. Bots have no client
//! to draw theirs, so setting it changes nothing visible.
//!
//! # Lifetime
//!
//! An overlay stays until it is changed or cleared, or the player entity is
//! recreated, as when the player reconnects or the map changes. TF2 resets it
//! nowhere else, so it survives death, respawns, class and team changes, and
//! round restarts. It also outlives the plugin that set it, so a plugin should
//! clear the overlays it set before it unloads.
//!
//! A player has one overlay, which map scripts, the `SetScriptOverlayMaterial`
//! input and other plugins set too, and the last of them wins.
//! [`ScreenOverlay::clear_if`] clears an overlay only while it is still the
//! expected one.
//!
//! # Materials
//!
//! A client that lacks the material draws the error material over its whole
//! screen instead: unlike `r_screenoverlay` and `env_screenoverlay`, this slot
//! does not refuse it. Every client has TF2's own overlay materials, listed in
//! [`OverlayMaterial::STOCK`]. For a custom material, add its `.vmt` file, at
//! [`OverlayMaterial::vmt_path`], and each texture (`.vtf`) the `.vmt` names,
//! also under `materials/`, with [`add_downloadable`] for each level, before
//! clients connect to it.
//!
//! Whether a client receives those files depends on the server's download
//! settings, such as `sv_allowdownload` and `sv_downloadurl`, and the client's
//! own, such as `cl_allowdownload` and `cl_downloadfilter`. The server's
//! `sv_pure` whitelist can still keep a client from using files it downloaded.
//!
//! # Alternatives
//!
//! If the game lacks the native method, which methods report as
//! [`OverlayError::UnsupportedMethod`], [`ServerTools::accept_input`] with the
//! `SetScriptOverlayMaterial` input and an [`InputValue::String`] sets the same
//! overlay. It adds the name to the game's string pool and, if a map script
//! gave the player a script scope, runs the scope's
//! `InputSetScriptOverlayMaterial` function, which can skip the input and may
//! set an overlay of its own (`game/server/baseentity.cpp:4421-4443`). A colour
//! tint or flash needs no material: send the [`Fade`] user message instead.
//!
//! # Unverified
//!
//! Overlays have not been tested on Linux servers. The drawing order over a
//! condition overlay, the error material for a missing material, downloaded
//! custom materials, how long an overlay lasts and what spectators see follow
//! from the SDK's source and have not been observed on a live server.
//!
//! [`add_downloadable`]: crate::interfaces::network_string_tables::add_downloadable
//! [`Fade`]: crate::user_messages::messages::Fade
//! [`InputValue::String`]: crate::inputs::InputValue::String
//! [`ServerTools::accept_input`]: crate::interfaces::ServerTools::accept_input

#[cfg(test)]
#[path = "../tests/tf2/overlays.rs"]
mod tests;

use crate::entities::Entity;
use crate::tf2::script_binding::{self as binding, BindingError};
use crate::{Game, Server};
use std::borrow::Cow;
use std::ffi::{CStr, CString};

// The overlay name is a `char[MAX_PATH]` on both targets.
const _: () = assert!(MAX_MATERIAL_LEN == sdk_raw::tier0::MAX_PATH - 1);

/// The extension material names omit, compared ignoring ASCII case, and
/// which [`OverlayMaterial::vmt_path`] appends.
const EXTENSION: &[u8; 4] = b".vmt";

/// The directory material names are relative to, compared ignoring ASCII
/// case, and which [`OverlayMaterial::vmt_path`] prepends.
const MATERIALS_DIRECTORY: &[u8; 10] = b"materials/";

/// The longest material name, in bytes, an overlay holds: its `char[MAX_PATH]`
/// storage less the terminator. The game silently truncates longer names.
pub const MAX_MATERIAL_LEN: usize =
	size_of::<sys::CPlayerLocalData_NetworkVar_m_szScriptOverlayMaterial>() - 1;

/// [`OverlayMaterial::new`] refused a material name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MaterialNameError {
	/// The name is empty, which would clear the overlay instead.
	#[error("the material name is empty")]
	Empty,

	/// The name is longer than [`MAX_MATERIAL_LEN`] bytes, which the game
	/// would silently truncate.
	#[error("the material name is {len} bytes long, but at most {MAX_MATERIAL_LEN} fit")]
	TooLong {
		/// The name's length in bytes.
		len: usize,
	},

	/// The name contains a byte that is not printable ASCII, such as a
	/// control byte, or a backslash, quote, or semicolon.
	#[error("the material name contains the byte {byte:#04x} at index {index}")]
	InvalidByte {
		/// The byte's index in the name.
		index: usize,

		/// The refused byte.
		byte: u8,
	},

	/// The name starts with `materials/`, which material names are relative
	/// to.
	#[error("the material name starts with `materials/`, which names are relative to")]
	MaterialsDirectory,

	/// The name ends with the `.vmt` extension, which material names omit.
	#[error("the material name ends with `.vmt`, which names omit")]
	Extension,
}

/// A screen overlay could not be changed or read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OverlayError {
	/// The server is not running Team Fortress 2.
	#[error("screen overlays require Team Fortress 2")]
	UnsupportedGame,

	/// The entity's data maps do not include `CBasePlayer`'s.
	#[error("the entity is not a player")]
	NotAPlayer,

	/// The player is pending deletion, so its overlay is not changed or read.
	#[error("the player is marked for deletion")]
	MarkedForDeletion,

	/// The player's script class descriptors lack the native method, or its
	/// signature differs from the SDK's. The
	/// [module documentation](self#alternatives) describes a fallback.
	#[error("the game does not expose the expected native overlay method")]
	UnsupportedMethod,

	/// The native method's binding adapter reported failure.
	#[error("the native overlay method rejected the call")]
	Rejected,
}

impl From<BindingError> for OverlayError {
	fn from(error: BindingError) -> Self {
		match error {
			BindingError::Unavailable | BindingError::SignatureMismatch => Self::UnsupportedMethod,
			BindingError::Rejected => Self::Rejected,
		}
	}
}

/// The name of a material to draw as a [`ScreenOverlay`], such as
/// `effects/jarate_overlay`: relative to the `materials` directory, and
/// without the `.vmt` extension.
///
/// A name is 1 to [`MAX_MATERIAL_LEN`] bytes of printable ASCII, other than a
/// backslash (use `/`), a quote, or a semicolon. It does not start with
/// `materials/` or end with `.vmt`, in any case. Whether a client finds the
/// material is up to the client: see the
/// [module documentation](self#materials).
///
/// Equality and hashing compare names exactly, while
/// [`ScreenOverlay::clear_if`] ignores ASCII case, as TF2 does when it compares
/// its condition overlays (`FStrEq`). To compare names as `clear_if` does,
/// compare their bytes with `eq_ignore_ascii_case`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OverlayMaterial(Cow<'static, CStr>);

impl OverlayMaterial {
	/// Bleeding (`effects/bleed_overlay`).
	#[doc(alias("TF_SCREEN_OVERLAY_MATERIAL_BLEED"))]
	pub const BLEED: Self = Self::stock(c"effects/bleed_overlay");

	/// Burning (`effects/imcookin`).
	#[doc(alias("TF_SCREEN_OVERLAY_MATERIAL_BURNING"))]
	pub const BURNING: Self = Self::stock(c"effects/imcookin");

	/// Covered in gas, from the Gas Passer (`effects/gas_overlay`).
	#[doc(alias("TF_SCREEN_OVERLAY_MATERIAL_GAS"))]
	pub const GAS: Self = Self::stock(c"effects/gas_overlay");

	/// BLU's ÜberCharge (`effects/invuln_overlay_blue`).
	#[doc(alias("TF_SCREEN_OVERLAY_MATERIAL_INVULN_BLUE"))]
	pub const INVULN_BLUE: Self = Self::stock(c"effects/invuln_overlay_blue");

	/// RED's ÜberCharge (`effects/invuln_overlay_red`).
	#[doc(alias("TF_SCREEN_OVERLAY_MATERIAL_INVULN_RED"))]
	pub const INVULN_RED: Self = Self::stock(c"effects/invuln_overlay_red");

	/// Covered in Jarate, which TF2 also shows for marked for death and the
	/// swimming curse (`effects/jarate_overlay`).
	#[doc(alias(
		"TF_SCREEN_OVERLAY_MATERIAL_SWIMMING_CURSE",
		"TF_SCREEN_OVERLAY_MATERIAL_URINE"
	))]
	pub const JARATE: Self = Self::stock(c"effects/jarate_overlay");

	/// Intercepting a pass in PASS Time (`effects/dodge_overlay`).
	#[doc(alias("TF_SCREEN_OVERLAY_MATERIAL_PHASE"))]
	pub const PHASE: Self = Self::stock(c"effects/dodge_overlay");

	/// A cloaked Spy (`effects/stealth_overlay`).
	#[doc(alias("TF_SCREEN_OVERLAY_MATERIAL_STEALTH"))]
	pub const STEALTH: Self = Self::stock(c"effects/stealth_overlay");

	/// The overlay materials TF2 itself draws, which every TF2 client has
	/// (`game/shared/tf/tf_player_shared.cpp:223-234`). TF2 defines
	/// `effects/milk_screen` there too, but does not draw it.
	pub const STOCK: [Self; 8] = [
		Self::BLEED,
		Self::BURNING,
		Self::GAS,
		Self::INVULN_BLUE,
		Self::INVULN_RED,
		Self::JARATE,
		Self::PHASE,
		Self::STEALTH,
	];

	/// Checks a material name, such as `effects/example_overlay`, borrowing a
	/// `&'static CStr` or taking a `CString` without copying it.
	///
	/// Fails with a [`MaterialNameError`] for a name the
	/// [type documentation](Self) does not allow.
	pub fn new(name: impl Into<Cow<'static, CStr>>) -> Result<Self, MaterialNameError> {
		let name = name.into();

		check_name(&name)?;

		Ok(Self(name))
	}

	/// A stock material, whose name is checked when the constant is evaluated.
	const fn stock(name: &'static CStr) -> Self {
		match check_name(name) {
			Ok(()) => Self(Cow::Borrowed(name)),
			Err(_) => panic!("a stock overlay material name is invalid"),
		}
	}

	/// The material name, such as `effects/jarate_overlay`.
	pub fn as_cstr(&self) -> &CStr {
		&self.0
	}

	/// The material's `.vmt` file, relative to the game's search paths, such as
	/// `materials/effects/jarate_overlay.vmt`: the path to add a custom
	/// material to the downloads by, as the
	/// [module documentation](self#materials) describes.
	///
	/// ```
	/// use source_sdk_2013::Server;
	/// use source_sdk_2013::interfaces::network_string_tables::{AddDownloadableError, add_downloadable};
	/// use source_sdk_2013::tf2::overlays::OverlayMaterial;
	///
	/// assert_eq!(
	///     OverlayMaterial::JARATE.vmt_path().as_c_str(),
	///     c"materials/effects/jarate_overlay.vmt"
	/// );
	///
	/// /// Offers `example/overlay` to the clients that connect to this level.
	/// fn add_overlay(server: Server<'_>) -> Result<(), AddDownloadableError> {
	///     let material = OverlayMaterial::new(c"example/overlay").unwrap();
	///
	///     add_downloadable(server, &material.vmt_path())?;
	///     // The texture the `.vmt` names as its `$basetexture`.
	///     add_downloadable(server, c"materials/example/overlay.vtf")?;
	///     Ok(())
	/// }
	/// ```
	pub fn vmt_path(&self) -> CString {
		let name = self.as_cstr().to_bytes();
		let mut path =
			Vec::with_capacity(MATERIALS_DIRECTORY.len() + name.len() + EXTENSION.len() + 1);

		path.extend_from_slice(MATERIALS_DIRECTORY);
		path.extend_from_slice(name);
		path.extend_from_slice(EXTENSION);

		// SAFETY: A `CStr`'s bytes and both constants contain no NUL.
		unsafe { CString::from_vec_unchecked(path) }
	}
}

/// One TF2 player's screen overlay, within the current engine callback.
///
/// The [module documentation](self) describes how clients draw it and how
/// long it stays.
///
/// Methods fail with [`OverlayError::MarkedForDeletion`] for a player pending
/// deletion, [`OverlayError::UnsupportedMethod`] when the game lacks the
/// expected native method, and [`OverlayError::Rejected`] when the method's
/// binding reports failure.
#[doc(alias("m_szScriptOverlayMaterial"))]
#[derive(Debug, Clone, Copy)]
pub struct ScreenOverlay<'s> {
	player: Entity<'s>,
}

impl<'s> ScreenOverlay<'s> {
	/// Wraps `player`'s overlay. Fails with [`OverlayError::UnsupportedGame`]
	/// outside TF2, or [`OverlayError::NotAPlayer`] unless `player`'s data maps
	/// include `CBasePlayer`'s, as those of TF2's players and bots do.
	pub fn new(server: Server<'s>, player: Entity<'s>) -> Result<Self, OverlayError> {
		if server.game() != Game::TeamFortress2 {
			return Err(OverlayError::UnsupportedGame);
		}

		if !player
			.data_maps()
			.any(|map| map.class_name() == Some(c"CBasePlayer"))
		{
			return Err(OverlayError::NotAPlayer);
		}

		Ok(Self { player })
	}

	/// Refuses players marked for deletion.
	fn check_live(self) -> Result<(), OverlayError> {
		if self.player.is_marked_for_deletion() {
			Err(OverlayError::MarkedForDeletion)
		} else {
			Ok(())
		}
	}

	/// Removes the overlay, whoever set it, so the client stops drawing it.
	#[doc(alias("SetScriptOverlayMaterial"))]
	pub fn clear(self) -> Result<(), OverlayError> {
		self.set_name(c"")
	}

	/// Removes the overlay only if it is `expected`, and returns whether it
	/// did. Names are compared ignoring ASCII case, as TF2 compares its own
	/// condition overlays before clearing them (`FStrEq`).
	///
	/// Clear an overlay this way when the effect that set it ends, or before
	/// the plugin unloads, so as not to remove an overlay that a map script
	/// or another plugin has set since.
	pub fn clear_if(self, expected: &OverlayMaterial) -> Result<bool, OverlayError> {
		let matches = self.get()?.is_some_and(|current| {
			current
				.to_bytes()
				.eq_ignore_ascii_case(expected.as_cstr().to_bytes())
		});

		if matches {
			self.clear()?;
		}

		Ok(matches)
	}

	/// A copy of the overlay's material name, or `None` if there is no
	/// overlay. A map script or another plugin may have set a name that
	/// [`OverlayMaterial::new`] refuses.
	#[doc(alias("GetScriptOverlayMaterial"))]
	pub fn get(self) -> Result<Option<CString>, OverlayError> {
		self.check_live()?;

		// SAFETY: The checked native method is `CBasePlayer`'s inline
		// `GetScriptOverlayMaterial` (`game/server/player.h:1224`), which runs
		// no other code and returns the player's own
		// `m_Local.m_szScriptOverlayMaterial` array, allocated as long as the
		// player is. `call_string` copies it before any other game code runs.
		let name = unsafe {
			binding::call_string(
				self.player,
				c"CBasePlayer",
				c"GetScriptOverlayMaterial",
				&mut [],
			)
		}?;

		Ok(name.filter(|name| !name.is_empty()))
	}

	/// The player whose overlay this is.
	pub const fn player(self) -> Entity<'s> {
		self.player
	}

	/// Shows `material` over the player's screen in place of any overlay set
	/// before, until it is changed or cleared, or the player entity is
	/// recreated. A client that lacks the material draws the error material
	/// instead.
	#[doc(alias("SetScriptOverlayMaterial"))]
	pub fn set(self, material: &OverlayMaterial) -> Result<(), OverlayError> {
		self.set_name(material.as_cstr())
	}

	/// Sets the overlay to `name`, which is empty to clear it, or no longer
	/// than [`MAX_MATERIAL_LEN`] bytes.
	fn set_name(self, name: &CStr) -> Result<(), OverlayError> {
		self.check_live()?;

		// SAFETY: The checked native method is `CBasePlayer`'s inline
		// `SetScriptOverlayMaterial` (`game/server/player.h:1225-1234`), which
		// copies the name into the player's own
		// `m_Local.m_szScriptOverlayMaterial` through `GetForModify`, so that
		// the engine sends it. It keeps no pointer to the name, which outlives
		// the call, runs no script, and frees no entity.
		unsafe {
			binding::call(
				self.player,
				c"CBasePlayer",
				c"SetScriptOverlayMaterial",
				&mut [binding::string(name)],
				binding::VOID,
			)
		}?;

		Ok(())
	}
}

/// Checks a material name against [`OverlayMaterial`]'s rules.
const fn check_name(name: &CStr) -> Result<(), MaterialNameError> {
	let bytes = name.to_bytes();

	if bytes.is_empty() {
		return Err(MaterialNameError::Empty);
	}

	if bytes.len() > MAX_MATERIAL_LEN {
		return Err(MaterialNameError::TooLong { len: bytes.len() });
	}

	let mut index = 0;

	while index < bytes.len() {
		let byte = bytes[index];

		if !matches!(byte, b' '..=b'~') || matches!(byte, b'"' | b'\'' | b';' | b'\\') {
			return Err(MaterialNameError::InvalidByte { index, byte });
		}

		index += 1;
	}

	if let Some(directory) = bytes.first_chunk::<10>()
		&& directory.eq_ignore_ascii_case(MATERIALS_DIRECTORY)
	{
		return Err(MaterialNameError::MaterialsDirectory);
	}

	if let Some(extension) = bytes.last_chunk::<4>()
		&& extension.eq_ignore_ascii_case(EXTENSION)
	{
		return Err(MaterialNameError::Extension);
	}

	Ok(())
}
