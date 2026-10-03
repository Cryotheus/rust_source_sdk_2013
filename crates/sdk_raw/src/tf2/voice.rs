//! Hand-written ABI of TF2's voice lines and scenes.
//!
//! TF2 speaks a voice line by playing a choreographed scene through the scene
//! entities of `game/server/sceneentity.cpp`, whose classes the generated
//! bindings do not lay out.

/// The size, terminator included, of the buffer a scene's path is copied into
/// when it is played, past which the game silently truncates it.
///
/// This is `CChoreoScene::MAX_SCENE_FILENAME` from `game/shared/choreoscene.h`,
/// which sizes `CInstancedSceneEntity::m_szInstanceFilename` in
/// `game/server/sceneentity.cpp`. Neither class has a generated layout to
/// check it against.
#[doc(alias = "m_szInstanceFilename")]
pub const MAX_SCENE_FILENAME: usize = 128;
