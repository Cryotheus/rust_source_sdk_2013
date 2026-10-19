#include <cstddef>
#include <cstdint>
#include <string_view>
#include <type_traits>

#include <ISmmPlugin.h>

#if defined(METAMOD_BRIDGE_STABLE) == defined(METAMOD_BRIDGE_DEV)
	#error "Build each Metamod shell with exactly one channel selected"
#elif defined(METAMOD_BRIDGE_STABLE)
	#if METAMOD_PLAPI_VERSION != 16
		#error "The stable Metamod shell requires the 1.12 build 1226 plugin headers"
	#endif

	static_assert(offsetof(MetamodVersionInfo, pl_min) == 16);
	static_assert(offsetof(MetamodVersionInfo, source_engine) == 24);

	#define RUST_SHELL_INSTANCE cpp_metamod_plugin_stable_instance
	#define RUST_SHELL_LISTEN_LEVELS cpp_metamod_listen_levels_stable
#else
	#if METAMOD_PLAPI_VERSION != 18
		#error "The dev Metamod shell requires the plugin API 18 headers of 2.0, as in builds 1469 through 1472"
	#endif

	static_assert(offsetof(MetamodVersionInfo, pl_min) == 8);
	static_assert(offsetof(MetamodVersionInfo, source_engine) == 16);

	#define RUST_SHELL_INSTANCE cpp_metamod_plugin_dev_instance
	#define RUST_SHELL_LISTEN_LEVELS cpp_metamod_listen_levels_dev
#endif

// Just for now.
// Hopefully I'll remember to add 32bit support.
static_assert(sizeof(void *) == 8);
static_assert(sizeof(SourceMM::ISmmAPI) == sizeof(void *));
static_assert(std::is_abstract_v<SourceMM::ISmmAPI>);
static_assert(!std::has_virtual_destructor_v<SourceMM::ISmmAPI>);
static_assert(std::has_virtual_destructor_v<SourceMM::ISmmPlugin>);
static_assert(std::is_same_v<SourceMM::PluginId, int>);

// Keep the handwritten Rust vtable declarations tied to the exact signatures
// in the configured Metamod headers. Slot order is asserted on the Rust side.
#define ASSERT_METHOD(type, name, return_type, ...) \
	static_assert(std::is_same_v<decltype(&type::name), return_type (type::*)(__VA_ARGS__)>)

#define ASSERT_SMM_METHOD(name, return_type, ...) ASSERT_METHOD(SourceMM::ISmmAPI, name, return_type, __VA_ARGS__)

ASSERT_SMM_METHOD(LogMsg, void, SourceMM::ISmmPlugin *, const char *, ...);
ASSERT_SMM_METHOD(GetEngineFactory, CreateInterfaceFn, bool);
ASSERT_SMM_METHOD(GetPhysicsFactory, CreateInterfaceFn, bool);
ASSERT_SMM_METHOD(GetFileSystemFactory, CreateInterfaceFn, bool);
ASSERT_SMM_METHOD(GetServerFactory, CreateInterfaceFn, bool);
ASSERT_SMM_METHOD(GetCGlobals, CGlobalVars *);
ASSERT_SMM_METHOD(RegisterConCommandBase, bool, SourceMM::ISmmPlugin *, ConCommandBase *);
ASSERT_SMM_METHOD(UnregisterConCommandBase, void, SourceMM::ISmmPlugin *, ConCommandBase *);
ASSERT_SMM_METHOD(ConPrint, void, const char *);
ASSERT_SMM_METHOD(ConPrintf, void, const char *, ...);
ASSERT_SMM_METHOD(GetApiVersions, void, int &, int &, int &, int &);

#if defined(METAMOD_BRIDGE_STABLE)
	ASSERT_SMM_METHOD(GetShVersions, void, int &, int &);
#endif

ASSERT_SMM_METHOD(AddListener, void, SourceMM::ISmmPlugin *, SourceMM::IMetamodListener *);
ASSERT_SMM_METHOD(MetaFactory, void *, const char *, int *, SourceMM::PluginId *);
ASSERT_SMM_METHOD(FormatIface, int, char *, std::size_t);
ASSERT_SMM_METHOD(InterfaceSearch, void *, CreateInterfaceFn, const char *, int, int *);
ASSERT_SMM_METHOD(GetBaseDir, const char *);
ASSERT_SMM_METHOD(PathFormat, std::size_t, char *, std::size_t, const char *, ...);

#if defined(METAMOD_BRIDGE_STABLE)
	ASSERT_SMM_METHOD(ClientConPrintf, void, edict_t *, const char *, ...);
#else
	ASSERT_SMM_METHOD(ClientConPrintf, void, MMSPlayer_t, const char *, ...);
