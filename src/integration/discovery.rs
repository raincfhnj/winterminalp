use crate::TerminalChannel;

use super::{IntegrationConfig, TargetMode, TerminalSettingsTarget};

#[must_use]
pub fn discover_targets(config: &IntegrationConfig) -> Vec<TerminalSettingsTarget> {
    match &config.target_mode {
        TargetMode::Explicit(targets) => targets.clone(),
        TargetMode::Auto => auto_candidates(&config.local_app_data)
            .into_iter()
            .filter(|target| target.settings_path.is_file())
            .collect(),
    }
}

fn auto_candidates(local_app_data: &std::path::Path) -> [TerminalSettingsTarget; 4] {
    let packages = local_app_data.join("Packages");
    [
        TerminalSettingsTarget {
            channel: TerminalChannel::Stable,
            settings_path: packages
                .join("Microsoft.WindowsTerminal_8wekyb3d8bbwe")
                .join("LocalState")
                .join("settings.json"),
        },
        TerminalSettingsTarget {
            channel: TerminalChannel::Preview,
            settings_path: packages
                .join("Microsoft.WindowsTerminalPreview_8wekyb3d8bbwe")
                .join("LocalState")
                .join("settings.json"),
        },
        TerminalSettingsTarget {
            channel: TerminalChannel::Canary,
            settings_path: packages
                .join("Microsoft.WindowsTerminalCanary_8wekyb3d8bbwe")
                .join("LocalState")
                .join("settings.json"),
        },
        TerminalSettingsTarget {
            channel: TerminalChannel::Unpackaged,
            settings_path: local_app_data
                .join("Microsoft")
                .join("Windows Terminal")
                .join("settings.json"),
        },
    ]
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn auto_discovers_only_initialized_channels() {
        let temp = tempfile::tempdir().expect("temporary directory should be created");
        let stable = temp
            .path()
            .join("Packages")
            .join("Microsoft.WindowsTerminal_8wekyb3d8bbwe")
            .join("LocalState")
            .join("settings.json");
        fs::create_dir_all(stable.parent().expect("fixture should have a parent"))
            .expect("fixture directory should be created");
        fs::write(&stable, b"{}\n").expect("fixture should be written");

        let config = IntegrationConfig::new(temp.path(), temp.path().join("state"));
        let targets = discover_targets(&config);

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].channel, TerminalChannel::Stable);
        assert_eq!(targets[0].settings_path, stable);
    }
}
