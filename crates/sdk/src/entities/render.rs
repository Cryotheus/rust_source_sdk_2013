//! How clients draw an entity: its effects (`m_fEffects`), render mode and
//! color, model, skin and body groups, and fade scale.
//!
//! These are networked variables, which clients draw the entity from, and
//! the setters here record their changes for networking, as the game's own
//! do. [`Entity::set_effects`] also updates the entity's transmit state, as
//! drawing an entity with [`Effects::NO_DRAW`] stops it being sent.

#[cfg(test)]
#[path = "../tests/entities/render.rs"]
mod tests;

use crate::entities::Entity;
use crate::entities::fields::{BaseField, FieldError, PooledString};
use crate::interfaces::{ModelInfo, ValveEngine};
use crate::math::Color32;
use sdk_raw::edicts::FL_EDICT_DIRTY_PVS_INFORMATION;

use sdk_raw::entities::{
	EF_BONEMERGE, EF_BONEMERGE_FASTCULL, EF_BRIGHTLIGHT, EF_DIMLIGHT, EF_ITEM_BLINK, EF_NODRAW,
	EF_NOINTERP, EF_NORECEIVESHADOW, EF_NOSHADOW, EF_PARENT_ANIMATES,
};

use sdk_raw::util::cstr::borrow_cstr;
use std::ffi::{CStr, CString, c_int};

/// `m_fEffects`, the `effects` key value.
static EFFECTS: BaseField<c_int> = BaseField::new(c"m_fEffects", sys::_fieldtypes_FIELD_INTEGER);

/// `m_nModelIndex`, the `modelindex` key value.
static MODEL_INDEX: BaseField<i16> = BaseField::new(c"m_nModelIndex", sys::_fieldtypes_FIELD_SHORT);

/// `m_ModelName`, the `model` key value.
static MODEL_NAME: BaseField<PooledString> =
	BaseField::new(c"m_ModelName", sys::_fieldtypes_FIELD_MODELNAME);

/// `m_clrRender`, the `rendercolor` key value.
static RENDER_COLOR: BaseField<Color32> =
	BaseField::new(c"m_clrRender", sys::_fieldtypes_FIELD_COLOR32);

/// `m_nRenderMode`, the `rendermode` key value.
static RENDER_MODE: BaseField<u8> =
	BaseField::new(c"m_nRenderMode", sys::_fieldtypes_FIELD_CHARACTER);

/// `m_nTransmitStateOwnedCounter`, how many entities own the entity's
/// transmit state.
static TRANSMIT_STATE_OWNERS: BaseField<u8> = BaseField::new(
	c"m_nTransmitStateOwnedCounter",
	sys::_fieldtypes_FIELD_CHARACTER,
);

bitflags::bitflags! {
	/// An entity's effects (`m_fEffects`), the `EF_*` values from
	/// `public/const.h`, which clients draw it with.
	///
	/// Effects read from an entity keep every bit, including those without a
	/// constant here, such as those a game defines.
	#[doc(alias("m_fEffects"))]
	#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
	pub struct Effects: c_int {
		/// `EF_BONEMERGE`: the entity is drawn with its move parent's bones,
		/// as worn items are.
		#[doc(alias("EF_BONEMERGE"))]
		const BONE_MERGE = EF_BONEMERGE;

		/// `EF_BONEMERGE_FASTCULL`: a bone-merged entity is culled with its
		/// parent, rather than by its own bounds.
		#[doc(alias("EF_BONEMERGE_FASTCULL"))]
		const BONE_MERGE_FAST_CULL = EF_BONEMERGE_FASTCULL;

		/// `EF_BRIGHTLIGHT`: the entity casts a bright light around it.
		#[doc(alias("EF_BRIGHTLIGHT"))]
		const BRIGHT_LIGHT = EF_BRIGHTLIGHT;

		/// `EF_DIMLIGHT`: the entity casts a dim light around it.
		#[doc(alias("EF_DIMLIGHT"))]
		const DIM_LIGHT = EF_DIMLIGHT;

		/// `EF_ITEM_BLINK`: the entity blinks, as items about to respawn do.
		#[doc(alias("EF_ITEM_BLINK"))]
		const ITEM_BLINK = EF_ITEM_BLINK;

		/// `EF_NODRAW`: the entity is not drawn, and is not sent to clients
		/// unless entities move with it.
		#[doc(alias("EF_NODRAW"))]
		const NO_DRAW = EF_NODRAW;

		/// `EF_NOINTERP`: clients do not interpolate the entity's movement,
		/// as after a teleport.
		#[doc(alias("EF_NOINTERP"))]
		const NO_INTERP = EF_NOINTERP;

		/// `EF_NORECEIVESHADOW`: other shadows are not drawn on the entity.
		#[doc(alias("EF_NORECEIVESHADOW"))]
		const NO_RECEIVE_SHADOW = EF_NORECEIVESHADOW;

		/// `EF_NOSHADOW`: the entity casts no shadow.
		#[doc(alias("EF_NOSHADOW"))]
		const NO_SHADOW = EF_NOSHADOW;

		/// `EF_PARENT_ANIMATES`: the entity's move parent animates, so its
		/// own position changes as it does.
		#[doc(alias("EF_PARENT_ANIMATES"))]
		const PARENT_ANIMATES = EF_PARENT_ANIMATES;

		// Bits without a constant here, which games may set.
		const _ = !0;
	}
}

