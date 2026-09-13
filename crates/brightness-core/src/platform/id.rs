/// Converts `\\.\DISPLAY1` to a shell-friendly slug such as `display1`.
pub fn display_slug(device_name: &str) -> String {
    device_name
        .trim_start_matches('\\')
        .trim_start_matches('.')
        .trim_start_matches('\\')
        .to_ascii_lowercase()
}

pub fn normalize_monitor_id(id: &str) -> String {
    id.replace('\\', "/").to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::{display_slug, normalize_monitor_id};

    #[test]
    fn slug_from_device_name() {
        assert_eq!(display_slug(r"\\.\DISPLAY1"), "display1");
    }

    #[test]
    fn normalize_legacy_id() {
        let legacy = r"ddc:\\.\DISPLAY1#Generic PnP Monitor";
        assert_eq!(
            normalize_monitor_id(legacy),
            "ddc://./display1#generic pnp monitor"
        );
    }
}
