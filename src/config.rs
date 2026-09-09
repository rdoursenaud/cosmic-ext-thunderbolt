// SPDX-FileCopyrightText: 2026 Raphaël Doursenaud <raphael@doursenaud.fr>
// SPDX-License-Identifier: GPL-3.0-only
pub const APP_ID: &str = "fr.doursenaud.raphael.cosmic-ext-thunderbolt";

use cosmic::cosmic_config::{self, CosmicConfigEntry, cosmic_config_derive::CosmicConfigEntry};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, CosmicConfigEntry, Default)]
#[version = 1]
pub struct ThunderboltAppletConfig {
    pub show_host_device: bool,
}