/// How clients blend an entity's model with what is behind it
/// (`RenderMode_t`).
#[doc(alias("RenderMode_t", "m_nRenderMode"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RenderMode {
	/// `kRenderNormal`: drawn opaque, ignoring the render color's alpha.
	#[doc(alias("kRenderNormal"))]
	Normal,

	/// `kRenderTransColor`: drawn translucent, with the render color's alpha.
	#[doc(alias("kRenderTransColor"))]
	TransColor,

	/// `kRenderTransTexture`: drawn translucent, with the alpha of the
	/// render color and the texture.
	#[doc(alias("kRenderTransTexture"))]
	TransTexture,

	/// `kRenderGlow`: drawn additively as a glow of a fixed size on screen,
	/// unoccluded.
	#[doc(alias("kRenderGlow"))]
	Glow,

	/// `kRenderTransAlpha`: drawn translucent, with the texture's alpha.
	#[doc(alias("kRenderTransAlpha"))]
	TransAlpha,

	/// `kRenderTransAdd`: drawn additively.
	#[doc(alias("kRenderTransAdd"))]
	TransAdd,

	/// `kRenderEnvironmental`: not drawn, as for environmental effects.
	#[doc(alias("kRenderEnvironmental"))]
	Environmental,

	/// `kRenderTransAddFrameBlend`: drawn additively, blending the frames
	/// of an animated texture.
	#[doc(alias("kRenderTransAddFrameBlend"))]
	TransAddFrameBlend,

	/// `kRenderTransAlphaAdd`: drawn additively, with the texture's alpha.
	#[doc(alias("kRenderTransAlphaAdd"))]
	TransAlphaAdd,

	/// `kRenderWorldGlow`: drawn as [`Glow`](Self::Glow) is, but at a size
	/// that shrinks with distance.
	#[doc(alias("kRenderWorldGlow"))]
	WorldGlow,

	/// `kRenderNone`: not drawn at all, though still sent to clients.
	#[doc(alias("kRenderNone"))]
	None,
}

/// Why [`Entity::set_model`] set no model.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SetModelError {
	/// The server has not precached the model, or it is a dynamic model.
	#[error("the model {model:?} is not precached")]
	NotPrecached {
		/// The model asked for.
		model: CString,
	},
}

impl RenderMode {
	/// The mode a `RenderMode_t` value stands for, or `None` for a value
	/// past `kRenderNone`.
	pub const fn from_raw(value: u8) -> Option<Self> {
		Some(match value as sys::RenderMode_t {
			sys::RenderMode_t_kRenderNormal => Self::Normal,
			sys::RenderMode_t_kRenderTransColor => Self::TransColor,
			sys::RenderMode_t_kRenderTransTexture => Self::TransTexture,
			sys::RenderMode_t_kRenderGlow => Self::Glow,
			sys::RenderMode_t_kRenderTransAlpha => Self::TransAlpha,
			sys::RenderMode_t_kRenderTransAdd => Self::TransAdd,
			sys::RenderMode_t_kRenderEnvironmental => Self::Environmental,
			sys::RenderMode_t_kRenderTransAddFrameBlend => Self::TransAddFrameBlend,
			sys::RenderMode_t_kRenderTransAlphaAdd => Self::TransAlphaAdd,
			sys::RenderMode_t_kRenderWorldGlow => Self::WorldGlow,
			sys::RenderMode_t_kRenderNone => Self::None,
			_ => return None,
		})
	}

