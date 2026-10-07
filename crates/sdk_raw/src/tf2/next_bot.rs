//! Hand-written values of TF2's NextBot actors, whose interfaces (`INextBot`,
//! `ILocomotion`, `IBody` and `IVision`, in `game/server/NextBot`) the
//! generated bindings describe: the slot of the method that finds an entity's
//! `INextBot`.

use crate::vtable_slot;

/// The slot of `CBaseEntity::MyNextBotPointer`, which returns an entity's
/// `INextBot`, or null for an entity that is not a NextBot actor. TF2's
/// players and combat characters keep it in the same slot.
pub const MY_NEXT_BOT_POINTER_SLOT: usize = vtable_slot!(
	sys::CBaseEntity__bindgen_vtable,
	CBaseEntity_MyNextBotPointer
);

const _: () = assert!(
	MY_NEXT_BOT_POINTER_SLOT
		== vtable_slot!(
			sys::CBaseCombatCharacter__bindgen_vtable,
			CBaseCombatCharacter_MyNextBotPointer
		)
		&& MY_NEXT_BOT_POINTER_SLOT
			== vtable_slot!(sys::CTFPlayer__bindgen_vtable, CTFPlayer_MyNextBotPointer)
);
