//! T3 Craft for Horizon.
//!
//! All of the mod's logic lives in the standalone `t3craft_engine_core` crate
//! under `src/engine`; the mixin helpers under `src/mixins/` call it through
//! the re-exports below. The client mixins compile into copies of the game
//! functions they patch.

pub use engine_core::*;

use mod_api::{Hooks, KeyRegistration, Registrar, RegistrationContext};

#[derive(Default)]
struct Mod;

#[mod_api::horizon_mod]
impl Hooks for Mod {
    fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
        registrar.register_key_mapping(
            KeyRegistration::new(KEY_OPEN, KEY_OPEN_DEFAULT).label(KEY_OPEN_LABEL),
        );
        registrar.register_key_mapping(
            KeyRegistration::new(KEY_HUD, KEY_HUD_DEFAULT).label(KEY_HUD_LABEL),
        );
    }
}
