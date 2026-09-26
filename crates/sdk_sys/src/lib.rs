//! Agabaraya.

use std::mem::offset_of;

const SLOT_SIZE: usize = size_of::<*const ()>();

cfg_select! {
	all(target_os = "linux", target_arch = "x86", target_env = "gnu") => {
		compile_error!("Not yet supported");
	}

	all(target_os = "linux", target_arch = "x86_64", target_env = "gnu") => {
		mod linux_64;

		pub use linux_64::*;

		/// `CBaseEntity::GetDataDescMap` in the Source SDK 2013 primary vtable.
		/// Derived from `game/server/cbase.h` with the Itanium ABI model.
		pub const CBASEENTITY_DATAMAP_VTABLE_SLOT: usize = 12;
		/// `CBaseEntity::Teleport` in the Source SDK 2013 primary vtable.
		/// Derived from `game/server/cbase.h` with the Itanium ABI model.
		pub const CBASEENTITY_TELEPORT_VTABLE_SLOT: usize = 111;
		/// `CBaseEntity::Teleport` in TF2's game DLL.
		/// Verified against SourceMod's `sdktools.games/game.tf.txt` gamedata.
		pub const CBASEENTITY_TF2_TELEPORT_VTABLE_SLOT: usize = 115;
		/// `CBaseEntity::AcceptInput` in the primary vtable. TF2 declares its own
		/// virtual methods after it, so the slot is the same for every game.
		/// Derived from `game/server/baseentity.h` with the Itanium ABI model, and
		/// verified against SourceMod's `sdktools.games/game.tf.txt` gamedata.
		pub const CBASEENTITY_ACCEPTINPUT_VTABLE_SLOT: usize = 39;

		const _: () = {
			assert!(
				offset_of!(CBaseEntity__bindgen_vtable, CBaseEntity_GetDataDescMap)
					== SLOT_SIZE * CBASEENTITY_DATAMAP_VTABLE_SLOT
			);
			assert!(
				offset_of!(CBaseEntity__bindgen_vtable, CBaseEntity_AcceptInput)
					== SLOT_SIZE * CBASEENTITY_ACCEPTINPUT_VTABLE_SLOT
			);
			assert!(
				offset_of!(CBaseEntity__bindgen_vtable, CBaseEntity_Teleport)
					== SLOT_SIZE * CBASEENTITY_TELEPORT_VTABLE_SLOT
			);

			// `AcceptInput` takes a `variant_t` by value. Its handle member has a
			// user-provided copy constructor, so both ABIs pass a pointer to a copy
			// the caller owns, which the generated signature takes as `*mut variant_t`.
			assert!(size_of::<variant_t>() == 24 && align_of::<variant_t>() == 8);
			assert!(offset_of!(variant_t, eVal) == 16);
			assert!(offset_of!(variant_t, fieldType) == 20);
			assert!(size_of::<string_t>() == SLOT_SIZE);

			assert!(
				offset_of!(
					IGameEventVisitor2__bindgen_vtable,
					IGameEventVisitor2_VisitLocal
				) == 0
			);
			assert!(
				offset_of!(
					IGameEventVisitor2__bindgen_vtable,
					IGameEventVisitor2_VisitBool
				) == SLOT_SIZE * 6
			);
			assert!(size_of::<IGameEventVisitor2__bindgen_vtable>() == SLOT_SIZE * 7);

			assert!(offset_of!(IGameEvent__bindgen_vtable, IGameEvent_complete_destructor) == 0);
			assert!(
				offset_of!(IGameEvent__bindgen_vtable, IGameEvent_deleting_destructor) == SLOT_SIZE
			);
			assert!(
				offset_of!(IGameEvent__bindgen_vtable, IGameEvent_ForEventData) == SLOT_SIZE * 21
			);
			assert!(size_of::<IGameEvent__bindgen_vtable>() == SLOT_SIZE * 22);

			assert!(
				offset_of!(
					IGameEventListener2__bindgen_vtable,
					IGameEventListener2_complete_destructor
				) == 0
			);
			assert!(
				offset_of!(
					IGameEventListener2__bindgen_vtable,
					IGameEventListener2_deleting_destructor
				) == SLOT_SIZE
			);
			assert!(
				offset_of!(
					IGameEventListener2__bindgen_vtable,
					IGameEventListener2_FireGameEvent
				) == SLOT_SIZE * 2
			);
			assert!(size_of::<IGameEventListener2__bindgen_vtable>() == SLOT_SIZE * 3);

			// The Itanium ABI places the complete-object destructor first and the
			// deleting destructor second, followed by the declared virtual methods.
			assert!(
				offset_of!(
					IGameEventManager2__bindgen_vtable,
					IGameEventManager2_complete_destructor
				) == 0
			);
			assert!(
				offset_of!(
					IGameEventManager2__bindgen_vtable,
					IGameEventManager2_deleting_destructor
				) == SLOT_SIZE
			);
			assert!(
				offset_of!(
					IGameEventManager2__bindgen_vtable,
					IGameEventManager2_LoadEventsFromFile
				) == SLOT_SIZE * 2
			);
			assert!(
				offset_of!(
					IGameEventManager2__bindgen_vtable,
					IGameEventManager2_AddListener
				) == SLOT_SIZE * 4
			);
			assert!(
				offset_of!(
					IGameEventManager2__bindgen_vtable,
					IGameEventManager2_RemoveListener
				) == SLOT_SIZE * 6
			);
			assert!(
				offset_of!(
					IGameEventManager2__bindgen_vtable,
					IGameEventManager2_UnserializeEvent
				) == SLOT_SIZE * 13
			);
			assert!(size_of::<IGameEventManager2__bindgen_vtable>() == SLOT_SIZE * 14);

			//no virtual destructor
			assert!(
				offset_of!(IVEngineServer__bindgen_vtable, IVEngineServer_ServerCommand)
					== SLOT_SIZE * 36
			);

			assert!(size_of::<IServerTools>() == SLOT_SIZE);
			assert!(
				offset_of!(
					IServerTools__bindgen_vtable,
					IServerTools_complete_destructor
				) == 0
			);
			assert!(
				offset_of!(
					IServerTools__bindgen_vtable,
					IServerTools_deleting_destructor
				) == SLOT_SIZE
			);
			assert!(
				offset_of!(IServerTools__bindgen_vtable, IServerTools_FirstEntity) == SLOT_SIZE * 8
			);
			assert!(
				offset_of!(IServerTools__bindgen_vtable, IServerTools_NextEntity) == SLOT_SIZE * 9
			);
			assert!(
				offset_of!(IServerTools__bindgen_vtable, IServerTools_GetKeyValue)
					== SLOT_SIZE * 11
			);
			assert!(
				offset_of!(
					IServerTools__bindgen_vtable,
					IServerTools_FindEntityProcedural
				) == SLOT_SIZE * 52
			);
			assert!(size_of::<IServerTools__bindgen_vtable>() == SLOT_SIZE * 53);

			// The Itanium ABI keeps overloads in declaration order.
			assert!(
				offset_of!(IServerTools__bindgen_vtable, IServerTools_SetKeyValue)
					== SLOT_SIZE * 12
			);
			assert!(
				offset_of!(IServerTools__bindgen_vtable, IServerTools_SetKeyValue2)
					== SLOT_SIZE * 14
			);

			//no virtual destructor
			assert!(
				offset_of!(
					IServerGameDLL__bindgen_vtable,
					IServerGameDLL_GetAllServerClasses
				) == SLOT_SIZE * 11
			);
			assert!(size_of::<IServerGameDLL__bindgen_vtable>() == SLOT_SIZE * 48);

			// Per-client channels, as `GetPlayerNetInfo` returns them. Each slot after
			// the destructor is one later than in TF2's engine.dll, which is MSVC-built.
			assert!(
				offset_of!(INetChannel__bindgen_vtable, INetChannel_complete_destructor)
					== SLOT_SIZE * 26
			);
			assert!(
				offset_of!(INetChannel__bindgen_vtable, INetChannel_deleting_destructor)
					== SLOT_SIZE * 27
			);
			assert!(
				offset_of!(INetChannel__bindgen_vtable, INetChannel_SendNetMsg) == SLOT_SIZE * 38
			);
			assert!(
				offset_of!(INetChannel__bindgen_vtable, INetChannel_SendData) == SLOT_SIZE * 39
			);
			assert!(
				offset_of!(INetChannel__bindgen_vtable, INetChannel_GetSequenceData)
					== SLOT_SIZE * 51
			);
			assert!(
				offset_of!(INetChannel__bindgen_vtable, INetChannel_IsOverflowed) == SLOT_SIZE * 55
			);
			assert!(
				offset_of!(
					INetChannel__bindgen_vtable,
					INetChannel_HasPendingReliableData
				) == SLOT_SIZE * 57
			);
			assert!(
				offset_of!(INetChannel__bindgen_vtable, INetChannel_GetNumBitsWritten)
					== SLOT_SIZE * 63
			);
			assert!(size_of::<INetChannel__bindgen_vtable>() == SLOT_SIZE * 69);

			assert!(offset_of!(INetMessage__bindgen_vtable, INetMessage_complete_destructor) == 0);
			assert!(
				offset_of!(INetMessage__bindgen_vtable, INetMessage_deleting_destructor)
					== SLOT_SIZE
			);
			assert!(offset_of!(INetMessage__bindgen_vtable, INetMessage_Process) == SLOT_SIZE * 4);
			assert!(
				offset_of!(INetMessage__bindgen_vtable, INetMessage_WriteToBuffer) == SLOT_SIZE * 6
			);
			assert!(offset_of!(INetMessage__bindgen_vtable, INetMessage_GetType) == SLOT_SIZE * 8);
			assert!(offset_of!(INetMessage__bindgen_vtable, INetMessage_GetName) == SLOT_SIZE * 10);
			assert!(
				offset_of!(INetMessage__bindgen_vtable, INetMessage_ToString) == SLOT_SIZE * 12
			);
			assert!(offset_of!(INetMessage__bindgen_vtable, INetMessage_GetSize) == SLOT_SIZE * 14);
			assert!(size_of::<INetMessage__bindgen_vtable>() == SLOT_SIZE * 15);

			// `REPLAY_ENABLED` adds `ProcessSaveReplay`, as in TF2's engine, whose
			// `CGameClient` handler vtable has 15 slots under MSVC and 16 here.
			assert!(
				offset_of!(
					IClientMessageHandler__bindgen_vtable,
					IClientMessageHandler_complete_destructor
				) == 0
			);
			assert!(
				offset_of!(
					IClientMessageHandler__bindgen_vtable,
					IClientMessageHandler_deleting_destructor
				) == SLOT_SIZE
			);
			assert!(
				offset_of!(
					IClientMessageHandler__bindgen_vtable,
					IClientMessageHandler_ProcessTick
				) == SLOT_SIZE * 2
			);
			assert!(
				offset_of!(
					IClientMessageHandler__bindgen_vtable,
					IClientMessageHandler_ProcessVoiceData
				) == SLOT_SIZE * 8
			);
			assert!(
				offset_of!(
					IClientMessageHandler__bindgen_vtable,
					IClientMessageHandler_ProcessSaveReplay
				) == SLOT_SIZE * 14
			);
			assert!(
				offset_of!(
					IClientMessageHandler__bindgen_vtable,
					IClientMessageHandler_ProcessCmdKeyValues
				) == SLOT_SIZE * 15
			);
			assert!(size_of::<IClientMessageHandler__bindgen_vtable>() == SLOT_SIZE * 16);

			assert!(offset_of!(IServer__bindgen_vtable, IServer_complete_destructor) == 0);
			assert!(offset_of!(IServer__bindgen_vtable, IServer_deleting_destructor) == SLOT_SIZE);
			assert!(offset_of!(IServer__bindgen_vtable, IServer_GetClient) == SLOT_SIZE * 7);
			assert!(size_of::<IServer__bindgen_vtable>() == SLOT_SIZE * 37);

			assert!(offset_of!(IClient__bindgen_vtable, IClient_complete_destructor) == 0);
			assert!(offset_of!(IClient__bindgen_vtable, IClient_deleting_destructor) == SLOT_SIZE);
			assert!(offset_of!(IClient__bindgen_vtable, IClient_GetPlayerSlot) == SLOT_SIZE * 15);
			assert!(offset_of!(IClient__bindgen_vtable, IClient_GetNetChannel) == SLOT_SIZE * 19);
			assert!(offset_of!(IClient__bindgen_vtable, IClient_IsReplay) == SLOT_SIZE * 37);
			assert!(size_of::<IClient__bindgen_vtable>() == SLOT_SIZE * 41);

			assert!(
				offset_of!(
					IRecipientFilter__bindgen_vtable,
					IRecipientFilter_complete_destructor
				) == 0
			);
			assert!(
				offset_of!(
					IRecipientFilter__bindgen_vtable,
					IRecipientFilter_deleting_destructor
				) == SLOT_SIZE
			);
			assert!(
				offset_of!(
					IRecipientFilter__bindgen_vtable,
					IRecipientFilter_IsReliable
				) == SLOT_SIZE * 2
			);
			assert!(
				offset_of!(
					IRecipientFilter__bindgen_vtable,
					IRecipientFilter_GetRecipientIndex
				) == SLOT_SIZE * 5
			);
			assert!(size_of::<IRecipientFilter__bindgen_vtable>() == SLOT_SIZE * 6);

			//no virtual destructor, so the slots are the same under both ABIs
			assert!(
				offset_of!(
					IVEngineServer__bindgen_vtable,
					IVEngineServer_GetPlayerNetInfo
				) == SLOT_SIZE * 20
			);
			assert!(
				offset_of!(
					IVEngineServer__bindgen_vtable,
					IVEngineServer_EntityMessageBegin
				) == SLOT_SIZE * 42
			);
			assert!(
				offset_of!(
					IVEngineServer__bindgen_vtable,
					IVEngineServer_UserMessageBegin
				) == SLOT_SIZE * 43
			);
			assert!(
				offset_of!(IVEngineServer__bindgen_vtable, IVEngineServer_MessageEnd)
					== SLOT_SIZE * 44
			);
			assert!(
				offset_of!(IVEngineServer__bindgen_vtable, IVEngineServer_SetView)
					== SLOT_SIZE * 48
			);
			assert!(
				offset_of!(
					IVEngineServer__bindgen_vtable,
					IVEngineServer_CrosshairAngle
				) == SLOT_SIZE * 50
			);
			assert!(
				offset_of!(
					IVEngineServer__bindgen_vtable,
					IVEngineServer_GetClientConVarValue
				) == SLOT_SIZE * 55
			);
			assert!(
				offset_of!(IVEngineServer__bindgen_vtable, IVEngineServer_GetIServer)
					== SLOT_SIZE * 119
			);
		};
	}

	all(target_os = "windows", target_arch = "x86", target_env = "msvc") => {
		compile_error!("Not yet supported");
	}

	all(target_os = "windows", target_arch = "x86_64", target_env = "msvc") => {
		mod windows_64;

		pub use windows_64::*;

		/// `CBaseEntity::GetDataDescMap` in the Source SDK 2013 primary vtable.
		/// Derived from `game/server/cbase.h` with the MSVC ABI model.
		pub const CBASEENTITY_DATAMAP_VTABLE_SLOT: usize = 11;
		/// `CBaseEntity::Teleport` in the Source SDK 2013 primary vtable.
		/// Derived from `game/server/cbase.h` with the MSVC ABI model.
		pub const CBASEENTITY_TELEPORT_VTABLE_SLOT: usize = 110;
		/// `CBaseEntity::Teleport` in TF2's game DLL.
		/// Verified against SourceMod's `sdktools.games/game.tf.txt` gamedata.
		pub const CBASEENTITY_TF2_TELEPORT_VTABLE_SLOT: usize = 114;
		/// `CBaseEntity::AcceptInput` in the primary vtable. TF2 declares its own
		/// virtual methods after it, so the slot is the same for every game.
		/// Derived from `game/server/baseentity.h` with the MSVC ABI model, and
		/// verified against SourceMod's `sdktools.games/game.tf.txt` gamedata.
		pub const CBASEENTITY_ACCEPTINPUT_VTABLE_SLOT: usize = 38;

		const _: () = {
			assert!(
				offset_of!(CBaseEntity__bindgen_vtable, CBaseEntity_GetDataDescMap)
					== SLOT_SIZE * CBASEENTITY_DATAMAP_VTABLE_SLOT
			);
			assert!(
				offset_of!(CBaseEntity__bindgen_vtable, CBaseEntity_AcceptInput)
					== SLOT_SIZE * CBASEENTITY_ACCEPTINPUT_VTABLE_SLOT
			);
			assert!(
				offset_of!(CBaseEntity__bindgen_vtable, CBaseEntity_Teleport)
					== SLOT_SIZE * CBASEENTITY_TELEPORT_VTABLE_SLOT
			);

			// `AcceptInput` takes a `variant_t` by value. Its handle member has a
			// user-provided copy constructor, so both ABIs pass a pointer to a copy
			// the caller owns, which the generated signature takes as `*mut variant_t`.
			assert!(size_of::<variant_t>() == 24 && align_of::<variant_t>() == 8);
			assert!(offset_of!(variant_t, eVal) == 16);
			assert!(offset_of!(variant_t, fieldType) == 20);
			assert!(size_of::<string_t>() == SLOT_SIZE);

			assert!(
				offset_of!(
					IGameEventVisitor2__bindgen_vtable,
					IGameEventVisitor2_VisitLocal
				) == 0
			);
			assert!(
				offset_of!(
					IGameEventVisitor2__bindgen_vtable,
					IGameEventVisitor2_VisitBool
				) == SLOT_SIZE * 6
			);
			assert!(size_of::<IGameEventVisitor2__bindgen_vtable>() == SLOT_SIZE * 7);

			assert!(offset_of!(IGameEvent__bindgen_vtable, IGameEvent_destructor) == 0);
			assert!(
				offset_of!(IGameEvent__bindgen_vtable, IGameEvent_ForEventData) == SLOT_SIZE * 20
			);
			assert!(size_of::<IGameEvent__bindgen_vtable>() == SLOT_SIZE * 21);

			assert!(
				offset_of!(
					IGameEventListener2__bindgen_vtable,
					IGameEventListener2_destructor
				) == 0
			);
			assert!(
				offset_of!(
					IGameEventListener2__bindgen_vtable,
					IGameEventListener2_FireGameEvent
				) == SLOT_SIZE
			);
			assert!(size_of::<IGameEventListener2__bindgen_vtable>() == SLOT_SIZE * 2);

			// IGameEventManager2 overrides IBaseInterface's virtual destructor,
			// followed by the twelve methods declared in igameevents.h.
			assert!(
				offset_of!(
					IGameEventManager2__bindgen_vtable,
					IGameEventManager2_destructor
				) == 0
			);
			assert!(
				offset_of!(
					IGameEventManager2__bindgen_vtable,
					IGameEventManager2_LoadEventsFromFile
				) == SLOT_SIZE
			);
			assert!(
				offset_of!(
					IGameEventManager2__bindgen_vtable,
					IGameEventManager2_AddListener
				) == SLOT_SIZE * 3
			);
			assert!(
				offset_of!(
					IGameEventManager2__bindgen_vtable,
					IGameEventManager2_RemoveListener
				) == SLOT_SIZE * 5
			);
			assert!(
				offset_of!(
					IGameEventManager2__bindgen_vtable,
					IGameEventManager2_UnserializeEvent
				) == SLOT_SIZE * 12
			);
			assert!(size_of::<IGameEventManager2__bindgen_vtable>() == SLOT_SIZE * 13);

			//no virtual destructor
			assert!(
				offset_of!(IVEngineServer__bindgen_vtable, IVEngineServer_ServerCommand)
					== SLOT_SIZE * 36
			);

			assert!(size_of::<IServerTools>() == SLOT_SIZE);
			assert!(offset_of!(IServerTools__bindgen_vtable, IServerTools_destructor) == 0);
			assert!(
				offset_of!(IServerTools__bindgen_vtable, IServerTools_FirstEntity) == SLOT_SIZE * 7
			);
			assert!(
				offset_of!(IServerTools__bindgen_vtable, IServerTools_NextEntity) == SLOT_SIZE * 8
			);
			assert!(
				offset_of!(IServerTools__bindgen_vtable, IServerTools_GetKeyValue)
					== SLOT_SIZE * 10
			);
			assert!(
				offset_of!(
					IServerTools__bindgen_vtable,
					IServerTools_FindEntityProcedural
				) == SLOT_SIZE * 51
			);
			assert!(size_of::<IServerTools__bindgen_vtable>() == SLOT_SIZE * 52);

			// MSVC emits each new overload set in reverse declaration order, so
			// `SetKeyValue(const char *, const Vector &)` precedes the `const
			// char *` overload. SourceMod's `DispatchKeyValue*` gamedata shows
			// the same reversal for `CBaseEntity::KeyValue`.
			assert!(
				offset_of!(IServerTools__bindgen_vtable, IServerTools_SetKeyValue2)
					== SLOT_SIZE * 11
			);
			assert!(
				offset_of!(IServerTools__bindgen_vtable, IServerTools_SetKeyValue)
					== SLOT_SIZE * 13
			);

			//no virtual destructor
			assert!(
				offset_of!(
					IServerGameDLL__bindgen_vtable,
					IServerGameDLL_GetAllServerClasses
				) == SLOT_SIZE * 11
			);
			assert!(size_of::<IServerGameDLL__bindgen_vtable>() == SLOT_SIZE * 48);

			// Per-client channels, as `GetPlayerNetInfo` returns them. These match the
			// slots of `CNetChan` in TF2's engine.dll, where `SendData` is at 38.
			assert!(
				offset_of!(INetChannel__bindgen_vtable, INetChannel_destructor) == SLOT_SIZE * 26
			);
			assert!(
				offset_of!(INetChannel__bindgen_vtable, INetChannel_SendNetMsg) == SLOT_SIZE * 37
			);
			assert!(
				offset_of!(INetChannel__bindgen_vtable, INetChannel_SendData) == SLOT_SIZE * 38
			);
			assert!(
				offset_of!(INetChannel__bindgen_vtable, INetChannel_GetSequenceData)
					== SLOT_SIZE * 50
			);
			assert!(
				offset_of!(INetChannel__bindgen_vtable, INetChannel_IsOverflowed) == SLOT_SIZE * 54
			);
			assert!(
				offset_of!(
					INetChannel__bindgen_vtable,
					INetChannel_HasPendingReliableData
				) == SLOT_SIZE * 56
			);
			assert!(
				offset_of!(INetChannel__bindgen_vtable, INetChannel_GetNumBitsWritten)
					== SLOT_SIZE * 62
			);
			assert!(size_of::<INetChannel__bindgen_vtable>() == SLOT_SIZE * 68);

			assert!(offset_of!(INetMessage__bindgen_vtable, INetMessage_destructor) == 0);
			assert!(offset_of!(INetMessage__bindgen_vtable, INetMessage_Process) == SLOT_SIZE * 3);
			assert!(
				offset_of!(INetMessage__bindgen_vtable, INetMessage_WriteToBuffer) == SLOT_SIZE * 5
			);
			assert!(offset_of!(INetMessage__bindgen_vtable, INetMessage_GetType) == SLOT_SIZE * 7);
			assert!(offset_of!(INetMessage__bindgen_vtable, INetMessage_GetName) == SLOT_SIZE * 9);
			assert!(
				offset_of!(INetMessage__bindgen_vtable, INetMessage_ToString) == SLOT_SIZE * 11
			);
			assert!(offset_of!(INetMessage__bindgen_vtable, INetMessage_GetSize) == SLOT_SIZE * 13);
			assert!(size_of::<INetMessage__bindgen_vtable>() == SLOT_SIZE * 14);

			// `REPLAY_ENABLED` adds `ProcessSaveReplay`, as in TF2's engine, whose
			// `CGameClient` handler vtable has 15 slots under MSVC.
			assert!(
				offset_of!(
					IClientMessageHandler__bindgen_vtable,
					IClientMessageHandler_destructor
				) == 0
			);
			assert!(
				offset_of!(
					IClientMessageHandler__bindgen_vtable,
					IClientMessageHandler_ProcessTick
				) == SLOT_SIZE
			);
			assert!(
				offset_of!(
					IClientMessageHandler__bindgen_vtable,
					IClientMessageHandler_ProcessVoiceData
				) == SLOT_SIZE * 7
			);
			assert!(
				offset_of!(
					IClientMessageHandler__bindgen_vtable,
					IClientMessageHandler_ProcessSaveReplay
				) == SLOT_SIZE * 13
			);
			assert!(
				offset_of!(
					IClientMessageHandler__bindgen_vtable,
					IClientMessageHandler_ProcessCmdKeyValues
				) == SLOT_SIZE * 14
			);
			assert!(size_of::<IClientMessageHandler__bindgen_vtable>() == SLOT_SIZE * 15);

			assert!(offset_of!(IServer__bindgen_vtable, IServer_destructor) == 0);
			assert!(offset_of!(IServer__bindgen_vtable, IServer_GetClient) == SLOT_SIZE * 6);
			assert!(size_of::<IServer__bindgen_vtable>() == SLOT_SIZE * 36);

			assert!(offset_of!(IClient__bindgen_vtable, IClient_destructor) == 0);
			assert!(offset_of!(IClient__bindgen_vtable, IClient_GetPlayerSlot) == SLOT_SIZE * 14);
			assert!(offset_of!(IClient__bindgen_vtable, IClient_GetNetChannel) == SLOT_SIZE * 18);
			assert!(offset_of!(IClient__bindgen_vtable, IClient_IsReplay) == SLOT_SIZE * 36);
			assert!(size_of::<IClient__bindgen_vtable>() == SLOT_SIZE * 40);

			assert!(
				offset_of!(
					IRecipientFilter__bindgen_vtable,
					IRecipientFilter_destructor
				) == 0
			);
			assert!(
				offset_of!(
					IRecipientFilter__bindgen_vtable,
					IRecipientFilter_IsReliable
				) == SLOT_SIZE
			);
			assert!(
				offset_of!(
					IRecipientFilter__bindgen_vtable,
					IRecipientFilter_GetRecipientIndex
				) == SLOT_SIZE * 4
			);
			assert!(size_of::<IRecipientFilter__bindgen_vtable>() == SLOT_SIZE * 5);

			//no virtual destructor, so the slots are the same under both ABIs
			assert!(
				offset_of!(
					IVEngineServer__bindgen_vtable,
					IVEngineServer_GetPlayerNetInfo
				) == SLOT_SIZE * 20
			);
			assert!(
				offset_of!(
					IVEngineServer__bindgen_vtable,
					IVEngineServer_EntityMessageBegin
				) == SLOT_SIZE * 42
			);
			assert!(
				offset_of!(
					IVEngineServer__bindgen_vtable,
					IVEngineServer_UserMessageBegin
				) == SLOT_SIZE * 43
			);
			assert!(
				offset_of!(IVEngineServer__bindgen_vtable, IVEngineServer_MessageEnd)
					== SLOT_SIZE * 44
			);
			assert!(
				offset_of!(IVEngineServer__bindgen_vtable, IVEngineServer_SetView)
					== SLOT_SIZE * 48
			);
			assert!(
				offset_of!(
					IVEngineServer__bindgen_vtable,
					IVEngineServer_CrosshairAngle
				) == SLOT_SIZE * 50
			);
			assert!(
				offset_of!(
					IVEngineServer__bindgen_vtable,
					IVEngineServer_GetClientConVarValue
				) == SLOT_SIZE * 55
			);
			assert!(
				offset_of!(IVEngineServer__bindgen_vtable, IVEngineServer_GetIServer)
					== SLOT_SIZE * 119
			);
		};
	}

	_ => {
		compile_error!("Unsupported target");
	}
}
