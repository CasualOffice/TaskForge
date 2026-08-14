#![no_main]

use casual_task_plugin_contract::{Contribution, ExtensionPoint, PluginId, Provider};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() > 4 * 1024 {
        return;
    }
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let mut parts = text.splitn(3, '\0');
    let id = parts.next().unwrap_or_default();
    let slug = parts.next().unwrap_or_default();
    let title = parts.next().unwrap_or_default();
    if let Ok(id) = PluginId::parse(id) {
        let _ = Contribution::new(
            ExtensionPoint::UiTaskPanel,
            Provider::Plugin(id),
            slug,
            title,
        );
    }
});