#endif

ASSERT_SMM_METHOD(VInterfaceMatch, void *, CreateInterfaceFn, const char *, int);
ASSERT_SMM_METHOD(EnableVSPListener, void);
ASSERT_SMM_METHOD(GetGameDLLVersion, int);
ASSERT_SMM_METHOD(GetUserMessageCount, int);
ASSERT_SMM_METHOD(FindUserMessage, int, const char *, int *);
ASSERT_SMM_METHOD(GetUserMessage, const char *, int, int *);
ASSERT_SMM_METHOD(GetVSPVersion, int);
ASSERT_SMM_METHOD(GetSourceEngineBuild, int);
ASSERT_SMM_METHOD(GetVSPInfo, IServerPluginCallbacks *, int *);
ASSERT_SMM_METHOD(Format, std::size_t, char *, std::size_t, const char *, ...);
ASSERT_SMM_METHOD(FormatArgs, std::size_t, char *, std::size_t, const char *, va_list);

#if defined(METAMOD_BRIDGE_DEV)
	ASSERT_SMM_METHOD(RegisterConCommand, bool, SourceMM::ISmmPlugin *, ProviderConCommand *);
	ASSERT_SMM_METHOD(RegisterConVar, bool, SourceMM::ISmmPlugin *, ProviderConVar *);
	ASSERT_SMM_METHOD(UnregisterConCommand, void, SourceMM::ISmmPlugin *, ProviderConCommand *);
	ASSERT_SMM_METHOD(UnregisterConVar, void, SourceMM::ISmmPlugin *, ProviderConVar *);
	ASSERT_SMM_METHOD(GetDetourInterface, void *, SourceMM::PluginId);
#endif

#undef ASSERT_SMM_METHOD