	/// The mode's `RenderMode_t` value.
	pub const fn to_raw(self) -> u8 {
		(match self {
			Self::Normal => sys::RenderMode_t_kRenderNormal,
			Self::TransColor => sys::RenderMode_t_kRenderTransColor,
			Self::TransTexture => sys::RenderMode_t_kRenderTransTexture,
			Self::Glow => sys::RenderMode_t_kRenderGlow,
			Self::TransAlpha => sys::RenderMode_t_kRenderTransAlpha,
			Self::TransAdd => sys::RenderMode_t_kRenderTransAdd,
			Self::Environmental => sys::RenderMode_t_kRenderEnvironmental,
			Self::TransAddFrameBlend => sys::RenderMode_t_kRenderTransAddFrameBlend,
			Self::TransAlphaAdd => sys::RenderMode_t_kRenderTransAlphaAdd,
			Self::WorldGlow => sys::RenderMode_t_kRenderWorldGlow,
			Self::None => sys::RenderMode_t_kRenderNone,
		}) as u8
	}
}

impl<'s> Entity<'s> {
	/// The entity's body groups (`m_nBody`), the `body` key value, which
	/// pick among the parts of its model, as the hats of TF2's classes.
	/// Fails for an entity without a model to animate, which no
	/// `CBaseAnimating` datamap describes.
	#[doc(alias("GetBody", "m_nBody"))]
	pub fn body(self) -> Result<c_int, FieldError> {
		self.data_field(c"m_nBody")
	}

	/// The entity's effects (`m_fEffects`).
	#[doc(alias("GetEffects", "IsEffectActive", "m_fEffects"))]
	pub fn effects(self) -> Result<Effects, FieldError> {
		EFFECTS.read(self).map(Effects::from_bits_retain)
	}

	/// The scale of the distance at which clients fade the entity out
	/// (`m_flFadeScale`), the `fadescale` key value. Fails as
	/// [`Self::body`] does.
	#[doc(alias("m_flFadeScale"))]
	pub fn fade_scale(self) -> Result<f32, FieldError> {
		self.data_field(c"m_flFadeScale")
	}

	/// The index of the entity's model in the server's model precache table
	/// (`m_nModelIndex`), or 0 for none.
	#[doc(alias("GetModelIndex", "m_nModelIndex"))]
	pub fn model_index(self) -> Result<c_int, FieldError> {
		MODEL_INDEX.read(self).map(c_int::from)
	}

	/// The name of the entity's model (`m_ModelName`), such as
	/// `models/props_gameplay/resupply_locker.mdl`, or `*1` for the map's
	/// first brush model, or `None` if it has none.
	#[doc(alias("GetModelName", "m_ModelName"))]
	pub fn model_name(self) -> Result<Option<&'s CStr>, FieldError> {
		let name = MODEL_NAME.read(self)?;

