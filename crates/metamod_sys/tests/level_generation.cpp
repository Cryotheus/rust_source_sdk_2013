// Includes the production listener and shell; no duplicate counter model.
#include "../src/cpp/bridge.cpp"
#include <cassert>

namespace {
unsigned init_calls = 0, shutdown_calls = 0;
void init(void *, const char *) { ++init_calls; }
void shutdown(void *) { ++shutdown_calls; }
bool load(void *, void *, char *, std::size_t, bool) { return true; }
void all_loaded(void *) {}
bool accept(void *, char *, std::size_t) { return true; }

void registered_listener() {
	// Registration itself needs a live Metamod API and is verified on-server.
	// Supply its owned result here to exercise production notification/getter code.
	level_route = {init, shutdown, nullptr, false};
	level_listener_added = true;
}

std::uint64_t generation() {
	std::uint64_t value = UINT64_MAX;
	assert(RUST_SHELL_LEVEL_GENERATION(&value));
	return value;
}
}

int main() {
	const RustPluginCallbacks callbacks = {load, all_loaded, accept, accept, accept, accept};
	RUST_SHELL_CREATE(&callbacks, {});
	std::uint64_t value = 42;
	assert(!RUST_SHELL_LEVEL_GENERATION(&value) && value == 42);
	assert(plugin.Load(1, nullptr, nullptr, 0, false));
	assert(!RUST_SHELL_LEVEL_GENERATION(&value));
	registered_listener();
	assert(generation() == 0);
	assert(!RUST_SHELL_LEVEL_GENERATION(nullptr));
	level_listener.OnLevelInit("cp_dustbowl", nullptr, nullptr, nullptr, false, false);
	assert(generation() == 1 && init_calls == 1);
	assert(plugin.Pause(nullptr, 0));
	assert(generation() == 1); // An ordinary pause alone is not a new map.
	level_listener.OnLevelShutdown();
	level_listener.OnLevelInit("cp_dustbowl", nullptr, nullptr, nullptr, false, false);
	assert(generation() == 3 && init_calls == 1 && shutdown_calls == 0);
	level_listener.OnLevelShutdown();
	level_listener.OnLevelInit("ctf_2fort", nullptr, nullptr, nullptr, false, false);
	assert(generation() == 5 && init_calls == 1 && shutdown_calls == 0);
	assert(plugin.Unpause(nullptr, 0));
	assert(generation() == 5);
	level_listener.OnLevelShutdown();
	assert(generation() == 6 && shutdown_calls == 1);
	level_route.generation = UINT64_MAX;
	assert(generation() == UINT64_MAX);
	level_listener.OnLevelShutdown();
	value = 42;
	assert(!RUST_SHELL_LEVEL_GENERATION(&value) && value == 42);
	assert(plugin.Unload(nullptr, 0));
	assert(!RUST_SHELL_LEVEL_GENERATION(&value));
	assert(plugin.Load(2, nullptr, nullptr, 0, false));
	registered_listener();
	assert(generation() == 0 && !level_route.exhausted && !level_route.paused);
	assert(plugin.Unload(nullptr, 0));
}