// The hooking libraries' interfaces, which Rust calls and implements through
// the handwritten declarations of `metamod_source_sys::sourcehook` and
// `metamod_source_sys::khook`.
#if defined(METAMOD_BRIDGE_STABLE)
	static_assert(std::string_view(MMIFACE_SOURCEHOOK) == "ISourceHook");
	static_assert(SH_IFACE_VERSION == 5);
	static_assert(SH_IMPL_VERSION == 5);
	static_assert(SH_HOOKMAN_VERSION == 1);

	static_assert(sizeof(META_RES) == sizeof(int));
	static_assert(MRES_IGNORED == 0 && MRES_HANDLED == 1 && MRES_OVERRIDE == 2 && MRES_SUPERCEDE == 3);
	static_assert(sizeof(SourceHook::ISourceHook::AddHookMode) == sizeof(int));
	static_assert(SourceHook::ISourceHook::Hook_Normal == 0);
	static_assert(SourceHook::ISourceHook::Hook_VP == 1);
	static_assert(SourceHook::ISourceHook::Hook_DVP == 2);
	static_assert(std::is_same_v<SourceHook::Plugin, int>);
	static_assert(std::is_same_v<SourceHook::HookManagerPubFunc, int (*)(bool, SourceHook::IHookManagerInfo *)>);

	static_assert(sizeof(SourceHook::PassInfo) == 16);
	static_assert(offsetof(SourceHook::PassInfo, size) == 0);
	static_assert(offsetof(SourceHook::PassInfo, type) == 8);
	static_assert(offsetof(SourceHook::PassInfo, flags) == 12);
	static_assert(sizeof(SourceHook::PassInfo::V2Info) == 32);
	static_assert(sizeof(SourceHook::ProtoInfo) == 80);
	static_assert(offsetof(SourceHook::ProtoInfo, numOfParams) == 0);
	static_assert(offsetof(SourceHook::ProtoInfo, retPassInfo) == 8);
	static_assert(offsetof(SourceHook::ProtoInfo, paramsPassInfo) == 24);
	static_assert(offsetof(SourceHook::ProtoInfo, convention) == 32);
	static_assert(offsetof(SourceHook::ProtoInfo, retPassInfo2) == 40);
	static_assert(offsetof(SourceHook::ProtoInfo, paramsPassInfo2) == 72);
	static_assert(SourceHook::PassInfo::PassFlag_ByVal == 1);
	static_assert(SourceHook::ProtoInfo::CallConv_ThisCall == 1);

	static_assert(!std::has_virtual_destructor_v<SourceHook::ISourceHook>);
	static_assert(!std::has_virtual_destructor_v<SourceHook::IHookContext>);
	static_assert(!std::has_virtual_destructor_v<SourceHook::ISHDelegate>);
	static_assert(!std::has_virtual_destructor_v<SourceHook::IHookManagerInfo>);

	#define ASSERT_SH_METHOD(name, return_type, ...) ASSERT_METHOD(SourceHook::ISourceHook, name, return_type, __VA_ARGS__)

	ASSERT_SH_METHOD(GetIfaceVersion, int);
	ASSERT_SH_METHOD(GetImplVersion, int);
	ASSERT_SH_METHOD(AddHook, int, SourceHook::Plugin, SourceHook::ISourceHook::AddHookMode, void *, int,
		SourceHook::HookManagerPubFunc, SourceHook::ISHDelegate *, bool);
	ASSERT_SH_METHOD(RemoveHook, bool, SourceHook::Plugin, void *, int, SourceHook::HookManagerPubFunc,
		SourceHook::ISHDelegate *, bool);
	ASSERT_SH_METHOD(RemoveHookByID, bool, int);
	ASSERT_SH_METHOD(PauseHookByID, bool, int);
	ASSERT_SH_METHOD(UnpauseHookByID, bool, int);
	ASSERT_SH_METHOD(SetRes, void, META_RES);
	ASSERT_SH_METHOD(GetPrevRes, META_RES);
	ASSERT_SH_METHOD(GetStatus, META_RES);
	ASSERT_SH_METHOD(GetOrigRet, const void *);
	ASSERT_SH_METHOD(GetOverrideRet, const void *);
	ASSERT_SH_METHOD(GetIfacePtr, void *);
	ASSERT_SH_METHOD(GetOverrideRetPtr, void *);
	ASSERT_SH_METHOD(RemoveHookManager, void, SourceHook::Plugin, SourceHook::HookManagerPubFunc);
	ASSERT_SH_METHOD(SetIgnoreHooks, void, void *);
	ASSERT_SH_METHOD(ResetIgnoreHooks, void, void *);
	ASSERT_SH_METHOD(GetOrigVfnPtrEntry, void *, void *);
	ASSERT_SH_METHOD(DoRecall, void);
	ASSERT_SH_METHOD(SetupHookLoop, SourceHook::IHookContext *, SourceHook::IHookManagerInfo *, void *, void *, void **,
		META_RES *, META_RES *, META_RES *, const void *, void *);
	ASSERT_SH_METHOD(EndContext, void, SourceHook::IHookContext *);
	ASSERT_SH_METHOD(LogDebug, void, const char *, ...);

	#undef ASSERT_SH_METHOD

	ASSERT_METHOD(SourceHook::IHookContext, GetNext, SourceHook::ISHDelegate *);
	ASSERT_METHOD(SourceHook::IHookContext, GetOverrideRetPtr, void *);
	ASSERT_METHOD(SourceHook::IHookContext, GetOrigRetPtr, const void *);
	ASSERT_METHOD(SourceHook::IHookContext, ShouldCallOrig, bool);
	ASSERT_METHOD(SourceHook::ISHDelegate, IsEqual, bool, SourceHook::ISHDelegate *);
	ASSERT_METHOD(SourceHook::ISHDelegate, DeleteThis, void);
	ASSERT_METHOD(SourceHook::IHookManagerInfo, SetInfo, void, int, int, int, SourceHook::ProtoInfo *, void *);
#else
	static_assert(std::is_same_v<KHook::HookID_t, std::uint32_t>);
	static_assert(KHook::INVALID_HOOK == UINT32_MAX);
	static_assert(sizeof(KHook::Action) == 1);
	static_assert(static_cast<int>(KHook::Action::Ignore) == 0);
	static_assert(static_cast<int>(KHook::Action::Override) == 1);
	static_assert(static_cast<int>(KHook::Action::Supersede) == 2);
	static_assert(!std::has_virtual_destructor_v<KHook::IKHook>);

	#define ASSERT_KHOOK_METHOD(name, return_type, ...) ASSERT_METHOD(KHook::IKHook, name, return_type, __VA_ARGS__)

	ASSERT_KHOOK_METHOD(SetupHook, KHook::HookID_t, void *, void *, void *, void *, void *, void *, void *, unsigned int, bool);
	ASSERT_KHOOK_METHOD(SetupVirtualHook, KHook::HookID_t, void **, int, void *, void *, void *, void *, void *, void *,
		unsigned int, bool);
	ASSERT_KHOOK_METHOD(RemoveHook, void, KHook::HookID_t, bool, void (*)(KHook::HookID_t, void *), void *);
	ASSERT_KHOOK_METHOD(GetContextPtr, void *);
	ASSERT_KHOOK_METHOD(GetOriginalFunction, void *);
	ASSERT_KHOOK_METHOD(GetOriginalValuePtr, void *);
	ASSERT_KHOOK_METHOD(GetOverrideValuePtr, void *);
	ASSERT_KHOOK_METHOD(GetCurrentValuePtr, void *, bool);
	ASSERT_KHOOK_METHOD(DestroyReturnValue, void);
	ASSERT_KHOOK_METHOD(FindOriginal, void *, void *);
	ASSERT_KHOOK_METHOD(FindOriginalVirtual, void *, void **, int);
	ASSERT_KHOOK_METHOD(DoRecall, void *, KHook::Action, void *, std::size_t, void *, void *);
	ASSERT_KHOOK_METHOD(SaveReturnValue, void, KHook::Action, void *, std::size_t, void *, void *, bool);
	ASSERT_KHOOK_METHOD(LookupSignature, void *, void *, std::size_t, const char *);
	ASSERT_KHOOK_METHOD(WasOriginalFunctionSkipped, bool);

	#undef ASSERT_KHOOK_METHOD