		// SAFETY: Model names are pooled strings, which live until the level
		// ends, past `'s`.
		Ok(unsafe { borrow_cstr(name.0) }.filter(|name| !name.is_empty()))
	}

	/// The color clients tint the entity with, and the opacity some
	/// [render modes](RenderMode) draw it with (`m_clrRender`).
	#[doc(alias("GetRenderColor", "GetRenderAlpha", "m_clrRender"))]
	pub fn render_color(self) -> Result<Color32, FieldError> {
		RENDER_COLOR.read(self)
	}

	/// How clients blend the entity with what is behind it
	/// (`m_nRenderMode`), or `None` for a mode the SDK does not know.
	#[doc(alias("GetRenderMode", "m_nRenderMode"))]
	pub fn render_mode(self) -> Result<Option<RenderMode>, FieldError> {
		RENDER_MODE.read(self).map(RenderMode::from_raw)
	}

	/// Sets the entity's body groups (`m_nBody`), as
	/// `CBaseAnimating::SetBodygroup` does once it computed them. Fails as
	/// [`Self::body`] does.
	#[doc(alias("SetBodygroup", "m_nBody"))]
	pub fn set_body(self, engine: ValveEngine<'_>, body: c_int) -> Result<(), FieldError> {
		self.set_data_field(engine, c"m_nBody", body)
	}

	/// Sets the entity's effects (`m_fEffects`), as `CBaseEntity::SetEffects`
	/// does: when they change, the change is recorded for networking, and the
	/// entity updates its transmit state, as
	/// [`Self::update_transmit_state`] does, which [`Effects::NO_DRAW`]
	/// decides. Clearing [`Effects::NO_DRAW`] also has the engine find
	/// which areas the entity is in anew, as `RemoveEffects` does.
	#[doc(alias("SetEffects", "AddEffects", "RemoveEffects", "m_fEffects"))]
	pub fn set_effects(self, engine: ValveEngine<'_>, effects: Effects) -> Result<(), FieldError> {
		let old = self.effects()?;

		if old == effects {
			return Ok(());
		}

		EFFECTS.write(engine, self, effects.bits())?;

		if old.contains(Effects::NO_DRAW)
			&& !effects.contains(Effects::NO_DRAW)
			&& let Some(edict) = self.edict()
		{
			// SAFETY: The edict is the live entity's, whose flags the engine
			// reads on the main thread, as `MarkPVSInformationDirty` writes them.
			unsafe {
				let flags = &raw mut (*edict.as_ptr())._base.m_fStateFlags;

				flags.write(flags.read() | FL_EDICT_DIRTY_PVS_INFORMATION);
			}
		}

		self.update_transmit_state()
	}

	/// Sets the scale of the distance at which clients fade the entity out
	/// (`m_flFadeScale`). Fails as [`Self::body`] does.
	#[doc(alias("m_flFadeScale"))]
	pub fn set_fade_scale(self, engine: ValveEngine<'_>, scale: f32) -> Result<(), FieldError> {
		self.set_data_field(engine, c"m_flFadeScale", scale)
	}

	/// Gives the entity a precached model, such as `models/props_farm/box.mdl`,
	/// with its index, and the collision bounds it gives, through the entity's
	/// `SetModel`, which classes extend: `CBaseAnimating`'s also resets the
	/// entity's animation state, for instance.
	///
	/// Fails with [`SetModelError::NotPrecached`] if the server has not
	/// precached the model, for which the game's own `SetModel` would stop the
	/// server with an error, or if it is a dynamic model, which this does not
	/// set.
	#[doc(alias("SetModel", "SetEntityModel"))]
	pub fn set_model(self, models: ModelInfo<'_>, model: &CStr) -> Result<(), SetModelError> {
		if models.model_index(model).is_none() {
			return Err(SetModelError::NotPrecached {
				model: model.to_owned(),
			});
		}

		// SAFETY: The entity is live on the main thread, and the model is
		// precached, as `UTIL_SetModel` requires.
		unsafe { sdk_raw::entities::spawn::set_model(self.as_ptr(), model.as_ptr()) };
		Ok(())
	}

	/// Sets the color clients tint the entity with (`m_clrRender`), as
	/// `CBaseEntity::SetRenderColor` and `SetRenderColorA` do.
	#[doc(alias("SetRenderColor", "SetRenderColorA", "m_clrRender"))]
	pub fn set_render_color(
		self,
		engine: ValveEngine<'_>,
		color: Color32,
	) -> Result<(), FieldError> {
		RENDER_COLOR.write(engine, self, color)
	}

	/// Sets how clients blend the entity with what is behind it
	/// (`m_nRenderMode`), as `CBaseEntity::SetRenderMode` does.
	#[doc(alias("SetRenderMode", "m_nRenderMode"))]
	pub fn set_render_mode(
		self,
		engine: ValveEngine<'_>,
		mode: RenderMode,
	) -> Result<(), FieldError> {
		RENDER_MODE.write(engine, self, mode.to_raw())
	}

	/// Sets the entity's skin (`m_nSkin`), as the `skin` input does. Fails as
	/// [`Self::body`] does.
	#[doc(alias("m_nSkin"))]
	pub fn set_skin(self, engine: ValveEngine<'_>, skin: c_int) -> Result<(), FieldError> {
		self.set_data_field(engine, c"m_nSkin", skin)
	}

	/// The entity's skin (`m_nSkin`), the `skin` key value, which picks
	/// among its model's textures, as TF2 does for teams. Fails as
	/// [`Self::body`] does.
	#[doc(alias("GetSkin", "m_nSkin"))]
	pub fn skin(self) -> Result<c_int, FieldError> {
		self.data_field(c"m_nSkin")
	}

	/// Has the entity give its edict the transmit state its own state calls
	/// for, as `CBaseEntity::DispatchUpdateTransmitState` does after its
	/// effects or parent change.
	///
	/// Nothing happens while another entity owns the entity's transmit state
	/// (`m_nTransmitStateOwnedCounter`), as a character owns its weapons'.
	#[doc(alias("DispatchUpdateTransmitState", "UpdateTransmitState"))]
	pub fn update_transmit_state(self) -> Result<(), FieldError> {
		if TRANSMIT_STATE_OWNERS.read(self)? != 0 {
			return Ok(());
		}

		// SAFETY: The entity is live on the main thread, and no other entity
		// owns its transmit state.
		unsafe { sdk_raw::transmit::update_transmit_state(self.as_ptr()) };
		Ok(())
	}
}
