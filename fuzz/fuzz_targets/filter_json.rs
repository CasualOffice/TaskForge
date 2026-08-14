#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() > 16 * 1024 {
        return;
    }
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(data) {
        if let Ok(node) = casual_task_search::from_json(&value)
            && casual_task_search::validate(&node).is_ok()
        {
            let encoded = casual_task_search::to_json(&node);
            let _ = casual_task_search::from_json(&encoded);
        }
    }
});
