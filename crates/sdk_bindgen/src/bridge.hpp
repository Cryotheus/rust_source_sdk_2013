//we're making server-side plugins
#define GAME_DLL
#define IS_SERVER_DLL

//suppress engine stamp checks that cause signature mismatch panics
#define COPY_CHECK_STAMP

//required by igameevents.h
#include "public/tier0/platform.h"
#include "public/tier1/interface.h"

//clang's __m128 is always a vector type, never MSVC's union, so the element
//accessors ssemath.h spells `a.m128_f32[i]` without POSIX cannot compile when
//clang's own <xmmintrin.h> is found under the MSVC ABI. Their POSIX spelling
//is equivalent and nothing else in ssemath.h depends on POSIX, so parse just
//that header with it, before anything else includes it.
#if defined(COMPILER_MSVC) && !defined(POSIX)
#include "public/mathlib/vector.h"
#include "public/mathlib/mathlib.h"
#define POSIX
#include "public/mathlib/ssemath.h"
#undef POSIX
#endif

//bitbuf confuses clang
//so stop it from loading, and make stubs to replace the problematic types
#define BITBUF_H

class bf_write {};
class bf_read {};

//finally include what we wanted in the first place
#include "public/igameevents.h"
#include "public/eiface.h"
#include "public/toolframework/itoolentity.h"
#include "public/dt_send.h"

//other interfaces a server plugin can request from the engine and game factories
#include "public/icvar.h"
#include "public/ivoiceserver.h"
#include "public/networkstringtabledefs.h"
#include "public/engine/IEngineSound.h"
#include "public/engine/IEngineTrace.h"
#include "public/engine/iserverplugin.h"
#include "public/engine/ivmodelinfo.h"
#include "public/game/server/iplayerinfo.h"

//for gameinterface.h
#include "game/shared/predictioncopy.h"
#include "game/shared/ehandle.h"
#include "game/shared/baseplayer_shared.h"
#include "game/server/networkstringtable_gamedll.h"
#include "game/shared/mapentities_shared.h"

#include "game/server/gameinterface.h"

//TF2's engine is built with replay support, which adds virtual methods to the
//client message handler and to IClient. Without it, their later slots would be
//off by one. Nothing included above depends on it.
#define REPLAY_ENABLED

//per-client networking: net channels and their messages, recipient filters, and
//the engine's server and client objects
#include "public/inetchannel.h"
#include "public/inetmessage.h"
#include "public/inetmsghandler.h"
#include "public/irecipientfilter.h"
#include "public/iserver.h"
#include "public/iclient.h"