#endif

#undef ASSERT_METHOD

// Private dispatch addresses bind state to this shared library without ELF
// symbol interposition; C++ retains compiler-defined virtual objects only.
struct RustShellDispatch {
 bool (*load)(int, void *, int, void *, char *, std::size_t, bool);
 void (*all_loaded)(int, void *);
 bool (*query)(int, void *, char *, std::size_t);
 bool (*unload)(int, void *, char *, std::size_t);
 bool (*pause)(int, void *, char *, std::size_t, bool);
 const char *(*metadata)(int, int);
 void (*level)(int, const char *, bool);
};
static_assert(sizeof(RustShellDispatch) == 7 * sizeof(void *));
static_assert(offsetof(RustShellDispatch, level) == 6 * sizeof(void *));

namespace {
	const RustShellDispatch *dispatch;
	// Compiler-owned vtables/destructors preserve both Itanium and MSVC ABI.
	class RustLevelListener final : public SourceMM::IMetamodListener {
	public:
		void OnLevelInit(char const *map, char const *, char const *, char const *, bool, bool) override {
			dispatch->level(METAMOD_PLAPI_VERSION, map, true);
		}
		void OnLevelShutdown() override {
			dispatch->level(METAMOD_PLAPI_VERSION, nullptr, false);
		}
	};

	class RustMetamodPlugin final : public SourceMM::ISmmPlugin {
	public:
		int GetApiVersion() override { return METAMOD_PLAPI_VERSION; }
		bool Load(SourceMM::PluginId id, SourceMM::ISmmAPI *api, char *error, std::size_t length, bool late) override {
			return dispatch->load(METAMOD_PLAPI_VERSION, this, id, api, error, length, late);
		}
		void AllPluginsLoaded() override { dispatch->all_loaded(METAMOD_PLAPI_VERSION, this); }
		bool QueryRunning(char *error, std::size_t length) override { return dispatch->query(METAMOD_PLAPI_VERSION, this, error, length); }
		bool Unload(char *error, std::size_t length) override { return dispatch->unload(METAMOD_PLAPI_VERSION, this, error, length); }
		bool Pause(char *error, std::size_t length) override { return dispatch->pause(METAMOD_PLAPI_VERSION, this, error, length, true); }
		bool Unpause(char *error, std::size_t length) override { return dispatch->pause(METAMOD_PLAPI_VERSION, this, error, length, false); }
		const char *GetAuthor() override { return dispatch->metadata(METAMOD_PLAPI_VERSION, 0); }
		const char *GetName() override { return dispatch->metadata(METAMOD_PLAPI_VERSION, 1); }
		const char *GetDescription() override { return dispatch->metadata(METAMOD_PLAPI_VERSION, 2); }
		const char *GetURL() override { return dispatch->metadata(METAMOD_PLAPI_VERSION, 3); }
		const char *GetLicense() override { return dispatch->metadata(METAMOD_PLAPI_VERSION, 4); }
		const char *GetVersion() override { return dispatch->metadata(METAMOD_PLAPI_VERSION, 5); }
		const char *GetDate() override { return dispatch->metadata(METAMOD_PLAPI_VERSION, 6); }
		const char *GetLogTag() override { return dispatch->metadata(METAMOD_PLAPI_VERSION, 7); }
	};

	RustMetamodPlugin plugin;
	RustLevelListener level_listener;
}

extern "C" void *RUST_SHELL_INSTANCE(const RustShellDispatch *table) {
	dispatch = table;
	return &plugin;
}

// Rust validates registration and records its route before this native call;
// no Rust state guard is held if AddListener synchronously calls the listener.
extern "C" void RUST_SHELL_LISTEN_LEVELS(void *api) noexcept {
	static_cast<SourceMM::ISmmAPI *>(api)->AddListener(&plugin, &level_listener);
}
