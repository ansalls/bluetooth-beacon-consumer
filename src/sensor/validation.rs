//! Device validation module - identifies Govee devices by name

/// Check if the device is a Govee sensor based on its local name
pub fn is_govee_device(local_name: &Option<String>) -> bool {
    local_name
        .as_deref()
        .map(|n| n.starts_with("GVH") || n.starts_with("Govee"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_govee_device_identifies_gvh_prefix() {
        assert!(is_govee_device(&Some("GVH5075".to_string())));
    }

    #[test]
    fn is_govee_device_identifies_govee_prefix() {
        assert!(is_govee_device(&Some("Govee H5102".to_string())));
    }

    #[test]
    fn is_govee_device_rejects_other_brands() {
        assert!(!is_govee_device(&Some("OtherBrand".to_string())));
    }

    #[test]
    fn is_govee_device_rejects_none() {
        assert!(!is_govee_device(&None));
    }
}
